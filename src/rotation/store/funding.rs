//! Funding workflow journal. The source operation ID is allocated before any
//! calculation and remains immutable across crashes and ambiguous submissions.
pub(super) const RECOVERY_COUNT_SQL: &str = "SELECT COUNT(*) FROM funding_recovery r WHERE job_id=?1 AND NOT EXISTS(SELECT 1 FROM funding_quote_refreshes q WHERE q.operation_id=r.operation_id)";

use super::*;
// Absence of bytes never proves that an interrupted preparer did no work.
// Only untouched phases or an exactly bound, already committed closure suffice.
pub(crate) fn safely_unprepared(job: &FundingJob, facts: &FundingFacts) -> bool {
    (!facts.preparation_started
        && !facts.archived
        && matches!(job.phase, FundingPhase::Allocated | FundingPhase::Quoted))
        || facts.source_closed
}

pub(crate) fn can_recover_facts(
    job: &FundingJob,
    operation: Option<&crate::rotation::transaction::OperationStatus>,
    facts: &FundingFacts,
) -> bool {
    if (facts.preparation_started || facts.archived) && operation.is_none() && !facts.source_closed
    {
        return false;
    }
    if job.phase == FundingPhase::RefundPending {
        return false;
    }
    if job.phase != FundingPhase::RecoveryRequired {
        return true;
    }
    operation
        .map(|op| op.submission != "CONFIRMED")
        .unwrap_or_else(|| safely_unprepared(job, facts))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FundingPhase {
    Allocated,
    Quoted,
    Preparing,
    Prepared,
    DepositPending,
    Swapping,
    VerifyingCredit,
    Complete,
    RefundPending,
    RecoveryRequired,
}
impl FundingPhase {
    fn permits(&self, next: &Self) -> bool {
        use FundingPhase::*;
        matches!(
            (self, next),
            (Allocated, Quoted)
                | (Quoted, Preparing)
                | (Preparing, Prepared)
                | (Prepared, DepositPending)
                | (DepositPending, Swapping)
                | (Swapping, VerifyingCredit)
                | (DepositPending, VerifyingCredit)
                | (DepositPending | Swapping | VerifyingCredit, RefundPending)
                | (
                    Allocated
                        | Quoted
                        | Preparing
                        | Prepared
                        | DepositPending
                        | Swapping
                        | VerifyingCredit
                        | RefundPending,
                    RecoveryRequired
                )
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FundingJob {
    pub id: String,
    pub pool_id: String,
    pub pool_name: String,
    pub wallet_id: String,
    pub recipient: String,
    pub target: String,
    pub phase: FundingPhase,
    pub operation_id: String,
    pub attempts: u32,
    pub started_at: Option<u64>,
    pub error_streak: u32,
    pub timed_out: bool,
    pub next_poll: u64,
    pub last_error: Option<String>,
}
/// Exact persisted obligations for one attempt. This is a read snapshot of the
/// existing journals, not another durable work state or source of authority.
#[derive(Debug)]
pub struct FundingFacts {
    pub enabled: bool,
    pub readiness_recorded: bool,
    pub readiness_disputed: bool,
    pub accepted_quote: bool,
    pub archived: bool,
    /// Exact operation/job archive plus zero remaining source liability.
    pub source_closed: bool,
    /// Accepted owner provenance overrides stale derived workflow phases.
    pub preparation_started: bool,
    pub provider_success: bool,
    pub provider_failed: bool,
    pub provider_refunded: bool,
    pub provider_incomplete: bool,
    pub source_submission: Option<String>,
    pub source_attempts: u32,
    /// Exact unsubmitted bytes outside their ordinary preparation workflow.
    pub hidden_prepared: bool,
}
impl FundingFacts {
    pub fn provider_review_required(&self) -> bool {
        self.provider_incomplete
            || (self.provider_success && (self.provider_failed || self.provider_refunded))
    }
    pub fn status_needed(&self) -> bool {
        self.accepted_quote
            && !self.archived
            && self.source_attempts > 0
            && !self.provider_success
            && !self.provider_refunded
    }
    pub(crate) fn observations_needed(&self) -> bool {
        !self.readiness_recorded
            || self.readiness_disputed
            || self.status_needed()
            || self.source_reconciliation_needed()
            || ((self.preparation_started || self.archived)
                && !self.source_closed
                && self.source_submission.is_none())
    }
    pub fn source_reconciliation_needed(&self) -> bool {
        self.hidden_prepared
            || self.source_attempts > 0
                && self
                    .source_submission
                    .as_deref()
                    .is_some_and(|state| !matches!(state, "CONFIRMED" | "EXPIRED"))
    }
}

/// Historical conversion preflight fixture. Runtime opens now require schema17
/// before any mutation; this preserves the original unsupported-proof regression.
#[cfg(test)]
pub(super) fn preflight_funding_upgrade(db: &Connection, version: u32) -> Result<()> {
    if version >= 15 {
        return Ok(());
    }
    let has_jobs: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='funding_jobs')",
        [],
        |r| r.get(0),
    )?;
    if has_jobs {
        let legacy_complete: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM funding_jobs WHERE state='COMPLETE')",
            [],
            |r| r.get(0),
        )?;
        ensure!(
            !legacy_complete,
            "legacy_destination_readiness_support_missing: explicit conversion preflight required"
        );
    }
    for table in [
        "funding_jobs",
        "payment_attempts",
        "outgoing",
        "budget_entries",
    ] {
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table],
            |r| r.get(0),
        )?;
        if exists {
            // Identifiers come only from the fixed table list above.
            let occupied: bool =
                db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table})"), [], |r| {
                    r.get(0)
                })?;
            ensure!(!occupied, "legacy_financial_state_requires_conversion");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationRequirement {
    pub operation_id: Option<String>,
    pub amount: String,
    pub recorded: bool,
}

/// Transaction-local interpretation of persisted accepted terms. Callers must
/// never substitute current configuration for an outstanding operation's output.
pub fn applicable_requirement(db: &Connection, job: &str) -> Result<DestinationRequirement> {
    type RequirementRow = (
        Option<String>,
        Option<String>,
        String,
        String,
        bool,
        Option<String>,
        Option<String>,
        String,
    );
    let (recorded_operation, recorded_amount, original, operation, accepted, output, recipient, address):
        RequirementRow = db.query_row(
        "SELECT f.readiness_operation,f.readiness_requirement,f.target,j.operation_id,j.quote IS NOT NULL AND NOT EXISTS(SELECT 1 FROM funding_recovery r WHERE r.operation_id=j.operation_id),j.accepted_output,j.accepted_recipient,w.address FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id JOIN wallets w ON w.id=f.wallet_id WHERE f.id=?1",
        [job], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)))?;
    if let Some(amount) = recorded_amount {
        return Ok(DestinationRequirement {
            operation_id: recorded_operation,
            amount,
            recorded: true,
        });
    }
    if accepted {
        let output = output.context("accepted destination requirement missing")?;
        ensure!(
            recipient
                .as_deref()
                .context("accepted recipient missing")?
                .parse::<alloy_primitives::Address>()?
                == address.parse::<alloy_primitives::Address>()?,
            "accepted destination binding mismatch"
        );
        amount(&output)?;
        Ok(DestinationRequirement {
            operation_id: Some(operation),
            amount: output,
            recorded: false,
        })
    } else {
        Ok(DestinationRequirement {
            operation_id: None,
            amount: original,
            recorded: false,
        })
    }
}

/// Shared credit fact for the funding worker and background reconciler. The
/// caller verifies the opaque Base certificate and holds its publication guard;
/// this transaction records the original support without changing later balances.
pub(crate) fn record_readiness(
    db: &Connection,
    wallet: &str,
    support: &super::evidence::BalanceSupport,
) -> Result<bool> {
    support.observation.validate()?;
    let (job,address):(String,String)=db.query_row(
        "SELECT f.id,w.address FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id WHERE f.wallet_id=?1",
        [wallet],|r|Ok((r.get(0)?,r.get(1)?)))?;
    ensure!(
        support.observation.wallet == wallet
            && support
                .observation
                .address
                .parse::<alloy_primitives::Address>()?
                == address.parse::<alloy_primitives::Address>()?,
        "readiness support wallet mismatch"
    );
    let requirement = applicable_requirement(db, &job)?;
    if requirement.recorded {
        return Ok(!db.query_row(
            "SELECT readiness_disputed IS NOT NULL FROM funding_jobs WHERE id=?1",
            [&job],
            |r| r.get::<_, bool>(0),
        )?);
    }
    let capacity = support.observation.available();
    if capacity < amount(&requirement.amount)? {
        return Ok(false);
    }
    ensure!(db.execute("UPDATE funding_jobs SET readiness_operation=?2,readiness_requirement=?3,readiness_capacity=?4,readiness_hash=?5,readiness_height=?6,readiness_support=?7,readiness_latest_hash=?8,readiness_endpoint=?9,readiness_checkpoint_hash=?10 WHERE id=?1 AND readiness_requirement IS NULL",
        params![job,requirement.operation_id,requirement.amount,capacity.to_string(),support.observation.confirmed.anchor.hash,i64::try_from(support.observation.confirmed.anchor.height)?,serde_json::to_string(support)?,support.observation.latest.anchor.hash,support.observation.endpoint,support.checkpoint.as_ref().map(|anchor|anchor.hash.as_str())])?==1,"funding readiness conflict");
    Ok(true)
}

/// Keep the historical observation readable while preventing disputed support
/// from proving completion or destination closure. Call in the evidence owner's
/// existing transaction; source liabilities and operation identity are untouched.
pub fn dispute_support(db: &Connection, wallet: &str, hash: &str, reason: &str) -> Result<()> {
    ensure!(
        matches!(reason, "chain_repair_required" | "evidence_repair_required"),
        "invalid readiness dispute reason"
    );
    db.execute(
        "UPDATE funding_jobs SET readiness_disputed=?3 WHERE wallet_id=?1 AND (readiness_hash=?2 OR readiness_latest_hash=?2 OR readiness_checkpoint_hash=?2)",
        params![wallet, hash, reason],
    )?;
    Ok(())
}

pub fn dispute_endpoint_support(db: &Connection, endpoint: &str) -> Result<()> {
    db.execute(
        "UPDATE funding_jobs SET readiness_disputed=COALESCE(readiness_disputed,'endpoint_reverification_required') WHERE readiness_endpoint=?1",
        [endpoint],
    )?;
    Ok(())
}

fn source_attempt_closed(db: &Connection, operation: &str) -> Result<bool> {
    let (_, owner_unclosed) = super::preparation::source_state(db, operation)?;
    Ok(db.query_row("SELECT (NOT ?2 AND NOT EXISTS(SELECT 1 FROM outgoing WHERE id=?1) AND NOT EXISTS(SELECT 1 FROM budget_entries WHERE id=?1)) OR (EXISTS(SELECT 1 FROM expired_operations WHERE id=?1) AND EXISTS(SELECT 1 FROM outgoing WHERE id=?1 AND state='RESOLVED') AND EXISTS(SELECT 1 FROM budget_entries WHERE id=?1 AND reserved=0 AND consumed=0))",params![operation,owner_unclosed],|r|r.get(0))?)
}

impl Store {
    /// One configured group, two independent vacancy commits. Failure in one
    /// slot never rolls back another or changes an existing operation.
    pub(crate) fn fill_funding_vacancies(&mut self, name: &str) -> Result<()> {
        let pool: Option<String> = self
            .db
            .query_row(
                "SELECT id FROM pools WHERE name=?1 AND enabled=1",
                [name],
                |r| r.get(0),
            )
            .optional()?;
        let Some(pool) = pool else {
            return Ok(());
        };
        let mut failure = None;
        for kind in [super::AllocationKind::Small, super::AllocationKind::Large] {
            if let Err(error) = self.allocate_capacity_kind(&pool, kind, true) {
                failure.get_or_insert(error);
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub fn funding_facts(&self, job: &str, operation: &str) -> Result<FundingFacts> {
        read_funding_facts(&self.db, job, operation)
    }

    pub fn require_funding_attempt(&self, job: &str, max_attempts: u32) -> Result<()> {
        self.funding_restriction
            .require_new_funding("funding_attempt_start")?;
        self.check_job_permit(job, 1)?;
        ensure!(
            self.funding_recovery_count(job)? <= u64::from(max_attempts),
            "recovery_attempt_limit_reached"
        );
        Ok(())
    }
    pub(crate) fn current_funding_claim(&self, job: &str) -> Result<super::capacity::FundingClaim> {
        super::capacity::current_funding_claim(&self.db, job)?
            .context("funding claim disabled or changed")
    }

    pub(crate) fn accepted_quote_suits_current_claim(
        &self,
        job: &str,
        operation: &str,
    ) -> Result<bool> {
        let claim = self.current_funding_claim(job)?;
        let output: String = self.db.query_row(
            "SELECT accepted_output FROM funding_progress WHERE job_id=?1 AND operation_id=?2",
            params![job, operation],
            |r| r.get(0),
        )?;
        let output = amount(&output)?;
        Ok(output >= claim.target && output <= claim.maximum_output)
    }

    /// Validate the current process policy at the serialized source owner. Exact
    /// journal binding prevents a queued command from following a replaced attempt.
    pub(crate) fn require_preparation_authority(
        &self,
        operation: &str,
        authority: Option<&super::super::transaction::ManagedFundingAuthority>,
    ) -> Result<()> {
        let binding: Option<(String, bool)> = self.db.query_row(
            "SELECT j.job_id,p.enabled FROM funding_progress j JOIN funding_jobs f ON f.id=j.job_id JOIN wallets w ON w.id=f.wallet_id JOIN pools p ON p.id=w.pool_id WHERE j.operation_id=?1",
            [operation],
            |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional()?;
        match (binding, authority) {
            (Some((job, enabled)), Some(authority)) => {
                ensure!(job == authority.job_id, "funding operation binding changed");
                ensure!(
                    enabled && authority.auto_fund,
                    super::super::error::FundingCondition::Disabled
                );
                self.require_funding_attempt(&job, authority.max_attempts)?;
                ensure!(
                    self.accepted_quote_suits_current_claim(&job, operation)?,
                    "funding quote unsuitable for current claim"
                );
                Ok(())
            }
            (None, None) => Ok(()),
            _ => anyhow::bail!("funding preparation authority missing or stale"),
        }
    }

    #[cfg(feature = "zcash")]
    pub(crate) fn require_preparation_binding(
        &self,
        operation: &str,
        pool: Option<&str>,
        recipient: &str,
        input: u64,
        deadline: u64,
    ) -> Result<()> {
        let binding: Option<(String,String)> = self.db.query_row(
            "SELECT j.job_id,w.pool_id FROM funding_progress j JOIN funding_jobs f ON f.id=j.job_id JOIN wallets w ON w.id=f.wallet_id WHERE j.operation_id=?1",
            [operation], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((job, expected_pool)) = binding {
            ensure!(
                pool == Some(expected_pool.as_str()),
                "funding source pool binding mismatch"
            );
            let quote: crate::rotation::near::Quote =
                serde_json::from_slice(&self.funding_quote_for_operation(&job, operation)?)?;
            ensure!(
                quote.deposit.as_deref() == Some(recipient)
                    && quote.input == input
                    && quote.deadline == deadline,
                "persisted funding source binding mismatch"
            );
        }
        Ok(())
    }

    /// Accept preparation at the actual treasury owner, atomically against D.
    pub fn begin_funding_preparation(
        &mut self,
        operation: &str,
        authority: Option<&super::super::transaction::ManagedFundingAuthority>,
    ) -> Result<()> {
        self.require_preparation_authority(operation, authority)?;
        self.require_unstarted_preparation(operation)?;
        let Some(authority) = authority else {
            return Ok(());
        };
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let changed=tx.execute("UPDATE funding_progress SET phase='\"PREPARING\"',last_error=NULL WHERE job_id=?1 AND operation_id=?2 AND phase='\"QUOTED\"' AND EXISTS(SELECT 1 FROM funding_jobs f WHERE f.id=job_id AND f.readiness_requirement IS NULL AND f.state!='COMPLETE')",params![authority.job_id,operation])?;
        ensure!(changed == 1, "funding preparation boundary changed");
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn funding_recovery_context(
        &self,
        job: &str,
    ) -> Result<(FundingJob, Option<OperationStatus>)> {
        let job = self.funding_job(job)?.context("recovery job missing")?;
        let present: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM treasury_operations WHERE id=?1)",
            [&job.operation_id],
            |r| r.get(0),
        )?;
        let operation = present
            .then(|| self.operation(&job.operation_id))
            .transpose()?;
        Ok((job, operation))
    }

    pub(crate) fn funding_sync_observation(&self) -> Result<SyncObservation> {
        let observation: String = self.db.query_row(
            "SELECT observation FROM treasury_sync WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&observation)?)
    }

    pub(crate) fn funding_wallet_type(&self, job: &str) -> Result<String> {
        Ok(self.db.query_row(
            "SELECT allocation_kind FROM funding_jobs WHERE id=?1",
            [job],
            |r| r.get(0),
        )?)
    }

    pub fn funding_source_ready(&self, now: u64) -> Result<bool> {
        let pending: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE state='PREPARED')",
            [],
            |r| r.get(0),
        )?;
        if pending {
            return Ok(false);
        }
        let observation: Option<String> = self
            .db
            .query_row(
                "SELECT observation FROM treasury_sync WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let Some(observation) = observation else {
            return Ok(false);
        };
        let observation: SyncObservation = serde_json::from_str(&observation)?;
        let revision: i64 = self.db.query_row(
            "SELECT revision FROM snapshots ORDER BY revision DESC LIMIT 1",
            [],
            |r| r.get(0),
        )?;
        Ok(observation.fresh(now, revision))
    }

    pub(crate) fn funding_preparation_context(&self) -> Result<(String, i64, SyncObservation)> {
        let revision = self.db.query_row(
            "SELECT revision FROM snapshots ORDER BY revision DESC LIMIT 1",
            [],
            |r| r.get(0),
        )?;
        Ok((self.id.clone(), revision, self.funding_sync_observation()?))
    }

    /// Snapshot current configuration at a proven-clean quote boundary. This is
    /// unsigned request history, never a source reservation or an accepted quote.
    pub fn select_quote_terms(
        &mut self,
        job: &str,
        operation: &str,
        target: &str,
        ceiling: &str,
    ) -> Result<()> {
        self.funding_restriction
            .require_new_funding("funding_quote_start")?;
        self.check_job_permit(job, 1)?;
        let claim = self.current_funding_claim(job)?;
        ensure!(
            amount(target)? == claim.target && amount(ceiling)? == claim.maximum_output,
            "funding quote policy changed"
        );
        ensure!(
            amount(target)? > U256::ZERO && amount(ceiling)? >= amount(target)?,
            "invalid funding quote terms"
        );
        let changed=self.db.execute("UPDATE funding_progress SET quote_target=?3,quote_ceiling=?4 WHERE job_id=?1 AND operation_id=?2 AND phase='\"ALLOCATED\"' AND quote IS NULL AND NOT EXISTS(SELECT 1 FROM outgoing WHERE id=?2) AND NOT EXISTS(SELECT 1 FROM budget_entries WHERE id=?2) AND NOT EXISTS(SELECT 1 FROM treasury_preparations WHERE operation_id=?2) AND EXISTS(SELECT 1 FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id JOIN pools p ON p.id=w.pool_id WHERE f.id=?1 AND f.state='QUEUED' AND f.readiness_requirement IS NULL AND w.role IN ('ALLOCATED','ACTIVE','RETAINED') AND p.enabled=1)",params![job,operation,target,ceiling])?;
        ensure!(changed == 1, "funding quote boundary changed");
        Ok(())
    }

    /// Resolve under the caller's store transaction: accepted terms override original
    /// intent, while a recorded readiness observation remains immutable after spending.
    pub fn destination_requirement(&self, job: &str) -> Result<DestinationRequirement> {
        applicable_requirement(&self.db, job)
    }

    /// Record destination readiness independently from the operation's financial
    /// completion. The owner publishes roles in the same surrounding transaction.
    #[cfg(test)]
    pub fn record_destination_readiness(
        &mut self,
        job: &str,
        capacity: &str,
        hash: &str,
        height: i64,
    ) -> Result<bool> {
        let (wallet,address):(String,String)=self.db.query_row("SELECT w.id,w.address FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id WHERE f.id=?1",[job],|r|Ok((r.get(0)?,r.get(1)?)))?;
        ensure!(!hash.is_empty() && height >= 0, "missing chain evidence");
        let height = u64::try_from(height)?;
        let point = crate::rotation::base::evidence::BalancePoint {
            anchor: crate::rotation::base::Anchor {
                height,
                hash: format!("0x{height:064x}"),
            },
            amount: amount(capacity)?,
        };
        let support = super::evidence::BalanceSupport {
            checkpoint: None,
            observation: crate::rotation::base::evidence::BalanceObservation {
                wallet: wallet.clone(),
                address,
                network: "eip155:8453".into(),
                asset: crate::payment::USDC.into(),
                endpoint: "a".repeat(64),
                confirmed: point.clone(),
                latest: crate::rotation::base::evidence::BalancePoint {
                    anchor: crate::rotation::base::Anchor {
                        height: height + 1,
                        hash: format!("0x{:064x}", height + 1),
                    },
                    amount: point.amount,
                },
                confirmations: 1,
                valid_until: u64::MAX,
            },
        };
        record_readiness(&self.db, &wallet, &support)
    }

    /// Completion is a conjunction of exact original-operation facts. Destination
    /// spending and successful excess returns do not revoke those facts.
    pub fn finish_funding_if_proven(&mut self, job: &str, operation: &str) -> Result<bool> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let completed = finish_if_proven(&tx, job, operation)?;
        tx.commit()?;
        if completed {
            self.funding_conditions.remove(job);
        }
        Ok(completed)
    }

    /// Source publication derives phase from all committed provider facts. It
    /// cannot overwrite a concurrently observed success, refund or review result.
    pub fn reconcile_funding_phase(&mut self, job: &str, operation: &str) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        derive_operation_phase(&tx, job, operation)?;
        tx.commit()?;
        Ok(())
    }

    /// Persist only adapter-validated status, bound to the exact accepted operation.
    /// Terminal facts accumulate so a contradictory response cannot erase evidence.
    #[cfg(feature = "zcash")]
    pub fn record_provider_outcome(
        &mut self,
        job: &str,
        operation: &str,
        status: &crate::rotation::near::SwapStatus,
        observed_at: u64,
    ) -> Result<()> {
        use crate::rotation::near::SwapStatus;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let changed = tx.execute("UPDATE funding_progress SET provider_success=MAX(provider_success,?3),provider_failed=MAX(provider_failed,?4),provider_refunded=MAX(provider_refunded,?5),provider_incomplete=MAX(provider_incomplete,?6),provider_observed_at=MAX(COALESCE(provider_observed_at,0),?7) WHERE job_id=?1 AND operation_id=?2 AND quote IS NOT NULL", params![job,operation,matches!(status,SwapStatus::Success),matches!(status,SwapStatus::Failed),matches!(status,SwapStatus::Refunded),matches!(status,SwapStatus::IncompleteDeposit),i64::try_from(observed_at)?])?;
        ensure!(changed == 1, "funding operation correlation mismatch");
        derive_operation_phase(&tx, job, operation)?;
        tx.commit()?;
        Ok(())
    }
}

fn source_accounted(db: &Connection, operation: &str) -> Result<bool> {
    let facts:Option<(String,i64,i64,String)>=db.query_row("SELECT o.facts,b.consumed,b.reserved,x.state FROM treasury_operations o JOIN budget_entries b ON b.id=o.id JOIN outgoing x ON x.id=o.id WHERE o.id=?1 AND o.submission='CONFIRMED'",[operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((facts, consumed, reserved, state)) = facts else {
        return Ok(false);
    };
    if consumed == 0 {
        return Ok(false);
    }
    let facts: crate::rotation::transaction::TransactionFacts = serde_json::from_str(&facts)?;
    let gross = facts
        .amount_zatoshis
        .checked_add(facts.fee_zatoshis)
        .context("source gross expenditure overflow")?;
    ensure!(
        u64::try_from(consumed)? == gross && reserved == 0 && state == "RESOLVED",
        "funding source accounting integrity conflict"
    );
    Ok(true)
}

type ProviderFlags = (bool, bool, bool, bool);

fn provider_flags(row: &rusqlite::Row<'_>, start: usize) -> rusqlite::Result<ProviderFlags> {
    Ok((
        row.get(start)?,
        row.get(start + 1)?,
        row.get(start + 2)?,
        row.get(start + 3)?,
    ))
}

pub(super) fn finish_if_proven(tx: &Connection, job: &str, operation: &str) -> Result<bool> {
    let (ready, correlation, quote, flags, confirmed, gross):
            (bool, Option<String>, bool, rusqlite::Result<ProviderFlags>, bool, bool) = tx.query_row(
            "SELECT f.readiness_requirement IS NOT NULL AND f.readiness_support IS NOT NULL AND f.readiness_disputed IS NULL,f.readiness_operation,j.quote IS NOT NULL AND NOT EXISTS(SELECT 1 FROM funding_recovery r WHERE r.operation_id=j.operation_id),j.provider_success,j.provider_failed,j.provider_refunded,j.provider_incomplete,EXISTS(SELECT 1 FROM treasury_operations o WHERE o.id=j.operation_id AND o.submission='CONFIRMED'),EXISTS(SELECT 1 FROM budget_entries b WHERE b.id=j.operation_id AND b.consumed>0 AND b.reserved=0) FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id WHERE f.id=?1 AND j.operation_id=?2", params![job,operation], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,provider_flags(r,3),r.get(7)?,r.get(8)?)))?;
    let (success, failed, refunded, incomplete) =
        flags.context(super::scheduling::FundingDecodeFailure)?;
    let wallet: String = tx.query_row(
        "SELECT wallet_id FROM funding_jobs WHERE id=?1",
        [job],
        |r| r.get(0),
    )?;
    let ready = ready && !destination_obligation_pending(tx, &wallet)?;
    let outcome = if ready
        && !quote
        && !failed
        && !refunded
        && !incomplete
        && correlation.as_deref().is_none_or(|id| id == operation)
    {
        ensure!(
            source_attempt_closed(tx, operation)?,
            "destination satisfaction has unresolved source effects"
        );
        let signed: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1)",
            [operation],
            |r| r.get(0),
        )?;
        Some(if signed { "RECOVERED" } else { "EXTERNAL" })
    } else if ready
        && correlation.as_deref() == Some(operation)
        && success
        && !failed
        && !refunded
        && !incomplete
        && confirmed
        && gross
        && source_accounted(tx, operation)?
    {
        Some("SUCCESS")
    } else {
        None
    };
    if let Some(outcome) = outcome {
        tx.execute("UPDATE funding_jobs SET state='COMPLETE',completion_outcome=?2 WHERE id=?1 AND state!='COMPLETE'",params![job,outcome])?;
        tx.execute(
            "UPDATE funding_progress SET phase='\"COMPLETE\"',last_error=NULL WHERE job_id=?1",
            [job],
        )?;
    }
    Ok(outcome.is_some())
}

/// Publication is driven by destination readiness, never financial completion.
/// Exact configured claims govern publication of funded roles.
pub(crate) fn publish_destination_readiness(db: &Connection, wallet: &str) -> Result<()> {
    let (job, operation): (String,String) = db.query_row(
        "SELECT f.id,j.operation_id FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id WHERE f.wallet_id=?1 AND f.readiness_requirement IS NOT NULL AND f.readiness_disputed IS NULL",
        [wallet], |r| Ok((r.get(0)?,r.get(1)?)))?;
    ensure!(
        readiness_supported(db, wallet)?,
        "destination_readiness_missing"
    );
    super::capacity::publish_funded_role(db, &job)?;
    finish_if_proven(db, &job, &operation)?;
    Ok(())
}

fn derive_operation_phase(db: &Connection, job: &str, operation: &str) -> Result<()> {
    let matched: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM funding_progress WHERE job_id=?1 AND operation_id=?2)",
        params![job, operation],
        |r| r.get(0),
    )?;
    ensure!(matched, "funding operation correlation mismatch");
    db.execute("UPDATE funding_progress SET phase='\"RECOVERY_REQUIRED\"',last_error='funding_integrity_conflict; contradictory completed provider outcome' WHERE job_id=?1 AND operation_id=?2 AND (provider_incomplete=1 OR (provider_success=1 AND (provider_failed=1 OR provider_refunded=1))) AND EXISTS(SELECT 1 FROM funding_jobs f WHERE f.id=job_id AND f.state='COMPLETE')",params![job,operation])?;
    db.execute("UPDATE funding_progress SET phase=CASE WHEN provider_incomplete=1 OR (provider_success=1 AND (provider_failed=1 OR provider_refunded=1)) THEN '\"RECOVERY_REQUIRED\"' WHEN provider_failed=1 OR provider_refunded=1 THEN '\"REFUND_PENDING\"' WHEN provider_success=1 THEN '\"VERIFYING_CREDIT\"' WHEN EXISTS(SELECT 1 FROM treasury_operations o WHERE o.id=operation_id AND o.submission='CONFIRMED') THEN '\"SWAPPING\"' WHEN phase='\"PREPARED\"' AND EXISTS(SELECT 1 FROM treasury_operations o WHERE o.id=operation_id AND o.attempts>0) THEN '\"DEPOSIT_PENDING\"' ELSE phase END WHERE job_id=?1 AND operation_id=?2 AND phase NOT IN ('\"RECOVERY_REQUIRED\"','\"COMPLETE\"')", params![job,operation])?;
    Ok(())
}

/// Historical D is immutable evidence, not a current balance requirement. Every
/// financial consumer validates its full tuple and persisted correlation indexes.
pub(crate) fn readiness_supported(db: &Connection, wallet: &str) -> Result<bool> {
    if !readiness_recorded_valid(db, wallet)? {
        return Ok(false);
    }
    Ok(db.query_row(
        "SELECT readiness_disputed IS NULL FROM funding_jobs WHERE wallet_id=?1",
        [wallet],
        |r| r.get(0),
    )?)
}

/// Structural historical D validity is distinct from whether its original support
/// remains undisputed. Fresh payment evidence can establish current capacity.
pub(crate) fn readiness_recorded_valid(db: &Connection, wallet: &str) -> Result<bool> {
    type Saved = (
        String,
        String,
        String,
        String,
        i64,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
    );
    let saved:Option<Saved>=db.query_row("SELECT f.readiness_support,f.readiness_requirement,f.readiness_capacity,f.readiness_hash,f.readiness_height,f.readiness_latest_hash,f.readiness_endpoint,f.readiness_checkpoint_hash,f.readiness_disputed,f.readiness_operation,j.operation_id,w.address,j.accepted_output,j.accepted_recipient,f.target FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id JOIN wallets w ON w.id=f.wallet_id WHERE f.wallet_id=?1 AND f.readiness_requirement IS NOT NULL",[wallet],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?,r.get(13)?,r.get(14)?))).optional().map_err(|error| {
        if matches!(error, rusqlite::Error::InvalidColumnType(..) | rusqlite::Error::FromSqlConversionFailure(..) | rusqlite::Error::IntegralValueOutOfRange(..)) {
            anyhow::Error::new(error).context(crate::rotation::base::evidence::EvidenceFailure::MalformedSupport)
        } else {
            error.into()
        }
    })?;
    let Some((
        support,
        requirement,
        capacity,
        hash,
        height,
        latest,
        endpoint,
        checkpoint,
        _dispute,
        correlation,
        operation,
        address,
        accepted,
        recipient,
        original,
    )) = saved
    else {
        return Ok(false);
    };
    (|| -> Result<bool> {
        let support: super::evidence::BalanceSupport = serde_json::from_str(&support)?;
        support.observation.validate()?;
        ensure!(
            support.observation.wallet == wallet
                && support
                    .observation
                    .address
                    .parse::<alloy_primitives::Address>()?
                    == address.parse::<alloy_primitives::Address>()?,
            "readiness support wallet mismatch"
        );
        ensure!(
            support.observation.available() == amount(&capacity)?
                && amount(&capacity)? >= amount(&requirement)?
                && support.observation.confirmed.anchor.hash == hash
                && i64::try_from(support.observation.confirmed.anchor.height)? == height
                && support.observation.latest.anchor.hash == latest
                && support.observation.endpoint == endpoint
                && support.checkpoint.as_ref().map(|a| a.hash.as_str()) == checkpoint.as_deref(),
            "readiness support tuple mismatch"
        );
        if let Some(correlation) = correlation {
            ensure!(
                correlation == operation
                    && accepted.as_deref() == Some(requirement.as_str())
                    && recipient
                        .as_deref()
                        .context("readiness accepted recipient missing")?
                        .parse::<alloy_primitives::Address>()?
                        == address.parse::<alloy_primitives::Address>()?,
                "readiness operation correlation mismatch"
            );
        } else {
            ensure!(
                requirement == original && accepted.is_none(),
                "external readiness requirement mismatch"
            );
        }
        Ok(true)
    })()
    .context(crate::rotation::base::evidence::EvidenceFailure::MalformedSupport)
}

/// Destination disposition is separate from source-accounting completion. A
/// successful original operation may still owe source reconciliation after its
/// destination is closed; an uncertain/failed attempt cannot use that exception.
pub fn destination_obligation_pending(db: &Connection, wallet: &str) -> Result<bool> {
    type DispositionRow = (
        bool,
        Option<String>,
        bool,
        Option<String>,
        String,
        bool,
        bool,
        bool,
        bool,
        bool,
        bool,
    );
    let (ready, support, disputed, correlation, operation, accepted, archived, success, failed, refunded, incomplete):
        DispositionRow = db.query_row(
        "SELECT f.readiness_requirement IS NOT NULL,f.readiness_support,f.readiness_disputed IS NOT NULL,f.readiness_operation,j.operation_id,j.quote IS NOT NULL,EXISTS(SELECT 1 FROM funding_recovery r WHERE r.operation_id=j.operation_id),j.provider_success,j.provider_failed,j.provider_refunded,j.provider_incomplete FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id WHERE f.wallet_id=?1",[wallet],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?)))?;
    if !ready || support.is_none() || disputed || failed || refunded || incomplete {
        return Ok(true);
    }
    if !readiness_supported(db, wallet)? {
        return Ok(true);
    }
    ensure!(
        correlation.as_deref().is_none_or(|id| id == operation),
        "readiness operation correlation mismatch"
    );
    if success && correlation.as_deref() == Some(operation.as_str()) {
        return Ok(false);
    }
    if accepted && !archived {
        return Ok(true);
    }
    Ok(!source_attempt_closed(db, &operation)?)
}

pub(super) fn read_funding_facts(
    db: &Connection,
    job: &str,
    operation: &str,
) -> Result<FundingFacts> {
    let (mut facts, flags, phase) = db.query_row(
            "SELECT f.readiness_requirement IS NOT NULL,f.readiness_disputed IS NOT NULL,j.quote IS NOT NULL,EXISTS(SELECT 1 FROM funding_recovery r WHERE r.operation_id=j.operation_id AND r.job_id=j.job_id),j.provider_success,j.provider_failed,j.provider_refunded,j.provider_incomplete,CASE WHEN EXISTS(SELECT 1 FROM expired_operations e WHERE e.id=j.operation_id) THEN 'EXPIRED' ELSE o.submission END,COALESCE(o.attempts,0),EXISTS(SELECT 1 FROM wallets w JOIN pools p ON p.id=w.pool_id WHERE w.id=f.wallet_id AND p.enabled=1),j.phase FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id LEFT JOIN treasury_operations o ON o.id=j.operation_id WHERE f.id=?1 AND j.operation_id=?2",
            params![job,operation], |r| Ok((FundingFacts {
                readiness_recorded:r.get(0)?, readiness_disputed:r.get(1)?, accepted_quote:r.get(2)?, archived:r.get(3)?, source_closed: false, preparation_started: false,
                provider_success:false, provider_failed:false, provider_refunded:false, provider_incomplete:false,
                source_submission:r.get(8)?, source_attempts:r.get(9)?,enabled:r.get(10)?,hidden_prepared:false,
            }, provider_flags(r, 4), r.get::<_,String>(11)?)),
        ).context("funding operation binding changed")?;
    (
        facts.provider_success,
        facts.provider_failed,
        facts.provider_refunded,
        facts.provider_incomplete,
    ) = flags.context(super::scheduling::FundingDecodeFailure)?;
    let phase: FundingPhase =
        serde_json::from_str(&phase).context(super::scheduling::FundingDecodeFailure)?;
    facts.hidden_prepared = facts.source_submission.as_deref() == Some("PREPARED")
        && facts.source_attempts == 0
        && matches!(
            phase,
            FundingPhase::Allocated | FundingPhase::Quoted | FundingPhase::Complete
        );
    facts.preparation_started = super::preparation::source_state(db, operation)?.0;
    facts.source_closed = facts.archived && source_attempt_closed(db, operation)?;
    Ok(facts)
}

fn completed_error(error: Option<&str>) -> Option<&str> {
    // Destination credit does not establish source confirmation.
    error.filter(|e| {
        e.starts_with("source_reconciliation_failed;")
            || e.starts_with("funding_integrity_conflict;")
            || e.starts_with("base_credit_unverified;")
    })
}
impl Store {
    /// Recovery history survives restarts and operation replacement.
    pub fn funding_recovery_count(&self, job: &str) -> Result<u64> {
        let count: i64 = self.db.query_row(RECOVERY_COUNT_SQL, [job], |r| r.get(0))?;
        Ok(u64::try_from(count)?)
    }

    /// Called only after a typed response proving the preparer was never invoked.
    /// Absence of bytes by itself is not authority to repeat preparation.
    pub fn defer_unstarted_preparation(&mut self, job: &str) -> Result<()> {
        let operation: String = self.db.query_row(
            "SELECT operation_id FROM funding_progress WHERE job_id=?1",
            [job],
            |r| r.get(0),
        )?;
        self.defer_unstarted_preparation_for_operation(job, &operation)
    }
    pub fn defer_unstarted_preparation_for_operation(
        &mut self,
        job: &str,
        expected_operation: &str,
    ) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (operation, phase): (String, String) = tx.query_row(
            "SELECT operation_id,phase FROM funding_progress WHERE job_id=?1 AND operation_id=?2",
            params![job, expected_operation],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            serde_json::from_str::<FundingPhase>(&phase)? == FundingPhase::Preparing,
            "preparation is not pending"
        );
        let touched: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1) OR EXISTS(SELECT 1 FROM budget_entries WHERE id=?1) OR EXISTS(SELECT 1 FROM treasury_preparations WHERE operation_id=?1)", [&operation], |r| r.get(0))?;
        ensure!(
            !touched,
            "preparation has durable effects; recovery required"
        );
        tx.execute(
            "UPDATE funding_progress SET phase=?2 WHERE job_id=?1",
            params![job, serde_json::to_string(&FundingPhase::Quoted)?],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Archive proven-unprepared work without authorizing another attempt. This
    /// cleanup remains available when current funding policy is disabled/exhausted.
    /// PREPARING is deliberately excluded: missing bytes do not prove nonexecution.
    pub fn close_unprepared_funding(&mut self, job: &str) -> Result<()> {
        let operation: String = self.db.query_row(
            "SELECT operation_id FROM funding_progress WHERE job_id=?1",
            [job],
            |r| r.get(0),
        )?;
        self.close_unprepared_funding_for_operation(job, &operation)
    }
    pub fn close_unprepared_funding_for_operation(
        &mut self,
        job: &str,
        expected_operation: &str,
    ) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (operation, phase): (String, String) = tx.query_row(
            "SELECT operation_id,phase FROM funding_progress WHERE job_id=?1 AND operation_id=?2",
            params![job, expected_operation],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let archived: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM funding_recovery WHERE operation_id=?1 AND job_id=?2)",
            params![operation, job],
            |r| r.get(0),
        )?;
        if archived {
            ensure!(
                source_attempt_closed(&tx, &operation)?,
                "funding closure proof missing"
            );
            return Ok(());
        }
        let phase: FundingPhase = serde_json::from_str(&phase)?;
        ensure!(
            matches!(phase, FundingPhase::Allocated | FundingPhase::Quoted),
            "preparation outcome unknown"
        );
        let touched: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1) OR EXISTS(SELECT 1 FROM budget_entries WHERE id=?1) OR EXISTS(SELECT 1 FROM treasury_preparations WHERE operation_id=?1)",[&operation],|r|r.get(0))?;
        ensure!(!touched, "unprepared closure has source effects");
        archive_unprepared(&tx, job, &operation)?;
        tx.commit()?;
        Ok(())
    }

    /// Operator-requested safe closure. New identity and dispatch require the
    /// separate profile-aware recovery path and its current attempt allowance.
    pub fn recover_unprepared_funding(&mut self, job: &str) -> Result<()> {
        self.close_unprepared_funding(job)
    }
    /// Reauthorize a previously closed operation without rewriting its history.
    /// The closed attempt already counts against the durable recovery allowance.
    pub fn reauthorize_closed_funding(&mut self, job: &str, max_attempts: u32) -> Result<()> {
        let operation: String = self.db.query_row(
            "SELECT operation_id FROM funding_progress WHERE job_id=?1",
            [job],
            |r| r.get(0),
        )?;
        self.reauthorize_closed_funding_for_operation(job, &operation, max_attempts)
    }
    pub fn reauthorize_closed_funding_for_operation(
        &mut self,
        job: &str,
        expected_operation: &str,
        max_attempts: u32,
    ) -> Result<()> {
        self.require_funding_attempt(job, max_attempts)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure!(
            super::capacity::current_funding_claim(&tx, job)?.is_some(),
            "funding claim disabled or changed"
        );
        let operation: String = tx.query_row(
            "SELECT j.operation_id FROM funding_progress j JOIN funding_jobs f ON f.id=j.job_id JOIN wallets w ON w.id=f.wallet_id JOIN pools p ON p.id=w.pool_id WHERE f.id=?1 AND j.operation_id=?2 AND f.state!='COMPLETE' AND f.readiness_requirement IS NULL AND w.role IN ('ALLOCATED','ACTIVE','RETAINED') AND p.enabled=1 AND j.phase='\"RECOVERY_REQUIRED\"' AND EXISTS(SELECT 1 FROM funding_recovery r WHERE r.operation_id=j.operation_id AND r.job_id=j.job_id)",
            params![job,expected_operation], |r| r.get(0))?;
        ensure!(
            source_attempt_closed(&tx, &operation)?,
            "funding closure proof missing"
        );
        tx.execute("DELETE FROM funding_refunds WHERE job_id=?1", [job])?;
        tx.execute("DELETE FROM funding_health WHERE job_id=?1", [job])?;
        tx.execute("UPDATE funding_progress SET operation_id=?2,phase='\"ALLOCATED\"',quote=NULL,provider_success=0,provider_failed=0,provider_refunded=0,provider_incomplete=0,provider_observed_at=NULL,quote_target=NULL,quote_ceiling=NULL,accepted_output=NULL,accepted_recipient=NULL,attempts=0,next_poll=0,last_error=NULL WHERE job_id=?1",params![job,Uuid::new_v4().to_string()])?;
        tx.commit()?;
        Ok(())
    }
    pub fn refresh_unprepared_quote(&mut self, job: &str) -> Result<()> {
        let operation: String = self.db.query_row(
            "SELECT operation_id FROM funding_progress WHERE job_id=?1",
            [job],
            |r| r.get(0),
        )?;
        self.refresh_unprepared_quote_for_operation(job, &operation)
    }
    pub fn refresh_unprepared_quote_for_operation(
        &mut self,
        job: &str,
        expected_operation: &str,
    ) -> Result<()> {
        self.funding_restriction
            .require_new_funding("funding_attempt_reset")?;
        self.check_job_permit(job, 1)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure!(
            super::capacity::current_funding_claim(&tx, job)?.is_some(),
            "funding claim disabled or changed"
        );
        let (operation, phase): (String, String) = tx.query_row("SELECT j.operation_id,j.phase FROM funding_progress j JOIN funding_jobs f ON f.id=j.job_id JOIN wallets w ON w.id=f.wallet_id WHERE j.job_id=?1 AND j.operation_id=?2 AND f.state!='COMPLETE' AND f.readiness_requirement IS NULL AND w.role IN ('ALLOCATED','ACTIVE','RETAINED') AND EXISTS(SELECT 1 FROM pools p WHERE p.id=w.pool_id AND p.enabled=1)", params![job,expected_operation], |r| Ok((r.get(0)?,r.get(1)?)))?;
        let parsed: FundingPhase = serde_json::from_str(&phase)?;
        let attempts: u32 = tx.query_row(
            "SELECT attempts FROM funding_progress WHERE job_id=?1",
            [job],
            |r| r.get(0),
        )?;
        ensure!(
            parsed == FundingPhase::Quoted,
            "only unprepared quotes can refresh"
        );
        let signed: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1)",
            [&operation],
            |r| r.get(0),
        )?;
        ensure!(!signed, "recovery refused: operation has signed bytes");
        let reserved: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM budget_entries WHERE id=?1) OR EXISTS(SELECT 1 FROM treasury_preparations WHERE operation_id=?1)",
            [&operation],
            |r| r.get(0),
        )?;
        ensure!(
            !reserved,
            "recovery refused: preparation has durable effects"
        );
        tx.execute("INSERT INTO funding_recovery(operation_id,job_id,phase,quote,refund,provider_success,provider_failed,provider_refunded,provider_incomplete,provider_observed_at,quote_target,quote_ceiling,accepted_output,accepted_recipient) SELECT j.operation_id,j.job_id,j.phase,j.quote,r.address,j.provider_success,j.provider_failed,j.provider_refunded,j.provider_incomplete,j.provider_observed_at,j.quote_target,j.quote_ceiling,j.accepted_output,j.accepted_recipient FROM funding_progress j LEFT JOIN funding_refunds r ON r.job_id=j.job_id WHERE j.job_id=?1", [job])?;
        tx.execute(
            "INSERT INTO funding_quote_refreshes(operation_id) VALUES(?1)",
            [&operation],
        )?;
        tx.execute("DELETE FROM funding_refunds WHERE job_id=?1", [job])?;
        tx.execute("UPDATE funding_progress SET operation_id=?2,phase='\"ALLOCATED\"',quote=NULL,provider_success=0,provider_failed=0,provider_refunded=0,provider_incomplete=0,provider_observed_at=NULL,quote_target=NULL,quote_ceiling=NULL,accepted_output=NULL,accepted_recipient=NULL,attempts=?3,next_poll=0,last_error=NULL WHERE job_id=?1", params![job,Uuid::new_v4().to_string(),attempts])?;
        tx.execute("DELETE FROM funding_health WHERE job_id=?1", [job])?;
        tx.commit()?;
        Ok(())
    }

    pub fn refund_address(&self, job: &str) -> Result<Option<String>> {
        let bytes: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT address FROM funding_refunds WHERE job_id=?1",
                [job],
                |r| r.get(0),
            )
            .optional()?;
        bytes
            .map(|bytes| {
                let plain = unseal(
                    &self.key,
                    &format!("v1:{}:{}:refund:{job}", self.id, self.network.name()),
                    &bytes,
                )?;
                Ok(std::str::from_utf8(&plain)?.to_owned())
            })
            .transpose()
    }
    pub fn save_refund_address(
        &mut self,
        job: &str,
        address: &str,
        revision: i64,
        snapshot: &[u8],
    ) -> Result<i64> {
        self.save_wallet_snapshot(revision, snapshot, None, Some((job, address)))
    }

    pub fn funding_job(&self, id: &str) -> Result<Option<FundingJob>> {
        Ok(read_jobs_matching(&self.db, Some(id))?.into_iter().next())
    }
    /// Read exact operation journals committed atomically with allocation.
    /// Missing provenance is corruption, never permission to invent an attempt.
    pub fn funding_jobs(&self) -> Result<Vec<FundingJob>> {
        read_jobs(&self.db)
    }
    /// Persist a quote encrypted; it links treasury refunds to destination keys.
    /// Repeated delivery must match exactly and cannot replace an accepted quote.
    pub fn save_funding_quote(&mut self, id: &str, quote: &[u8]) -> Result<()> {
        self.save_quote(id, quote, None, None)
    }
    /// Persist the accepted bridge output separately from the immutable original
    /// allocation target, after validating the current claim's allowed range.
    #[cfg(feature = "zcash")]
    pub fn save_funding_quote_with_target(
        &mut self,
        id: &str,
        quote: &[u8],
        target: &str,
    ) -> Result<()> {
        let parsed: crate::rotation::near::Quote = serde_json::from_slice(quote)?;
        ensure!(parsed.request["amount"] == target, "quote target mismatch");
        self.save_quote(id, quote, Some(target), None)
    }
    #[cfg(feature = "zcash")]
    pub fn save_funding_quote_for_operation(
        &mut self,
        id: &str,
        operation: &str,
        quote: &[u8],
        target: &str,
    ) -> Result<()> {
        let parsed: crate::rotation::near::Quote = serde_json::from_slice(quote)?;
        ensure!(parsed.request["amount"] == target, "quote target mismatch");
        self.save_quote(id, quote, Some(target), Some(operation))
    }
    fn save_quote(
        &mut self,
        id: &str,
        quote: &[u8],
        target: Option<&str>,
        expected_operation: Option<&str>,
    ) -> Result<()> {
        ensure!(!quote.is_empty(), "empty funding quote");
        let aad = format!("v1:{}:{}:funding:{id}", self.id, self.network.name());
        let encrypted = seal(&self.key, &aad, quote)?;
        let tx = self.db.transaction()?;
        let (phase, previous, operation): (String, Option<Vec<u8>>, String) = tx.query_row(
            "SELECT phase,quote,operation_id FROM funding_progress WHERE job_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        ensure!(
            expected_operation.is_none_or(|expected| expected == operation),
            "funding operation correlation mismatch"
        );
        if let Some(previous) = previous {
            ensure!(
                unseal(&self.key, &aad, &previous)?.as_slice() == quote,
                "funding_quote_conflict"
            );
        } else {
            self.funding_restriction
                .require_new_funding("funding_quote_acceptance")?;
            ensure!(
                serde_json::from_str::<FundingPhase>(&phase)? == FundingPhase::Allocated,
                "funding phase conflict"
            );
            let parsed: serde_json::Value =
                serde_json::from_slice(quote).context("invalid accepted funding quote")?;
            let output = parsed["request"]["amount"]
                .as_str()
                .context("accepted output missing")?;
            let recipient = parsed["request"]["recipient"]
                .as_str()
                .context("accepted recipient missing")?;
            ensure!(
                amount(output)? > U256::ZERO && target.is_none_or(|target| target == output),
                "quote target mismatch"
            );
            let (_wallet, address, enabled, ready, pending): (String,String,bool,bool,bool) = tx.query_row(
                "SELECT w.id,w.address,p.enabled,f.readiness_requirement IS NOT NULL,EXISTS(SELECT 1 FROM outgoing o WHERE o.id=j.operation_id) OR EXISTS(SELECT 1 FROM budget_entries b WHERE b.id=j.operation_id) OR EXISTS(SELECT 1 FROM treasury_preparations t WHERE t.operation_id=j.operation_id) FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id JOIN pools p ON p.id=w.pool_id JOIN funding_progress j ON j.job_id=f.id WHERE f.id=?1 AND f.state='QUEUED' AND w.role IN ('ALLOCATED','ACTIVE','RETAINED')", [id],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
            ensure!(
                enabled && !ready && !pending,
                "funding quote boundary changed"
            );
            ensure!(
                recipient.parse::<alloy_primitives::Address>()?
                    == address.parse::<alloy_primitives::Address>()?,
                "accepted destination binding mismatch"
            );
            let claim = super::capacity::current_funding_claim(&tx, id)?
                .context("funding claim disabled or changed")?;
            ensure!(
                amount(output)? >= claim.target && amount(output)? <= claim.maximum_output,
                "funding quote outside current terms"
            );
            let (selected, ceiling): (Option<String>, Option<String>) = tx.query_row(
                "SELECT quote_target,quote_ceiling FROM funding_progress WHERE job_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if expected_operation.is_some() || selected.is_some() {
                let selected = selected.context("funding quote terms missing")?;
                let ceiling = ceiling.context("funding quote ceiling missing")?;
                ensure!(
                    amount(output)? >= amount(&selected)? && amount(output)? <= amount(&ceiling)?,
                    "funding quote outside selected terms"
                );
            }
            tx.execute(
                "UPDATE funding_progress SET phase='\"QUOTED\"',quote=?2,accepted_output=?3,accepted_recipient=?4 WHERE job_id=?1",
                params![id, encrypted, output, recipient],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn funding_quote_for_operation(
        &self,
        id: &str,
        operation: &str,
    ) -> Result<Zeroizing<Vec<u8>>> {
        let bound: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM funding_progress WHERE job_id=?1 AND operation_id=?2)",
            params![id, operation],
            |r| r.get(0),
        )?;
        ensure!(bound, "funding operation binding changed");
        self.funding_quote(id)
    }
    pub fn funding_quote(&self, id: &str) -> Result<Zeroizing<Vec<u8>>> {
        let bytes: Vec<u8> = self.db.query_row(
            "SELECT quote FROM funding_progress WHERE job_id=?1",
            [id],
            |r| r.get(0),
        )?;
        unseal(
            &self.key,
            &format!("v1:{}:{}:funding:{id}", self.id, self.network.name()),
            &bytes,
        )
    }
    pub fn advance_funding(
        &mut self,
        id: &str,
        expected: FundingPhase,
        next: FundingPhase,
    ) -> Result<()> {
        let operation: String = self.db.query_row(
            "SELECT operation_id FROM funding_progress WHERE job_id=?1",
            [id],
            |r| r.get(0),
        )?;
        self.advance_funding_operation(id, &operation, expected, next)
    }
    pub fn advance_funding_operation(
        &mut self,
        id: &str,
        operation: &str,
        expected: FundingPhase,
        next: FundingPhase,
    ) -> Result<()> {
        ensure!(expected.permits(&next), "invalid funding transition");
        if next == FundingPhase::Preparing {
            self.funding_restriction
                .require_new_funding("funding_preparation_start")?;
            self.check_job_permit(id, 1)?;
        }
        let changed = self.db.execute(
            "UPDATE funding_progress SET phase=?3,last_error=CASE WHEN ?4 THEN last_error ELSE NULL END WHERE job_id=?1 AND phase=?2 AND operation_id=?5 AND EXISTS(SELECT 1 FROM funding_jobs f WHERE f.id=job_id AND f.state!='COMPLETE' AND (?6=0 OR f.readiness_requirement IS NULL))",
            params![
                id,
                serde_json::to_string(&expected)?,
                serde_json::to_string(&next)?,
                next == FundingPhase::RecoveryRequired,
                operation,
                next == FundingPhase::Preparing
            ],
        )?;
        ensure!(changed == 1, "funding phase conflict");
        Ok(())
    }
    pub fn mark_funding_timeout(&mut self, id: &str) -> Result<()> {
        let operation: String = self.db.query_row(
            "SELECT operation_id FROM funding_progress WHERE job_id=?1",
            [id],
            |row| row.get(0),
        )?;
        self.mark_funding_operation_timeout(id, &operation)
    }
    pub(crate) fn mark_funding_operation_timeout(
        &mut self,
        id: &str,
        operation: &str,
    ) -> Result<()> {
        let current: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM funding_progress WHERE job_id=?1 AND operation_id=?2)",
            params![id, operation],
            |r| r.get(0),
        )?;
        if !current {
            return Ok(());
        }
        let observations = read_funding_facts(&self.db, id, operation)?.observations_needed();
        self.db.execute("INSERT INTO funding_health(job_id,timed_out) SELECT ?1,1 WHERE EXISTS(SELECT 1 FROM funding_progress j JOIN funding_jobs f ON f.id=j.job_id WHERE j.job_id=?1 AND j.operation_id=?2 AND (f.state!='COMPLETE' OR ?3)) ON CONFLICT(job_id) DO UPDATE SET timed_out=1",params![id,operation,observations])?;
        Ok(())
    }
    /// Check before advancing to PREPARING; the owner still rechecks at reserve.
    pub fn check_funding_capacity(&self, instant: u64, input: u64) -> Result<()> {
        self.require_spend_ready(instant, input)
    }
    pub fn defer_funding(
        &mut self,
        id: &str,
        next_poll: u64,
        error: Option<&str>,
        quote_attempt: bool,
    ) -> Result<()> {
        self.defer_funding_result(id, None, next_poll, error, quote_attempt)
    }
    pub fn defer_funding_result(
        &mut self,
        id: &str,
        operation: Option<&str>,
        next_poll: u64,
        error: Option<&str>,
        quote_attempt: bool,
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        if let Some(operation) = operation {
            let current: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM funding_progress WHERE job_id=?1 AND operation_id=?2)",
                params![id, operation],
                |r| r.get(0),
            )?;
            if !current {
                return Ok(());
            }
        }
        let complete: bool = tx.query_row(
            "SELECT state='COMPLETE' FROM funding_jobs WHERE id=?1",
            [id],
            |r| r.get(0),
        )?;
        let complete = if complete {
            let current: String = tx.query_row(
                "SELECT operation_id FROM funding_progress WHERE job_id=?1",
                [id],
                |r| r.get(0),
            )?;
            !read_funding_facts(&tx, id, &current)?.observations_needed()
        } else {
            false
        };
        let error = if complete {
            completed_error(error)
        } else {
            error
        };
        ensure!(tx.execute("UPDATE funding_progress SET next_poll=?2,last_error=?3,attempts=attempts+?4 WHERE job_id=?1",params![id,i64::try_from(next_poll)?,error,u32::from(quote_attempt)])? == 1, "unknown funding job");
        tx.execute("INSERT INTO funding_health(job_id,error_streak) VALUES (?1,?2) ON CONFLICT(job_id) DO UPDATE SET error_streak=CASE WHEN excluded.error_streak=0 THEN 0 ELSE MIN(error_streak+1,32) END",params![id,u32::from(error.is_some())])?;
        tx.execute("UPDATE funding_health SET timed_out=0 WHERE job_id=?1 AND ?2 AND EXISTS(SELECT 1 FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id JOIN treasury_operations o ON o.id=j.operation_id WHERE f.id=?1 AND f.state='COMPLETE' AND o.submission='CONFIRMED')",params![id,complete])?;
        tx.commit()?;
        Ok(())
    }
}
pub(super) fn read_jobs(db: &Connection) -> Result<Vec<FundingJob>> {
    read_jobs_matching(db, None)
}
fn read_jobs_matching(db: &Connection, job: Option<&str>) -> Result<Vec<FundingJob>> {
    let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let query = format!(
        "SELECT f.id,w.pool_id,p.name,w.id,w.address,f.target,CASE WHEN {version}<15 AND f.state='COMPLETE' THEN '\"COMPLETE\"' ELSE j.phase END,j.operation_id,j.attempts,j.next_poll,j.last_error FROM funding_jobs f LEFT JOIN funding_progress j ON j.job_id=f.id JOIN wallets w ON w.id=f.wallet_id JOIN pools p ON p.id=w.pool_id"
    );
    let query = if job.is_some() {
        format!("{query} WHERE f.id=?1")
    } else {
        format!("{query} ORDER BY f.rowid")
    };
    let mut stmt = db.prepare(&query)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(job), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6),
            r.get::<_, String>(7)?,
            r.get::<_, u32>(8)?,
            r.get::<_, i64>(9),
            r.get::<_, Option<String>>(10)?,
        ))
    })?;
    rows.map(|r| {
        let (
            id,
            pool_id,
            pool_name,
            wallet_id,
            recipient,
            target,
            phase,
            operation_id,
            attempts,
            next_poll,
            last_error,
        ) = r.context("missing or invalid funding operation journal")?;
        let (started_at,error_streak,timed_out): (Option<i64>,u32,bool) = if version >= 9 {
            db.query_row("SELECT started_at,error_streak,timed_out FROM funding_health WHERE job_id=?1",[&id],|r|Ok((|| -> rusqlite::Result<(Option<i64>,u32,bool)> { Ok((r.get(0)?,r.get(1)?,r.get(2)?)) })())).optional()?.unwrap_or(Ok((None,0,false))).context(super::scheduling::FundingDecodeFailure)?
        } else {(None,0,false)};
        let phase = phase.context(super::scheduling::FundingDecodeFailure)?;
        let parsed_phase: FundingPhase = serde_json::from_str(&phase).context(super::scheduling::FundingDecodeFailure)?;
        // Also normalize historical completed rows for read-only wallet status.
        let completed_without_observations = parsed_phase == FundingPhase::Complete
            && (version < 16 || !read_funding_facts(db, &id, &operation_id)?.observations_needed());
        let last_error = if completed_without_observations {
            completed_error(last_error.as_deref()).map(str::to_owned)
        } else {
            last_error
        };
        let last_error = last_error.or_else(||timed_out.then(||"swap_timeout; slower reconciliation continues; inspect source and refund status".into()));
        Ok(FundingJob {
            started_at: started_at.map(u64::try_from).transpose().context(super::scheduling::FundingDecodeFailure)?,
            error_streak: if completed_without_observations && last_error.is_none() { 0 } else { error_streak },
            timed_out,
            id,
            pool_id,
            pool_name,
            wallet_id,
            recipient,
            target,
            phase: parsed_phase,
            operation_id,
            attempts,
            next_poll: u64::try_from(next_poll.context(super::scheduling::FundingDecodeFailure)?).context(super::scheduling::FundingDecodeFailure)?,
            last_error,
        })
    })
    .collect()
}

#[cfg(test)]
mod completion_tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Store, FundingJob) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        super::super::test_support::declare_two_reserves(
            &mut store,
            "pool",
            "0.000010",
            Some(U256::from(1_000_000)),
            0,
        )
        .unwrap();
        let job = store.funding_jobs().unwrap().remove(0);
        (dir, store, job)
    }

    #[test]
    fn missing_operation_journal_is_never_synthesized_or_hidden() {
        let (dir, mut store, job) = fixture();
        let treasury = store.id().to_owned();
        let other = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|other| other.id != job.id)
            .unwrap();
        store
            .db
            .execute("DELETE FROM funding_progress WHERE job_id=?1", [&job.id])
            .unwrap();
        for reopened in [false, true] {
            if reopened {
                drop(store);
                store = Store::open(
                    &dir.path().join("state"),
                    &dir.path().join("key"),
                    &treasury,
                )
                .unwrap();
            }
            let boundary = store.funding_work_boundary().unwrap();
            for _ in 0..2 {
                assert!(
                    store
                        .funding_job(&job.id)
                        .unwrap_err()
                        .to_string()
                        .contains("funding operation journal")
                );
                assert!(
                    store
                        .funding_jobs()
                        .unwrap_err()
                        .to_string()
                        .contains("funding operation journal")
                );
                assert!(store.funding_work_page("", boundary).is_err());
                assert!(store.status().is_err());
                assert!(store.funding_job("unknown").unwrap().is_none());
                assert_eq!(
                    store.funding_job(&other.id).unwrap().unwrap().operation_id,
                    other.operation_id
                );
                assert_eq!(
                    store
                        .db
                        .query_row(
                            "SELECT COUNT(*) FROM funding_progress WHERE job_id=?1",
                            [&job.id],
                            |r| r.get::<_, i64>(0)
                        )
                        .unwrap(),
                    0
                );
            }
        }
    }

    #[test]
    fn vacancy_owner_commits_siblings_independently_and_rechecks_denial() {
        let (_dir, mut store, _) = fixture();
        let reserves = crate::rotation::config::ReservesConfig {
            small: crate::rotation::config::ReserveConfig {
                funding_amount_usdc: "1".into(),
                max_funding_amount_usdc: None,
            },
            large: Some(crate::rotation::config::ReserveConfig {
                funding_amount_usdc: "8".into(),
                max_funding_amount_usdc: None,
            }),
        };
        let pool = store
            .ensure_capacity_group("vacancies", &reserves, Some(U256::from(10_000_000)), 0)
            .unwrap();
        store.db.execute_batch("CREATE TRIGGER fail_large BEFORE INSERT ON funding_jobs WHEN NEW.allocation_kind='large' BEGIN SELECT RAISE(ABORT,'fixture_large_failure'); END;").unwrap();
        assert!(store.fill_funding_vacancies("vacancies").is_err());
        let count: i64 = store
            .db
            .query_row(
                "SELECT COUNT(*) FROM funding_jobs WHERE pool_id=?1",
                [&pool],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "large failure must preserve the committed small claim"
        );
        store.db.execute_batch("DROP TRIGGER fail_large").unwrap();
        store.deny_new_funding();
        assert!(store.fill_funding_vacancies("vacancies").is_err());
        let count_after: i64 = store
            .db
            .query_row(
                "SELECT COUNT(*) FROM funding_jobs WHERE pool_id=?1",
                [&pool],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_after, count);
    }

    #[test]
    fn legacy_upgrade_preflight_preserves_schema_and_exact_stored_bytes() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE funding_jobs(id TEXT PRIMARY KEY,state TEXT); CREATE TABLE outgoing(id TEXT PRIMARY KEY,raw BLOB); INSERT INTO funding_jobs VALUES('job','COMPLETE'); INSERT INTO outgoing VALUES('operation',X'00FF0102'); PRAGMA user_version=12;").unwrap();
        let before: (String, Vec<u8>) = db
            .query_row(
                "SELECT f.state,o.raw FROM funding_jobs f CROSS JOIN outgoing o",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let error = preflight_funding_upgrade(&db, 12).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("legacy_destination_readiness_support_missing")
        );
        let version: u32 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        let after: (String, Vec<u8>) = db
            .query_row(
                "SELECT f.state,o.raw FROM funding_jobs f CROSS JOIN outgoing o",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(version, 12);
        assert_eq!(before, after);
        db.execute("UPDATE funding_jobs SET state='QUEUED'", [])
            .unwrap();
        assert!(
            preflight_funding_upgrade(&db, 12)
                .unwrap_err()
                .to_string()
                .contains("legacy_financial_state_requires_conversion")
        );
        db.execute_batch("DELETE FROM funding_jobs; DELETE FROM outgoing;")
            .unwrap();
        preflight_funding_upgrade(&db, 12).unwrap();
        preflight_funding_upgrade(&Connection::open_in_memory().unwrap(), 0).unwrap();
    }

    #[test]
    fn malformed_or_inconsistent_historical_readiness_never_proves_completion() {
        for corruption in [
            "UPDATE funding_jobs SET readiness_support='not json' WHERE id=?1",
            "UPDATE funding_jobs SET readiness_capacity='99' WHERE id=?1",
            "UPDATE funding_jobs SET readiness_hash='wrong' WHERE id=?1",
        ] {
            let (_dir, mut store, job) = fixture();
            store
                .record_destination_readiness(&job.id, &job.target, "credit", 1)
                .unwrap();
            store.db.execute(corruption, [&job.id]).unwrap();
            assert!(
                store
                    .finish_funding_if_proven(&job.id, &job.operation_id)
                    .is_err()
            );
            let state: String = store
                .db
                .query_row(
                    "SELECT state FROM funding_jobs WHERE id=?1",
                    [&job.id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(state, "QUEUED");
            store
                .close_unprepared_funding_for_operation(&job.id, &job.operation_id)
                .unwrap();
            assert!(
                store
                    .reauthorize_closed_funding_for_operation(&job.id, &job.operation_id, 1)
                    .is_err()
            );
            assert_eq!(
                store.funding_job(&job.id).unwrap().unwrap().operation_id,
                job.operation_id
            );
        }
    }

    #[test]
    fn readiness_and_source_owner_intent_have_one_serialized_winner() {
        use crate::rotation::transaction::{ManagedFundingAuthority, TransactionFacts};
        for credit_first in [true, false] {
            let (_dir, mut store, job) = fixture();
            let quote = serde_json::to_vec(
                &serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}}),
            )
            .unwrap();
            store.save_funding_quote(&job.id, &quote).unwrap();
            let authority = ManagedFundingAuthority {
                job_id: job.id.clone(),
                auto_fund: true,
                max_attempts: 0,
            };
            if !credit_first {
                store
                    .begin_funding_preparation(&job.operation_id, Some(&authority))
                    .unwrap();
            }
            store
                .record_destination_readiness(&job.id, &job.target, "credit", 1)
                .unwrap();
            if credit_first {
                assert!(
                    store
                        .begin_funding_preparation(&job.operation_id, Some(&authority))
                        .is_err()
                );
                let effects:bool=store.db.query_row("SELECT EXISTS(SELECT 1 FROM outgoing) OR EXISTS(SELECT 1 FROM budget_entries)",[],|r|r.get(0)).unwrap();
                assert!(!effects);
                store
                    .close_unprepared_funding_for_operation(&job.id, &job.operation_id)
                    .unwrap();
                assert!(
                    store
                        .finish_funding_if_proven(&job.id, &job.operation_id)
                        .unwrap()
                );
            } else {
                // A later destination role change cannot discard accepted source work.
                store
                    .db
                    .execute(
                        "UPDATE wallets SET role='RETIRED' WHERE id=?1",
                        [&job.wallet_id],
                    )
                    .unwrap();
                store
                    .reserve(&job.operation_id, Some(&job.pool_id), 1, 100)
                    .unwrap();
                let revision = store.snapshot().unwrap().0;
                store
                    .prepare_with_facts(
                        &job.operation_id,
                        revision,
                        b"snapshot",
                        b"exact signed bytes",
                        Some(TransactionFacts {
                            txid: "transaction".into(),
                            expiry_height: 100,
                            amount_zatoshis: 50,
                            fee_zatoshis: 10,
                            deadline: u64::MAX,
                        }),
                    )
                    .unwrap();
                assert_eq!(
                    store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
                    b"exact signed bytes"
                );
                store
                    .request_broadcast(&job.operation_id, 1, 1, false)
                    .unwrap();
                assert_eq!(store.operation(&job.operation_id).unwrap().attempts, 1);
            }
        }
    }

    #[cfg(feature = "zcash")]
    #[test]
    fn source_preparation_binds_the_original_pool_and_exact_accepted_source_terms() {
        let (_dir, mut store, job) = fixture();
        let quote = crate::rotation::near::Quote {
            request: serde_json::json!({"amount":job.target,"recipient":job.recipient}),
            response: serde_json::json!({}),
            input: 50,
            deadline: 1000,
            deposit: Some("deposit".into()),
        };
        store
            .save_funding_quote(&job.id, &serde_json::to_vec(&quote).unwrap())
            .unwrap();
        store
            .require_preparation_binding(&job.operation_id, Some(&job.pool_id), "deposit", 50, 1000)
            .unwrap();
        for (pool, deposit, input, deadline) in [
            (None, "deposit", 50, 1000),
            (Some("other-pool"), "deposit", 50, 1000),
            (Some(job.pool_id.as_str()), "other-deposit", 50, 1000),
            (Some(job.pool_id.as_str()), "deposit", 51, 1000),
            (Some(job.pool_id.as_str()), "deposit", 50, 1001),
        ] {
            assert!(
                store
                    .require_preparation_binding(&job.operation_id, pool, deposit, input, deadline)
                    .is_err()
            );
        }
        assert!(store.status().unwrap().treasury_operations.is_empty());
    }

    #[test]
    fn treasury_owner_rechecks_policy_and_exact_attempt_before_preparation() {
        use crate::rotation::transaction::ManagedFundingAuthority;
        let (_dir, mut store, job) = fixture();
        let mut authority = ManagedFundingAuthority {
            job_id: job.id.clone(),
            auto_fund: false,
            max_attempts: 0,
        };
        assert!(
            store
                .require_preparation_authority(&job.operation_id, Some(&authority))
                .is_err()
        );
        authority.auto_fund = true;
        assert!(
            store
                .require_preparation_authority(&job.operation_id, Some(&authority))
                .is_err(),
            "an accepted quote is required"
        );
        assert!(
            store
                .require_preparation_authority(&job.operation_id, None)
                .is_err()
        );
        let quote = serde_json::to_vec(
            &serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}}),
        )
        .unwrap();
        store.save_funding_quote(&job.id, &quote).unwrap();
        store
            .require_preparation_authority(&job.operation_id, Some(&authority))
            .unwrap();
        store
            .refresh_unprepared_quote_for_operation(&job.id, &job.operation_id)
            .unwrap();
        assert!(
            store
                .require_preparation_authority(&job.operation_id, Some(&authority))
                .is_err()
        );
        let current = store.funding_job(&job.id).unwrap().unwrap();
        store.save_funding_quote(&current.id, &quote).unwrap();
        store
            .require_preparation_authority(&current.operation_id, Some(&authority))
            .unwrap();
        store
            .db
            .execute("UPDATE pools SET enabled=0 WHERE id=?1", [&job.pool_id])
            .unwrap();
        assert!(
            store
                .require_preparation_authority(&current.operation_id, Some(&authority))
                .is_err()
        );
        let effects: bool = store
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM outgoing) OR EXISTS(SELECT 1 FROM budget_entries)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!effects);
    }

    #[test]
    fn clean_quote_policy_changes_preserve_original_intent_and_stop_after_readiness() {
        let (_dir, mut store, job) = fixture();
        let configured = |target: &str, ceiling: &str| crate::rotation::config::ReservesConfig {
            small: crate::rotation::config::ReserveConfig {
                funding_amount_usdc: target.into(),
                max_funding_amount_usdc: Some(ceiling.into()),
            },
            large: None,
        };
        store
            .configure_capacity_group(
                &job.pool_id,
                Some(U256::from(1_000_000)),
                0,
                &configured("0.000020", "0.000030"),
            )
            .unwrap();
        store
            .select_quote_terms(&job.id, &job.operation_id, "20", "30")
            .unwrap();
        store
            .configure_capacity_group(
                &job.pool_id,
                Some(U256::from(1_000_000)),
                0,
                &configured("0.000002", "0.000003"),
            )
            .unwrap();
        store
            .select_quote_terms(&job.id, &job.operation_id, "2", "3")
            .unwrap();
        let saved:(String,String,String)=store.db.query_row("SELECT f.target,j.quote_target,j.quote_ceiling FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id WHERE f.id=?1",[&job.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(saved, (job.target.clone(), "2".into(), "3".into()));
        store
            .record_destination_readiness(&job.id, &job.target, "block", 1)
            .unwrap();
        assert!(
            store
                .select_quote_terms(&job.id, &job.operation_id, "20", "30")
                .is_err()
        );
        assert_eq!(
            store.destination_requirement(&job.id).unwrap().amount,
            job.target
        );
    }

    #[cfg(feature = "zcash")]
    #[test]
    fn destination_closure_does_not_wait_for_source_accounting_but_requires_undisputed_support() {
        use crate::rotation::base::{
            Anchor,
            evidence::{BalanceObservation, BalancePoint},
        };
        let (_dir, mut store, job) = fixture();
        let quote = serde_json::to_vec(
            &serde_json::json!({"request":{"amount":"10","recipient":job.recipient}}),
        )
        .unwrap();
        store.save_funding_quote(&job.id, &quote).unwrap();
        store.db.execute("UPDATE funding_progress SET accepted_output='10',accepted_recipient=?2 WHERE job_id=?1",params![job.id,job.recipient]).unwrap();
        store
            .record_provider_outcome(
                &job.id,
                &job.operation_id,
                &crate::rotation::near::SwapStatus::Success,
                1,
            )
            .unwrap();
        assert!(destination_obligation_pending(&store.db, &job.wallet_id).unwrap());
        let support = super::super::evidence::BalanceSupport {
            checkpoint: None,
            observation: BalanceObservation {
                wallet: job.wallet_id.clone(),
                address: job.recipient.clone(),
                network: "eip155:8453".into(),
                asset: crate::payment::USDC.into(),
                endpoint: "a".repeat(64),
                confirmations: 1,
                confirmed: BalancePoint {
                    anchor: Anchor {
                        height: 1,
                        hash: format!("0x{:064x}", 1),
                    },
                    amount: U256::from(10),
                },
                latest: BalancePoint {
                    anchor: Anchor {
                        height: 2,
                        hash: format!("0x{:064x}", 2),
                    },
                    amount: U256::from(10),
                },
                valid_until: 100,
            },
        };
        let mut insufficient = support.clone();
        insufficient.observation.latest.amount = U256::from(9);
        assert!(!record_readiness(&store.db, &job.wallet_id, &insufficient).unwrap());
        assert!(record_readiness(&store.db, &job.wallet_id, &support).unwrap());
        let mut spent = support.clone();
        spent.observation.latest.amount = U256::ZERO;
        assert!(record_readiness(&store.db, &job.wallet_id, &spent).unwrap());
        let saved: String = store
            .db
            .query_row(
                "SELECT readiness_capacity FROM funding_jobs WHERE id=?1",
                [&job.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(saved, "10");
        assert!(!destination_obligation_pending(&store.db, &job.wallet_id).unwrap());
        assert!(
            !store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        dispute_support(
            &store.db,
            &job.wallet_id,
            &support.observation.confirmed.anchor.hash,
            "chain_repair_required",
        )
        .unwrap();
        assert!(destination_obligation_pending(&store.db, &job.wallet_id).unwrap());
        assert!(store.destination_requirement(&job.id).unwrap().recorded);
        assert_eq!(
            store.funding_job(&job.id).unwrap().unwrap().operation_id,
            job.operation_id
        );
    }

    #[test]
    fn recorded_requirement_survives_spending_and_policy_changes() {
        let (_dir, mut store, job) = fixture();
        let mut reserves = super::super::test_support::reserves("0.000010");
        reserves.small.max_funding_amount_usdc = Some("0.000020".into());
        store
            .configure_capacity_group(&job.pool_id, Some(U256::from(1_000_000)), 0, &reserves)
            .unwrap();
        let quote = serde_json::to_vec(
            &serde_json::json!({"request":{"amount":"20","recipient":job.recipient}}),
        )
        .unwrap();
        store.save_funding_quote(&job.id, &quote).unwrap();
        assert!(
            !store
                .record_destination_readiness(&job.id, "10", "block", 1)
                .unwrap()
        );
        assert!(
            store
                .record_destination_readiness(&job.id, "20", "block", 1)
                .unwrap()
        );
        assert!(
            store
                .record_destination_readiness(&job.id, "0", "later", 2)
                .unwrap()
        );
        let requirement = store.destination_requirement(&job.id).unwrap();
        assert_eq!(requirement.amount, "20");
        assert_eq!(
            requirement.operation_id.as_deref(),
            Some(job.operation_id.as_str())
        );
        assert!(
            !store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
    }

    #[test]
    fn external_readiness_closes_without_inventing_source_or_provider_facts() {
        let (_dir, mut store, job) = fixture();
        assert!(
            store
                .record_destination_readiness(&job.id, &job.target, "block", 1)
                .unwrap()
        );
        assert!(
            store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        let outcome: String = store
            .db
            .query_row(
                "SELECT completion_outcome FROM funding_jobs WHERE id=?1",
                [&job.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(outcome, "EXTERNAL");
        assert_eq!(
            store
                .db
                .query_row("SELECT COUNT(*) FROM outgoing", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn readiness_closes_a_clean_quote_without_starting_replacement() {
        let (_dir, mut store, job) = fixture();
        let quote = serde_json::to_vec(
            &serde_json::json!({"request":{"amount":"10","recipient":job.recipient}}),
        )
        .unwrap();
        store.save_funding_quote(&job.id, &quote).unwrap();
        store
            .record_destination_readiness(&job.id, "10", "block", 1)
            .unwrap();
        store.close_unprepared_funding(&job.id).unwrap();
        assert!(
            store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        assert_eq!(
            store.destination_requirement(&job.id).unwrap().operation_id,
            Some(job.operation_id)
        );
    }

    #[test]
    fn mismatched_destination_cannot_record_readiness() {
        let (_dir, mut store, job) = fixture();
        let quote = serde_json::to_vec(
            &serde_json::json!({"request":{"amount":"10","recipient":"another-wallet"}}),
        )
        .unwrap();
        assert!(store.save_funding_quote(&job.id, &quote).is_err());
        let recorded: bool = store
            .db
            .query_row(
                "SELECT readiness_requirement IS NOT NULL FROM funding_jobs WHERE id=?1",
                [&job.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!recorded);
    }

    #[test]
    fn early_serving_claim_retains_quote_refresh_and_closed_attempt_authority() {
        for role in ["ACTIVE", "RETAINED"] {
            let (_dir, mut store, job) = fixture();
            let jobs: i64 = store
                .db
                .query_row("SELECT COUNT(*) FROM funding_jobs", [], |r| r.get(0))
                .unwrap();
            super::super::capacity::consume_reserve(&store.db, &job.pool_id, &job.wallet_id)
                .unwrap();
            if role == "RETAINED" {
                store
                    .db
                    .execute(
                        "UPDATE wallets SET role='RETAINED' WHERE id=?1",
                        [&job.wallet_id],
                    )
                    .unwrap();
            }
            let claim = store.current_funding_claim(&job.id).unwrap();
            store
                .select_quote_terms(
                    &job.id,
                    &job.operation_id,
                    &claim.target.to_string(),
                    &claim.maximum_output.to_string(),
                )
                .unwrap();
            let quote = serde_json::to_vec(&serde_json::json!({"request":{"amount":claim.target.to_string(),"recipient":job.recipient}})).unwrap();
            store.save_funding_quote(&job.id, &quote).unwrap();
            store
                .refresh_unprepared_quote_for_operation(&job.id, &job.operation_id)
                .unwrap();
            let current = store.funding_job(&job.id).unwrap().unwrap();
            assert_ne!(current.operation_id, job.operation_id);
            assert_eq!(current.wallet_id, job.wallet_id);
            store
                .close_unprepared_funding_for_operation(&job.id, &current.operation_id)
                .unwrap();
            store
                .reauthorize_closed_funding_for_operation(&job.id, &current.operation_id, 3)
                .unwrap();
            assert!(store.current_funding_claim(&job.id).is_ok());
            assert_eq!(
                store
                    .db
                    .query_row(
                        "SELECT role FROM wallets WHERE id=?1",
                        [&job.wallet_id],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                role
            );
            assert_eq!(
                store
                    .db
                    .query_row("SELECT COUNT(*) FROM funding_jobs", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                jobs
            );
        }
    }

    #[test]
    fn stale_operation_cannot_advance_a_refreshed_quote() {
        let (_dir, mut store, job) = fixture();
        let quote = &serde_json::to_vec(
            &serde_json::json!({"request":{"amount":"10","recipient":job.recipient}}),
        )
        .unwrap();
        store.save_funding_quote(&job.id, quote).unwrap();
        store.refresh_unprepared_quote(&job.id).unwrap();
        assert!(
            store
                .save_quote(&job.id, quote, None, Some(&job.operation_id))
                .is_err()
        );
        store.save_funding_quote(&job.id, quote).unwrap();
        assert!(
            store
                .refresh_unprepared_quote_for_operation(&job.id, &job.operation_id)
                .is_err()
        );
        assert!(
            store
                .defer_unstarted_preparation_for_operation(&job.id, &job.operation_id)
                .is_err()
        );
        assert!(
            store
                .close_unprepared_funding_for_operation(&job.id, &job.operation_id)
                .is_err()
        );
        assert!(
            store
                .reauthorize_closed_funding_for_operation(&job.id, &job.operation_id, 1)
                .is_err()
        );
        store
            .defer_funding_result(
                &job.id,
                Some(&job.operation_id),
                99999,
                Some("late old-operation error"),
                false,
            )
            .unwrap();
        assert!(
            store
                .advance_funding_operation(
                    &job.id,
                    &job.operation_id,
                    FundingPhase::Quoted,
                    FundingPhase::Preparing
                )
                .is_err()
        );
        let current = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.id == job.id)
            .unwrap();
        assert_ne!(current.operation_id, job.operation_id);
        assert_eq!(current.phase, FundingPhase::Quoted);
        assert_eq!(current.next_poll, 0);
        assert!(current.last_error.is_none());
    }

    #[test]
    fn safe_closure_preserves_identity_and_does_not_dispatch_another_attempt() {
        let (_dir, mut store, job) = fixture();
        store
            .save_funding_quote(
                &job.id,
                &serde_json::to_vec(
                    &serde_json::json!({"request":{"amount":"10","recipient":job.recipient}}),
                )
                .unwrap(),
            )
            .unwrap();
        store.close_unprepared_funding(&job.id).unwrap();
        store.close_unprepared_funding(&job.id).unwrap();
        let current = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.id == job.id)
            .unwrap();
        assert_eq!(current.operation_id, job.operation_id);
        assert_eq!(current.phase, FundingPhase::RecoveryRequired);
        assert_eq!(store.funding_recovery_count(&job.id).unwrap(), 1);
        assert!(store.funding_quote(&job.id).is_ok());
        assert!(
            !store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
    }

    #[cfg(feature = "zcash")]
    #[test]
    fn success_requires_source_accounting_and_keeps_original_credit_after_spending() {
        use crate::rotation::near::SwapStatus;
        let (_dir, mut store, job) = fixture();
        store
            .save_funding_quote(
                &job.id,
                &serde_json::to_vec(
                    &serde_json::json!({"request":{"amount":"10","recipient":job.recipient}}),
                )
                .unwrap(),
            )
            .unwrap();
        store
            .record_destination_readiness(&job.id, "10", "block", 1)
            .unwrap();
        store
            .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Success, 1)
            .unwrap();
        assert!(
            !store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        store.db.execute("INSERT INTO budget_entries(id,pool_id,day,original_day,requested,reserved,consumed) VALUES(?1,?2,0,0,100,100,0)",params![job.operation_id,job.pool_id]).unwrap();
        store
            .db
            .execute(
                "INSERT INTO outgoing(id,state,raw,revision) VALUES(?1,'PREPARED',X'00',1)",
                [&job.operation_id],
            )
            .unwrap();
        let facts = crate::rotation::transaction::TransactionFacts {
            txid: "source".into(),
            expiry_height: 100,
            amount_zatoshis: 90,
            fee_zatoshis: 10,
            deadline: u64::MAX,
        };
        store.db.execute("INSERT INTO treasury_operations(id,facts,submission,attempts) VALUES(?1,?2,'CONFIRMED',1)",params![job.operation_id,serde_json::to_string(&facts).unwrap()]).unwrap();
        // A confirmation label alone cannot bypass gross expenditure accounting.
        assert!(
            !store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        store.confirm_spend(&job.operation_id, 100, 0).unwrap();
        store
            .db
            .execute(
                "UPDATE wallets SET balance='0' WHERE id=?1",
                [&job.wallet_id],
            )
            .unwrap();
        assert!(
            store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        assert!(
            store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        assert_eq!(store.destination_requirement(&job.id).unwrap().amount, "10");
    }

    #[cfg(feature = "zcash")]
    fn submit_source_for_status(store: &mut Store, job: &FundingJob) {
        use crate::rotation::transaction::{ManagedFundingAuthority, TransactionFacts};
        assert!(
            !store
                .funding_facts(&job.id, &job.operation_id)
                .unwrap()
                .status_needed(),
            "a quote alone must not trigger status I/O"
        );
        store
            .begin_funding_preparation(
                &job.operation_id,
                Some(&ManagedFundingAuthority {
                    job_id: job.id.clone(),
                    auto_fund: true,
                    max_attempts: 3,
                }),
            )
            .unwrap();
        store
            .reserve(&job.operation_id, Some(&job.pool_id), 1, 100)
            .unwrap();
        let revision = store.snapshot().unwrap().0;
        store
            .prepare_with_facts(
                &job.operation_id,
                revision,
                b"snapshot",
                b"signed fixture bytes",
                Some(TransactionFacts {
                    txid: "transaction".into(),
                    expiry_height: 100,
                    amount_zatoshis: 50,
                    fee_zatoshis: 10,
                    deadline: u64::MAX,
                }),
            )
            .unwrap();
        assert!(
            !store
                .funding_facts(&job.id, &job.operation_id)
                .unwrap()
                .status_needed(),
            "private prepared bytes must not trigger status I/O"
        );
        store
            .request_broadcast(&job.operation_id, 1, 1, false)
            .unwrap();
        assert!(
            store
                .funding_facts(&job.id, &job.operation_id)
                .unwrap()
                .status_needed()
        );
    }

    #[cfg(feature = "zcash")]
    #[test]
    fn incomplete_deposit_keeps_observing_until_a_terminal_provider_fact() {
        use crate::rotation::near::SwapStatus;
        for terminal in [SwapStatus::Success, SwapStatus::Refunded] {
            let (_dir, mut store, job) = fixture();
            assert!(
                !store
                    .funding_facts(&job.id, &job.operation_id)
                    .unwrap()
                    .status_needed()
            );
            let quote = serde_json::to_vec(
                &serde_json::json!({"request":{"amount":"10","recipient":job.recipient}}),
            )
            .unwrap();
            store.save_funding_quote(&job.id, &quote).unwrap();
            submit_source_for_status(&mut store, &job);
            store.disable_pool(&job.pool_name).unwrap();
            for (n, status) in [
                SwapStatus::Processing,
                SwapStatus::IncompleteDeposit,
                SwapStatus::Failed,
            ]
            .iter()
            .enumerate()
            {
                store
                    .record_provider_outcome(&job.id, &job.operation_id, status, n as u64 + 1)
                    .unwrap();
                assert!(
                    store
                        .funding_facts(&job.id, &job.operation_id)
                        .unwrap()
                        .status_needed(),
                    "pause or a nonterminal observation must not abandon submitted work"
                );
            }
            store
                .record_provider_outcome(&job.id, &job.operation_id, &terminal, 4)
                .unwrap();
            assert!(
                !store
                    .funding_facts(&job.id, &job.operation_id)
                    .unwrap()
                    .status_needed()
            );
            assert_eq!(
                store.funding_job(&job.id).unwrap().unwrap().phase,
                FundingPhase::RecoveryRequired
            );
            assert_eq!(store.operation(&job.operation_id).unwrap().attempts, 1);
        }
    }

    #[cfg(feature = "zcash")]
    #[test]
    fn provider_conflicts_preserve_both_facts_and_require_review() {
        use crate::rotation::near::SwapStatus;
        let (_dir, mut store, job) = fixture();
        store
            .save_funding_quote(
                &job.id,
                &serde_json::to_vec(
                    &serde_json::json!({"request":{"amount":"10","recipient":job.recipient}}),
                )
                .unwrap(),
            )
            .unwrap();
        submit_source_for_status(&mut store, &job);
        store
            .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Success, 1)
            .unwrap();
        assert!(
            !store
                .funding_facts(&job.id, &job.operation_id)
                .unwrap()
                .status_needed()
        );
        store
            .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Processing, 2)
            .unwrap();
        store
            .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Refunded, 3)
            .unwrap();
        let (success, refunded, phase): (bool,bool,String) = store.db.query_row("SELECT provider_success,provider_refunded,phase FROM funding_progress WHERE job_id=?1",[&job.id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert!(success && refunded);
        assert_eq!(
            serde_json::from_str::<FundingPhase>(&phase).unwrap(),
            FundingPhase::RecoveryRequired
        );
        assert!(
            !store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        assert!(
            store
                .record_provider_outcome(&job.id, "another-operation", &SwapStatus::Success, 4)
                .is_err()
        );
    }
}

#[cfg(test)]
mod recovery_classification_tests {
    use super::*;

    #[test]
    fn recovery_eligibility_uses_bound_closure_not_mutable_diagnostics() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"fixture")?;
        let identity = store.id().to_owned();
        super::super::test_support::declare_two_reserves(
            &mut store,
            "recovery",
            "1",
            Some(U256::from(10)),
            0,
        )?;
        let jobs = store.funding_jobs()?;
        let unknown = &jobs[0];
        store.advance_funding(
            &unknown.id,
            FundingPhase::Allocated,
            FundingPhase::RecoveryRequired,
        )?;
        for diagnostic in [
            None,
            Some("quote_refresh_exhausted; no preparation started"),
            Some("funding_quote_failed; forged"),
            Some("funding_attempt_closed; forged"),
        ] {
            store.defer_funding(&unknown.id, 0, diagnostic, false)?;
            let job = store.funding_job(&unknown.id)?.unwrap();
            let facts = store.funding_facts(&job.id, &job.operation_id)?;
            assert!(!can_recover_facts(&job, None, &facts));
            assert!(store.close_unprepared_funding(&job.id).is_err());
            assert_eq!(store.funding_recovery_count(&job.id)?, 0);
            assert_eq!(
                store.funding_job(&job.id)?.unwrap().operation_id,
                job.operation_id
            );
        }
        let closed = &jobs[1];
        store.close_unprepared_funding(&closed.id)?;
        for diagnostic in [
            None,
            Some("funding_recovery_requires_operator; conflicting_transaction_or_input_evidence"),
            Some("arbitrary annotation"),
        ] {
            store.db.execute(
                "UPDATE funding_progress SET last_error=?2 WHERE job_id=?1",
                params![closed.id, diagnostic],
            )?;
            let job = store.funding_job(&closed.id)?.unwrap();
            let facts = store.funding_facts(&job.id, &job.operation_id)?;
            assert!(facts.source_closed && can_recover_facts(&job, None, &facts));
            store.close_unprepared_funding(&job.id)?;
            assert_eq!(store.funding_recovery_count(&job.id)?, 1);
        }
        drop(store);
        let mut store = Store::open(&state, &key, &identity)?;
        let job = store.funding_job(&closed.id)?.unwrap();
        assert!(
            store
                .funding_facts(&job.id, &job.operation_id)?
                .source_closed
        );
        store.db.execute("INSERT INTO budget_entries(id,pool_id,day,original_day,requested,reserved,consumed) VALUES (?1,?2,1,1,10,10,0)", params![job.operation_id, job.pool_id])?;
        assert!(
            !store
                .funding_facts(&job.id, &job.operation_id)?
                .source_closed
        );
        assert!(store.close_unprepared_funding(&job.id).is_err());
        assert!(store.reauthorize_closed_funding(&job.id, 3).is_err());
        assert_eq!(
            store.funding_job(&job.id)?.unwrap().operation_id,
            job.operation_id
        );
        store.db.execute(
            "DELETE FROM budget_entries WHERE id=?1",
            [&job.operation_id],
        )?;
        store.deny_new_funding();
        assert!(store.reauthorize_closed_funding(&job.id, 3).is_err());
        assert_eq!(
            store.funding_job(&job.id)?.unwrap().operation_id,
            job.operation_id
        );
        assert_eq!(store.funding_recovery_count(&job.id)?, 1);
        // An archive under a different job is not provenance for this operation.
        store.db.execute(
            "UPDATE funding_recovery SET job_id=?2 WHERE operation_id=?1",
            params![job.operation_id, unknown.id],
        )?;
        assert!(
            !store
                .funding_facts(&job.id, &job.operation_id)?
                .source_closed
        );
        assert!(store.close_unprepared_funding(&job.id).is_err());
        Ok(())
    }
}

/// Archive the exact closed source attempt without granting replacement authority.
pub(super) fn archive_unprepared(db: &Connection, job: &str, operation: &str) -> Result<()> {
    db.execute("INSERT INTO funding_recovery(operation_id,job_id,phase,quote,refund,provider_success,provider_failed,provider_refunded,provider_incomplete,provider_observed_at,quote_target,quote_ceiling,accepted_output,accepted_recipient) SELECT j.operation_id,j.job_id,j.phase,j.quote,r.address,j.provider_success,j.provider_failed,j.provider_refunded,j.provider_incomplete,j.provider_observed_at,j.quote_target,j.quote_ceiling,j.accepted_output,j.accepted_recipient FROM funding_progress j LEFT JOIN funding_refunds r ON r.job_id=j.job_id WHERE j.job_id=?1 AND j.operation_id=?2",params![job,operation])?;
    db.execute("UPDATE funding_progress SET phase='\"RECOVERY_REQUIRED\"',last_error='funding_attempt_closed; current authority required for another attempt' WHERE job_id=?1 AND operation_id=?2",params![job,operation])?;
    Ok(())
}
