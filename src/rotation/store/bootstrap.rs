//! Bounded startup reads over configured groups and their destination paths.
//! No partial Status, full wallet inventory, refund history, or new readiness ledger.
pub(super) const STARTUP_CLAIM_SQL: &str =
    "SELECT id FROM funding_jobs WHERE pool_id=?1 AND allocation_kind=?2 AND claim_generation=?3";

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
    /// An owned source reservation is a wait, not proof of capital exhaustion.
    pub source_pending: bool,
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
    pub readiness_dispute: Option<String>,
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
        source_pending: bool,
    ) -> StartupDisposition {
        use StartupDisposition::*;
        if self.destination_ready {
            return DestinationReady;
        }
        if self.facts.readiness_disputed {
            return if self.readiness_dispute.as_deref() == Some("endpoint_reverification_required")
            {
                Waiting
            } else {
                Obstructed("destination_support_requires_review")
            };
        }
        if self.facts.provider_review_required()
            || self.facts.provider_failed
            || self.facts.provider_refunded
        {
            return Obstructed("funding_outcome_requires_review");
        }
        // Original accepted bytes/outcomes remain observed even if permission to
        // start another attempt has since been disabled or exhausted.
        if self
            .operation
            .as_ref()
            .is_some_and(|operation| operation.submission != "EXPIRED")
        {
            return Waiting;
        }
        if self.recovery_count > u64::from(max_attempts) {
            return Obstructed("recovery_attempt_limit_reached");
        }
        if self.job.phase == funding::FundingPhase::RecoveryRequired
            && !crate::rotation::store::funding::can_recover_facts(
                &self.job,
                self.operation.as_ref(),
                &self.facts,
            )
        {
            return Obstructed("preparation_or_swap_requires_review");
        }
        if !self.facts.enabled {
            return Obstructed("funding_group_disabled");
        }
        use super::super::error::FundingCondition as Condition;
        let reason = match self.condition {
            Some(Condition::Disabled) => "funding_disabled",
            Some(Condition::QualificationDenied) => "qualification_funding_denied",
            Some(
                Condition::CostLimit
                | Condition::AmountLimit
                | Condition::InputLimit
                | Condition::AuthenticationRequired,
            ) => "funding_policy_requires_review",
            Some(Condition::EvidenceInvalid) => "funding_evidence_requires_review",
            Some(Condition::InsufficientFunds) if !source_pending => {
                "treasury_insufficient_spendable_funds"
            }
            _ => {
                return if self.facts.accepted_quote || source_pending {
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
                    let readiness =
                        capacity::payment_ownership(&self.db, &pool, &wallet).and_then(|owner| {
                            if owner.is_some()
                                && !funding::readiness_recorded_valid(&self.db, &wallet)?
                            {
                                Ok(None)
                            } else {
                                Ok(owner)
                            }
                        });
                    match readiness {
                        Ok(owner) => owner.is_some_and(|owner| {
                            owner.role == "READY"
                                && owner.slot.as_deref() == Some(claim.slot.as_str())
                                && owner.claim_generation == Some(claim.generation)
                        }),
                        Err(error) if error.downcast_ref::<super::super::base::evidence::EvidenceFailure>()
                            == Some(&super::super::base::evidence::EvidenceFailure::MalformedSupport) => {
                                tracing::warn!(category = "malformed_durable_evidence_support", "Reserve readiness rejected; serving eligibility uses independent funded identities");
                                false
                            },
                        Err(error) => return Err(error),
                    }
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
                source_pending: self.db.query_row(
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
        captured: Option<&[StartupClaim]>,
    ) -> Result<Option<Vec<(String, &'static str)>>> {
        let group = self.startup_groups(&[name.to_owned()])?.remove(0);
        let serving = captured.is_none();
        if serving && group.serving_ready {
            return Ok(None);
        }
        let claims = captured.unwrap_or(&group.claims);
        let paths = self.startup_claim_paths(&group.pool, claims)?;
        if paths.len() != claims.len() {
            return Ok(None);
        }
        let mut blocked = Vec::new();
        for path in paths {
            match path.disposition(max_attempts, group.source_pending) {
                StartupDisposition::Obstructed(reason) => {
                    if blocked.len() < 8 {
                        blocked.push((path.job.id, reason));
                    }
                }
                StartupDisposition::DestinationReady if !serving => {}
                _ => return Ok(None),
            }
        }
        Ok((!blocked.is_empty()).then_some(blocked))
    }

    /// Read the saved confirmed balance of each captured claim, never its target.
    pub(crate) fn startup_claim_balances(
        &self,
        pool: &str,
        claims: &[StartupClaim],
    ) -> Result<Vec<(String, String)>> {
        ensure!(claims.len() <= 2, "startup claim inventory too large");
        claims.iter().map(|claim| {
            let (balance, height, hash): (String, Option<i64>, Option<String>) = self.db.query_row(
                "SELECT w.balance,w.block_height,w.block_hash FROM funding_jobs j JOIN wallets w ON w.id=j.wallet_id AND w.pool_id=j.pool_id WHERE j.pool_id=?1 AND j.allocation_kind=?2 AND j.claim_generation=?3",
                params![pool, claim.slot, claim.generation],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).context("bootstrap confirmed wallet balance unavailable")?;
            ensure!(height.is_some() && hash.is_some(), "bootstrap confirmed wallet balance has no saved block evidence");
            let amount = balance.parse::<U256>().context("invalid saved bootstrap wallet balance")?;
            Ok((claim.slot.clone(), alloy_primitives::utils::format_units(amount, 6)?))
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
            let saved: Option<String> = self
                .db
                .query_row(
                    STARTUP_CLAIM_SQL,
                    params![pool, claim.slot, claim.generation],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = saved {
                result.push(self.startup_path(&id)?);
            }
        }
        Ok(result)
    }

    fn startup_path(&self, id: &str) -> Result<StartupPath> {
        let (job, operation) = self.funding_recovery_context(id)?;
        let facts = self.funding_facts(id, &job.operation_id)?;
        let destination_ready = funding::readiness_supported(&self.db, &job.wallet_id)?;
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
            readiness_dispute: self.db.query_row(
                "SELECT readiness_disputed FROM funding_jobs WHERE id=?1",
                [id],
                |r| r.get(0),
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_status_contract_has_explicit_authority_and_claims() {
        let (_dir, store, _pool) = fixture();
        let value = serde_json::to_value(store.status().unwrap()).unwrap();
        assert_eq!(value["schema_version"], 2);
        assert_eq!(value["state_schema_version"], 16);
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
            "../../../tests/fixtures/capacity/address-status-fields-v2.json"
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
                .all(|path| path.destination_ready && path.facts.readiness_recorded)
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
        assert!(
            !paths
                .iter()
                .find(|path| path.job.id == jobs[0].id)
                .unwrap()
                .destination_ready
        );
        assert!(
            store
                .startup_obstruction("configured", 3, None)
                .unwrap()
                .is_none()
        );
        let blocked = store
            .startup_obstruction("configured", 3, Some(&group.claims))
            .unwrap()
            .unwrap();
        assert_eq!(
            blocked,
            vec![(jobs[0].id.clone(), "destination_support_requires_review")]
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
                    .startup_obstruction("configured", 3, None)
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
                    .startup_obstruction("configured", 3, None)
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
                .startup_obstruction("configured", 3, None)
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
        for job in &jobs {
            store.defer_funding(
                &job.id,
                0,
                Some("funding_cost_limit_exceeded; forged policy observation"),
                false,
            )?;
        }
        assert!(store.startup_obstruction("typed", 3, None)?.is_none());
        for job in &jobs {
            store.observe_funding_condition(
                &job.id,
                &job.operation_id,
                Some(FundingCondition::CostLimit),
            )?;
            store.defer_funding(&job.id, 0, Some("arbitrary diagnostic"), false)?;
        }
        assert_eq!(
            store.startup_obstruction("typed", 3, None)?.unwrap().len(),
            2
        );
        for job in &jobs {
            store.observe_funding_condition(&job.id, "obsolete-operation", None)?;
        }
        assert_eq!(
            store.startup_obstruction("typed", 3, None)?.unwrap().len(),
            2
        );
        for job in &jobs {
            store.observe_funding_condition(&job.id, &job.operation_id, None)?;
        }
        assert!(store.startup_obstruction("typed", 3, None)?.is_none());
        store.pause_new_funding();
        assert!(
            store
                .startup_obstruction("typed", 3, None)?
                .unwrap()
                .iter()
                .all(|(_, reason)| *reason == "funding_disabled")
        );
        assert!(store.status()?.treasury_operations.is_empty());
        assert_eq!(store.status()?.snapshot_revision, 1);
        Ok(())
    }
}
