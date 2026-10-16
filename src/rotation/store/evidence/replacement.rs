//! Atomic current-chain adoption. Original financial proofs remain immutable audit.
use super::*;
use crate::rotation::base::{
    AuthorizationOutcome, BaseRpc, EvidenceCapture, PendingAuthorization,
    replacement::{ReplacementBalance, ReplacementPage, ReplacementPass},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

const PAGE: usize = super::super::history::PAGE_SIZE;
const ROWS: &str = "SELECT a.id,json_object('id',a.id,'pool',a.pool_id,'wallet',a.wallet_id,'payer',a.payer,'payee',a.payee,'nonce',a.nonce,'valid_after',a.valid_after,'valid_before',a.valid_before,'generation',a.generation,'requirements_hash',a.requirements_hash,'amount',a.amount,'disputed',a.resolution_disputed,'original_present',r.attempt_id IS NOT NULL,'effective_present',e.attempt_id IS NOT NULL,'original',json_object('outcome',r.outcome,'height',r.height,'hash',r.hash,'block_time',r.block_time,'support',r.support),'effective',json_object('outcome',e.outcome,'height',e.height,'hash',e.hash,'block_time',e.block_time,'support',e.support)) FROM payment_attempts a LEFT JOIN payment_resolutions r ON r.attempt_id=a.id LEFT JOIN effective_payment_resolutions e ON e.attempt_id=a.id WHERE a.wallet_id=?1 AND a.state='RESOLVED' AND a.id>?2 ORDER BY a.id LIMIT ?3";

#[derive(Deserialize)]
struct Row {
    id: String,
    pool: String,
    wallet: String,
    payer: String,
    payee: String,
    nonce: String,
    valid_after: u64,
    valid_before: u64,
    amount: String,
    disputed: u8,
    original_present: u8,
    effective_present: u8,
}
#[derive(Clone)]
struct Snapshot {
    id: String,
    pool: String,
    wallet: String,
    address: String,
    capture: EvidenceCapture,
    debit: Option<Anchor>,
}
fn digest_row(digest: &mut Sha256, raw: &str) {
    digest.update((raw.len() as u64).to_be_bytes());
    digest.update(raw.as_bytes());
}
fn rows(db: &Connection, wallet: &str, after: &str) -> Result<Vec<(String, String)>> {
    Ok(db
        .prepare_cached(ROWS)?
        .query_map(params![wallet, after, PAGE as i64], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?)
}
impl Row {
    fn authorization(&self, snapshot: &Snapshot) -> Result<PendingAuthorization> {
        ensure!(
            self.pool == snapshot.pool
                && self.wallet == snapshot.wallet
                && self.payer.parse::<alloy_primitives::Address>()?
                    == snapshot.address.parse::<alloy_primitives::Address>()?
                && self.disputed == 1
                && self.valid_after < self.valid_before
                && amount(&self.amount)? > U256::ZERO,
            EvidenceFailure::MalformedSupport
        );
        self.payee.parse::<alloy_primitives::Address>()?;
        self.nonce.parse::<alloy_primitives::B256>()?;
        ensure!(
            self.original_present == 1 && self.effective_present == 1,
            EvidenceFailure::MalformedSupport
        );
        Ok(PendingAuthorization {
            id: self.id.clone(),
            wallet: self.wallet.clone(),
            payer: self.payer.clone(),
            nonce: self.nonce.clone(),
            valid_before: self.valid_before,
        })
    }
}

impl Store {
    fn replacement_snapshot(
        &self,
        pool: &str,
        wallet: &str,
        id: String,
    ) -> Result<Option<Snapshot>> {
        let (address, repair): (String, Option<String>) = self.db.query_row(
            "SELECT address,evidence_repair FROM wallets WHERE id=?1 AND pool_id=?2",
            params![wallet, pool],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if repair.as_deref() != Some("chain_repair_required") {
            return Ok(None);
        }
        let undisputed: bool = self
            .db
            .query_row(UNDISPUTED_SQL, [wallet], |row| row.get(0))?;
        ensure!(!undisputed, EvidenceFailure::ChainRepairRequired);
        let mut capture = self.capture_evidence(wallet)?;
        capture.previous = None;
        capture.previous_confirmed_covered = false;
        Ok(Some(Snapshot {
            id,
            pool: pool.into(),
            wallet: wallet.into(),
            address,
            capture,
            debit: debit_checkpoint(&self.db, wallet)?,
        }))
    }
    fn begin_replacement_staging(
        &mut self,
        pool: &str,
        wallet: &str,
        id: String,
    ) -> Result<Option<Snapshot>> {
        let Some(snapshot) = self.replacement_snapshot(pool, wallet, id)? else {
            return Ok(None);
        };
        // Validate total liability once at acquisition and again at adoption;
        // page boundary CAS must not rescan the entire wallet for every page.
        check_reserved_releases(&self.db, wallet)?;
        self.db.execute_batch("CREATE TEMP TABLE IF NOT EXISTS resolution_replacement_passes(id TEXT PRIMARY KEY,wallet TEXT UNIQUE NOT NULL); CREATE TEMP TABLE IF NOT EXISTS resolution_replacement_stage(pass TEXT NOT NULL,attempt TEXT NOT NULL,original TEXT NOT NULL,outcome TEXT NOT NULL,height INTEGER NOT NULL,hash TEXT NOT NULL,block_time INTEGER NOT NULL,support TEXT NOT NULL,endpoint TEXT NOT NULL,PRIMARY KEY(pass,attempt));")?;
        let inserted = self.db.execute(
            "INSERT OR IGNORE INTO resolution_replacement_passes VALUES(?1,?2)",
            params![snapshot.id, snapshot.wallet],
        )?;
        Ok((inserted == 1).then_some(snapshot))
    }
    fn clear_replacement_staging(&mut self, id: &str) -> Result<()> {
        // The guard may run after cancellation before the first accepted store job.
        let exists: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name='resolution_replacement_passes')",[],|row|row.get(0))?;
        if exists {
            self.db.execute(
                "DELETE FROM resolution_replacement_stage WHERE pass=?1",
                [id],
            )?;
            self.db.execute(
                "DELETE FROM resolution_replacement_passes WHERE id=?1",
                [id],
            )?;
        }
        Ok(())
    }
    fn validate_replacement_snapshot(&self, snapshot: &Snapshot) -> Result<()> {
        let current = self
            .replacement_snapshot(&snapshot.pool, &snapshot.wallet, snapshot.id.clone())?
            .context(EvidenceFailure::Invalidated)?;
        ensure!(
            current.capture == snapshot.capture
                && current.debit == snapshot.debit
                && current.address == snapshot.address,
            EvidenceFailure::Invalidated
        );
        let owner: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM resolution_replacement_passes WHERE id=?1 AND wallet=?2)",
            params![snapshot.id, snapshot.wallet],
            |row| row.get(0),
        )?;
        ensure!(owner, EvidenceFailure::Invalidated);
        Ok(())
    }
    fn stage_replacement_page(
        &mut self,
        snapshot: &Snapshot,
        pass: &ReplacementPass,
        originals: Vec<(String, String)>,
        page: ReplacementPage,
    ) -> Result<()> {
        self.validate_replacement_snapshot(snapshot)?;
        ensure!(
            page.pass() == pass.id()
                && page.unresolved().is_empty()
                && page.resolutions().len() == originals.len()
                && originals.len() <= PAGE,
            EvidenceFailure::Invalidated
        );
        let tx = self.db.transaction()?;
        for ((id, raw), certificate) in originals.iter().zip(page.resolutions()) {
            let row: Row =
                serde_json::from_str(raw).map_err(|_| EvidenceFailure::MalformedSupport)?;
            let auth = row.authorization(snapshot)?;
            ensure!(
                id == &auth.id
                    && certificate.authorization() == &auth
                    && certificate.anchor() == pass.anchor()
                    && certificate.block_time() == pass.block_time()
                    && certificate.endpoint() == pass.endpoint(),
                EvidenceFailure::ScopeMismatch
            );
            check_capture(
                &tx,
                self.evidence_namespace,
                certificate.capture(),
                certificate.endpoint(),
                certificate.endpoint_generation(),
            )?;
            let outcome = match certificate.outcome() {
                AuthorizationOutcome::Used => "USED",
                AuthorizationOutcome::ExpiredUnused => {
                    ensure!(
                        certificate.block_time() > auth.valid_before,
                        EvidenceFailure::MalformedSupport
                    );
                    "EXPIRED_UNUSED"
                }
            };
            let support = serde_json::to_string(&ResolutionSupport {
                wallet: &auth.wallet,
                payer: &auth.payer,
                nonce: &auth.nonce,
                endpoint: certificate.endpoint(),
                network: "eip155:8453",
                asset: crate::payment::USDC,
                anchor: certificate.anchor(),
                outcome: certificate.outcome(),
                block_time: certificate.block_time(),
                checkpoint: Some(pass.anchor().clone()),
            })?;
            tx.execute(
                "INSERT INTO resolution_replacement_stage VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    snapshot.id,
                    id,
                    raw,
                    outcome,
                    i64::try_from(certificate.anchor().height)?,
                    certificate.anchor().hash,
                    i64::try_from(certificate.block_time())?,
                    support,
                    certificate.endpoint()
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    fn commit_replacement(
        &mut self,
        snapshot: Snapshot,
        pass: Arc<ReplacementPass>,
        fresh: ReplacementBalance,
        digest: String,
        expected: usize,
    ) -> Result<()> {
        self.validate_replacement_snapshot(&snapshot)?;
        ensure!(
            fresh.pass() == pass.id() && pass.capture() == &snapshot.capture,
            EvidenceFailure::Invalidated
        );
        let certificate = fresh
            .view()
            .balance_certificates
            .get(&snapshot.wallet)
            .context(EvidenceFailure::MalformedSupport)?;
        let observation = certificate.observation();
        check_balance_integrity(&self.db, self.evidence_namespace, certificate)?;
        self.check_journal_support_or_invalidate(&snapshot.wallet, observation)?;
        ensure!(
            certificate.is_fresh(crate::rotation::base::now()?)?
                && observation.endpoint == pass.endpoint()
                && observation.confirmed.anchor.height >= pass.anchor().height
                && snapshot
                    .debit
                    .as_ref()
                    .is_none_or(|anchor| observation.confirmed.anchor.height >= anchor.height),
            EvidenceFailure::Invalidated
        );
        let tx = self.db.transaction()?;
        let mut actual_digest = Sha256::new();
        let mut after = String::new();
        let mut count = 0;
        loop {
            let page = rows(&tx, &snapshot.wallet, &after)?;
            if page.is_empty() {
                break;
            }
            for (id, raw) in page {
                let staged: Option<String> = tx.query_row("SELECT original FROM resolution_replacement_stage WHERE pass=?1 AND attempt=?2",params![snapshot.id,id],|row|row.get(0)).optional()?;
                ensure!(
                    staged.as_deref() == Some(raw.as_str()),
                    EvidenceFailure::Invalidated
                );
                digest_row(&mut actual_digest, &raw);
                after = id;
                count += 1;
            }
        }
        let staged_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM resolution_replacement_stage WHERE pass=?1",
            [&snapshot.id],
            |row| row.get(0),
        )?;
        ensure!(count > 0, EvidenceFailure::MalformedSupport);
        ensure!(
            count == expected
                && i64::try_from(count)? == staged_count
                && format!("{:x}", actual_digest.finalize()) == digest,
            EvidenceFailure::Invalidated
        );
        // New pending admissions are allowed, but their exact liabilities must
        // remain well formed and reserved in this same adoption transaction.
        exposure(&tx, &snapshot.wallet)?;
        // The complete-set CAS can make progress for longer than this balance
        // remains fresh. It never renews the original admission authority.
        let verified_at = crate::rotation::base::now()?;
        ensure!(certificate.is_fresh(verified_at)?, EvidenceFailure::Expired);
        let support = serde_json::to_string(&BalanceSupport {
            observation: observation.clone(),
            checkpoint: Some(observation.confirmed.anchor.clone()),
        })?;
        tx.execute("INSERT INTO payment_resolution_repairs(id,wallet_id,verified_at,original_financial_height,original_financial_hash,original_debit_height,original_debit_hash,evidence_digest,resolution_count,confirmed_height,confirmed_hash,latest_height,latest_hash,endpoint,balance_support) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",params![snapshot.id,snapshot.wallet,i64::try_from(verified_at)?,snapshot.capture.checkpoint.as_ref().map(|a|i64::try_from(a.height)).transpose()?,snapshot.capture.checkpoint.as_ref().map(|a|&a.hash),snapshot.debit.as_ref().map(|a|i64::try_from(a.height)).transpose()?,snapshot.debit.as_ref().map(|a|&a.hash),digest,i64::try_from(count)?,i64::try_from(observation.confirmed.anchor.height)?,observation.confirmed.anchor.hash,i64::try_from(observation.latest.anchor.height)?,observation.latest.anchor.hash,observation.endpoint,support])?;
        tx.execute("INSERT INTO payment_resolution_replacements(repair_id,attempt_id,outcome,height,hash,block_time,support,endpoint) SELECT pass,attempt,outcome,height,hash,block_time,support,endpoint FROM resolution_replacement_stage WHERE pass=?1 ORDER BY attempt",[&snapshot.id])?;
        tx.execute("UPDATE payment_attempts SET resolution_disputed=0 WHERE wallet_id=?1 AND state='RESOLVED' AND resolution_disputed=1",[&snapshot.wallet])?;
        let has_debit = snapshot.debit.is_some() || tx.query_row("SELECT EXISTS(SELECT 1 FROM resolution_replacement_stage WHERE pass=?1 AND outcome='USED')", [&snapshot.id], |row| row.get::<_, bool>(0))?;
        tx.execute("UPDATE wallets SET evidence_repair=NULL,financial_height=?2,financial_hash=?3,debit_height=CASE WHEN ?4 THEN ?2 ELSE NULL END,debit_hash=CASE WHEN ?4 THEN ?3 ELSE NULL END WHERE id=?1 AND evidence_repair='chain_repair_required'",params![snapshot.wallet,i64::try_from(observation.confirmed.anchor.height)?,observation.confirmed.anchor.hash,has_debit])?;
        check_balance(&tx, self.evidence_namespace, certificate)?;
        exposure(&tx, &snapshot.wallet)?;
        bump_generation(&tx, "wallet", &snapshot.wallet)?;
        tx.execute(
            "DELETE FROM resolution_replacement_stage WHERE pass=?1",
            [&snapshot.id],
        )?;
        tx.execute(
            "DELETE FROM resolution_replacement_passes WHERE id=?1",
            [&snapshot.id],
        )?;
        tx.commit()?;
        self.evidence_cache.remove(&snapshot.wallet);
        self.notify_capacity_changed(&snapshot.pool);
        Ok(())
    }
}

/// Created before the first accepted store job, including cancellation while
/// waiting for its response. Cleanup runs through the same serialized owner.
struct StagingOwner {
    store: StoreHandle,
    id: String,
}
impl Drop for StagingOwner {
    fn drop(&mut self) {
        let store = self.store.clone();
        let id = self.id.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if store
                    .call(move |s| s.clear_replacement_staging(&id))
                    .await
                    .is_err()
                {
                    tracing::debug!(
                        category = "replacement_staging_cleanup",
                        "Temporary replacement staging owner closed"
                    );
                }
            });
        }
    }
}

pub(super) async fn replace_wallet(
    store: StoreHandle,
    base: BaseRpc,
    pool: String,
    wallet: String,
) -> Result<bool> {
    let owner = StagingOwner {
        store: store.clone(),
        id: Uuid::new_v4().to_string(),
    };
    let id = owner.id.clone();
    let snapshot = store
        .call(move |s| s.begin_replacement_staging(&pool, &wallet, id))
        .await?;
    let Some(snapshot) = snapshot else {
        return Ok(false);
    };
    let pass = Arc::new(
        base.begin_replacement(
            snapshot.capture.clone(),
            snapshot.address.clone(),
            snapshot.debit.as_ref().map(|anchor| anchor.height),
        )
        .await?,
    );
    let mut after = String::new();
    let mut digest = Sha256::new();
    let mut count = 0;
    loop {
        let captured = snapshot.clone();
        let cursor = after.clone();
        let originals = store
            .call(move |s| {
                s.validate_replacement_snapshot(&captured)?;
                rows(&s.db, &captured.wallet, &cursor)
            })
            .await?;
        if originals.is_empty() {
            break;
        }
        let authorizations = originals
            .iter()
            .map(|(_, raw)| {
                serde_json::from_str::<Row>(raw)
                    .map_err(|_| anyhow::Error::from(EvidenceFailure::MalformedSupport))?
                    .authorization(&snapshot)
            })
            .collect::<Result<Vec<_>>>()?;
        let page = base.replacement_page(&pass, authorizations).await?;
        if !page.unresolved().is_empty() {
            return Ok(false);
        };
        for (id, raw) in &originals {
            after = id.clone();
            digest_row(&mut digest, raw);
            count += 1;
        }
        let captured = snapshot.clone();
        let scoped = pass.clone();
        store
            .call(move |s| s.stage_replacement_page(&captured, &scoped, originals, page))
            .await?;
    }
    if count == 0 {
        ensure!(
            snapshot.capture.checkpoint.is_none() && snapshot.debit.is_none(),
            EvidenceFailure::MalformedSupport
        );
        return Ok(false);
    }
    let fresh = base.finish_replacement(&pass).await?;
    let digest = format!("{:x}", digest.finalize());
    store
        .call(move |s| s.commit_replacement(snapshot, pass, fresh, digest, count))
        .await?;
    Ok(true)
}

#[cfg(test)]
mod tests;
