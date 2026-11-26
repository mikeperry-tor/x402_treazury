//! Observation demand derives from existing claims and transaction journals.
use super::*;
use std::collections::BTreeSet;

// Keep this lookup rooted in current slots, followed by the unique wallet/job
// binding. Historical allocations must not grow the serialized demand check.
// CROSS JOIN preserves that access order even as the funding journal grows.
const CURRENT_DEMAND: &str = "SELECT f.id,j.operation_id FROM pools p CROSS JOIN reserve_claims c ON c.pool_id=p.id CROSS JOIN funding_jobs f ON f.wallet_id=c.wallet_id AND f.pool_id=c.pool_id AND f.allocation_kind=c.slot AND f.claim_generation=c.generation CROSS JOIN funding_progress j ON j.job_id=f.id WHERE p.name=?1 AND p.enabled=1 AND c.enabled=1 AND f.state!='COMPLETE' AND f.readiness_requirement IS NULL";

const MAX_LOGGED_WALLETS: usize = 5;

/// Why a Zcash treasury wallet scan is running. Ordered by operator actionability
/// so the primary prose reason highlights actionable waits first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TreasurySyncReason {
    WalletFundingDegradedInsufficientZec,
    WalletFundingInsufficientZec,
    DirectSend,
    OutgoingTransaction,
    FundingRefund,
    DirectReceive,
    WalletFunding,
    StartupDiscovery,
    RequestedSync,
    PrepareTransaction,
    SubmitTransaction,
    ReconcileTransaction,
    RecoverExpired,
}

impl TreasurySyncReason {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::WalletFundingDegradedInsufficientZec => {
                "wallet_funding_degraded_insufficient_zec"
            }
            Self::WalletFundingInsufficientZec => "wallet_funding_insufficient_zec",
            Self::DirectSend => "direct_send",
            Self::OutgoingTransaction => "outgoing_transaction",
            Self::FundingRefund => "funding_refund",
            Self::DirectReceive => "direct_receive",
            Self::WalletFunding => "wallet_funding",
            Self::StartupDiscovery => "startup_discovery",
            Self::RequestedSync => "requested_sync",
            Self::PrepareTransaction => "prepare_transaction",
            Self::SubmitTransaction => "submit_transaction",
            Self::ReconcileTransaction => "reconcile_transaction",
            Self::RecoverExpired => "recover_expired",
        }
    }

    pub(crate) fn summary(self) -> &'static str {
        match self {
            Self::WalletFundingDegradedInsufficientZec => {
                "still waiting for ZEC to fund a wallet group with no funded USDC wallets; send ZEC to the 'wallet status --config FILE' receive address"
            }
            Self::WalletFundingInsufficientZec => {
                "still waiting for ZEC to fund reserve wallets; send ZEC to the 'wallet status --config FILE' receive address"
            }
            Self::DirectSend => "pending direct swap send",
            Self::OutgoingTransaction => "awaiting outgoing ZEC confirmation",
            Self::FundingRefund => "awaiting funding refund confirmation",
            Self::DirectReceive => "awaiting incoming ZEC confirmation",
            Self::WalletFunding => "pending wallet funding",
            Self::StartupDiscovery => "startup check",
            Self::RequestedSync => "requested sync",
            Self::PrepareTransaction => "preparing ZEC transaction",
            Self::SubmitTransaction => "submitting ZEC transaction",
            Self::ReconcileTransaction => "reconciling ZEC transaction",
            Self::RecoverExpired => "checking expired ZEC transaction",
        }
    }
}

/// Active observation reasons and associated wallet pool names for a Zcash scan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TreasuryObservationDemand {
    reasons: BTreeSet<TreasurySyncReason>,
    wallets: BTreeSet<String>,
}

impl TreasuryObservationDemand {
    pub(crate) fn from_reason(reason: TreasurySyncReason) -> Self {
        let mut reasons = BTreeSet::new();
        reasons.insert(reason);
        Self {
            reasons,
            wallets: BTreeSet::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.reasons.is_empty()
    }

    pub(crate) fn is_degraded(&self) -> bool {
        self.reasons
            .contains(&TreasurySyncReason::WalletFundingDegradedInsufficientZec)
    }

    #[cfg(test)]
    pub(crate) fn insert_reason(&mut self, reason: TreasurySyncReason) {
        self.reasons.insert(reason);
    }

    #[cfg(test)]
    pub(crate) fn insert_wallet(&mut self, wallet: impl Into<String>) {
        self.wallets.insert(wallet.into());
    }

    pub(crate) fn reason_codes(&self) -> String {
        self.reasons
            .iter()
            .map(|reason| reason.code())
            .collect::<Vec<_>>()
            .join(",")
    }

    pub(crate) fn summary(&self) -> String {
        let Some(primary) = self.reasons.iter().next().copied() else {
            return String::new();
        };
        let extra = self.reasons.len().saturating_sub(1);
        match extra {
            0 => primary.summary().to_owned(),
            1 => format!("{} (+1 other pending task)", primary.summary()),
            n => format!("{} (+{n} other pending tasks)", primary.summary()),
        }
    }

    pub(crate) fn formatted_wallets(&self) -> Option<String> {
        if self.wallets.is_empty() {
            return None;
        }
        let mut names: Vec<&str> = self
            .wallets
            .iter()
            .take(MAX_LOGGED_WALLETS)
            .map(String::as_str)
            .collect();
        let remaining = self.wallets.len().saturating_sub(MAX_LOGGED_WALLETS);
        if remaining > 0 {
            let overflow = format!("+{remaining}");
            names.push(&overflow);
            return Some(names.join(","));
        }
        Some(names.join(","))
    }
}

impl StoreHandle {
    pub(crate) fn treasury_changes(&self) -> std::sync::Arc<tokio::sync::Notify> {
        self.treasury_changes.clone()
    }
}
impl Store {
    pub(crate) fn treasury_observation_demand(
        &self,
        configured: &[String],
        bootstrap: Option<&crate::rotation::assignment::BootstrapPolicy>,
    ) -> Result<TreasuryObservationDemand> {
        #[cfg(test)]
        self.inventory_read_fault(inventory::Read::TreasuryDemand)?;
        let mut demand = TreasuryObservationDemand::default();
        // A validated transaction can require confirmation/expiry evidence even
        // after destination credit, stale phases, or removed configuration.
        let mut prepared = self.db.prepare(
            "SELECT DISTINCT p.name FROM outgoing x JOIN treasury_operations t ON t.id=x.id LEFT JOIN funding_progress j ON j.operation_id=x.id LEFT JOIN funding_recovery r ON r.operation_id=x.id LEFT JOIN funding_jobs f ON f.id=COALESCE(j.job_id,r.job_id) LEFT JOIN pools p ON p.id=f.pool_id WHERE x.state='PREPARED'",
        )?;
        let mut accepted = false;
        for row in prepared.query_map([], |r| r.get::<_, Option<String>>(0))? {
            accepted = true;
            if let Some(pool) = row? {
                demand.wallets.insert(pool);
            }
        }
        if accepted {
            demand
                .reasons
                .insert(TreasurySyncReason::OutgoingTransaction);
        }
        if let Some(refund_pools) = check_or_keep(&demand, self.funding_refund_scan_pools())?
            && !refund_pools.is_empty()
        {
            demand.reasons.insert(TreasurySyncReason::FundingRefund);
            demand.wallets.extend(refund_pools);
        }
        if check_or_keep(&demand, self.direct_receive_needed())? == Some(true) {
            demand.reasons.insert(TreasurySyncReason::DirectReceive);
        }
        if check_or_keep(&demand, self.direct_source_needed())? == Some(true) {
            demand.reasons.insert(TreasurySyncReason::DirectSend);
        }
        // Invalid published bytes keep their owner. Unpublished intent history
        // cannot hide an unrelated current claim after SDK reconstruction.
        if check_or_keep(&demand, self.preparation_pending())?.unwrap_or(true)
            || self.funding_restriction != super::super::restriction::FundingRestriction::Ordinary
        {
            return Ok(demand);
        }
        let current = self.collect_current_demand(configured, bootstrap, &mut demand);
        let _ = check_or_keep(&demand, current)?;
        if demand
            .reasons
            .contains(&TreasurySyncReason::WalletFundingDegradedInsufficientZec)
        {
            demand
                .reasons
                .remove(&TreasurySyncReason::WalletFundingInsufficientZec);
            demand.reasons.remove(&TreasurySyncReason::WalletFunding);
        } else if demand
            .reasons
            .contains(&TreasurySyncReason::WalletFundingInsufficientZec)
        {
            demand.reasons.remove(&TreasurySyncReason::WalletFunding);
        }
        Ok(demand)
    }

    fn collect_current_demand(
        &self,
        configured: &[String],
        bootstrap: Option<&crate::rotation::assignment::BootstrapPolicy>,
        demand: &mut TreasuryObservationDemand,
    ) -> Result<()> {
        for name in configured {
            let mut query = self.db.prepare(CURRENT_DEMAND)?;
            for entry in query.query_map([name], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })? {
                let (id, operation) = entry?;
                match self.funding_facts(&id, &operation) {
                    Ok(facts) if facts.new_funding_allowed => {
                        let insufficient = self.funding_conditions.get(&id).is_some_and(
                            |(recorded, condition)| {
                                recorded == &operation
                                    && *condition
                                        == super::super::error::FundingCondition::InsufficientFunds
                            },
                        );
                        let reason = if insufficient {
                            if self.pool_shortfall_is_degraded(name, bootstrap)? {
                                TreasurySyncReason::WalletFundingDegradedInsufficientZec
                            } else {
                                TreasurySyncReason::WalletFundingInsufficientZec
                            }
                        } else {
                            TreasurySyncReason::WalletFunding
                        };
                        demand.reasons.insert(reason);
                        demand.wallets.insert(name.clone());
                    }
                    Err(error) if error.is::<scheduling::FundingDecodeFailure>() => {
                        // The funding owner reports this row's scoped payload
                        // failure. It cannot hide independent valid claims.
                        self.scope_funding_decode_failure(&id, error)?;
                    }
                    Err(error) => return Err(error),
                    Ok(_) => {}
                }
            }
        }
        Ok(())
    }

    /// True when `pool_name` has no funded, non-`low_balance` USDC wallet and
    /// no incomplete `ALLOCATED` claim already settling on NEAR (accounting for
    /// pre-credit `[wallet_bootstrap]` fallback to `bootstrap_group`), so a
    /// treasury ZEC shortfall leaves the group unable to serve calls.
    pub(crate) fn pool_shortfall_is_degraded(
        &self,
        pool_name: &str,
        bootstrap: Option<&crate::rotation::assignment::BootstrapPolicy>,
    ) -> Result<bool> {
        let pool_id: Option<String> = self
            .db
            .query_row("SELECT id FROM pools WHERE name=?1", [pool_name], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(pool_id) = pool_id else {
            return Ok(true);
        };
        if self.pool_has_funded_or_settling(&pool_id)? {
            return Ok(false);
        }
        // A deferred group that has not yet received its first initial credit
        // still serves API calls via its configured bootstrap_group.
        if let Some(policy) = bootstrap
            && policy
                .deferred_groups
                .iter()
                .any(|group| group == pool_name)
            && !self.has_initial_credit(&pool_id)?
        {
            let bootstrap_id: Option<String> = self
                .db
                .query_row(
                    "SELECT id FROM pools WHERE name=?1",
                    [&policy.bootstrap_group],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(bootstrap_id) = bootstrap_id
                && self.pool_has_funded_or_settling(&bootstrap_id)?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn pool_has_funded_or_settling(&self, pool_id: &str) -> Result<bool> {
        Ok(self.direct_pool_has_funded_wallet_by_id(pool_id)?
            || self.pool_has_settling_claim(pool_id)?)
    }

    fn direct_pool_has_funded_wallet_by_id(&self, pool_id: &str) -> Result<bool> {
        let mut query = self.db.prepare(
            "SELECT w.balance FROM wallets w WHERE w.pool_id=?1 AND w.low_balance=0 AND (w.role IN ('ACTIVE','RETAINED') OR (w.role='READY' AND EXISTS(SELECT 1 FROM reserve_claims c WHERE c.wallet_id=w.id AND c.enabled=1)))",
        )?;
        let mut rows = query.query([pool_id])?;
        while let Some(row) = rows.next()? {
            if row
                .get_ref(0)?
                .as_str()
                .ok()
                .and_then(|value| amount(value).ok())
                .is_some_and(|value| value > U256::ZERO)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn pool_has_settling_claim(&self, pool_id: &str) -> Result<bool> {
        let mut jobs = self.db.prepare(
            "SELECT f.id FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id WHERE f.pool_id=?1 AND w.role='ALLOCATED' AND f.state!='COMPLETE' AND f.readiness_requirement IS NULL ORDER BY f.id",
        )?;
        for entry in jobs.query_map([pool_id], |r| r.get::<_, String>(0))? {
            let id = entry?;
            let path = match self.startup_path(&id) {
                Ok(path) => path,
                Err(error) if error.is::<scheduling::FundingDecodeFailure>() => {
                    self.scope_funding_decode_failure(&id, error)?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            if is_settling_claim(&path) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Returns a machine obstruction reason when a managed pool is disabled or
    /// has no funded, non-`low_balance` USDC wallets, no settling swap, no
    /// vacant enabled claim able to progress, and every incomplete enabled
    /// `ALLOCATED` funding claim is blocked from progressing in-process.
    pub(crate) fn pool_funding_obstruction(&self, pool_id: &str) -> Result<Option<&'static str>> {
        let enabled: bool = self
            .db
            .query_row(
                "SELECT enabled=1 AND payment_limit IS NOT NULL FROM pools WHERE id=?1",
                [pool_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false);
        if !enabled {
            return Ok(Some("funding_group_disabled"));
        }
        if self.direct_pool_has_funded_wallet_by_id(pool_id)? {
            return Ok(None);
        }
        let treasury_source_pending = self.preparation_pending()?;
        let mut jobs = self.db.prepare(
            "SELECT f.id, c.target FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id LEFT JOIN reserve_claims c ON c.pool_id=f.pool_id AND c.slot=f.allocation_kind AND c.generation=f.claim_generation AND c.wallet_id=f.wallet_id AND c.enabled=1 WHERE f.pool_id=?1 AND w.role='ALLOCATED' AND f.state!='COMPLETE' AND f.readiness_requirement IS NULL ORDER BY f.id",
        )?;
        let mut obstructed: Option<&'static str> = None;
        let mut min_insufficient_target: Option<U256> = None;
        for entry in jobs.query_map([pool_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })? {
            let (id, claim_target) = entry?;
            let path = match self.startup_path(&id) {
                Ok(path) => path,
                Err(error) if error.is::<scheduling::FundingDecodeFailure>() => {
                    self.scope_funding_decode_failure(&id, error)?;
                    if claim_target.is_some() {
                        obstructed.get_or_insert("funding_data_requires_review");
                    }
                    continue;
                }
                Err(error) => return Err(error),
            };
            // A disabled reserve claim cannot start new funding attempts; it
            // only keeps the pool waiting when an in-flight swap can still
            // settle and publish as RETAINED.
            let Some(claim_target) = claim_target else {
                if is_settling_claim(&path) {
                    return Ok(None);
                }
                continue;
            };
            let closed_expired = path.facts.source_closed
                && path
                    .operation
                    .as_ref()
                    .is_some_and(|op| op.submission == "EXPIRED");
            let disclosed_spend = !path.facts.source_never_disclosed()
                && !closed_expired
                && !path.facts.preparation_accepted
                && !path.facts.source_pending
                && (path.facts.source_submission.is_some() || path.facts.source_attempts > 0);
            if !path.destination_ready
                && !path.facts.claim_fulfilled
                && disclosed_spend
                && !path.facts.provider_refunded
                && (path.facts.provider_failed || path.facts.provider_incomplete)
            {
                obstructed.get_or_insert("funding_outcome_requires_review");
                continue;
            }
            match path.disposition(treasury_source_pending) {
                super::bootstrap::StartupDisposition::DestinationReady
                | super::bootstrap::StartupDisposition::Waiting
                | super::bootstrap::StartupDisposition::Actionable => return Ok(None),
                super::bootstrap::StartupDisposition::Obstructed(reason) => {
                    if reason == "treasury_insufficient_spendable_funds" {
                        obstructed = Some(reason);
                        if let Ok(target) = amount(&claim_target) {
                            min_insufficient_target = Some(
                                min_insufficient_target
                                    .map_or(target, |current| current.min(target)),
                            );
                        }
                    } else {
                        obstructed.get_or_insert(reason);
                    }
                }
            }
        }
        if let Some(reason) = obstructed {
            if self.funding_restriction == super::super::restriction::FundingRestriction::Ordinary
                && self.funding_permits.is_none()
                && self.has_actionable_vacant_claim(pool_id, reason, min_insufficient_target)?
            {
                return Ok(None);
            }
            return Ok(Some(reason));
        }
        let has_enabled_claims: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM reserve_claims WHERE pool_id=?1 AND enabled=1)",
            [pool_id],
            |r| r.get(0),
        )?;
        if !has_enabled_claims {
            return Ok(Some("funding_group_disabled"));
        }
        match self.funding_restriction {
            super::super::restriction::FundingRestriction::Ordinary => Ok(None),
            super::super::restriction::FundingRestriction::Paused => Ok(Some("funding_disabled")),
            super::super::restriction::FundingRestriction::DenyNewFunding => {
                Ok(Some("qualification_funding_denied"))
            }
        }
    }

    fn has_actionable_vacant_claim(
        &self,
        pool_id: &str,
        obstructed_reason: &str,
        min_insufficient_target: Option<U256>,
    ) -> Result<bool> {
        let mut vacant = self.db.prepare(
            "SELECT target FROM reserve_claims WHERE pool_id=?1 AND enabled=1 AND wallet_id IS NULL",
        )?;
        let mut rows = vacant.query([pool_id])?;
        while let Some(row) = rows.next()? {
            if obstructed_reason != "treasury_insufficient_spendable_funds" {
                return Ok(true);
            }
            let Some(min_target) = min_insufficient_target else {
                continue;
            };
            if row
                .get_ref(0)?
                .as_str()
                .ok()
                .and_then(|value| amount(value).ok())
                .is_some_and(|target| target < min_target)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[cfg(test)]
    pub(crate) fn treasury_observation_needed(&self, configured: &[String]) -> Result<bool> {
        Ok(!self
            .treasury_observation_demand(configured, None)?
            .is_empty())
    }
}

/// Preserve already-proven observation demand if a later unrelated probe hits a
/// non-busy error; transient busy errors still propagate for retry deferral.
fn check_or_keep<T>(demand: &TreasuryObservationDemand, result: Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if !demand.is_empty() && !inventory::transient_read(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn is_settling_claim(path: &super::bootstrap::StartupPath) -> bool {
    path.destination_ready
        || path.facts.claim_fulfilled
        || (path.facts.credit_needed()
            && !path.facts.provider_failed
            && !path.facts.provider_refunded
            && !path.facts.provider_incomplete
            && path.facts.source_submission.as_deref() != Some("EXPIRED"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        (dir, store)
    }
    fn allocate(store: &mut Store) -> funding::FundingJob {
        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "pool",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        store.funding_jobs().unwrap().remove(0)
    }
    fn prepared(store: &mut Store, job: &funding::FundingJob, valid: bool) {
        store
            .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
            .unwrap();
        let revision = store.snapshot().unwrap().0;
        store
            .prepare_with_facts(
                &job.operation_id,
                revision,
                b"next",
                crate::rotation::store::test_support::transaction_raw(100).as_slice(),
                valid.then(|| TransactionFacts {
                    txid: crate::rotation::store::test_support::transaction_txid(100),
                    expiry_height: 100,
                    amount_zatoshis: 50,
                    fee_zatoshis: 10,
                    deadline: Some(10000),
                }),
            )
            .unwrap();
    }
    #[test]
    fn current_claims_need_configured_authority_but_destination_waiting_does_not_scan_zec() {
        let (_dir, mut store) = fixture();
        let job = allocate(&mut store);
        let groups = vec!["pool".to_owned()];
        assert!(store.treasury_observation_needed(&groups).unwrap());
        assert!(!store.treasury_observation_needed(&[]).unwrap());
        prepared(&mut store, &job, true);
        // Source submission consumes new-deposit eligibility, independently of
        // destination credit/provider observations still needed after confirmation.
        store
            .db
            .execute(
                "UPDATE treasury_operations SET attempts=1 WHERE id=?1",
                [&job.operation_id],
            )
            .unwrap();
        store.confirm_spend(&job.operation_id).unwrap();
        let facts = store.funding_facts(&job.id, &job.operation_id).unwrap();
        assert!(facts.credit_needed() && !facts.new_funding_allowed);
        assert!(!store.treasury_observation_needed(&groups).unwrap());
    }
    #[test]
    fn completed_uncredited_claims_do_not_scan_but_signed_work_and_new_claims_do() {
        let (_dir, mut store) = fixture();
        let job = allocate(&mut store);
        let groups = vec!["pool".to_owned()];
        assert!(store.treasury_observation_needed(&groups).unwrap());
        // A historical completion can retain an uncredited current slot. It
        // must not request scans for work the funding inventory excludes.
        store
            .db
            .execute(
                "UPDATE funding_jobs SET state='COMPLETE' WHERE id=?1",
                [&job.id],
            )
            .unwrap();
        store
            .db
            .execute(
                "UPDATE funding_progress SET phase='\"COMPLETE\"' WHERE job_id=?1",
                [&job.id],
            )
            .unwrap();
        assert!(!store.treasury_observation_needed(&groups).unwrap());
        assert!(
            store
                .funding_work_page("", store.funding_work_boundary().unwrap())
                .unwrap()
                .is_empty()
        );
        assert!(
            !store
                .funding_facts(&job.id, &job.operation_id)
                .unwrap()
                .initial_credit_recorded
        );
        assert!(store.prepared_bytes(&job.operation_id).is_err());

        // Completion labels cannot abandon validated signed liabilities.
        prepared(&mut store, &job, true);
        assert!(store.treasury_observation_needed(&[]).unwrap());
        assert_eq!(
            store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
            crate::rotation::store::test_support::transaction_raw(100).as_slice()
        );
        store.confirm_spend(&job.operation_id).unwrap();
        assert!(!store.treasury_observation_needed(&groups).unwrap());

        // A replacement claim independently requests discovery again.
        store
            .db
            .execute_batch("UPDATE reserve_claims SET wallet_id=NULL,generation=generation+1")
            .unwrap();
        assert!(
            store
                .allocate_capacity_kind(&job.pool_id, super::super::AllocationKind::Small, true)
                .unwrap()
                .is_some()
        );
        assert!(store.treasury_observation_needed(&groups).unwrap());
    }
    #[test]
    fn demand_reads_current_bindings_without_decoding_historical_jobs() {
        for transition in [
            "UPDATE reserve_claims SET wallet_id=NULL,generation=generation+1",
            "UPDATE reserve_claims SET generation=generation+1",
            "UPDATE reserve_claims SET enabled=0",
            "UPDATE reserve_claims SET slot='large'",
        ] {
            let (_dir, mut store) = fixture();
            let old = allocate(&mut store);
            store.db.execute_batch(transition).unwrap();
            store
                .db
                .execute(
                    "UPDATE wallets SET role='RETIRED' WHERE id=?1",
                    [&old.wallet_id],
                )
                .unwrap();
            // A decoder sentinel: even extracting the operation identity fails
            // if demand enumerates this obsolete allocation before filtering it.
            store
                .db
                .execute(
                    "UPDATE funding_progress SET operation_id=X'FF' WHERE job_id=?1",
                    [&old.id],
                )
                .unwrap();
            assert!(!store.treasury_observation_needed(&["pool".into()]).unwrap());

            // Restore a vacant enabled slot and bind a replacement allocation;
            // the obsolete journal must not hide the new claim's demand.
            store.db.execute_batch("UPDATE reserve_claims SET enabled=1,slot='small',wallet_id=NULL,generation=generation+1").unwrap();
            let replacement = store
                .allocate_capacity_kind(&old.pool_id, super::super::AllocationKind::Small, true)
                .unwrap();
            assert!(replacement.is_some());
            assert!(store.treasury_observation_needed(&["pool".into()]).unwrap());
        }
    }
    #[test]
    fn current_demand_uses_point_lookups_instead_of_funding_history() {
        let (_dir, mut store) = fixture();
        allocate(&mut store);
        let mut query = store
            .db
            .prepare(&format!("EXPLAIN QUERY PLAN {CURRENT_DEMAND}"))
            .unwrap();
        let plan = query
            .query_map(["pool"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for (table, key) in [
            ("p", "name=?"),
            ("c", "pool_id=?"),
            ("f", "wallet_id=?"),
            ("j", "job_id=?"),
        ] {
            assert!(plan.iter().any(|step| step.starts_with(&format!("SEARCH {table} ")) && step.contains(key)), "{plan:?}");
        }
    }
    #[test]
    fn validated_pending_work_survives_credit_stale_phase_and_removed_configuration() {
        let (_dir, mut store) = fixture();
        let job = allocate(&mut store);
        prepared(&mut store, &job, true);
        store
            .record_funding_balance(
                &job.id,
                &job.operation_id,
                &job.wallet_id,
                &job.recipient,
                U256::from(1),
            )
            .unwrap();
        store
            .db
            .execute(
                "UPDATE funding_progress SET phase='\"COMPLETE\"' WHERE job_id=?1",
                [&job.id],
            )
            .unwrap();
        store.disable_pool("pool").unwrap();
        store.deny_new_funding();
        assert!(store.treasury_observation_needed(&[]).unwrap());
        assert_eq!(
            store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
            crate::rotation::store::test_support::transaction_raw(100).as_slice()
        );
    }
    #[test]
    fn unpublished_intent_does_not_hide_claims_but_invalid_published_bytes_keep_their_fence() {
        for publish_invalid in [false, true] {
            let (_dir, mut store) = fixture();
            let job = allocate(&mut store);
            if publish_invalid {
                prepared(&mut store, &job, false);
            } else {
                store
                    .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
                    .unwrap();
            }
            assert_eq!(
                store
                    .treasury_observation_needed(&["pool".to_owned()])
                    .unwrap(),
                !publish_invalid
            );
            assert!(
                store
                    .require_unstarted_preparation(&job.operation_id)
                    .is_err()
            );
            assert!(store.abandon_unprepared(&job.operation_id).is_err());
            assert_eq!(
                store.require_preparation_available().is_err(),
                publish_invalid
            );
        }
    }
    #[test]
    fn malformed_provider_row_cannot_hide_valid_siblings_or_force_discovery_alone() {
        let (_dir, mut store) = fixture();
        let malformed = allocate(&mut store);
        store
            .db
            .execute(
                "UPDATE funding_progress SET provider_success='invalid' WHERE job_id=?1",
                [&malformed.id],
            )
            .unwrap();
        assert!(
            store
                .treasury_observation_needed(&["pool".to_owned()])
                .unwrap()
        );
        // An unknown optional provider hint cannot suppress a current enabled
        // funding claim, or invent demand outside configured groups.
        assert!(!store.treasury_observation_needed(&[]).unwrap());
        assert!(
            !store
                .funding_facts(&malformed.id, &malformed.operation_id)
                .unwrap()
                .provider_success
        );
        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "sibling",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        assert!(
            store
                .treasury_observation_needed(&["pool".to_owned(), "sibling".to_owned()])
                .unwrap()
        );
        // Valid signed ownership is observed independently of malformed provider flags.
        prepared(&mut store, &malformed, true);
        assert!(store.treasury_observation_needed(&[]).unwrap());
    }
    #[tokio::test]
    async fn capacity_notification_retains_a_wakeup_before_the_waiter_registers() {
        let (_dir, mut store) = fixture();
        let changed = store.treasury_changes.clone();
        allocate(&mut store);
        use futures_util::FutureExt;
        assert!(changed.notified().now_or_never().is_some());
        assert!(changed.notified().now_or_never().is_none());
    }
    #[test]
    fn observation_demand_reports_reasons_wallets_and_shortfall_priority() {
        let (_dir, mut store) = fixture();
        let job = allocate(&mut store);
        let groups = vec!["pool".to_owned()];

        // Pending managed funding without an insufficient-funds condition.
        let demand = store.treasury_observation_demand(&groups, None).unwrap();
        assert_eq!(demand.reason_codes(), "wallet_funding");
        assert_eq!(demand.formatted_wallets().as_deref(), Some("pool"));
        assert!(!demand.summary().contains("other pending"));

        // Recording InsufficientFunds on a pool with no usable wallet reports a
        // degraded wallet group shortfall.
        store
            .observe_funding_condition(
                &job.id,
                &job.operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
            )
            .unwrap();
        let demand = store.treasury_observation_demand(&groups, None).unwrap();
        assert_eq!(
            demand.reason_codes(),
            "wallet_funding_degraded_insufficient_zec"
        );
        assert_eq!(demand.formatted_wallets().as_deref(), Some("pool"));

        // Adding a usable ACTIVE wallet shifts the shortfall to reserve funding.
        store
            .db
            .execute(
                "INSERT INTO wallets(id,pool_id,sequence,address,key,target,role,balance,low_balance) VALUES ('active-1',?1,99,'0x0000000000000000000000000000000000000001',X'01','5000000','ACTIVE','5000000',0)",
                [&job.pool_id],
            )
            .unwrap();
        let demand = store.treasury_observation_demand(&groups, None).unwrap();
        assert_eq!(demand.reason_codes(), "wallet_funding_insufficient_zec");
        assert_eq!(demand.formatted_wallets().as_deref(), Some("pool"));

        // Marking the active wallet low_balance, zero, empty, or malformed degrades the group.
        store
            .db
            .execute("UPDATE wallets SET low_balance=1 WHERE id='active-1'", [])
            .unwrap();
        assert_eq!(
            store
                .treasury_observation_demand(&groups, None)
                .unwrap()
                .reason_codes(),
            "wallet_funding_degraded_insufficient_zec"
        );
        for balance in ["0", "", "unknown"] {
            store
                .db
                .execute(
                    "UPDATE wallets SET low_balance=0,balance=?1 WHERE id='active-1'",
                    [balance],
                )
                .unwrap();
            assert_eq!(
                store
                    .treasury_observation_demand(&groups, None)
                    .unwrap()
                    .reason_codes(),
                "wallet_funding_degraded_insufficient_zec"
            );
        }
        store
            .db
            .execute(
                "UPDATE wallets SET balance='5000000' WHERE id='active-1'",
                [],
            )
            .unwrap();

        // A sibling pool with ordinary pending wallet_funding collapses into the
        // active shortfall reason while still listing both pools in wallets.
        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "alpha",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let both = vec!["pool".to_owned(), "alpha".to_owned()];
        let demand = store.treasury_observation_demand(&both, None).unwrap();
        assert_eq!(demand.reason_codes(), "wallet_funding_insufficient_zec");
        assert_eq!(demand.formatted_wallets().as_deref(), Some("alpha,pool"));
        assert!(!demand.summary().contains("other pending"));

        // If the sibling pool also hits InsufficientFunds without a usable wallet,
        // degraded takes precedence and collapses the reserve shortfall reason.
        let sibling = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.pool_name == "alpha")
            .unwrap();
        store
            .observe_funding_condition(
                &sibling.id,
                &sibling.operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
            )
            .unwrap();
        let demand = store.treasury_observation_demand(&both, None).unwrap();
        assert_eq!(
            demand.reason_codes(),
            "wallet_funding_degraded_insufficient_zec"
        );
        assert_eq!(demand.formatted_wallets().as_deref(), Some("alpha,pool"));
        assert!(!demand.summary().contains("other pending"));

        // Preparing the sibling's outgoing transaction serializes new funding
        // claims behind that outgoing work, so only OutgoingTransaction applies.
        prepared(&mut store, &sibling, true);
        let demand = store.treasury_observation_demand(&both, None).unwrap();
        assert_eq!(demand.reason_codes(), "outgoing_transaction");
        assert_eq!(demand.formatted_wallets().as_deref(), Some("alpha"));
        assert!(!demand.summary().contains("other pending"));

        // Distinct task types preserve actionability order and task-count suffixes.
        let mut two = demand.clone();
        two.insert_reason(TreasurySyncReason::WalletFundingInsufficientZec);
        assert_eq!(
            two.reason_codes(),
            "wallet_funding_insufficient_zec,outgoing_transaction"
        );
        assert!(two.summary().ends_with("(+1 other pending task)"));

        let mut three = demand;
        three.insert_reason(TreasurySyncReason::WalletFundingDegradedInsufficientZec);
        three.insert_reason(TreasurySyncReason::DirectReceive);
        assert_eq!(
            three.reason_codes(),
            "wallet_funding_degraded_insufficient_zec,outgoing_transaction,direct_receive"
        );
        assert!(three.summary().ends_with("(+2 other pending tasks)"));
    }
    #[test]
    fn observation_demand_bounds_logged_wallet_names() {
        let mut demand = TreasuryObservationDemand::from_reason(TreasurySyncReason::WalletFunding);
        for name in ["w1", "w2", "w3", "w4", "w5", "w6", "w7"] {
            demand.wallets.insert(name.to_owned());
        }
        assert_eq!(
            demand.formatted_wallets().as_deref(),
            Some("w1,w2,w3,w4,w5,+2")
        );
    }
    #[test]
    fn observation_demand_collects_refund_pools_and_isolates_later_probe_errors() {
        let (_dir, mut store) = fixture();
        let job = allocate(&mut store);
        prepared(&mut store, &job, true);
        store
            .db
            .execute(
                "UPDATE treasury_operations SET attempts=1 WHERE id=?1",
                [&job.operation_id],
            )
            .unwrap();
        store.confirm_spend(&job.operation_id).unwrap();
        store
            .db
            .execute(
                "UPDATE funding_progress SET quote=?2,provider_refunded=1 WHERE job_id=?1",
                params![
                    &job.id,
                    br#"{"request":{"refundTo":"u1refund"}}"#.as_slice()
                ],
            )
            .unwrap();
        let demand = store.treasury_observation_demand(&[], None).unwrap();
        assert_eq!(demand.reason_codes(), "funding_refund");
        assert_eq!(demand.formatted_wallets().as_deref(), Some("pool"));

        // An unrelated corrupt direct_operations row cannot hide already-proven
        // refund or outgoing observation demand.
        store
            .db
            .execute(
                "INSERT INTO direct_operations(id,owner,kind,request_id,payload,observe_receive) VALUES ('corrupt','owner','receive','req',X'FF',1)",
                [],
            )
            .unwrap();
        let demand = store.treasury_observation_demand(&[], None).unwrap();
        assert_eq!(demand.reason_codes(), "funding_refund");
        assert_eq!(demand.formatted_wallets().as_deref(), Some("pool"));
    }
    #[tokio::test]
    async fn pool_funding_obstruction_tracks_shortfall_and_wakes_waiters() {
        use futures_util::FutureExt;
        let (_dir, mut store) = fixture();
        let job = allocate(&mut store);
        assert_eq!(store.pool_funding_obstruction(&job.pool_id).unwrap(), None);

        let changes = store.capacity_changes(&job.pool_id);
        let notified = changes.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        store
            .observe_funding_condition(
                &job.id,
                &job.operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
            )
            .unwrap();
        assert!(notified.now_or_never().is_some());
        assert_eq!(
            store.pool_funding_obstruction(&job.pool_id).unwrap(),
            Some("treasury_insufficient_spendable_funds")
        );

        // An in-flight prepared outgoing transaction (even on a sibling pool)
        // clears the shortfall obstruction because treasury ZEC notes are shared.
        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "sibling",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let sibling = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.pool_name == "sibling")
            .unwrap();
        prepared(&mut store, &sibling, true);
        assert_eq!(store.pool_funding_obstruction(&job.pool_id).unwrap(), None);

        // Confirming the sibling's spend clears stale InsufficientFunds (in case
        // a concurrent deposit arrived in the same scan) and wakes parked
        // waiters so they wait for a fresh funding assessment.
        let after_sibling = changes.notified();
        tokio::pin!(after_sibling);
        after_sibling.as_mut().enable();
        store
            .db
            .execute(
                "UPDATE treasury_operations SET attempts=1 WHERE id=?1",
                [&sibling.operation_id],
            )
            .unwrap();
        store.confirm_spend(&sibling.operation_id).unwrap();
        assert!(after_sibling.now_or_never().is_some());
        assert_eq!(store.pool_funding_obstruction(&job.pool_id).unwrap(), None);

        // Re-observing InsufficientFunds after the fresh assessment restores
        // the shortfall obstruction and wakes waiters again.
        let after_reassess = changes.notified();
        tokio::pin!(after_reassess);
        after_reassess.as_mut().enable();
        store
            .observe_funding_condition(
                &job.id,
                &job.operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
            )
            .unwrap();
        assert!(after_reassess.now_or_never().is_some());
        assert_eq!(
            store.pool_funding_obstruction(&job.pool_id).unwrap(),
            Some("treasury_insufficient_spendable_funds")
        );

        // Expiring a sibling outgoing transaction unlocks its spent input note,
        // so it must also clear stale InsufficientFunds rather than falsely
        // obstructing waiters before FundingWorker re-assesses, while still
        // preserving non-shortfall obstructions (such as CostLimit).
        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "expiring",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let expiring = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.pool_name == "expiring")
            .unwrap();
        store
            .accept_preparation(&expiring.operation_id, Some(&expiring.pool_id), 100, None)
            .unwrap();
        let base_revision: i64 = store
            .db
            .query_row(
                "SELECT revision FROM snapshots ORDER BY revision DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        store
            .prepare_with_facts(
                &expiring.operation_id,
                base_revision,
                b"expired-snap",
                crate::rotation::store::test_support::transaction_raw(20).as_slice(),
                Some(TransactionFacts {
                    txid: crate::rotation::store::test_support::transaction_txid(20),
                    expiry_height: 20,
                    amount_zatoshis: 80,
                    fee_zatoshis: 20,
                    deadline: Some(1000),
                }),
            )
            .unwrap();
        let next_revision = base_revision + 1;
        let expiring_revision = store
            .save_sync_snapshot(
                next_revision,
                b"expired-sync",
                Some(SyncObservation {
                    phase: SyncPhase::Ready,
                    last_error: None,
                    snapshot_revision: next_revision + 1,
                    started_snapshot_revision: None,
                    checked_at: Some(100),
                    checkpoint_at: 100,
                    scanned_blocks: 30,
                    target_height: Some(30),
                    observed_tip_height: Some(30),
                    height: Some(30),
                    confirmations: 2,
                    confirmed_pool_balances_zatoshis: None,
                    confirmed_shielded_zatoshis: 1000,
                    spendable_shielded_zatoshis: 1000,
                }),
            )
            .unwrap();
        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "policy_blocked",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let policy_job = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.pool_name == "policy_blocked")
            .unwrap();
        store
            .observe_funding_condition(
                &policy_job.id,
                &policy_job.operation_id,
                Some(crate::rotation::error::FundingCondition::CostLimit),
            )
            .unwrap();
        assert_eq!(store.pool_funding_obstruction(&job.pool_id).unwrap(), None);
        let policy_changes = store.capacity_changes(&policy_job.pool_id);
        let after_expiry_policy = policy_changes.notified();
        tokio::pin!(after_expiry_policy);
        after_expiry_policy.as_mut().enable();
        let after_expiry = changes.notified();
        tokio::pin!(after_expiry);
        after_expiry.as_mut().enable();
        store
            .resolve_expired(&expiring.operation_id, expiring_revision)
            .unwrap();
        assert!(after_expiry.now_or_never().is_some());
        assert!(after_expiry_policy.now_or_never().is_some());
        assert_eq!(store.pool_funding_obstruction(&job.pool_id).unwrap(), None);
        assert_eq!(
            store.pool_funding_obstruction(&policy_job.pool_id).unwrap(),
            Some("funding_policy_requires_review")
        );

        // A confirmed ZEC swap settling on NEAR in another slot of the same
        // pool keeps obstruction None and demand non-degraded even while the
        // first slot has InsufficientFunds.
        store
            .ensure_capacity_group(
                "pool",
                &crate::rotation::test_capacity::two_reserves("5"),
                Some(U256::from(1_000_000u64)),
                0,
            )
            .unwrap();
        let large_id = store
            .allocate_capacity_kind(&job.pool_id, super::super::AllocationKind::Large, true)
            .unwrap()
            .unwrap();
        let large_job = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.id == large_id)
            .unwrap();
        prepared(&mut store, &large_job, true);
        store
            .db
            .execute(
                "UPDATE treasury_operations SET attempts=1 WHERE id=?1",
                [&large_job.operation_id],
            )
            .unwrap();
        store.confirm_spend(&large_job.operation_id).unwrap();
        store
            .observe_funding_condition(
                &job.id,
                &job.operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
            )
            .unwrap();
        assert_eq!(store.pool_funding_obstruction(&job.pool_id).unwrap(), None);
        let demand = store
            .treasury_observation_demand(&["pool".to_owned()], None)
            .unwrap();
        assert_eq!(demand.reason_codes(), "wallet_funding_insufficient_zec");
        assert!(!demand.is_degraded());

        // When the settling sibling slot fails on NEAR, capacity waiters wake
        // up and the pool becomes obstructed and degraded.
        store
            .db
            .execute(
                "UPDATE funding_progress SET quote='{}' WHERE job_id=?1",
                [&large_job.id],
            )
            .unwrap();
        let after_fail = changes.notified();
        tokio::pin!(after_fail);
        after_fail.as_mut().enable();
        store
            .record_provider_outcome(
                &large_job.id,
                &large_job.operation_id,
                &crate::rotation::near::SwapStatus::Failed,
                10,
            )
            .unwrap();
        assert!(after_fail.now_or_never().is_some());
        assert_eq!(
            store.pool_funding_obstruction(&job.pool_id).unwrap(),
            Some("treasury_insufficient_spendable_funds")
        );
        let demand = store
            .treasury_observation_demand(&["pool".to_owned()], None)
            .unwrap();
        assert!(
            demand
                .reason_codes()
                .contains("wallet_funding_degraded_insufficient_zec"),
            "{}",
            demand.reason_codes()
        );
        assert!(demand.is_degraded());
    }

    #[test]
    fn pool_funding_obstruction_detects_paused_policy_and_incomplete_deposit_blocks() {
        let (_dir, mut store) = fixture();
        let job = allocate(&mut store);

        // Pausing funding (auto_fund = false or store-only mode) obstructs an
        // existing unprepared ALLOCATED wallet instead of treating it as settling.
        store.pause_new_funding();
        assert_eq!(
            store.pool_funding_obstruction(&job.pool_id).unwrap(),
            Some("funding_disabled")
        );

        // Policy obstructions such as AmountLimit unblock even when auto_fund is on.
        let (_dir2, mut policy_store) = fixture();
        let policy_job = allocate(&mut policy_store);
        policy_store
            .observe_funding_condition(
                &policy_job.id,
                &policy_job.operation_id,
                Some(crate::rotation::error::FundingCondition::AmountLimit),
            )
            .unwrap();
        assert_eq!(
            policy_store
                .pool_funding_obstruction(&policy_job.pool_id)
                .unwrap(),
            Some("funding_policy_requires_review")
        );

        // An already-submitted swap remains Waiting (not obstructed) even when
        // auto_fund is paused, until NEAR reports a terminal/incomplete outcome.
        let (_dir3, mut settling_store) = fixture();
        let settling_job = allocate(&mut settling_store);
        prepared(&mut settling_store, &settling_job, true);
        settling_store
            .db
            .execute(
                "UPDATE treasury_operations SET attempts=1 WHERE id=?1",
                [&settling_job.operation_id],
            )
            .unwrap();
        settling_store
            .db
            .execute(
                "UPDATE funding_progress SET quote='{}' WHERE job_id=?1",
                [&settling_job.id],
            )
            .unwrap();
        settling_store
            .confirm_spend(&settling_job.operation_id)
            .unwrap();
        settling_store.pause_new_funding();
        assert_eq!(
            settling_store
                .pool_funding_obstruction(&settling_job.pool_id)
                .unwrap(),
            None
        );

        settling_store
            .record_provider_outcome(
                &settling_job.id,
                &settling_job.operation_id,
                &crate::rotation::near::SwapStatus::IncompleteDeposit,
                20,
            )
            .unwrap();
        assert_eq!(
            settling_store
                .pool_funding_obstruction(&settling_job.pool_id)
                .unwrap(),
            Some("funding_outcome_requires_review")
        );

        // An unprepared quote that observed provider_failed (never disclosed)
        // remains Actionable so automatic quote refresh can recover it.
        let (_dir3b, mut unprepared_store) = fixture();
        let unprepared_job = allocate(&mut unprepared_store);
        unprepared_store
            .db
            .execute(
                "UPDATE funding_progress SET quote='{}' WHERE job_id=?1",
                [&unprepared_job.id],
            )
            .unwrap();
        unprepared_store
            .record_provider_outcome(
                &unprepared_job.id,
                &unprepared_job.operation_id,
                &crate::rotation::near::SwapStatus::Failed,
                19,
            )
            .unwrap();
        assert_eq!(
            unprepared_store
                .pool_funding_obstruction(&unprepared_job.pool_id)
                .unwrap(),
            None
        );

        // Monotonic transition from IncompleteDeposit/Failed to Refunded clears
        // the review obstruction so automatic refund recovery can run.
        let (_dir4, mut refund_store) = fixture();
        let refund_job = allocate(&mut refund_store);
        prepared(&mut refund_store, &refund_job, true);
        refund_store
            .db
            .execute(
                "UPDATE treasury_operations SET attempts=1 WHERE id=?1",
                [&refund_job.operation_id],
            )
            .unwrap();
        refund_store
            .db
            .execute(
                "UPDATE funding_progress SET quote='{}' WHERE job_id=?1",
                [&refund_job.id],
            )
            .unwrap();
        refund_store
            .confirm_spend(&refund_job.operation_id)
            .unwrap();
        refund_store
            .record_provider_outcome(
                &refund_job.id,
                &refund_job.operation_id,
                &crate::rotation::near::SwapStatus::Failed,
                21,
            )
            .unwrap();
        assert_eq!(
            refund_store
                .pool_funding_obstruction(&refund_job.pool_id)
                .unwrap(),
            Some("funding_outcome_requires_review")
        );
        refund_store
            .record_provider_outcome(
                &refund_job.id,
                &refund_job.operation_id,
                &crate::rotation::near::SwapStatus::Refunded,
                22,
            )
            .unwrap();
        assert_eq!(
            refund_store
                .pool_funding_obstruction(&refund_job.pool_id)
                .unwrap(),
            None
        );

        // A disabled pool is obstructed even if it still holds a historical
        // funded ACTIVE wallet (matching discovery_ready / default_identity).
        let (_dir5, mut disabled_store) = fixture();
        let disabled_job = allocate(&mut disabled_store);
        disabled_store
            .db
            .execute(
                "INSERT INTO wallets(id,pool_id,sequence,address,key,target,role,balance,low_balance) VALUES ('hist-active',?1,99,'0x0000000000000000000000000000000000000004',X'04','5000000','ACTIVE','5000000',0)",
                [&disabled_job.pool_id],
            )
            .unwrap();
        disabled_store
            .db
            .execute(
                "UPDATE pools SET enabled=0 WHERE id=?1",
                [&disabled_job.pool_id],
            )
            .unwrap();
        assert_eq!(
            disabled_store
                .pool_funding_obstruction(&disabled_job.pool_id)
                .unwrap(),
            Some("funding_group_disabled")
        );

        // A READY wallet whose reserve claim is disabled does not count as
        // funded for discovery, so shortfall on an active claim still blocks.
        let (_dir6, mut ready_store) = fixture();
        let ready_job = allocate(&mut ready_store);
        ready_store
            .db
            .execute(
                "INSERT INTO wallets(id,pool_id,sequence,address,key,target,role,balance,low_balance) VALUES ('disabled-ready',?1,99,'0x0000000000000000000000000000000000000005',X'05','5000000','READY','5000000',0)",
                [&ready_job.pool_id],
            )
            .unwrap();
        ready_store
            .observe_funding_condition(
                &ready_job.id,
                &ready_job.operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
            )
            .unwrap();
        assert_eq!(
            ready_store
                .pool_funding_obstruction(&ready_job.pool_id)
                .unwrap(),
            Some("treasury_insufficient_spendable_funds")
        );

        // An unprepared ALLOCATED job on a disabled large reserve_claim must NOT
        // falsely obstruct an enabled pool that has an enabled vacant small
        // reserve_claim under Ordinary funding, while Paused still reports
        // funding_disabled.
        let (_dir7, mut mixed_claim_store) = fixture();
        let mixed_pool = mixed_claim_store
            .ensure_capacity_group(
                "mixed_claims",
                &crate::rotation::test_capacity::two_reserves("5"),
                Some(U256::from(1_000_000u64)),
                0,
            )
            .unwrap();
        mixed_claim_store
            .allocate_capacity_kind(&mixed_pool, super::super::AllocationKind::Large, true)
            .unwrap()
            .unwrap();
        // Disable the large reserve claim while leaving small enabled and vacant.
        mixed_claim_store
            .ensure_capacity_group(
                "mixed_claims",
                &crate::rotation::test_capacity::small_reserve("5"),
                Some(U256::from(1_000_000u64)),
                0,
            )
            .unwrap();
        assert_eq!(
            mixed_claim_store
                .pool_funding_obstruction(&mixed_pool)
                .unwrap(),
            None
        );
        mixed_claim_store.pause_new_funding();
        assert_eq!(
            mixed_claim_store
                .pool_funding_obstruction(&mixed_pool)
                .unwrap(),
            Some("funding_disabled")
        );

        // A sole enabled ALLOCATED job with malformed funding data returns
        // funding_data_requires_review rather than stalling forever; adding a
        // vacant enabled sibling claim clears the block under Ordinary funding
        // while Paused remains obstructed.
        let (_dir8, mut decode_store) = fixture();
        let other_job = allocate(&mut decode_store);
        let decode_pool = decode_store
            .ensure_capacity_group(
                "decode_pool",
                &crate::rotation::test_capacity::small_reserve("5"),
                Some(U256::from(1_000_000u64)),
                0,
            )
            .unwrap();
        let small_id = decode_store
            .allocate_capacity_kind(&decode_pool, super::super::AllocationKind::Small, true)
            .unwrap()
            .unwrap();
        let small_job = decode_store.funding_job(&small_id).unwrap().unwrap();
        decode_store
            .db
            .execute(
                "INSERT INTO funding_recovery(operation_id,job_id,phase) VALUES (?1,?2,'\"ALLOCATED\"')",
                rusqlite::params![&small_job.operation_id, &other_job.id],
            )
            .unwrap();
        assert_eq!(
            decode_store.pool_funding_obstruction(&decode_pool).unwrap(),
            Some("funding_data_requires_review")
        );
        // Enabling a vacant large claim allows Ordinary funding to allocate it.
        decode_store
            .ensure_capacity_group(
                "decode_pool",
                &crate::rotation::test_capacity::two_reserves("5"),
                Some(U256::from(1_000_000u64)),
                0,
            )
            .unwrap();
        assert_eq!(
            decode_store.pool_funding_obstruction(&decode_pool).unwrap(),
            None
        );
        decode_store.pause_new_funding();
        assert_eq!(
            decode_store.pool_funding_obstruction(&decode_pool).unwrap(),
            Some("funding_data_requires_review")
        );

        // With InsufficientFunds on a two-reserve pool (small=$5, large=$20):
        // a vacant smaller claim ($5) can still succeed when large ($20) hit
        // InsufficientFunds, whereas a vacant larger claim ($20) cannot clear
        // InsufficientFunds already observed on small ($5).
        let (_dir9, mut size_store) = fixture();
        let mut two = crate::rotation::test_capacity::two_reserves("5");
        two.large = Some(crate::rotation::config::ReserveConfig {
            funding_amount_usdc: Some("20".into()),
            min_funding_amount_usdc: None,
            max_funding_amount_usdc: None,
        });
        let size_pool = size_store
            .ensure_capacity_group("size_pool", &two, Some(U256::from(1_000_000u64)), 0)
            .unwrap();
        let large_id = size_store
            .allocate_capacity_kind(&size_pool, super::super::AllocationKind::Large, true)
            .unwrap()
            .unwrap();
        let large_job = size_store.funding_job(&large_id).unwrap().unwrap();
        size_store
            .observe_funding_condition(
                &large_job.id,
                &large_job.operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
            )
            .unwrap();
        assert_eq!(
            size_store.pool_funding_obstruction(&size_pool).unwrap(),
            None
        );
        // Now test small ($5) InsufficientFunds with vacant large ($20): obstructed.
        let (_dir10, mut small_short_store) = fixture();
        let small_short_pool = small_short_store
            .ensure_capacity_group("small_short", &two, Some(U256::from(1_000_000u64)), 0)
            .unwrap();
        let small_only_id = small_short_store
            .allocate_capacity_kind(&small_short_pool, super::super::AllocationKind::Small, true)
            .unwrap()
            .unwrap();
        let small_only_job = small_short_store
            .funding_job(&small_only_id)
            .unwrap()
            .unwrap();
        small_short_store
            .observe_funding_condition(
                &small_only_job.id,
                &small_only_job.operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
            )
            .unwrap();
        assert_eq!(
            small_short_store
                .pool_funding_obstruction(&small_short_pool)
                .unwrap(),
            Some("treasury_insufficient_spendable_funds")
        );
        // Raising the obstructed small claim's configured reserve target above
        // the vacant large slot ($30 > $20) uses the current reserve_claims
        // floor rather than the historical funding_jobs.target ($5), so the
        // smaller vacant claim ($20) can still progress.
        let mut resized = two.clone();
        resized.small = crate::rotation::config::ReserveConfig {
            funding_amount_usdc: Some("30".into()),
            min_funding_amount_usdc: None,
            max_funding_amount_usdc: None,
        };
        small_short_store
            .ensure_capacity_group("small_short", &resized, Some(U256::from(1_000_000u64)), 0)
            .unwrap();
        assert_eq!(
            small_short_store
                .pool_funding_obstruction(&small_short_pool)
                .unwrap(),
            None
        );

        // Disabled claims with non-settling states (expired or refunded) must
        // not mask an obstructed enabled claim, whereas a disabled claim with
        // an actively settling NEAR swap still keeps the pool waiting.
        let (_dir11, mut disabled_claim_store) = fixture();
        let dc_pool = disabled_claim_store
            .ensure_capacity_group(
                "dc_pool",
                &crate::rotation::test_capacity::two_reserves("5"),
                Some(U256::from(1_000_000u64)),
                0,
            )
            .unwrap();
        let dc_small_id = disabled_claim_store
            .allocate_capacity_kind(&dc_pool, super::super::AllocationKind::Small, true)
            .unwrap()
            .unwrap();
        let dc_large_id = disabled_claim_store
            .allocate_capacity_kind(&dc_pool, super::super::AllocationKind::Large, true)
            .unwrap()
            .unwrap();
        let dc_small = disabled_claim_store
            .funding_job(&dc_small_id)
            .unwrap()
            .unwrap();
        let dc_large = disabled_claim_store
            .funding_job(&dc_large_id)
            .unwrap()
            .unwrap();
        prepared(&mut disabled_claim_store, &dc_large, true);
        disabled_claim_store
            .db
            .execute(
                "UPDATE treasury_operations SET attempts=1 WHERE id=?1",
                [&dc_large.operation_id],
            )
            .unwrap();
        disabled_claim_store
            .db
            .execute(
                "UPDATE funding_progress SET quote='{}' WHERE job_id=?1",
                [&dc_large.id],
            )
            .unwrap();
        disabled_claim_store
            .confirm_spend(&dc_large.operation_id)
            .unwrap();
        disabled_claim_store
            .observe_funding_condition(
                &dc_small.id,
                &dc_small.operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
            )
            .unwrap();
        // Disable large while its swap is still actively settling: stays None.
        disabled_claim_store
            .ensure_capacity_group(
                "dc_pool",
                &crate::rotation::test_capacity::small_reserve("5"),
                Some(U256::from(1_000_000u64)),
                0,
            )
            .unwrap();
        assert_eq!(
            disabled_claim_store
                .pool_funding_obstruction(&dc_pool)
                .unwrap(),
            None
        );
        // Once the disabled large claim receives Refunded, it can no longer
        // replace or fund the pool, so small's shortfall is surfaced immediately.
        disabled_claim_store
            .record_provider_outcome(
                &dc_large.id,
                &dc_large.operation_id,
                &crate::rotation::near::SwapStatus::Refunded,
                30,
            )
            .unwrap();
        assert_eq!(
            disabled_claim_store
                .pool_funding_obstruction(&dc_pool)
                .unwrap(),
            Some("treasury_insufficient_spendable_funds")
        );
    }

    #[tokio::test]
    async fn provider_success_and_finish_if_proven_clear_stale_shortfall_when_resolving_outgoing() {
        use futures_util::FutureExt;
        let (_dir, mut store) = fixture();
        let job = allocate(&mut store);
        store
            .observe_funding_condition(
                &job.id,
                &job.operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
            )
            .unwrap();
        let changes = store.capacity_changes(&job.pool_id);

        // 1. record_provider_outcome(Success) resolves a sibling PREPARED
        //    outgoing row via close_fulfilled_source: clears stale InsufficientFunds
        //    and wakes parked capacity waiters.
        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "provider_done",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let done_job = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.pool_name == "provider_done")
            .unwrap();
        store
            .db
            .execute(
                "UPDATE funding_progress SET quote='{}' WHERE job_id=?1",
                [&done_job.id],
            )
            .unwrap();
        prepared(&mut store, &done_job, true);
        assert_eq!(store.pool_funding_obstruction(&job.pool_id).unwrap(), None);
        let after_success = changes.notified();
        tokio::pin!(after_success);
        after_success.as_mut().enable();
        store
            .record_provider_outcome(
                &done_job.id,
                &done_job.operation_id,
                &crate::rotation::near::SwapStatus::Success,
                10,
            )
            .unwrap();
        assert!(after_success.now_or_never().is_some());
        assert_eq!(store.pool_funding_obstruction(&job.pool_id).unwrap(), None);

        // 2. finish_funding_if_proven resolving a PREPARED outgoing row also
        //    clears stale InsufficientFunds and wakes parked capacity waiters.
        store
            .observe_funding_condition(
                &job.id,
                &job.operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
            )
            .unwrap();
        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "proven_done",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let proven_job = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.pool_name == "proven_done")
            .unwrap();
        prepared(&mut store, &proven_job, true);
        store
            .db
            .execute(
                "UPDATE funding_progress SET quote='{}', provider_success=1 WHERE job_id=?1",
                [&proven_job.id],
            )
            .unwrap();
        let after_proven = changes.notified();
        tokio::pin!(after_proven);
        after_proven.as_mut().enable();
        store
            .finish_funding_if_proven(&proven_job.id, &proven_job.operation_id)
            .unwrap();
        assert!(after_proven.now_or_never().is_some());
        assert_eq!(store.pool_funding_obstruction(&job.pool_id).unwrap(), None);

        // 3. funding_operation_fulfilled resolving a PREPARED direct-swap
        //    outgoing row (direct_success) also clears stale InsufficientFunds
        //    and wakes parked capacity waiters.
        store
            .observe_funding_condition(
                &job.id,
                &job.operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
            )
            .unwrap();
        let direct_id = "direct-send-op";
        store
            .accept_preparation(direct_id, None, 100, None)
            .unwrap();
        let rev: i64 = store
            .db
            .query_row(
                "SELECT revision FROM snapshots ORDER BY revision DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        store
            .prepare_with_facts(direct_id, rev, b"direct-snap", b"direct-raw", None)
            .unwrap();
        let direct_record = super::direct_swap::DirectOperation {
            id: direct_id.into(),
            route: super::direct_swap::DirectRoute::Near,
            owner: "owner".into(),
            kind: "send".into(),
            request_id: "req-direct".into(),
            terms: serde_json::json!({}),
            authority: None,
            source_error: None,
            address: None,
            quote: None,
            status: Some(serde_json::json!({ "provider": { "status": "SUCCESS" } })),
            state: "quoted".into(),
        };
        let payload = seal(
            &store.key,
            &store.direct_aad(direct_id),
            &serde_json::to_vec(&direct_record).unwrap(),
        )
        .unwrap();
        store
            .db
            .execute(
                "INSERT INTO direct_operations(id,owner,kind,request_id,payload) VALUES (?1,'owner','send','req-direct',?2)",
                params![direct_id, payload],
            )
            .unwrap();
        assert_eq!(store.pool_funding_obstruction(&job.pool_id).unwrap(), None);
        let after_direct = changes.notified();
        tokio::pin!(after_direct);
        after_direct.as_mut().enable();
        assert!(store.funding_operation_fulfilled(direct_id).unwrap());
        assert!(after_direct.now_or_never().is_some());
        assert_eq!(store.pool_funding_obstruction(&job.pool_id).unwrap(), None);
    }

    #[test]
    fn pre_credit_deferred_group_is_not_degraded_while_bootstrap_group_swap_is_settling() {
        let (_dir, mut store) = fixture();
        let general = crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "general",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let gen_job = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.pool_id == general)
            .unwrap();
        prepared(&mut store, &gen_job, true);
        store
            .db
            .execute(
                "UPDATE treasury_operations SET attempts=1 WHERE id=?1",
                [&gen_job.operation_id],
            )
            .unwrap();
        store.confirm_spend(&gen_job.operation_id).unwrap();

        crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "financial",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let fin_job = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.pool_name == "financial")
            .unwrap();
        store
            .observe_funding_condition(
                &fin_job.id,
                &fin_job.operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
            )
            .unwrap();
        let policy = crate::rotation::assignment::BootstrapPolicy {
            bootstrap_group: "general".to_owned(),
            deferred_groups: vec!["financial".to_owned()],
        };
        assert!(
            !store
                .pool_shortfall_is_degraded("financial", Some(&policy))
                .unwrap()
        );
        let demand = store
            .treasury_observation_demand(&["financial".to_owned()], Some(&policy))
            .unwrap();
        assert_eq!(demand.reason_codes(), "wallet_funding_insufficient_zec");
        assert!(!demand.is_degraded());
    }

    #[test]
    fn deferred_group_degradation_respects_bootstrap_handoff() {
        let (_dir, mut store) = fixture();
        let general = crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "general",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        store
            .db
            .execute(
                "INSERT INTO wallets(id,pool_id,sequence,address,key,target,role,balance,low_balance) VALUES ('gen-active',?1,99,'0x0000000000000000000000000000000000000002',X'02','5000000','ACTIVE','5000000',0)",
                [&general],
            )
            .unwrap();
        let financial = crate::rotation::test_capacity::declare_and_allocate!(
            store,
            "financial",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(U256::from(1_000_000u64)),
            0
        )
        .unwrap();
        let fin_job = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.pool_name == "financial")
            .unwrap();
        store
            .observe_funding_condition(
                &fin_job.id,
                &fin_job.operation_id,
                Some(crate::rotation::error::FundingCondition::InsufficientFunds),
            )
            .unwrap();
        let policy = crate::rotation::assignment::BootstrapPolicy {
            bootstrap_group: "general".to_owned(),
            deferred_groups: vec!["financial".to_owned()],
        };
        let groups = vec!["financial".to_owned()];

        // While general has a funded wallet and financial has no initial credit,
        // financial is NOT degraded (stays INFO / reserve shortfall).
        let demand = store
            .treasury_observation_demand(&groups, Some(&policy))
            .unwrap();
        assert_eq!(demand.reason_codes(), "wallet_funding_insufficient_zec");
        assert!(!demand.is_degraded());

        // If general loses its funded wallet, financial becomes degraded (WARN).
        store
            .db
            .execute("UPDATE wallets SET low_balance=1 WHERE id='gen-active'", [])
            .unwrap();
        let demand = store
            .treasury_observation_demand(&groups, Some(&policy))
            .unwrap();
        assert_eq!(
            demand.reason_codes(),
            "wallet_funding_degraded_insufficient_zec"
        );
        assert!(demand.is_degraded());

        // Restore general's funded wallet, then mark financial as having
        // completed its first initial credit (bootstrap wallet no longer in use)
        // with its own active wallet now depleted: financial MUST still be
        // degraded (WARN) and never fall back to general.
        store
            .db
            .execute("UPDATE wallets SET low_balance=0 WHERE id='gen-active'", [])
            .unwrap();
        store
            .db
            .execute(
                "INSERT INTO wallets(id,pool_id,sequence,address,key,target,role,balance,low_balance) VALUES ('fin-active',?1,99,'0x0000000000000000000000000000000000000003',X'03','5000000','ACTIVE','0',1)",
                [&financial],
            )
            .unwrap();
        store
            .db
            .execute(
                "INSERT INTO funding_jobs(id,wallet_id,pool_id,target,state,allocation_kind,claim_generation,readiness_requirement) VALUES ('fin-initial-credit','fin-active',?1,'5000000','COMPLETE','small',0,'1')",
                [&financial],
            )
            .unwrap();
        assert!(store.has_initial_credit(&financial).unwrap());
        let demand = store
            .treasury_observation_demand(&groups, Some(&policy))
            .unwrap();
        assert_eq!(
            demand.reason_codes(),
            "wallet_funding_degraded_insufficient_zec"
        );
        assert!(demand.is_degraded());
        assert!(
            store
                .pool_shortfall_is_degraded("financial", Some(&policy))
                .unwrap()
        );
    }
}
