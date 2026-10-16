//! Narrow revalidation of complete, undisputed journals after a wallet-level
//! chain repair block. No signing, allocation or proof rewriting; exact revalidation restores release support.
use super::*;
use crate::rotation::base::{
    AuthorizationOutcome, BaseRpc, ChainQuery, EvidenceCapture, PendingAuthorization,
    historical::{HistoricalBalanceRequest, HistoricalResolutionRequest},
};
use sha2::{Digest, Sha256};

// Keyset pages bound memory, not how much historical work may finish.
const ROWS: &str = "SELECT a.reconciliation_sequence,json_object(
 'authorization',json_object('id',a.id,'wallet',a.wallet_id,'payer',a.payer,'nonce',a.nonce,'valid_before',a.valid_before),
 'amount',a.amount,'state',a.state,'resolution_disputed',(a.state='RESOLVED' AND (SELECT payment_resolutions_disputed FROM wallets WHERE id=a.wallet_id)),'payee',a.payee,'valid_after',a.valid_after,'requirements_hash',a.requirements_hash,'pool',a.pool_id,'generation',a.generation,
 'admission',json_object('height',e.block_height,'hash',e.block_hash,'balance',e.balance,'valid_until',e.valid_until,'support',e.support,'latest_height',e.latest_height,'latest_hash',e.latest_hash,'endpoint',e.endpoint,'reserved_before',e.reserved_before,'payer_role',e.payer_role,'reserve_slot',e.reserve_slot,'claim_generation',e.claim_generation),
 'resolution',CASE WHEN r.attempt_id IS NULL THEN NULL ELSE json_object('outcome',r.outcome,'height',r.height,'hash',r.hash,'block_time',r.block_time,'support',r.support) END)
 FROM payment_attempts a LEFT JOIN payment_admission_evidence e ON e.attempt_id=a.id
 LEFT JOIN payment_resolutions r ON r.attempt_id=a.id
 WHERE a.wallet_id=?1 AND a.reconciliation_sequence>?2 ORDER BY a.reconciliation_sequence LIMIT ?3";

#[derive(Deserialize)]
struct Admission {
    height: u64,
    hash: String,
    balance: String,
    valid_until: String,
    support: String,
    latest_height: u64,
    latest_hash: String,
    endpoint: String,
}
#[derive(Deserialize)]
struct Resolution {
    outcome: String,
    height: u64,
    hash: String,
    block_time: u64,
    support: String,
}
#[derive(Deserialize)]
struct Attempt {
    pool: String,
    authorization: PendingAuthorization,
    amount: String,
    state: String,
    resolution_disputed: u8,
    admission: Admission,
    resolution: Option<Resolution>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolutionSupport {
    wallet: String,
    payer: String,
    nonce: String,
    endpoint: String,
    network: String,
    asset: String,
    anchor: Anchor,
    outcome: String,
    block_time: u64,
    checkpoint: Option<Anchor>,
}
#[derive(Clone)]
struct Snapshot {
    wallet: String,
    pool: String,
    address: String,
    capture: EvidenceCapture,
    digest: String,
    header: String,
    readiness: BalanceSupport,
    resolutions: usize,
    attempts: usize,
}
struct Verified {
    snapshot: Snapshot,
    fresh: crate::rotation::base::ChainView,
    balances: usize,
}

fn decode<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T> {
    serde_json::from_str(raw).map_err(|_| EvidenceFailure::MalformedSupport.into())
}
fn rows(db: &Connection, wallet: &str, after: i64) -> Result<Vec<(i64, String)>> {
    Ok(db
        .prepare_cached(ROWS)?
        .query_map(
            params![wallet, after, super::super::history::PAGE_SIZE as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
        .collect::<rusqlite::Result<_>>()?)
}
fn hash_row(hash: &mut Sha256, raw: &str) {
    hash.update((raw.len() as u64).to_be_bytes());
    hash.update(raw.as_bytes());
}
fn balance_request(
    snapshot: &Snapshot,
    support: BalanceSupport,
) -> Result<HistoricalBalanceRequest> {
    support.observation.validate()?;
    ensure!(
        support.observation.wallet == snapshot.wallet
            && support.observation.address == snapshot.address,
        EvidenceFailure::ScopeMismatch
    );
    Ok(HistoricalBalanceRequest {
        capture: snapshot.capture.clone(),
        observation: support.observation,
        checkpoint: support.checkpoint,
    })
}
impl Attempt {
    fn admission(&self) -> Result<BalanceSupport> {
        let saved: BalanceSupport = decode(&self.admission.support)?;
        let o = &saved.observation;
        o.validate()?;
        ensure!(
            o.wallet == self.authorization.wallet
                && o.address == self.authorization.payer
                && o.confirmed.anchor.height == self.admission.height
                && o.confirmed.anchor.hash == self.admission.hash
                && o.latest.anchor.height == self.admission.latest_height
                && o.latest.anchor.hash == self.admission.latest_hash
                && o.endpoint == self.admission.endpoint
                && o.available() == amount(&self.admission.balance)?
                && o.valid_until.to_string() == self.admission.valid_until,
            EvidenceFailure::MalformedSupport
        );
        Ok(saved)
    }
    fn resolution(&self, capture: &EvidenceCapture) -> Result<Option<HistoricalResolutionRequest>> {
        match (self.state.as_str(), &self.resolution) {
            ("POSSIBLY_SUBMITTED", None) => Ok(None),
            ("RESOLVED", Some(r)) => {
                let saved: ResolutionSupport = decode(&r.support)?;
                ensure!(
                    saved.wallet == self.authorization.wallet
                        && saved.payer == self.authorization.payer
                        && saved.nonce == self.authorization.nonce
                        && saved.network == "eip155:8453"
                        && saved.asset.eq_ignore_ascii_case(crate::payment::USDC)
                        && saved.anchor.height == r.height
                        && saved.anchor.hash == r.hash
                        && saved.outcome == r.outcome
                        && saved.block_time == r.block_time,
                    EvidenceFailure::MalformedSupport
                );
                let outcome = match r.outcome.as_str() {
                    "USED" => AuthorizationOutcome::Used,
                    "EXPIRED_UNUSED" if r.block_time > self.authorization.valid_before => {
                        AuthorizationOutcome::ExpiredUnused
                    }
                    _ => anyhow::bail!(EvidenceFailure::MalformedSupport),
                };
                Ok(Some(HistoricalResolutionRequest {
                    capture: capture.clone(),
                    authorization: self.authorization.clone(),
                    endpoint: saved.endpoint,
                    anchor: saved.anchor,
                    outcome,
                    block_time: saved.block_time,
                    checkpoint: saved.checkpoint,
                }))
            }
            _ => anyhow::bail!(EvidenceFailure::MalformedSupport),
        }
    }
}

// Derive journal watermarks without replacing a conflicting same-height proof.
fn advance_original_anchor(saved: &mut Option<Anchor>, next: &Anchor) -> Result<()> {
    if let Some(previous) = saved.as_ref() {
        ensure!(
            previous.height != next.height || previous.hash == next.hash,
            EvidenceFailure::ResolutionConflict
        );
    }
    if saved
        .as_ref()
        .is_none_or(|previous| previous.height < next.height)
    {
        *saved = Some(next.clone());
    }
    Ok(())
}

impl Store {
    fn repair_snapshot(&self, pool: &str, wallet: &str) -> Result<Option<Snapshot>> {
        let (role,repair,address,debit_height,debit_hash): (String,Option<String>,String,Option<i64>,Option<String>) = self.db.query_row(
            "SELECT role,evidence_repair,address,debit_height,debit_hash FROM wallets WHERE id=?1 AND pool_id=?2",
            params![wallet,pool], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        if repair.as_deref() != Some("chain_repair_required")
            || !matches!(role.as_str(), "ACTIVE" | "RETAINED")
        {
            return Ok(None);
        }
        let readiness: Option<String> = self.db.query_row(
            "SELECT readiness_support FROM funding_jobs WHERE wallet_id=?1 AND state='COMPLETE' AND readiness_disputed IS NULL AND readiness_requirement IS NOT NULL AND EXISTS(SELECT 1 FROM funding_progress j WHERE j.job_id=funding_jobs.id AND j.phase='\"COMPLETE\"' AND j.provider_failed=0 AND j.provider_refunded=0 AND j.provider_incomplete=0) AND NOT EXISTS(SELECT 1 FROM wallet_retirements WHERE wallet_id=?1)",
            [wallet], |r| r.get(0)).optional()?.flatten();
        let Some(readiness) = readiness else {
            return Ok(None);
        };
        let mut capture = self.capture_evidence(wallet)?;
        capture.previous = None;
        capture.previous_confirmed_covered = false;
        let mut snapshot = Snapshot {
            wallet: wallet.into(),
            pool: pool.into(),
            address,
            capture,
            digest: String::new(),
            header: String::new(),
            readiness: decode(&readiness)?,
            resolutions: 0,
            attempts: 0,
        };
        balance_request(&snapshot, snapshot.readiness.clone())?;
        ensure!(
            super::super::funding::readiness_supported(&self.db, wallet)?,
            EvidenceFailure::MalformedSupport
        );
        let mut hash = Sha256::new();
        let header: String = self.db.query_row(
            "SELECT json_array(w.pool_id,w.role,w.address,w.balance,w.block_height,w.block_hash,w.financial_height,w.financial_hash,w.debit_height,w.debit_hash,w.evidence_repair,f.state,f.readiness_operation,f.readiness_requirement,f.readiness_capacity,f.readiness_support,f.readiness_hash,f.readiness_height,f.readiness_latest_hash,f.readiness_endpoint,f.readiness_checkpoint_hash,f.readiness_disputed,f.completion_outcome,j.operation_id,j.phase,j.provider_success,j.provider_failed,j.provider_refunded,j.provider_incomplete,j.accepted_output,j.accepted_recipient) FROM wallets w JOIN funding_jobs f ON f.wallet_id=w.id JOIN funding_progress j ON j.job_id=f.id WHERE w.id=?1", [wallet], |r|r.get(0))?;
        hash_row(&mut hash, &header);
        snapshot.header = header;
        let mut after = 0;
        let mut pending = U256::ZERO;
        let mut latest: Option<Anchor> = None;
        let mut debit: Option<Anchor> = None;
        loop {
            let page = rows(&self.db, wallet, after)?;
            if page.is_empty() {
                break;
            }
            for (sequence, raw) in page {
                ensure!(sequence > after, EvidenceFailure::MalformedSupport);
                after = sequence;
                hash_row(&mut hash, &raw);
                let attempt: Attempt = decode(&raw)?;
                balance_request(&snapshot, attempt.admission()?)?;
                ensure!(
                    attempt.pool == pool
                        && attempt.authorization.wallet == wallet
                        && attempt.authorization.payer == snapshot.address,
                    EvidenceFailure::ScopeMismatch
                );
                let liability = amount(&attempt.amount)?;
                snapshot.attempts += 1;
                ensure!(
                    attempt.resolution_disputed <= 1,
                    EvidenceFailure::MalformedSupport
                );
                if let Some(proof) = attempt.resolution(&snapshot.capture)? {
                    if attempt.resolution_disputed == 1 {
                        pending = pending
                            .checked_add(liability)
                            .context(EvidenceFailure::MalformedSupport)?;
                    }
                    snapshot.resolutions += 1;
                    advance_original_anchor(&mut latest, &proof.anchor)?;
                    if proof.outcome == AuthorizationOutcome::Used {
                        advance_original_anchor(&mut debit, &proof.anchor)?;
                    }
                } else {
                    ensure!(
                        attempt.resolution_disputed == 0,
                        EvidenceFailure::MalformedSupport
                    );
                    pending = pending
                        .checked_add(liability)
                        .context(EvidenceFailure::MalformedSupport)?;
                }
            }
        }
        let total: usize = self
            .db
            .query_row(
                "SELECT COUNT(*) FROM payment_attempts WHERE wallet_id=?1",
                [wallet],
                |r| r.get::<_, i64>(0),
            )?
            .try_into()?;
        ensure!(
            snapshot.resolutions > 0
                && snapshot.attempts == total
                && pending == exposure(&self.db, wallet)?
                && latest == snapshot.capture.checkpoint
                && debit
                    .as_ref()
                    .map(|a| i64::try_from(a.height))
                    .transpose()?
                    == debit_height
                && debit.as_ref().map(|a| &a.hash) == debit_hash.as_ref(),
            EvidenceFailure::MalformedSupport
        );
        snapshot.digest = format!("{:x}", hash.finalize());
        Ok(Some(snapshot))
    }
    fn commit_repair(&mut self, verified: Verified) -> Result<()> {
        let saved = &verified.snapshot;
        let current = self
            .repair_snapshot(&saved.pool, &saved.wallet)?
            .context(EvidenceFailure::Invalidated)?;
        ensure!(
            current.digest == saved.digest && current.capture == saved.capture,
            EvidenceFailure::Invalidated
        );
        let certificate = verified
            .fresh
            .balance_certificates
            .get(&saved.wallet)
            .context(EvidenceFailure::MalformedSupport)?;
        let o = certificate.observation();
        ensure!(
            certificate.capture() == &saved.capture
                && certificate.is_fresh(crate::rotation::base::now()?)?,
            EvidenceFailure::Expired
        );
        let tx = self.db.transaction()?;
        let changed = tx.execute("UPDATE wallets SET evidence_repair=NULL WHERE id=?1 AND evidence_repair='chain_repair_required'",[&saved.wallet])?;
        ensure!(changed == 1, EvidenceFailure::Invalidated);
        tx.execute(
            "UPDATE wallets SET payment_resolutions_disputed=0 WHERE id=?1",
            [&saved.wallet],
        )?;
        check_balance(&tx, self.evidence_namespace, certificate)?;
        tx.execute("INSERT INTO wallet_evidence_repairs(wallet_id,verified_at,evidence_digest,resolution_count,balance_count,confirmed_height,confirmed_hash,latest_height,latest_hash) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![saved.wallet,i64::try_from(crate::rotation::base::now()?)?,saved.digest,i64::try_from(saved.resolutions)?,i64::try_from(verified.balances)?,i64::try_from(o.confirmed.anchor.height)?,o.confirmed.anchor.hash,i64::try_from(o.latest.anchor.height)?,o.latest.anchor.hash])?;
        bump_generation(&tx, "wallet", &saved.wallet)?;
        tx.commit()?;
        self.evidence_cache.remove(&saved.wallet);
        self.notify_capacity_changed(&saved.pool);
        tracing::info!(category="wallet_evidence_revalidated", pool=%saved.pool, resolution_count=saved.resolutions,
            balance_count=verified.balances,"Wallet chain repair block cleared after canonical revalidation; original proofs and liabilities preserved");
        Ok(())
    }
}

// Owned by the maintenance wallet read. Cancellation before commit is read-only;
// once accepted, the serialized store job retains commit ownership.
pub(crate) async fn repair_wallet(
    store: StoreHandle,
    base: BaseRpc,
    pool: String,
    wallet: String,
) -> Result<bool> {
    let candidate = wallet.clone();
    let canonical = store
        .call(move |s| {
            let reason: Option<String> = s.db.query_row(
                "SELECT evidence_repair FROM wallets WHERE id=?1",
                [&candidate],
                |row| row.get(0),
            )?;
            let undisputed: bool =
                s.db.query_row(super::UNDISPUTED_SQL, [&candidate], |row| row.get(0))?;
            Ok(reason.as_deref() == Some("chain_repair_required") && !undisputed)
        })
        .await?;
    if canonical {
        return super::replacement::replace_wallet(store, base, pool, wallet).await;
    }
    let snapshot = store
        .call(move |s| s.repair_snapshot(&pool, &wallet))
        .await?;
    let Some(snapshot) = snapshot else {
        return Ok(false);
    };
    let verified = verify_snapshot(&store, &base, snapshot).await?;
    store.call(move |s| s.commit_repair(verified)).await?;
    Ok(true)
}

async fn verify_snapshot(
    store: &StoreHandle,
    base: &BaseRpc,
    snapshot: Snapshot,
) -> Result<Verified> {
    base.verify_historical_balance(balance_request(&snapshot, snapshot.readiness.clone())?)
        .await?;
    let mut verified_digest = Sha256::new();
    hash_row(&mut verified_digest, &snapshot.header);
    let mut after = 0;
    let mut attempts = 0;
    let mut resolutions = 0;
    loop {
        let wallet = snapshot.wallet.clone();
        let page = store.call(move |s| rows(&s.db, &wallet, after)).await?;
        if page.is_empty() {
            break;
        }
        for (sequence, raw) in page {
            after = sequence;
            hash_row(&mut verified_digest, &raw);
            let attempt: Attempt = decode(&raw)?;
            base.verify_historical_balance(balance_request(&snapshot, attempt.admission()?)?)
                .await?;
            if let Some(request) = attempt.resolution(&snapshot.capture)? {
                base.verify_historical_resolution(request).await?;
                resolutions += 1;
            }
            attempts += 1;
        }
    }
    ensure!(
        format!("{:x}", verified_digest.finalize()) == snapshot.digest
            && attempts == snapshot.attempts
            && resolutions == snapshot.resolutions,
        EvidenceFailure::Invalidated
    );
    // Historical proofs never supply a fresh balance or renew an admission clock.
    let fresh = base
        .view(ChainQuery {
            wallets: vec![(snapshot.wallet.clone(), snapshot.address.clone())],
            pending: vec![],
            anchor: None,
            captures: std::collections::BTreeMap::from([(
                snapshot.wallet.clone(),
                snapshot.capture.clone(),
            )]),
        })
        .await?;
    Ok(Verified {
        snapshot,
        fresh,
        balances: attempts + 1,
    })
}

#[cfg(test)]
mod tests;
