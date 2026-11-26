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
                "still waiting for ZEC to fund degraded wallet group; send to `wallet status` ZEC receive address"
            }
            Self::WalletFundingInsufficientZec => {
                "still waiting for ZEC to fund reserve wallets; send to `wallet status` ZEC receive address"
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
        let current = self.collect_current_demand(configured, &mut demand);
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
                            if self.pool_has_usable_wallet(name)? {
                                TreasurySyncReason::WalletFundingInsufficientZec
                            } else {
                                TreasurySyncReason::WalletFundingDegradedInsufficientZec
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

    fn pool_has_usable_wallet(&self, pool_name: &str) -> Result<bool> {
        let mut query = self.db.prepare(
            "SELECT w.balance FROM wallets w JOIN pools p ON p.id=w.pool_id WHERE p.name=?1 AND w.role IN ('ACTIVE','RETAINED','READY') AND w.low_balance=0",
        )?;
        let mut rows = query.query([pool_name])?;
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

    #[cfg(test)]
    pub(crate) fn treasury_observation_needed(&self, configured: &[String]) -> Result<bool> {
        Ok(!self.treasury_observation_demand(configured)?.is_empty())
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
        let demand = store.treasury_observation_demand(&groups).unwrap();
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
        let demand = store.treasury_observation_demand(&groups).unwrap();
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
        let demand = store.treasury_observation_demand(&groups).unwrap();
        assert_eq!(demand.reason_codes(), "wallet_funding_insufficient_zec");
        assert_eq!(demand.formatted_wallets().as_deref(), Some("pool"));

        // Marking the active wallet low_balance, zero, empty, or malformed degrades the group.
        store
            .db
            .execute("UPDATE wallets SET low_balance=1 WHERE id='active-1'", [])
            .unwrap();
        assert_eq!(
            store
                .treasury_observation_demand(&groups)
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
                    .treasury_observation_demand(&groups)
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
        let demand = store.treasury_observation_demand(&both).unwrap();
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
        let demand = store.treasury_observation_demand(&both).unwrap();
        assert_eq!(
            demand.reason_codes(),
            "wallet_funding_degraded_insufficient_zec"
        );
        assert_eq!(demand.formatted_wallets().as_deref(), Some("alpha,pool"));
        assert!(!demand.summary().contains("other pending"));

        // Preparing the sibling's outgoing transaction serializes new funding
        // claims behind that outgoing work, so only OutgoingTransaction applies.
        prepared(&mut store, &sibling, true);
        let demand = store.treasury_observation_demand(&both).unwrap();
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
        let demand = store.treasury_observation_demand(&[]).unwrap();
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
        let demand = store.treasury_observation_demand(&[]).unwrap();
        assert_eq!(demand.reason_codes(), "funding_refund");
        assert_eq!(demand.formatted_wallets().as_deref(), Some("pool"));
    }
}
