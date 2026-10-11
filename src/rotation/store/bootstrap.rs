//! Bounded startup reads over configured groups and their destination paths.
//! No partial Status, full wallet inventory, refund history, or new readiness ledger.
use super::funding::{FundingFacts, FundingJob};
use super::*;

pub(crate) const GROUP_PAGE: usize = 16;
pub(crate) const PATH_PAGE: usize = 64;

#[derive(Debug)]
pub(crate) struct StartupGroup {
    pub pool: String,
    pub name: String,
    pub enabled: bool,
    /// Role ownership and validated immutable D; not fresh payment capacity.
    pub serving_ready: bool,
    /// Current legacy ACTIVE + READY occupancy. Schema16 uses exact enabled claims.
    pub inventory_ready: bool,
    /// Capture once for explicit bootstrap. Future replacements are not its work.
    pub through_sequence: i64,
    /// An owned source reservation is a wait, not proof of capital exhaustion.
    pub source_pending: bool,
}

#[derive(Debug)]
pub(crate) struct StartupPath {
    pub sequence: i64,
    pub job: FundingJob,
    pub facts: FundingFacts,
    pub operation: Option<OperationStatus>,
    /// Existing exact support, including binding checks. Independent of phase/S/P.
    pub destination_ready: bool,
    pub recovery_count: u64,
    pub readiness_dispute: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartupDisposition {
    DestinationReady,
    Waiting,
    Actionable,
    Obstructed(&'static str),
}
impl StartupPath {
    /// Interpret the worker's committed facts and fixed diagnostic categories;
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
            && !crate::treasury::recovery::can_recover_facts(&self.job, self.operation.as_ref())
        {
            return Obstructed("preparation_or_swap_requires_review");
        }
        if !self.facts.enabled {
            return Obstructed("funding_group_disabled");
        }
        let category = self
            .job
            .last_error
            .as_deref()
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("");
        let reason = match category {
            "funding_disabled" => "funding_disabled",
            "qualification_funding_denied" => "qualification_funding_denied",
            "funding_cost_limit_exceeded"
            | "funding_amount_limit_exceeded"
            | "funding_input_cap_exceeded"
            | "near_authentication_required" => "funding_policy_requires_review",
            "funding_allocation_evidence_invalid" | "funding_integrity_conflict" => {
                "funding_evidence_requires_review"
            }
            "treasury_insufficient_spendable_funds" if !source_pending => {
                "treasury_insufficient_spendable_funds"
            }
            // Temporary legacy limits disappear with schema16/config7.
            "daily_funding_limit_exceeded"
            | "total_funding_limit_exceeded"
            | "treasury_budget_exceeded" => "funding_policy_requires_review",
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
    /// Caller pages its immutable configured name set, at most GROUP_PAGE per turn.
    /// Each group has at most one ACTIVE and one READY under schema15 constraints.
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
            let through_sequence: i64 = self.db.query_row(
                "SELECT COALESCE(MAX(sequence),-1) FROM wallets WHERE pool_id=?1",
                [&pool],
                |row| row.get(0),
            )?;
            let mut statement = self.db.prepare(
                "SELECT id,role,evidence_repair FROM wallets WHERE pool_id=?1 AND role IN ('ACTIVE','READY') ORDER BY role LIMIT 3",
            )?;
            let candidates = statement
                .query_map([&pool], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ensure!(candidates.len() <= 2, "startup role ownership conflict");
            let mut active = false;
            let mut ready = false;
            for (wallet, role, repair) in candidates {
                if repair.is_some() || !funding::readiness_supported(&self.db, &wallet)? {
                    continue;
                }
                match role.as_str() {
                    "ACTIVE" => {
                        ensure!(!active, "multiple startup active owners");
                        active = true;
                    }
                    "READY" => {
                        ensure!(!ready, "multiple startup ready owners");
                        ready = true;
                    }
                    _ => unreachable!(),
                }
            }
            result.push(StartupGroup {
                pool,
                name: name.clone(),
                enabled,
                serving_ready: enabled && (active || ready),
                inventory_ready: enabled && active && ready,
                through_sequence,
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
        serving: bool,
    ) -> Result<Option<Vec<(String, &'static str)>>> {
        let group = self.startup_groups(&[name.to_owned()])?.remove(0);
        if (serving && group.serving_ready) || (!serving && group.inventory_ready) {
            return Ok(None);
        }
        let paths = self.startup_paths_page(&group.pool, -1, group.through_sequence, PATH_PAGE)?;
        if paths.is_empty() || paths.len() == PATH_PAGE {
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

    /// Captured destination work, not funding completion. All current ALLOCATED
    /// paths remain visible, including disabled groups. Historical funded wallets
    /// with source-only backlog do not enlarge startup's path inventory.
    pub(crate) fn startup_paths_page(
        &self,
        pool: &str,
        after: i64,
        through: i64,
        limit: usize,
    ) -> Result<Vec<StartupPath>> {
        ensure!(limit > 0 && limit <= PATH_PAGE, "invalid startup path page");
        let mut statement = self.db.prepare(
            "SELECT w.sequence,f.id FROM wallets w JOIN funding_jobs f ON f.wallet_id=w.id WHERE w.pool_id=?1 AND w.sequence>?2 AND w.sequence<=?3 AND (w.role IN ('ALLOCATED','ACTIVE','READY')) ORDER BY w.sequence LIMIT ?4",
        )?;
        let rows = statement
            .query_map(
                params![pool, after, through, i64::try_from(limit)?],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(sequence, id)| {
                let (job, operation) = self.funding_recovery_context(&id)?;
                ensure!(job.pool_id == pool, "startup funding pool mismatch");
                let facts = self.funding_facts(&id, &job.operation_id)?;
                let destination_ready = funding::readiness_supported(&self.db, &job.wallet_id)?;
                Ok(StartupPath {
                    sequence,
                    job,
                    facts,
                    operation,
                    destination_ready,
                    recovery_count: self.funding_recovery_count(&id)?,
                    readiness_dispute: self.db.query_row(
                        "SELECT readiness_disputed FROM funding_jobs WHERE id=?1",
                        [&id],
                        |row| row.get(0),
                    )?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Store, String) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = store.ensure_pool("configured", "5").unwrap();
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
        let paths = store
            .startup_paths_page(&pool, -1, group.through_sequence, PATH_PAGE)
            .unwrap();
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
            store.ensure_pool(&format!("unrelated_{n}"), "5").unwrap();
        }
        let group = store
            .startup_groups(&["configured".into()])
            .unwrap()
            .remove(0);
        assert!(!group.serving_ready && !group.inventory_ready);
        let first = store
            .startup_paths_page(&pool, -1, group.through_sequence, 1)
            .unwrap();
        let second = store
            .startup_paths_page(&pool, first[0].sequence, group.through_sequence, 1)
            .unwrap();
        assert_eq!((first.len(), second.len()), (1, 1));
        assert_ne!(first[0].job.id, second[0].job.id);
        assert!(
            store
                .startup_paths_page(&pool, second[0].sequence, group.through_sequence, 1)
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .startup_groups(&vec!["configured".into(); GROUP_PAGE + 1])
                .is_err()
        );
        assert!(
            store
                .startup_paths_page(&pool, -1, group.through_sequence, PATH_PAGE + 1)
                .is_err()
        );
        assert!(store.startup_groups(&["absent".into()]).is_err());
    }
    #[test]
    fn disabled_group_and_disputed_destination_cannot_claim_ready_inventory() {
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
        assert!(!group.inventory_ready);
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
            }
            store
                .defer_funding(
                    &jobs[order[0]].id,
                    0,
                    Some("treasury_insufficient_spendable_funds; fixed worker result"),
                    false,
                )
                .unwrap();
            assert!(
                store
                    .startup_obstruction("configured", 3, true)
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
            assert_eq!(
                store
                    .startup_obstruction("configured", 3, true)
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
        let paths = store
            .startup_paths_page(&pool, -1, group.through_sequence, PATH_PAGE)
            .unwrap();
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
        assert!(
            store
                .startup_obstruction("configured", 3, true)
                .unwrap()
                .is_none()
        );
    }
}
