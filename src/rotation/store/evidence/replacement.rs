//! Atomic current-chain adoption. Original financial proofs remain immutable audit.
use super::*;
use crate::rotation::base::{
    AuthorizationOutcome, BaseRpc, EvidenceCapture, PendingAuthorization,
    replacement::{ReplacementBalance, ReplacementPage, ReplacementPass},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

const PAGE: usize = super::super::history::PAGE_SIZE;
const ROWS: &str = "SELECT a.id,json_object('id',a.id,'pool',a.pool_id,'wallet',a.wallet_id,'payer',a.payer,'payee',a.payee,'nonce',a.nonce,'valid_after',a.valid_after,'valid_before',a.valid_before,'generation',a.generation,'requirements_hash',a.requirements_hash,'amount',a.amount,'disputed',(SELECT payment_resolutions_disputed FROM wallets WHERE id=a.wallet_id),'original_present',r.attempt_id IS NOT NULL,'effective_present',e.attempt_id IS NOT NULL,'original',json_object('outcome',r.outcome,'height',r.height,'hash',r.hash,'block_time',r.block_time,'support',r.support),'effective',json_object('outcome',e.outcome,'height',e.height,'hash',e.hash,'block_time',e.block_time,'support',e.support)) FROM payment_attempts a LEFT JOIN payment_resolutions r ON r.attempt_id=a.id LEFT JOIN effective_payment_resolutions e ON e.attempt_id=a.id WHERE a.wallet_id=?1 AND a.state='RESOLVED' AND a.id>?2 ORDER BY a.id LIMIT ?3";

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
    revision: u64,
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

fn insert_inactive_set(db: &Connection, snapshot: &Snapshot, pass: &ReplacementPass) -> Result<()> {
    db.execute("INSERT INTO payment_resolution_repairs(id,wallet_id,activated,original_financial_height,original_financial_hash,original_debit_height,original_debit_hash,evidence_digest,resolution_count,confirmed_height,confirmed_hash,latest_height,latest_hash,endpoint) VALUES(?1,?2,0,?3,?4,?5,?6,'',0,?7,?8,?7,?8,?9)",params![snapshot.id,snapshot.wallet,snapshot.capture.checkpoint.as_ref().map(|a|i64::try_from(a.height)).transpose()?,snapshot.capture.checkpoint.as_ref().map(|a|&a.hash),snapshot.debit.as_ref().map(|a|i64::try_from(a.height)).transpose()?,snapshot.debit.as_ref().map(|a|&a.hash),i64::try_from(pass.anchor().height)?,pass.anchor().hash,pass.endpoint()])?;
    Ok(())
}

impl Store {
    fn replacement_snapshot(
        &self,
        pool: &str,
        wallet: &str,
        id: String,
    ) -> Result<Option<Snapshot>> {
        let (address, repair, disputed, revision): (String, Option<String>, bool, u64) = self.db.query_row(
            "SELECT address,evidence_repair,payment_resolutions_disputed,resolved_revision FROM wallets WHERE id=?1 AND pool_id=?2",
            params![wallet, pool], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, sql_u64(r, 3)?)),
        )?;
        if repair.as_deref() != Some("chain_repair_required") {
            return Ok(None);
        }
        let has_resolved: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM payment_attempts WHERE wallet_id=?1 AND state='RESOLVED')",
            [wallet],
            |r| r.get(0),
        )?;
        ensure!(
            disputed || !has_resolved,
            EvidenceFailure::ChainRepairRequired
        );
        let mut capture = self.capture_evidence(wallet)?;
        capture.previous = None;
        capture.previous_confirmed_covered = false;
        let debit = debit_checkpoint(&self.db, wallet)?;
        ensure!(
            has_resolved || (capture.checkpoint.is_none() && debit.is_none()),
            EvidenceFailure::MalformedSupport
        );
        Ok(Some(Snapshot {
            id,
            pool: pool.into(),
            wallet: wallet.into(),
            address,
            capture,
            debit,
            revision,
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
        // Exact amounts and original proof presence are checked in physical pages.
        // Acquiring the owner must not traverse a wallet's entire liability history.
        self.db.execute_batch("CREATE TEMP TABLE IF NOT EXISTS resolution_replacement_passes(id TEXT PRIMARY KEY,wallet TEXT UNIQUE NOT NULL,after_id TEXT NOT NULL DEFAULT '',count INTEGER NOT NULL DEFAULT 0,total TEXT NOT NULL DEFAULT '0',sealed INTEGER NOT NULL DEFAULT 0,has_debit INTEGER NOT NULL DEFAULT 0);")?;
        let inserted = self.db.execute(
            "INSERT OR IGNORE INTO resolution_replacement_passes(id,wallet) VALUES(?1,?2)",
            params![snapshot.id, snapshot.wallet],
        )?;
        Ok((inserted == 1).then_some(snapshot))
    }
    /// Returns true after one bounded deletion page finishes the inactive set.
    fn clear_replacement_staging(&mut self, id: &str) -> Result<bool> {
        let tx = self.db.transaction()?;
        tx.execute("DELETE FROM payment_resolution_replacements WHERE id IN (SELECT r.id FROM payment_resolution_replacements r JOIN payment_resolution_repairs p ON p.id=r.repair_id WHERE p.id=?1 AND p.activated=0 ORDER BY r.attempt_id LIMIT ?2)",params![id,PAGE as i64])?;
        let remaining: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM payment_resolution_replacements r JOIN payment_resolution_repairs p ON p.id=r.repair_id WHERE p.id=?1 AND p.activated=0)",[id],|r|r.get(0))?;
        if !remaining {
            tx.execute(
                "DELETE FROM payment_resolution_repairs WHERE id=?1 AND activated=0",
                [id],
            )?;
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name='resolution_replacement_passes')",[],|r|r.get(0))?;
            if exists {
                tx.execute(
                    "DELETE FROM resolution_replacement_passes WHERE id=?1",
                    [id],
                )?;
            }
        }
        tx.commit()?;
        Ok(!remaining)
    }
    fn clear_abandoned_replacement_page(&mut self, snapshot: &Snapshot) -> Result<bool> {
        self.validate_replacement_snapshot(snapshot)?;
        let id: Option<String> = self.db.query_row("SELECT id FROM payment_resolution_repairs WHERE wallet_id=?1 AND activated=0 AND id!=?2 ORDER BY sequence DESC LIMIT 1",params![snapshot.wallet,snapshot.id],|r|r.get(0)).optional()?;
        match id {
            Some(id) => {
                self.clear_replacement_staging(&id)?;
                Ok(false)
            }
            None => Ok(true),
        }
    }
    fn validate_replacement_snapshot(&self, snapshot: &Snapshot) -> Result<()> {
        let current = self
            .replacement_snapshot(&snapshot.pool, &snapshot.wallet, snapshot.id.clone())?
            .context(EvidenceFailure::Invalidated)?;
        ensure!(
            current.capture == snapshot.capture
                && current.debit == snapshot.debit
                && current.address == snapshot.address
                && current.revision == snapshot.revision,
            EvidenceFailure::Invalidated
        );
        let owner: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM resolution_replacement_passes WHERE id=?1 AND wallet=?2)",
            params![snapshot.id, snapshot.wallet],
            |r| r.get(0),
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
                && !originals.is_empty()
                && originals.len() <= PAGE,
            EvidenceFailure::Invalidated
        );
        let (after, total, sealed): (String, String, bool) = self.db.query_row(
            "SELECT after_id,total,sealed FROM resolution_replacement_passes WHERE id=?1",
            [&snapshot.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        ensure!(
            !sealed && rows(&self.db, &snapshot.wallet, &after)? == originals,
            EvidenceFailure::Invalidated
        );
        let mut total = amount(&total)?;
        let tx = self.db.transaction()?;
        // Inactive pages are durable but invisible to every authority consumer.
        // Only this per-wallet owner inserts them. Its exact-ID cleanup runs
        // after cancellation/accepted jobs; no second writer can mutate a live
        // set. Consequently sealed page accounting does not need a second
        // durable ledger or an unbounded final reread.
        if after.is_empty() {
            insert_inactive_set(&tx, snapshot, pass)?;
        }
        let mut has_debit = false;
        for ((id, raw), certificate) in originals.iter().zip(page.resolutions()) {
            let row: Row =
                serde_json::from_str(raw).map_err(|_| EvidenceFailure::MalformedSupport)?;
            let auth = row.authorization(snapshot)?;
            total = total
                .checked_add(amount(&row.amount)?)
                .context(EvidenceFailure::MalformedExposure)?;
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
                AuthorizationOutcome::Used => {
                    has_debit = true;
                    "USED"
                }
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
            tx.execute("INSERT INTO payment_resolution_replacements(repair_id,attempt_id,outcome,height,hash,block_time,support,endpoint) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![snapshot.id,id,outcome,i64::try_from(certificate.anchor().height)?,certificate.anchor().hash,i64::try_from(certificate.block_time())?,support,certificate.endpoint()])?;
        }
        tx.execute("UPDATE resolution_replacement_passes SET after_id=?2,count=count+?3,total=?4,has_debit=has_debit OR ?5 WHERE id=?1",params![snapshot.id,originals.last().unwrap().0,originals.len() as i64,total.to_string(),has_debit])?;
        tx.commit()?;
        Ok(())
    }
    fn seal_replacement(
        &mut self,
        snapshot: &Snapshot,
        digest: &str,
        expected: usize,
    ) -> Result<()> {
        self.validate_replacement_snapshot(snapshot)?;
        let (after, count): (String, i64) = self.db.query_row(
            "SELECT after_id,count FROM resolution_replacement_passes WHERE id=?1",
            [&snapshot.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            count > 0 || (snapshot.capture.checkpoint.is_none() && snapshot.debit.is_none()),
            EvidenceFailure::MalformedSupport
        );
        ensure!(
            i64::try_from(expected)? == count
                && rows(&self.db, &snapshot.wallet, &after)?.is_empty(),
            EvidenceFailure::Invalidated
        );
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE resolution_replacement_passes SET sealed=1 WHERE id=?1",
            [&snapshot.id],
        )?;
        tx.execute("UPDATE payment_resolution_repairs SET evidence_digest=?2,resolution_count=?3 WHERE id=?1 AND activated=0",params![snapshot.id,digest,count])?;
        tx.commit()?;
        Ok(())
    }
    fn replacement_liability_start(&self, snapshot: &Snapshot) -> Result<Liabilities> {
        self.validate_replacement_snapshot(snapshot)?;
        let revision = self.db.query_row(
            "SELECT liability_revision FROM wallets WHERE id=?1",
            [&snapshot.wallet],
            |r| sql_u64(r, 0),
        )?;
        let total: String = self.db.query_row(
            "SELECT total FROM resolution_replacement_passes WHERE id=?1 AND sealed=1",
            [&snapshot.id],
            |r| r.get(0),
        )?;
        Ok(Liabilities {
            revision,
            after: String::new(),
            total: amount(&total)?,
            complete: false,
        })
    }
    /// False means concurrent pending work changed; retain the historical pass.
    fn replacement_liability_page(
        &self,
        snapshot: &Snapshot,
        pending: &mut Liabilities,
    ) -> Result<bool> {
        self.validate_replacement_snapshot(snapshot)?;
        if self.liability_revision(&snapshot.wallet)? != pending.revision {
            return Ok(false);
        }
        let mut stmt = self.db.prepare_cached("SELECT id,pool_id,amount FROM payment_attempts WHERE wallet_id=?1 AND state='POSSIBLY_SUBMITTED' AND id>?2 ORDER BY id LIMIT ?3")?;
        let page = stmt
            .query_map(params![snapshot.wallet, pending.after, PAGE as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        pending.complete = page.is_empty();
        for (id, pool, raw) in page {
            let value = amount(&raw).map_err(|_| EvidenceFailure::MalformedExposure)?;
            ensure!(
                pool == snapshot.pool && value > U256::ZERO,
                EvidenceFailure::MalformedExposure
            );
            pending.total = pending
                .total
                .checked_add(value)
                .context(EvidenceFailure::MalformedExposure)?;
            pending.after = id;
        }
        Ok(true)
    }
    fn liability_revision(&self, wallet: &str) -> Result<u64> {
        Ok(self.db.query_row(
            "SELECT liability_revision FROM wallets WHERE id=?1",
            [wallet],
            |r| sql_u64(r, 0),
        )?)
    }
    fn commit_replacement(
        &mut self,
        snapshot: &Snapshot,
        pass: &ReplacementPass,
        fresh: &ReplacementBalance,
        validated: &Validated,
    ) -> Result<bool> {
        let pending = &validated.pending;
        self.validate_replacement_snapshot(snapshot)?;
        ensure!(
            pending.complete && fresh.pass() == pass.id() && pass.capture() == &snapshot.capture,
            EvidenceFailure::Invalidated
        );
        if self.liability_revision(&snapshot.wallet)? != pending.revision {
            return Ok(false);
        }
        let certificate = fresh
            .view()
            .balance_certificates
            .get(&snapshot.wallet)
            .context(EvidenceFailure::MalformedSupport)?;
        let observation = certificate.observation();
        ensure!(
            observation == &validated.observation,
            EvidenceFailure::Invalidated
        );
        check_balance_integrity(&self.db, self.evidence_namespace, certificate)?;
        if !certificate.is_fresh(crate::rotation::base::now()?)? {
            return Ok(false);
        }
        ensure!(
            observation.endpoint == pass.endpoint()
                && observation.confirmed.anchor.height >= pass.anchor().height
                && snapshot
                    .debit
                    .as_ref()
                    .is_none_or(|a| observation.confirmed.anchor.height >= a.height),
            EvidenceFailure::Invalidated
        );
        let (has_debit, has_resolutions): (bool, bool) = self.db.query_row(
            "SELECT has_debit,count>0 FROM resolution_replacement_passes WHERE id=?1 AND sealed=1",
            [&snapshot.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let support = serde_json::to_string(&BalanceSupport {
            observation: observation.clone(),
            checkpoint: has_resolutions.then(|| observation.confirmed.anchor.clone()),
        })?;
        let verified_at = crate::rotation::base::now()?;
        let tx = self.db.transaction()?;
        tx.execute("UPDATE payment_resolution_repairs SET activated=1,verified_at=?2,confirmed_height=?3,confirmed_hash=?4,latest_height=?5,latest_hash=?6,endpoint=?7,balance_support=?8 WHERE id=?1 AND activated=0",params![snapshot.id,i64::try_from(verified_at)?,i64::try_from(observation.confirmed.anchor.height)?,observation.confirmed.anchor.hash,i64::try_from(observation.latest.anchor.height)?,observation.latest.anchor.hash,observation.endpoint,support])?;
        tx.execute("UPDATE wallets SET payment_resolutions_disputed=0,evidence_repair=NULL,financial_height=CASE WHEN ?5 THEN ?2 ELSE NULL END,financial_hash=CASE WHEN ?5 THEN ?3 ELSE NULL END,debit_height=CASE WHEN ?4 THEN ?2 ELSE NULL END,debit_hash=CASE WHEN ?4 THEN ?3 ELSE NULL END WHERE id=?1 AND evidence_repair='chain_repair_required'",params![snapshot.wallet,i64::try_from(observation.confirmed.anchor.height)?,observation.confirmed.anchor.hash,has_debit || snapshot.debit.is_some(),has_resolutions])?;
        check_balance(&tx, self.evidence_namespace, certificate)?;
        bump_generation(&tx, "wallet", &snapshot.wallet)?;
        tx.execute(
            "DELETE FROM resolution_replacement_passes WHERE id=?1",
            [&snapshot.id],
        )?;
        tx.commit()?;
        self.evidence_cache.remove(&snapshot.wallet);
        self.notify_capacity_changed(&snapshot.pool);
        Ok(true)
    }
}
#[derive(Clone)]
struct Liabilities {
    revision: u64,
    after: String,
    total: U256,
    complete: bool,
}

#[derive(Clone, Default)]
struct SupportCursor {
    source: usize,
    after: String,
    sequence: i64,
    complete: bool,
}
struct Validated {
    pending: Liabilities,
    observation: BalanceObservation,
}
impl Store {
    fn replacement_support_page(
        &mut self,
        snapshot: &Snapshot,
        fresh: &ReplacementBalance,
        pending: &Liabilities,
        cursor: &mut SupportCursor,
    ) -> Result<bool> {
        self.validate_replacement_snapshot(snapshot)?;
        if self.liability_revision(&snapshot.wallet)? != pending.revision {
            return Ok(false);
        }
        let certificate = fresh
            .view()
            .balance_certificates
            .get(&snapshot.wallet)
            .context(EvidenceFailure::MalformedSupport)?;
        let observation = certificate.observation();
        let point = if cursor.source < 4 {
            &observation.confirmed
        } else {
            &observation.latest
        };
        let latest = cursor.source >= 4;
        let rows: Vec<(String, String, Option<String>)> = match cursor.source % 4 {
            0 => {
                let column = if latest { "latest_hash" } else { "block_hash" };
                let sql = format!(
                    "SELECT e.attempt_id,a.wallet_id,e.support FROM payment_admission_evidence e JOIN payment_attempts a ON a.id=e.attempt_id WHERE e.{column}=?1 AND e.attempt_id>?2 ORDER BY e.attempt_id LIMIT ?3"
                );
                self.db
                    .prepare_cached(&sql)?
                    .query_map(params![point.anchor.hash, cursor.after, PAGE as i64], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                    })?
                    .collect::<rusqlite::Result<_>>()?
            }
            1 => {
                let column = if latest {
                    "readiness_latest_hash"
                } else {
                    "readiness_hash"
                };
                let sql = format!(
                    "SELECT id,wallet_id,readiness_support FROM funding_jobs WHERE wallet_id=?1 AND {column}=?2 AND id>?3 LIMIT 1"
                );
                self.db
                    .prepare_cached(&sql)?
                    .query_map(
                        params![snapshot.wallet, point.anchor.hash, cursor.after],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )?
                    .collect::<rusqlite::Result<_>>()?
            }
            2 => {
                let column = if latest {
                    "latest_hash"
                } else {
                    "confirmed_hash"
                };
                let sql = format!(
                    "SELECT wallet_id,wallet_id,support FROM wallet_retirements WHERE wallet_id=?1 AND {column}=?2 AND wallet_id>?3 LIMIT 1"
                );
                self.db
                    .prepare_cached(&sql)?
                    .query_map(
                        params![snapshot.wallet, point.anchor.hash, cursor.after],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )?
                    .collect::<rusqlite::Result<_>>()?
            }
            _ => {
                let column = if latest {
                    "latest_hash"
                } else {
                    "confirmed_hash"
                };
                // Include inactive rows physically in the bounded page, but exclude
                // their unpublished support. Otherwise many abandoned sets could
                // hide unbounded work behind the activated predicate.
                let sql = format!(
                    "SELECT sequence,wallet_id,CASE WHEN activated=1 THEN balance_support ELSE NULL END FROM payment_resolution_repairs WHERE {column}=?1 AND wallet_id=?2 AND sequence>?3 ORDER BY sequence LIMIT ?4"
                );
                let values = self
                    .db
                    .prepare_cached(&sql)?
                    .query_map(
                        params![
                            point.anchor.hash,
                            snapshot.wallet,
                            cursor.sequence,
                            PAGE as i64
                        ],
                        |r| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, Option<String>>(2)?,
                            ))
                        },
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                if let Some((sequence, _, _)) = values.last() {
                    cursor.sequence = *sequence;
                }
                values
                    .into_iter()
                    .map(|(sequence, wallet, support)| (sequence.to_string(), wallet, support))
                    .collect()
            }
        };
        for (id, wallet, raw) in &rows {
            cursor.after = id.clone();
            if wallet != &snapshot.wallet {
                continue;
            }
            // Admission before typed support is audit-only; inactive sets likewise.
            if raw.is_none() && matches!(cursor.source % 4, 0 | 3) {
                continue;
            }
            if let Some(malformed) = support_conflict(raw.as_deref(), point, latest)? {
                self.invalidate_wallet_evidence(&snapshot.wallet, &point.anchor, false, true)?;
                anyhow::bail!(if malformed {
                    EvidenceFailure::MalformedSupport
                } else {
                    EvidenceFailure::AmountContradiction
                });
            }
        }
        if rows.is_empty() {
            cursor.source += 1;
            cursor.after.clear();
            cursor.sequence = 0;
            cursor.complete = cursor.source == 8;
        }
        Ok(true)
    }
}
fn support_conflict(
    raw: Option<&str>,
    point: &crate::rotation::base::evidence::BalancePoint,
    latest: bool,
) -> Result<Option<bool>> {
    let Some(raw) = raw else {
        return Ok(Some(true));
    };
    let value: serde_json::Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => return Ok(Some(true)),
    };
    for path in [
        "/observation/wallet",
        "/observation/address",
        "/observation/network",
        "/observation/asset",
        "/observation/endpoint",
        "/observation/confirmed/amount",
        "/observation/confirmed/anchor/hash",
        "/observation/latest/amount",
        "/observation/latest/anchor/hash",
    ] {
        if !value
            .pointer(path)
            .is_some_and(serde_json::Value::is_string)
        {
            return Ok(Some(true));
        }
    }
    for path in [
        "/observation/confirmations",
        "/observation/confirmed/anchor/height",
        "/observation/latest/anchor/height",
        "/observation/valid_until",
    ] {
        if !value.pointer(path).is_some_and(serde_json::Value::is_i64) {
            return Ok(Some(true));
        }
    }
    let checkpoint = value.get("checkpoint");
    if !checkpoint.is_some_and(|v| {
        v.is_null()
            || (v.is_object()
                && v.get("height").is_some_and(serde_json::Value::is_i64)
                && v.get("hash").is_some_and(serde_json::Value::is_string))
    }) {
        return Ok(Some(true));
    }
    let path = if latest {
        "/observation/latest"
    } else {
        "/observation/confirmed"
    };
    let point_value = value.pointer(path).unwrap();
    let encoded = serde_json::to_value(point.amount)?;
    if point_value["amount"] == encoded
        && point_value["anchor"]["height"] == point.anchor.height
        && point_value["anchor"]["hash"] == point.anchor.hash
    {
        return Ok(None);
    }
    let malformed = serde_json::from_str::<BalanceSupport>(raw)
        .ok()
        .is_none_or(|support| support.observation.validate().is_err());
    Ok(Some(malformed))
}

/// Created before the first accepted Store job; cancellation retains cleanup ownership.
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
                if cleanup_staging(store,id).await.is_err() {
                    tracing::error!(category="replacement_cleanup_stopped","Replacement cleanup stopped on a non-transient store failure; wallet repair ownership remains fenced");
                }
            });
        }
    }
}
async fn cleanup_staging(store: StoreHandle, id: String) -> Result<()> {
    loop {
        let key = id.clone();
        match store.call(move |s| s.clear_replacement_staging(&key)).await {
            Ok(true) => return Ok(()),
            Ok(false) => tokio::task::yield_now().await,
            Err(error) if matches!(error.downcast_ref::<rusqlite::Error>(),Some(rusqlite::Error::SqliteFailure(code,_)) if matches!(code.code,rusqlite::ErrorCode::DatabaseBusy|rusqlite::ErrorCode::DatabaseLocked)) =>
            {
                // Deletion is an idempotent transaction; this is not a retry of
                // financial publication, and the per-wallet owner remains held.
                tokio::time::sleep(crate::rotation::manager::MAINTENANCE_CADENCE).await;
            }
            Err(error) => return Err(error),
        }
    }
}
/// Historical pages survive pending-admission races and final balance expiry.
async fn validate_and_activate(
    store: &StoreHandle,
    base: &BaseRpc,
    snapshot: &Snapshot,
    pass: &Arc<ReplacementPass>,
) -> Result<()> {
    let mut fresh = Arc::new(base.finish_replacement(pass).await?);
    'retry: loop {
        if !fresh
            .view()
            .balance_certificates
            .get(&snapshot.wallet)
            .context(EvidenceFailure::MalformedSupport)?
            .is_fresh(crate::rotation::base::now()?)?
        {
            fresh = Arc::new(base.finish_replacement(pass).await?);
        }
        let captured = snapshot.clone();
        let mut pending = store
            .call(move |s| s.replacement_liability_start(&captured))
            .await?;
        while !pending.complete {
            let captured = snapshot.clone();
            let mut current = pending;
            let (valid, next) = store
                .call(move |s| {
                    let valid = s.replacement_liability_page(&captured, &mut current)?;
                    Ok((valid, current))
                })
                .await?;
            pending = next;
            if !valid {
                continue 'retry;
            }
        }
        let mut cursor = SupportCursor::default();
        while !cursor.complete {
            let captured = snapshot.clone();
            let current = pending.clone();
            let balance = fresh.clone();
            let mut next = cursor;
            let (valid, advanced) = store
                .call(move |s| {
                    let valid =
                        s.replacement_support_page(&captured, &balance, &current, &mut next)?;
                    Ok((valid, next))
                })
                .await?;
            cursor = advanced;
            if !valid {
                continue 'retry;
            }
        }
        let validated = Validated {
            pending,
            observation: fresh
                .view()
                .balance_certificates
                .get(&snapshot.wallet)
                .context(EvidenceFailure::MalformedSupport)?
                .observation()
                .clone(),
        };
        let captured = snapshot.clone();
        let scoped = pass.clone();
        let final_fresh = fresh.clone();
        if store
            .call(move |s| s.commit_replacement(&captured, &scoped, &final_fresh, &validated))
            .await?
        {
            return Ok(());
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
    loop {
        let captured = snapshot.clone();
        if store
            .call(move |s| s.clear_abandoned_replacement_page(&captured))
            .await?
        {
            break;
        }
    }
    let pass = Arc::new(
        base.begin_replacement(
            snapshot.capture.clone(),
            snapshot.address.clone(),
            snapshot.debit.as_ref().map(|a| a.height),
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
        }
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
        let captured = snapshot.clone();
        let scoped = pass.clone();
        store
            .call(move |s| {
                s.validate_replacement_snapshot(&captured)?;
                ensure!(
                    rows(&s.db, &captured.wallet, "")?.is_empty(),
                    EvidenceFailure::Invalidated
                );
                insert_inactive_set(&s.db, &captured, &scoped)
            })
            .await?;
    }
    let captured = snapshot.clone();
    let digest = format!("{:x}", digest.finalize());
    store
        .call(move |s| s.seal_replacement(&captured, &digest, count))
        .await?;
    validate_and_activate(&store, &base, &snapshot, &pass).await?;
    Ok(true)
}

#[cfg(test)]
mod tests;
