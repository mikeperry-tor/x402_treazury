//! Bounded startup reads over configured groups and their destination paths.
//! No partial Status, full wallet inventory, refund history, or new readiness ledger.
pub(super) const STARTUP_CLAIM_SQL: &str = "SELECT f.id,f.state='COMPLETE' AND f.readiness_requirement IS NULL AND w.role='RETAINED' FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id WHERE f.pool_id=?1 AND f.allocation_kind=?2 AND f.claim_generation=?3";

use super::funding::{FundingFacts, FundingJob};
use super::*;

pub(crate) const GROUP_PAGE: usize = 16;

#[derive(Debug)]
pub(crate) struct StartupGroup {
    pub pool: String,
    pub name: String,
    pub enabled: bool,
    /// Role ownership and validated immutable D; not fresh payment capacity.
    pub serving_ready: bool,
    /// Enabled current claims are funded and structurally bound.
    pub inventory_ready: bool,
    /// Treasury-wide owned source work is a wait, not proof of capital exhaustion.
    pub treasury_source_pending: bool,
    pub claims: Vec<StartupClaim>,
}

#[derive(Clone, Debug)]
pub(crate) struct StartupClaim {
    pub slot: String,
    pub generation: i64,
}

#[derive(Debug)]
pub(crate) struct StartupPath {
    pub wallet_type: String,
    pub job: FundingJob,
    pub facts: FundingFacts,
    pub operation: Option<OperationStatus>,
    /// Existing exact support, including binding checks. Independent of phase/S/P.
    pub destination_ready: bool,
    pub recovery_count: u64,
    pub condition: Option<super::super::error::FundingCondition>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartupDisposition {
    DestinationReady,
    Waiting,
    Actionable,
    Obstructed(&'static str),
}
impl StartupPath {
    /// Interpret committed obligations and typed current-process observations;
    /// this does not transition jobs, quote, forecast input cost, or authorize work.
    pub(crate) fn disposition(
        &self,
        max_attempts: u32,
        treasury_source_pending: bool,
    ) -> StartupDisposition {
        use StartupDisposition::*;
        if self.destination_ready {
            return DestinationReady;
        }
        let never_disclosed = self.facts.source_never_disclosed();
        let closed_expired = self.facts.source_closed
            && self
                .operation
                .as_ref()
                .is_some_and(|op| op.submission == "EXPIRED");
        let provider_terminal = self.facts.provider_failed || self.facts.provider_refunded;
        // Original accepted bytes/outcomes remain observed even if permission to
        // start another attempt has since been disabled or exhausted.
        if self.facts.claim_fulfilled
            || (!never_disclosed
                && !closed_expired
                && (self.facts.preparation_accepted
                    || self.facts.source_pending
                    || (!provider_terminal
                        && (self.facts.provider_incomplete
                            || self
                                .operation
                                .as_ref()
                                .is_some_and(|operation| operation.submission != "EXPIRED")))))
        {
            return Waiting;
        }
        let can_recover_original = funding::can_recover_facts(self.operation.as_ref(), &self.facts);
        let terminal_recovery = closed_expired || provider_terminal && can_recover_original;
        if provider_terminal && !terminal_recovery {
            return Obstructed("funding_outcome_requires_review");
        }
        if self.recovery_count > u64::from(max_attempts) {
            return Obstructed("recovery_attempt_limit_reached");
        }
        if self.job.phase == funding::FundingPhase::RecoveryRequired && !can_recover_original {
            return Obstructed("preparation_or_swap_requires_review");
        }
        if !self.facts.new_funding_allowed && !terminal_recovery {
            return Obstructed(match self.condition {
                Some(super::super::error::FundingCondition::Disabled) => "funding_disabled",
                Some(super::super::error::FundingCondition::QualificationDenied) => {
                    "qualification_funding_denied"
                }
                _ => "funding_group_disabled",
            });
        }
        use super::super::error::FundingCondition as Condition;
        let reason = match self.condition {
            Some(Condition::Disabled) => "funding_disabled",
            Some(Condition::QualificationDenied) => "qualification_funding_denied",
            Some(
                Condition::CostLimit | Condition::AmountLimit | Condition::AuthenticationRequired,
            ) => "funding_policy_requires_review",
            Some(Condition::EvidenceInvalid) => "funding_evidence_requires_review",
            Some(Condition::InsufficientFunds) if !treasury_source_pending => {
                "treasury_insufficient_spendable_funds"
            }
            _ => {
                return if !never_disclosed
                    && !closed_expired
                    && (self.facts.accepted_quote || treasury_source_pending)
                {
                    Waiting
                } else {
                    Actionable
                };
            }
        };
        Obstructed(reason)
    }
}

impl Store {
    pub(crate) fn observe_funding_condition(
        &mut self,
        job: &str,
        operation: &str,
        condition: Option<super::super::error::FundingCondition>,
    ) -> Result<()> {
        let current: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM funding_progress WHERE job_id=?1 AND operation_id=?2)",
            params![job, operation],
            |r| r.get(0),
        )?;
        if !current {
            return Ok(());
        }
        if let Some(condition) = condition {
            self.funding_conditions
                .insert(job.to_owned(), (operation.to_owned(), condition));
        } else {
            self.funding_conditions.remove(job);
        }
        Ok(())
    }

    /// Caller pages its immutable configured name set, at most GROUP_PAGE per turn.
    /// Each group has at most one ACTIVE and two enabled reserve claims.
    pub(crate) fn startup_groups(&self, names: &[String]) -> Result<Vec<StartupGroup>> {
        ensure!(names.len() <= GROUP_PAGE, "startup group page too large");
        let mut result = Vec::with_capacity(names.len());
        for name in names {
            let (pool, enabled): (String, bool) = self
                .db
                .query_row(
                    "SELECT id,enabled FROM pools WHERE name=?1",
                    [name],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?
                .context("bootstrap pool missing from state")?;
            let mut slots = self.db.prepare("SELECT slot,generation FROM reserve_claims WHERE pool_id=?1 AND enabled=1 ORDER BY slot")?;
            let claims = slots
                .query_map([&pool], |r| {
                    Ok(StartupClaim {
                        slot: r.get(0)?,
                        generation: r.get(1)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ensure!(claims.len() <= 2, "startup claim inventory conflict");
            let identity = capacity::default_identity(&self.db, &pool)?;
            let serving_ready = identity.is_some();
            let mut inventory_ready = !claims.is_empty();
            for claim in &claims {
                let wallet:Option<String> = self.db.query_row("SELECT wallet_id FROM reserve_claims WHERE pool_id=?1 AND slot=?2 AND generation=?3 AND enabled=1",params![pool,claim.slot,claim.generation],|r|r.get(0))?;
                let ready = if let Some(wallet) = wallet {
                    capacity::payment_ownership(&self.db, &pool, &wallet)?.is_some_and(|owner| {
                        owner.role == "READY"
                            && owner.slot.as_deref() == Some(claim.slot.as_str())
                            && owner.claim_generation == Some(claim.generation)
                    })
                } else {
                    false
                };
                inventory_ready &= ready;
            }
            result.push(StartupGroup {
                pool,
                name: name.clone(),
                enabled,
                serving_ready: enabled && serving_ready,
                inventory_ready: enabled && inventory_ready,
                claims,
                treasury_source_pending: self.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM outgoing WHERE state='PREPARED')",
                    [],
                    |row| row.get(0),
                )?,
            });
        }
        Ok(result)
    }

    /// Recheck the small current path set in one serialized read before an
    /// operator obstruction escapes. Saturated pages are incomplete, never proof
    /// that every path is blocked. No source/history scan is performed.
    pub(crate) fn startup_obstruction(
        &self,
        name: &str,
        max_attempts: u32,
        claims: &[StartupClaim],
    ) -> Result<Option<Vec<(String, &'static str)>>> {
        let group = self.startup_groups(&[name.to_owned()])?.remove(0);
        let paths = self.startup_claim_paths(&group.pool, claims)?;
        if paths.len() != claims.len() {
            return Ok(None);
        }
        let mut blocked = Vec::new();
        for path in paths {
            match path.disposition(max_attempts, group.treasury_source_pending) {
                StartupDisposition::Obstructed(reason) => {
                    if blocked.len() < 8 {
                        blocked.push((path.job.id, reason));
                    }
                }
                StartupDisposition::DestinationReady => {}
                _ => return Ok(None),
            }
        }
        Ok((!blocked.is_empty()).then_some(blocked))
    }

    /// Read the saved advisory estimate of each captured claim, never its target.
    pub(crate) fn startup_claim_balances(
        &self,
        pool: &str,
        claims: &[StartupClaim],
    ) -> Result<Vec<(String, String)>> {
        ensure!(claims.len() <= 2, "startup claim inventory too large");
        claims.iter().map(|claim| {
            let job = self.startup_claim_job(pool, claim)?.context("bootstrap replacement allocation pending")?;
            let balance: String = self.db.query_row(
                "SELECT w.balance FROM funding_jobs j JOIN wallets w ON w.id=j.wallet_id WHERE j.id=?1",
                [&job],
                |row| row.get(0),
            ).context("bootstrap wallet estimate unavailable")?;
            let display = amount(&balance).ok()
                .map(|value| alloy_primitives::utils::format_units(value, 6))
                .transpose()?.unwrap_or_else(|| "unknown".into());
            Ok((claim.slot.clone(), display))
        }).collect()
    }

    pub(crate) fn startup_claim_paths(
        &self,
        pool: &str,
        claims: &[StartupClaim],
    ) -> Result<Vec<StartupPath>> {
        ensure!(claims.len() <= 2, "startup claim inventory too large");
        let mut result = Vec::new();
        for claim in claims {
            if let Some(id) = self.startup_claim_job(pool, claim)? {
                result.push(self.startup_path(&id)?);
            }
        }
        Ok(result)
    }

    /// Follow only replaced, never-credited inventory. A credited generation
    /// satisfies this finite goal even if payments have since consumed it.
    fn startup_claim_job(&self, pool: &str, claim: &StartupClaim) -> Result<Option<String>> {
        let mut generation = claim.generation;
        loop {
            let row: Option<(String, bool)> = self
                .db
                .query_row(
                    STARTUP_CLAIM_SQL,
                    params![pool, claim.slot, generation],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            match row {
                None => return Ok(None),
                Some((id, false)) => return Ok(Some(id)),
                Some((_, true)) => {
                    generation = generation
                        .checked_add(1)
                        .context("reserve_generation_overflow")?
                }
            }
        }
    }

    fn startup_path(&self, id: &str) -> Result<StartupPath> {
        let job = self.funding_job(id)?.context("funding job missing")?;
        let facts = self.funding_facts(id, &job.operation_id)?;
        let operation = if facts.provider_success {
            None
        } else {
            self.funding_recovery_context(id)?.1
        };
        let destination_ready = funding::initial_credit_recorded(&self.db, &job.wallet_id)?;
        Ok(StartupPath {
            condition: match self.funding_restriction {
                super::super::restriction::FundingRestriction::Paused => {
                    Some(super::super::error::FundingCondition::Disabled)
                }
                super::super::restriction::FundingRestriction::DenyNewFunding => {
                    Some(super::super::error::FundingCondition::QualificationDenied)
                }
                super::super::restriction::FundingRestriction::Ordinary => self
                    .funding_conditions
                    .get(id)
                    .filter(|(operation, _)| operation == &job.operation_id)
                    .map(|(_, condition)| *condition),
            },
            wallet_type: self.funding_wallet_type(id)?,
            job,
            facts,
            operation,
            destination_ready,
            recovery_count: self.funding_recovery_count(id)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconstructed_unpublished_intent_uses_current_authority_with_zero_recovery_allowance() {
        use crate::rotation::{error::FundingCondition, near::SwapStatus};
        for disabled in [false, true] {
            let (_dir, mut store, _pool) = fixture();
            let job = store.funding_jobs().unwrap().remove(0);
            store.save_funding_quote(&job.id, &serde_json::to_vec(&serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}})).unwrap()).unwrap();
            store
                .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
                .unwrap();
            store
                .record_provider_outcome(
                    &job.id,
                    &job.operation_id,
                    &SwapStatus::IncompleteDeposit,
                    1,
                )
                .unwrap();
            // Before reconstruction, the accepted live calculation still owns its result.
            assert_eq!(
                store.startup_path(&job.id).unwrap().disposition(0, false),
                StartupDisposition::Waiting
            );
            store.close_unpublished_preparations().unwrap();
            if disabled {
                store.pause_new_funding();
            }
            let path = store.startup_path(&job.id).unwrap();
            assert!(path.facts.preparation_accepted && path.facts.source_closed);
            assert!(path.facts.provider_incomplete && path.operation.is_none());
            assert_eq!(path.recovery_count, 0);
            assert_eq!(
                path.disposition(0, false),
                if disabled {
                    StartupDisposition::Obstructed("funding_disabled")
                } else {
                    StartupDisposition::Actionable
                }
            );
            assert_eq!(
                path.condition,
                disabled.then_some(FundingCondition::Disabled)
            );
            assert!(
                store
                    .require_unstarted_preparation(&job.operation_id)
                    .is_err()
            );
            assert_eq!(
                store.funding_job(&job.id).unwrap().unwrap().operation_id,
                job.operation_id
            );
            // SUCCESS owns settlement independently of any unpublished-reset history.
            store
                .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Success, 2)
                .unwrap();
            assert_eq!(
                store.startup_path(&job.id).unwrap().disposition(0, false),
                StartupDisposition::Waiting
            );
        }
    }

    #[test]
    fn closed_untouched_quote_is_actionable_without_consuming_recovery_allowance() {
        use crate::rotation::near::SwapStatus;
        let (_dir, mut store, _pool) = fixture();
        let job = store.funding_jobs().unwrap().remove(0);
        store
            .save_funding_quote(
                &job.id,
                &serde_json::to_vec(
                    &serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}}),
                )
                .unwrap(),
            )
            .unwrap();
        store
            .record_provider_outcome(
                &job.id,
                &job.operation_id,
                &SwapStatus::IncompleteDeposit,
                1,
            )
            .unwrap();
        store
            .close_unprepared_funding_for_operation(&job.id, &job.operation_id)
            .unwrap();
        let path = store.startup_path(&job.id).unwrap();
        assert!(path.facts.source_never_disclosed());
        assert!(!path.facts.preparation_accepted);
        assert_eq!(path.recovery_count, 0);
        assert_eq!(path.disposition(0, false), StartupDisposition::Actionable);
        assert_eq!(
            store.funding_job(&job.id).unwrap().unwrap().operation_id,
            job.operation_id
        );
        assert!(!store.status().unwrap().outgoing_pending);
    }

    #[test]
    fn provider_reports_do_not_obstruct_live_original_observation() {
        use crate::rotation::{near::SwapStatus, transaction::TransactionFacts};
        let (_dir, mut store, _pool) = fixture();
        let captured = store
            .startup_groups(&["configured".into()])
            .unwrap()
            .remove(0)
            .claims;
        let jobs = store.funding_jobs().unwrap();
        for job in &jobs {
            let quote = serde_json::to_vec(
                &serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}}),
            )
            .unwrap();
            store.save_funding_quote(&job.id, &quote).unwrap();
            store
                .record_provider_outcome(
                    &job.id,
                    &job.operation_id,
                    &SwapStatus::IncompleteDeposit,
                    1,
                )
                .unwrap();
        }
        assert!(
            store
                .startup_obstruction("configured", 3, &captured)
                .unwrap()
                .is_none()
        );
        // The untouched quote above is closed without spending recovery authority.
        // Use a fresh current source to exercise live accepted and published work.
        let (_dir, mut store, pool) = fixture();
        let captured = store
            .startup_groups(&["configured".into()])
            .unwrap()
            .remove(0)
            .claims;
        let jobs = store.funding_jobs().unwrap();
        let job = &jobs[0];
        store
            .save_funding_quote(
                &job.id,
                &serde_json::to_vec(&serde_json::json!({
                    "request":{"amount":job.target,"recipient":job.recipient}
                }))
                .unwrap(),
            )
            .unwrap();
        store
            .record_provider_outcome(
                &job.id,
                &job.operation_id,
                &SwapStatus::IncompleteDeposit,
                1,
            )
            .unwrap();

        store
            .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
            .unwrap();
        assert!(
            store
                .startup_obstruction("configured", 3, &captured)
                .unwrap()
                .is_none(),
            "accepted original work still has an observation owner"
        );
        let revision = store.snapshot().unwrap().0;
        store
            .prepare_with_facts(
                &job.operation_id,
                revision,
                b"snapshot",
                crate::rotation::store::test_support::transaction_raw(100).as_slice(),
                Some(TransactionFacts {
                    txid: crate::rotation::store::test_support::transaction_txid(100),
                    expiry_height: 100,
                    amount_zatoshis: 50,
                    fee_zatoshis: 10,
                    deadline: Some(u64::MAX),
                }),
            )
            .unwrap();
        store
            .request_broadcast(&job.operation_id, 1, 1, false)
            .unwrap();
        assert!(
            store
                .startup_obstruction("configured", 3, &captured)
                .unwrap()
                .is_none()
        );
        store.confirm_spend(&job.operation_id).unwrap();
        assert!(
            store
                .startup_obstruction("configured", 3, &captured)
                .unwrap()
                .is_none(),
            "canonical source confirmation can still be followed by destination credit"
        );
        let group = store
            .startup_groups(&["configured".into()])
            .unwrap()
            .remove(0);
        let mut path = store
            .startup_claim_paths(&pool, &group.claims)
            .unwrap()
            .into_iter()
            .find(|path| path.job.id == job.id)
            .unwrap();
        assert!(!path.facts.preparation_accepted);
        // Model the existing expiry owner's closed archived result. Retained
        // preparation bytes are historical evidence, not active observation work.
        path.facts.source_closed = true;
        path.facts.source_pending = false;
        path.operation.as_mut().unwrap().submission = "EXPIRED".into();
        assert_eq!(path.disposition(3, false), StartupDisposition::Actionable);
        path.facts.source_pending = true;
        path.facts.source_closed = false;
        path.operation.as_mut().unwrap().submission = "UNKNOWN".into();
        assert_eq!(
            path.disposition(3, false),
            StartupDisposition::Waiting,
            "a canonical correction restores the original observation owner"
        );
        assert_eq!(store.operation(&job.operation_id).unwrap().attempts, 1);
        assert_eq!(store.funding_recovery_count(&job.id).unwrap(), 0);
    }

    #[test]
    fn operation_backed_recovery_keeps_original_ownership_and_waits_for_terminal_provider_facts() {
        use crate::rotation::{
            near::SwapStatus,
            store::preparation::{CalculationInputs, InputPool, OwnedInput},
            transaction::TransactionFacts,
        };
        let (_dir, mut store, _pool) = fixture();
        let jobs = store.funding_jobs().unwrap();
        assert_eq!(jobs.len(), 2);
        // Retain the exact original sources, accepted inputs and claim identities,
        // including the unrelated job, around each read-only assessment.
        let journal = |store: &Store| {
            let mut query = store.db.prepare(
                "SELECT f.*,j.*,x.*,o.*,n.*,c.* FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id LEFT JOIN outgoing x ON x.id=j.operation_id LEFT JOIN treasury_operations o ON o.id=j.operation_id LEFT JOIN operation_network n ON n.operation_id=j.operation_id LEFT JOIN reserve_claims c ON c.pool_id=f.pool_id AND c.slot=f.allocation_kind ORDER BY f.id",
            ).unwrap();
            let columns = query.column_count();
            let rows = query
                .query_map([], |row| {
                    (0..columns)
                        .map(|column| row.get::<_, rusqlite::types::Value>(column))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            let wallets: i64 = store
                .db
                .query_row("SELECT COUNT(*) FROM wallets", [], |row| row.get(0))
                .unwrap();
            (rows, wallets)
        };
        for (index, (job, terminal)) in jobs
            .iter()
            .zip([SwapStatus::Failed, SwapStatus::Refunded])
            .enumerate()
        {
            let quote = serde_json::to_vec(&serde_json::json!({
                "request":{"amount":job.target,"recipient":job.recipient}
            }))
            .unwrap();
            let expiry = 100 + index as u32;
            let raw = crate::rotation::store::test_support::transaction_raw(expiry);
            store.save_funding_quote(&job.id, &quote).unwrap();
            let selected = vec![OwnedInput {
                pool: InputPool::Orchard,
                txid: format!("{:064x}", index + 1),
                output_index: 0,
            }];
            store
                .accept_preparation(
                    &job.operation_id,
                    Some(&job.pool_id),
                    60,
                    Some(CalculationInputs {
                        destination: "original deposit".into(),
                        inputs: selected.clone(),
                    }),
                )
                .unwrap();
            let revision = store.snapshot().unwrap().0;
            store
                .prepare_with_facts(
                    &job.operation_id,
                    revision,
                    b"snapshot",
                    &raw,
                    Some(TransactionFacts {
                        txid: crate::rotation::store::test_support::transaction_txid(expiry),
                        expiry_height: expiry,
                        amount_zatoshis: 50,
                        fee_zatoshis: 10,
                        deadline: Some(u64::MAX),
                    }),
                )
                .unwrap();
            store
                .request_broadcast(&job.operation_id, 1, 1, false)
                .unwrap();
            let assess =
                |store: &mut Store, submission: &str, recoverable, retry_allowed, pending| {
                    let before = journal(store);
                    assert_eq!(store.prepared_inputs(&job.operation_id).unwrap(), selected);
                    let path = store.startup_path(&job.id).unwrap();
                    let operation = path.operation.as_ref().unwrap();
                    assert_eq!(operation.operation_id, job.operation_id);
                    assert_eq!(operation.submission, submission);
                    assert_eq!(operation.attempts, 1);
                    assert_eq!(path.recovery_count, 0);
                    assert!(!path.facts.preparation_accepted && path.facts.accepted_quote);
                    assert_eq!(path.facts.source_pending, pending);
                    // Nonterminal originals wait before recovery classification.
                    // A terminal provider report permits recovery assessment,
                    // but cannot authorize replacing a confirmed deposit.
                    assert_eq!(
                        funding::can_recover_facts(Some(operation), &path.facts),
                        recoverable
                    );
                    assert_eq!(
                        store
                            .funding_retry_allowed(&job.id, &job.operation_id)
                            .unwrap(),
                        retry_allowed
                    );
                    assert_eq!(path.disposition(3, false), StartupDisposition::Waiting);
                    assert_eq!(
                        store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
                        raw.as_slice()
                    );
                    assert_eq!(
                        store
                            .funding_quote_for_operation(&job.id, &job.operation_id)
                            .unwrap()
                            .as_slice(),
                        quote.as_slice()
                    );
                    assert!(
                        store
                            .require_unstarted_preparation(&job.operation_id)
                            .is_err()
                    );
                    assert!(store.abandon_unprepared(&job.operation_id).is_err());
                    assert_eq!(store.funding_jobs().unwrap().len(), jobs.len());
                    assert_eq!(store.prepared_inputs(&job.operation_id).unwrap(), selected);
                    assert_eq!(
                        journal(store),
                        before,
                        "assessment changed original accepted work"
                    );
                };
            assess(&mut store, "BROADCAST_REQUESTED", true, false, true);
            store.confirm_spend(&job.operation_id).unwrap();
            assess(&mut store, "CONFIRMED", false, false, false);
            store
                .record_provider_outcome(&job.id, &job.operation_id, &terminal, 2)
                .unwrap();
            let facts = store.funding_facts(&job.id, &job.operation_id).unwrap();
            assert_eq!(
                facts.provider_failed,
                matches!(terminal, SwapStatus::Failed)
            );
            assert_eq!(
                facts.provider_refunded,
                matches!(terminal, SwapStatus::Refunded)
            );
            assess(&mut store, "CONFIRMED", true, false, false);
        }
    }

    #[test]
    fn archived_success_waits_for_credit_without_inventing_current_success_or_disabled_policy() {
        use crate::rotation::{near::SwapStatus, transaction::TransactionFacts};
        let (_dir, mut store, _pool) = fixture();
        let old = store.funding_jobs().unwrap().remove(0);
        store
            .save_funding_quote(
                &old.id,
                &serde_json::to_vec(
                    &serde_json::json!({"request":{"amount":old.target,"recipient":old.recipient}}),
                )
                .unwrap(),
            )
            .unwrap();
        store
            .accept_preparation(&old.operation_id, Some(&old.pool_id), 60, None)
            .unwrap();
        store
            .prepare_with_facts(
                &old.operation_id,
                1,
                b"original SDK",
                crate::rotation::store::test_support::transaction_raw(100).as_slice(),
                Some(TransactionFacts {
                    txid: crate::rotation::store::test_support::transaction_txid(100),
                    expiry_height: 100,
                    amount_zatoshis: 50,
                    fee_zatoshis: 10,
                    deadline: None,
                }),
            )
            .unwrap();
        let revision = store.snapshot().unwrap().0;
        let observation = SyncObservation {
            phase: SyncPhase::Ready,
            last_error: None,
            snapshot_revision: revision,
            started_snapshot_revision: None,
            checked_at: Some(1),
            checkpoint_at: 1,
            scanned_blocks: 200,
            target_height: Some(200),
            observed_tip_height: Some(200),
            height: Some(200),
            confirmations: 2,
            confirmed_pool_balances_zatoshis: None,
            confirmed_shielded_zatoshis: 1000,
            spendable_shielded_zatoshis: 1000,
        };
        let revision = store
            .save_sync_snapshot(revision, b"expiry SDK", Some(observation))
            .unwrap();
        store.resolve_expired(&old.operation_id, revision).unwrap();
        store
            .record_provider_outcome(&old.id, &old.operation_id, &SwapStatus::Failed, 1)
            .unwrap();
        store
            .reauthorize_closed_funding_for_operation(&old.id, &old.operation_id, 3)
            .unwrap();
        let current = store.funding_job(&old.id).unwrap().unwrap();
        store
            .record_provider_outcome(&old.id, &old.operation_id, &SwapStatus::Success, 2)
            .unwrap();
        let path = store.startup_path(&old.id).unwrap();
        assert!(path.facts.claim_fulfilled && path.facts.credit_needed());
        assert!(!path.facts.provider_success && !path.facts.new_funding_allowed);
        assert_eq!(path.job.operation_id, current.operation_id);
        assert_eq!(path.disposition(3, false), StartupDisposition::Waiting);
        assert_eq!(store.funding_recovery_count(&old.id).unwrap(), 1);
    }

    #[test]
    fn current_status_contract_has_explicit_authority_and_claims() {
        let (_dir, store, _pool) = fixture();
        let value = serde_json::to_value(store.status().unwrap()).unwrap();
        assert_eq!(value["schema_version"], 6);
        assert_eq!(value["state_schema_version"], 23);
        let snapshot = store.qualification_state().unwrap();
        assert_eq!(snapshot["treasury_status"], value);
        assert_eq!(snapshot["lifecycle"]["schema_version"], 2);
        assert_eq!(snapshot["lifecycle"]["state_schema_version"], 23);
        assert_eq!(snapshot["lifecycle"]["pools"], value["pools"]);
        let job = &snapshot["lifecycle"]["funding_jobs"][0];
        assert!(
            crate::qualification::lifecycle::restart::observed(
                &snapshot["lifecycle"],
                job["pool_id"].as_str().unwrap(),
                job["id"].as_str().unwrap(),
                crate::qualification::lifecycle::restart::Checkpoint::QueuedRefill,
            )
            .unwrap()
        );
        let keys: Vec<_> = value["pools"][0]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let expected: Vec<String> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/capacity/pool-status-fields-v2.json"
        ))
        .unwrap();
        assert_eq!(keys, expected);
        assert_eq!(value["pools"][0]["payment_limit_atomic"], "1000000");
        assert_eq!(value["pools"][0]["retirement_tolerance_atomic"], "0");
        assert_eq!(value["pools"][0]["inventory_ready"], false);
        let wallet = &value["pools"][0]["addresses"][0];
        let keys: Vec<_> = wallet.as_object().unwrap().keys().cloned().collect();
        let expected: Vec<String> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/capacity/address-status-fields-v3.json"
        ))
        .unwrap();
        assert_eq!(keys, expected);
        assert!(wallet["runtime_balance_evidence"].is_null());
        assert_eq!(
            value["pools"][0]["retirement_accounting"]["automatic_recorded_confirmed_atomic"],
            "0"
        );
    }

    fn fixture() -> (tempfile::TempDir, Store, String) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "configured",
            crate::rotation::test_capacity::two_reserves("5"),
            Some(alloy_primitives::U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        (dir, store, pool)
    }
    #[test]
    fn destination_inventory_ignores_source_only_completion_backlog() {
        let (_dir, mut store, pool) = fixture();
        let jobs = store.funding_jobs().unwrap();
        for job in &jobs {
            let quote = serde_json::to_vec(
                &serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}}),
            )
            .unwrap();
            store.save_funding_quote(&job.id, &quote).unwrap();
            store
                .record_credit(&job.wallet_id, &job.target, "fixture", 100)
                .unwrap();
            store.mark_funding_timeout(&job.id).unwrap();
        }
        let group = store
            .startup_groups(&["configured".into()])
            .unwrap()
            .remove(0);
        assert!(group.serving_ready && group.inventory_ready);
        let paths = store.startup_claim_paths(&pool, &group.claims).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(
            paths
                .iter()
                .all(|path| path.destination_ready && path.facts.initial_credit_recorded)
        );
        assert!(
            paths
                .iter()
                .all(|path| path.job.phase != funding::FundingPhase::Complete)
        );
        assert!(paths.iter().all(|path| path.operation.is_none()));
    }
    #[test]
    fn pages_are_scoped_bounded_and_empty_is_not_a_readiness_claim() {
        let (_dir, mut store, pool) = fixture();
        for n in 0..40 {
            crate::rotation::test_capacity::declare_and_allocate!(
                store,
                &format!("unrelated_{n}"),
                crate::rotation::test_capacity::two_reserves("5"),
                Some(alloy_primitives::U256::from(1_000_000u64)),
                0
            )
            .unwrap();
        }
        let group = store
            .startup_groups(&["configured".into()])
            .unwrap()
            .remove(0);
        assert!(!group.serving_ready && !group.inventory_ready);
        let paths = store.startup_claim_paths(&pool, &group.claims).unwrap();
        assert_eq!(paths.len(), 2);
        assert_ne!(paths[0].job.id, paths[1].job.id);
        assert!(
            store
                .startup_groups(&vec!["configured".into(); GROUP_PAGE + 1])
                .is_err()
        );
        assert!(
            store
                .startup_claim_paths(&pool, &vec![group.claims[0].clone(); 4])
                .is_err()
        );
    }

    #[test]
    fn disputed_destination_preserves_funded_inventory_but_disabled_group_cannot_serve() {
        let (_dir, mut store, _pool) = fixture();
        let jobs = store.funding_jobs().unwrap();
        for job in &jobs {
            store
                .record_credit(&job.wallet_id, &job.target, "fixture", 100)
                .unwrap();
        }
        store
            .db
            .execute(
                "UPDATE funding_jobs SET readiness_disputed='fixture' WHERE id=?1",
                [&jobs[0].id],
            )
            .unwrap();
        let group = store
            .startup_groups(&["configured".into()])
            .unwrap()
            .remove(0);
        assert!(group.serving_ready && group.inventory_ready);
        let paths = store
            .startup_claim_paths(&group.pool, &group.claims)
            .unwrap();
        assert!(paths.iter().all(|path| path.destination_ready));
        assert!(
            store
                .startup_obstruction("configured", 3, &group.claims)
                .unwrap()
                .is_none()
        );
        store
            .db
            .execute("UPDATE pools SET enabled=0 WHERE name='configured'", [])
            .unwrap();
        let group = store
            .startup_groups(&["configured".into()])
            .unwrap()
            .remove(0);
        assert!(!group.enabled && !group.serving_ready && !group.inventory_ready);
    }
    #[test]
    fn obstruction_requires_every_path_and_rechecks_worker_facts() {
        let (_dir, mut store, pool) = fixture();
        let captured = store
            .startup_groups(&["configured".into()])
            .unwrap()
            .remove(0)
            .claims;
        let jobs = store.funding_jobs().unwrap();
        for order in [[0, 1], [1, 0]] {
            for job in &jobs {
                store.defer_funding(&job.id, 0, None, false).unwrap();
                store
                    .observe_funding_condition(&job.id, &job.operation_id, None)
                    .unwrap();
            }
            store
                .defer_funding(
                    &jobs[order[0]].id,
                    0,
                    Some("treasury_insufficient_spendable_funds; fixed worker result"),
                    false,
                )
                .unwrap();
            store
                .observe_funding_condition(
                    &jobs[order[0]].id,
                    &jobs[order[0]].operation_id,
                    Some(super::super::super::error::FundingCondition::InsufficientFunds),
                )
                .unwrap();
            assert!(
                store
                    .startup_obstruction("configured", 3, &captured)
                    .unwrap()
                    .is_none(),
                "one blocked quote cannot forecast group exhaustion"
            );
            store
                .defer_funding(
                    &jobs[order[1]].id,
                    0,
                    Some("funding_amount_limit_exceeded; fixed worker result"),
                    false,
                )
                .unwrap();
            store
                .observe_funding_condition(
                    &jobs[order[1]].id,
                    &jobs[order[1]].operation_id,
                    Some(super::super::super::error::FundingCondition::AmountLimit),
                )
                .unwrap();
            assert_eq!(
                store
                    .startup_obstruction("configured", 3, &captured)
                    .unwrap()
                    .unwrap()
                    .len(),
                2
            );
        }
        let group = store
            .startup_groups(&["configured".into()])
            .unwrap()
            .remove(0);
        let paths = store.startup_claim_paths(&pool, &group.claims).unwrap();
        let insufficient = paths
            .iter()
            .find(|p| {
                p.job
                    .last_error
                    .as_ref()
                    .unwrap()
                    .starts_with("treasury_insufficient")
            })
            .unwrap();
        assert!(matches!(
            insufficient.disposition(3, false),
            StartupDisposition::Obstructed("treasury_insufficient_spendable_funds")
        ));
        assert_eq!(
            insufficient.disposition(3, true),
            StartupDisposition::Waiting,
            "another owned reservation can resolve; not exhaustion"
        );
        store
            .defer_funding(
                &jobs[0].id,
                0,
                Some("near_rate_limited; backing off"),
                false,
            )
            .unwrap();
        store
            .observe_funding_condition(&jobs[0].id, &jobs[0].operation_id, None)
            .unwrap();
        assert!(
            store
                .startup_obstruction("configured", 3, &captured)
                .unwrap()
                .is_none()
        );
    }
}

#[cfg(test)]
mod typed_condition_tests {
    use super::*;
    use crate::rotation::error::FundingCondition;

    #[test]
    fn startup_obstructions_depend_on_typed_current_observations_not_saved_prose() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )?;
        super::super::test_support::declare_two_reserves(
            &mut store,
            "typed",
            "1",
            Some(U256::from(10)),
            0,
        )?;
        let jobs = store.funding_jobs()?;
        let captured = store.startup_groups(&["typed".into()])?.remove(0).claims;
        for job in &jobs {
            store.defer_funding(
                &job.id,
                0,
                Some("funding_cost_limit_exceeded; forged policy observation"),
                false,
            )?;
        }
        assert!(store.startup_obstruction("typed", 3, &captured)?.is_none());
        for job in &jobs {
            store.observe_funding_condition(
                &job.id,
                &job.operation_id,
                Some(FundingCondition::CostLimit),
            )?;
            store.defer_funding(&job.id, 0, Some("arbitrary diagnostic"), false)?;
        }
        assert_eq!(
            store
                .startup_obstruction("typed", 3, &captured)?
                .unwrap()
                .len(),
            2
        );
        for job in &jobs {
            store.observe_funding_condition(&job.id, "obsolete-operation", None)?;
        }
        assert_eq!(
            store
                .startup_obstruction("typed", 3, &captured)?
                .unwrap()
                .len(),
            2
        );
        for job in &jobs {
            store.observe_funding_condition(&job.id, &job.operation_id, None)?;
        }
        assert!(store.startup_obstruction("typed", 3, &captured)?.is_none());
        store.pause_new_funding();
        assert!(
            store
                .startup_obstruction("typed", 3, &captured)?
                .unwrap()
                .iter()
                .all(|(_, reason)| *reason == "funding_disabled")
        );
        assert!(store.status()?.treasury_operations.is_empty());
        assert_eq!(store.status()?.snapshot_revision, 1);
        Ok(())
    }
}
