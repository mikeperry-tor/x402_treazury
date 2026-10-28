//! Lifecycle demand, independent of disposable balance evidence and scalar flags.
use super::*;
use crate::rotation::base::{VerificationStage, evidence::EvidenceFailure};
use tokio::time::Instant;

#[derive(Default)]
pub(super) struct Demand {
    eligible: bool,
    pending: bool,
    due: Option<Instant>,
    failures: u32,
    deferred: Option<&'static str>,
}

pub(crate) struct ObservationWork {
    pub scalar_requested: bool,
    pub retirement_due: Option<Instant>,
}

pub(crate) struct ObservationRow {
    pub sequence: i64,
    pub wallet: Option<String>,
    pub work: Option<ObservationWork>,
}

impl Store {
    pub(crate) fn observation_page(
        &mut self,
        pool: &str,
        after: i64,
    ) -> Result<Vec<ObservationRow>> {
        let rows = self.db.prepare("SELECT sequence,id,refresh_pending=1 OR low_balance=1 FROM wallets WHERE pool_id=?1 AND role!='RETIRED' AND sequence>?2 ORDER BY sequence LIMIT 64")?
            .query_map(params![pool,after], |r| Ok((r.get::<_,i64>(0)?,r.get_ref(1)?.as_str().ok().map(str::to_owned),r.get::<_,bool>(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut work = Vec::with_capacity(rows.len());
        for (sequence, id, scalar) in rows {
            let retirement = if let Some(wallet) = &id {
                self.consider_retirement(pool, wallet)?;
                self.retirement_due(wallet)
            } else {
                None
            };
            let kind = (scalar || retirement.is_some()).then_some(ObservationWork {
                scalar_requested: scalar,
                retirement_due: retirement,
            });
            work.push(ObservationRow {
                sequence,
                wallet: id,
                work: kind,
            });
        }
        Ok(work)
    }

    pub(crate) fn retirement_cached_view(
        &self,
        pool: &str,
        wallet: &str,
    ) -> Result<Option<super::super::base::ChainView>> {
        let Some((_, certificate)) = self.evidence_cache.get(wallet) else {
            return Ok(None);
        };
        let Ok(support) = evidence::check_balance(&self.db, self.evidence_namespace, certificate)
        else {
            return Ok(None);
        };
        let (_, tolerance) = self.db_retirement_status(pool, wallet)?;
        let saved: String =
            self.db
                .query_row("SELECT balance FROM wallets WHERE id=?1", [wallet], |r| {
                    r.get(0)
                })?;
        let observation = support.observation;
        if observation.remainder() > tolerance
            || amount(&saved).ok() != Some(observation.available())
        {
            return Ok(None);
        }
        Ok(Some(super::super::base::ChainView {
            anchor: observation.confirmed.anchor.clone(),
            balances: [(wallet.to_owned(), observation.available())].into(),
            observations: [(wallet.to_owned(), observation)].into(),
            balance_certificates: [(wallet.to_owned(), certificate.clone())].into(),
            resolution_certificates: Default::default(),
            released: vec![],
            resolutions: Default::default(),
        }))
    }
    pub(crate) fn apply_retirement_tolerance(&mut self, name: &str, tolerance: u64) -> Result<()> {
        ensure!(
            tolerance <= i64::MAX as u64,
            "retirement tolerance overflow"
        );
        self.db.execute(
            "UPDATE pools SET retirement_tolerance=?2 WHERE name=?1",
            params![name, tolerance.to_string()],
        )?;
        Ok(())
    }

    pub(crate) fn db_retirement_status(&self, pool: &str, wallet: &str) -> Result<(String, U256)> {
        let (role, tolerance): (String, String) = self.db.query_row("SELECT w.role,p.retirement_tolerance FROM wallets w JOIN pools p ON p.id=w.pool_id WHERE w.id=?1 AND p.id=?2",params![wallet,pool],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let tolerance = amount(&tolerance)
            .context(EvidenceFailure::MalformedSupport)
            .context(VerificationStage("stored retirement tolerance"))?;
        Ok((role, tolerance))
    }
    pub(crate) fn retirement_eligible(&self, pool: &str, wallet: &str) -> Result<bool> {
        if !self.automatic_retirement_allowed() {
            return Ok(false);
        }
        let (enabled, role, balance, tolerance, claim): (bool, String, String, Option<String>, bool) = self.db.query_row(
            "SELECT p.enabled,w.role,w.balance,p.retirement_tolerance,EXISTS(SELECT 1 FROM reserve_claims c WHERE c.wallet_id=w.id AND c.enabled=1) FROM wallets w JOIN pools p ON p.id=w.pool_id WHERE w.id=?1 AND p.id=?2",
            params![wallet,pool], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        if !enabled || role != "RETAINED" || claim {
            return Ok(false);
        }
        let Some(tolerance) = tolerance else {
            return Ok(false);
        };
        let Some(balance) = amount(&balance).ok() else {
            return Ok(false);
        };
        let tolerance = amount(&tolerance)
            .context(EvidenceFailure::MalformedSupport)
            .context(VerificationStage("stored retirement tolerance"))?;
        Ok(balance <= tolerance && !funding::destination_obligation_pending(&self.db, wallet)?)
    }

    pub(crate) fn request_retirement_after_scalar(&mut self, wallet: &str) {
        if let Some(work) = self.retirement_work.get_mut(wallet) {
            work.pending = true;
        }
    }

    /// Metadata discovers eligibility transitions; canonical minimum publication
    /// never requests work when eligibility remains true.
    pub(crate) fn consider_retirement(&mut self, pool: &str, wallet: &str) -> Result<()> {
        let eligible = match self.retirement_eligible(pool, wallet) {
            Ok(eligible) => eligible,
            Err(error) if super::inventory::transient_read(&error) => return Err(error),
            Err(error) => {
                let work = self.retirement_work.entry(wallet.to_owned()).or_default();
                if work.deferred != Some("stored_records_require_operator") {
                    tracing::warn!(
                        pool,
                        wallet,
                        blocker = "stored_records_require_operator",
                        category = %super::super::base::safe_diagnostic(&error),
                        "Wallet retirement needs inspection of its stored records; its obligations remain owned and independent wallets continue"
                    );
                    work.deferred = Some("stored_records_require_operator");
                }
                work.pending = false;
                work.eligible = false;
                return Ok(());
            }
        };
        if eligible || self.retirement_work.contains_key(wallet) {
            let work = self.retirement_work.entry(wallet.to_owned()).or_default();
            work.pending |= eligible && !work.eligible;
            work.eligible = eligible;
            if !eligible {
                work.pending = false;
                // A successfully evaluated ineligible wallet no longer owns a
                // retirement request. Keep its schedule, but discard an obsolete
                // diagnostic so historical repair does not gate unrelated roles.
                work.deferred = None;
            }
        }
        Ok(())
    }

    /// Local paging may revisit metadata; completed true-to-true eligibility
    /// never requests network work, even if the saved minimum remains low.
    #[cfg(test)]
    pub(crate) fn retirement_page(
        &mut self,
        pool: &str,
        after: i64,
        through: i64,
    ) -> Result<Vec<(i64, String)>> {
        let rows = self.db.prepare("SELECT sequence,id FROM wallets WHERE pool_id=?1 AND sequence>?2 AND sequence<=?3 ORDER BY sequence LIMIT 64")?
            .query_map(params![pool,after,through], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (_, wallet) in &rows {
            self.consider_retirement(pool, wallet)?;
        }
        Ok(rows)
    }

    pub(crate) fn retirement_due(&self, wallet: &str) -> Option<Instant> {
        self.retirement_work
            .get(wallet)
            .filter(|work| work.eligible && work.pending)
            .map(|work| work.due.unwrap_or_else(Instant::now))
    }

    pub(crate) fn begin_retirement(&mut self, pool: &str, wallet: &str) -> Result<bool> {
        self.consider_retirement(pool, wallet)?;
        let Some(work) = self.retirement_work.get_mut(wallet) else {
            return Ok(false);
        };
        ensure!(
            work.deferred != Some("stored_records_require_operator") || work.pending,
            EvidenceFailure::MalformedSupport
        );
        if !work.pending || work.due.is_some_and(|due| due > Instant::now()) {
            return Ok(false);
        }
        work.pending = false;
        Ok(true)
    }

    pub(crate) fn retirement_unfinished(&self, pool: &str) -> Result<bool> {
        let mut query = self.db.prepare("SELECT id FROM wallets WHERE pool_id=?1")?;
        for id in query.query_map([pool], |r| r.get::<_, String>(0))? {
            if self.retirement_requested(&id?) {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(crate) fn retirement_requested(&self, wallet: &str) -> bool {
        self.retirement_work
            .get(wallet)
            .is_some_and(|work| work.pending)
    }

    pub(crate) fn finish_retirement(
        &mut self,
        pool: &str,
        wallet: &str,
        retry: bool,
        failed: bool,
        blocker: Option<&'static str>,
        cause: Option<String>,
    ) {
        let observed_remainder = self
            .evidence_cache
            .get(wallet)
            .map(|(_, proof)| proof.observation().remainder().to_string());
        let tolerance = self
            .db_retirement_status(pool, wallet)
            .ok()
            .map(|(_, amount)| amount.to_string());
        // Another canonical publisher may retire this wallet while its owner
        // acquires proofs. Completion never creates demand after its disposal.
        let Some(work) = self.retirement_work.get_mut(wallet) else {
            return;
        };
        work.pending |= retry;
        work.failures = if failed {
            work.failures.saturating_add(1)
        } else {
            0
        };
        let due = Instant::now() + super::observation::retry_delay(work.failures);
        work.due = Some(due);
        if work.deferred != blocker {
            if blocker == Some("stored_records_require_operator") {
                tracing::warn!(
                    pool,
                    wallet,
                    category = ?cause,
                    "Wallet retirement stopped because stored records need inspection; obligations remain owned and independent wallets continue"
                );
            } else if let Some(reason) = blocker {
                tracing::info!(
                    pool,
                    wallet,
                    blocker = reason,
                    category = ?cause,
                    observed_remainder_atomic=?observed_remainder,
                    tolerance_atomic=?tolerance,
                    "Wallet retirement is deferred; funds and payment obligations remain owned"
                );
            } else if work.deferred.is_some() {
                tracing::info!(
                    pool,
                    wallet,
                    "Wallet retirement check completed after a delay; funds remain owned"
                );
            }
            work.deferred = blocker;
        }
    }

    pub(crate) fn retirement_pending_page(
        &self,
        pool: &str,
        wallet: &str,
        after: i64,
        through: i64,
    ) -> Result<Vec<(i64, super::super::base::PendingAuthorization)>> {
        let mut query = self.db.prepare("SELECT reconciliation_sequence,id,payer,nonce,valid_before FROM payment_attempts WHERE pool_id=?1 AND wallet_id=?2 AND state='POSSIBLY_SUBMITTED' AND reconciliation_sequence>?3 AND reconciliation_sequence<=?4 ORDER BY reconciliation_sequence LIMIT 64")?;
        let rows = query
            .query_map(params![pool, wallet, after, through], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    super::super::base::PendingAuthorization {
                        id: r.get(1)?,
                        wallet: wallet.to_owned(),
                        payer: r.get(2)?,
                        nonce: r.get(3)?,
                        valid_before: sql_u64(r, 4)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (_, auth) in &rows {
            let _: alloy_primitives::Address = auth
                .payer
                .parse()
                .context(super::super::base::evidence::EvidenceFailure::MalformedSupport)
                .context(super::super::base::VerificationStage(
                    "stored payment authorization",
                ))?;
            let _: alloy_primitives::B256 = auth
                .nonce
                .parse()
                .context(super::super::base::evidence::EvidenceFailure::MalformedSupport)
                .context(super::super::base::VerificationStage(
                    "stored payment authorization",
                ))?;
        }
        Ok(rows)
    }
    pub(crate) fn retirement_pending_boundary(&self, pool: &str, wallet: &str) -> Result<i64> {
        let (boundary, valid): (i64, bool) = self.db.query_row("SELECT COALESCE(MAX(reconciliation_sequence),0),COUNT(*)=0 OR (COUNT(reconciliation_sequence)=COUNT(*) AND MIN(reconciliation_sequence)>0) FROM payment_attempts WHERE pool_id=?1 AND wallet_id=?2 AND state='POSSIBLY_SUBMITTED'", params![pool,wallet], |r|Ok((r.get(0)?,r.get(1)?)))?;
        ensure!(
            valid,
            super::super::base::evidence::EvidenceFailure::MalformedSupport
        );
        Ok(boundary)
    }
}
