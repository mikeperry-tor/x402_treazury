//! Funding workflow journal. The source operation ID is allocated before any
//! calculation and remains immutable across crashes and ambiguous submissions.
pub(super) const RECOVERY_COUNT_SQL: &str = "SELECT COUNT(*) FROM funding_recovery r WHERE job_id=?1 AND NOT EXISTS(SELECT 1 FROM funding_quote_refreshes q WHERE q.operation_id=r.operation_id)";

use super::*;
// The current schema journals acceptance before any SDK calculation. A missing
// accepted intent AND outgoing journal proves that this code never calculated.
pub(crate) fn safely_unprepared(facts: &FundingFacts) -> bool {
    facts.source_closed
        || (!facts.preparation_accepted
            && !facts.source_pending
            && facts.source_submission.is_none())
}
pub(crate) fn can_recover_facts(
    operation: Option<&crate::rotation::transaction::OperationStatus>,
    facts: &FundingFacts,
) -> bool {
    !facts.provider_success
        && operation
            .map(|op| {
                op.submission != "CONFIRMED" || facts.provider_failed || facts.provider_refunded
            })
            .unwrap_or_else(|| safely_unprepared(facts))
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
    /// Current claim eligibility for an uncredited, never-submitted attempt.
    /// Mutation boundaries still check actual preparation/submission authority.
    pub new_funding_allowed: bool,
    pub source_pending: bool,
    pub preparation_accepted: bool,
    pub initial_credit_recorded: bool,
    pub accepted_quote: bool,
    pub archived: bool,
    /// Exact operation/job archive plus zero remaining source liability.
    pub source_closed: bool,
    pub provider_success: bool,
    pub provider_failed: bool,
    pub provider_refunded: bool,
    pub provider_incomplete: bool,
    pub source_submission: Option<String>,
    pub source_attempts: u32,
}
impl FundingFacts {
    /// Fresh allocations cannot receive swap credit before source work is accepted.
    /// Keep uncertain, completed and recovered source attempts observable across restart.
    pub(crate) fn credit_needed(&self) -> bool {
        !self.initial_credit_recorded
            && (self.preparation_accepted
                || self.source_pending
                || self.source_submission.is_some()
                || self.source_attempts > 0
                || self.provider_success)
    }
    pub fn status_needed(&self) -> bool {
        self.accepted_quote
            && (self.source_attempts > 0
                || self.source_submission.as_deref() == Some("CONFIRMED")
                || self.provider_incomplete)
            && !self.provider_success
            && !self.provider_failed
            && !self.provider_refunded
    }
    pub fn source_reconciliation_needed(&self) -> bool {
        !self.provider_success && self.source_pending && self.source_attempts > 0
    }
}

/// Historical conversion preflight fixture. Runtime opens now require the current schema
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
        "SELECT f.readiness_operation,f.readiness_requirement,f.target,j.operation_id,j.quote IS NOT NULL,j.accepted_output,j.accepted_recipient,w.address FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id JOIN wallets w ON w.id=f.wallet_id WHERE f.id=?1",
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

/// The wallet's unique initial claim is consumed by its first positive observed
/// credit. It is never reopened by spending, reorgs, refunds or provider flags.
/// Existing readiness columns retain this fact; they no longer certify a target.
pub(crate) fn record_initial_credit(db: &Connection, wallet: &str, balance: U256) -> Result<bool> {
    if balance > U256::ZERO {
        db.execute("UPDATE funding_jobs SET readiness_operation=(SELECT operation_id FROM funding_progress WHERE job_id=funding_jobs.id),readiness_requirement=?2,readiness_capacity=?2 WHERE wallet_id=?1 AND readiness_requirement IS NULL",params![wallet,balance.to_string()])?;
    }
    initial_credit_recorded(db, wallet)
}
pub(crate) fn initial_credit_recorded(db: &Connection, wallet: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT readiness_requirement IS NOT NULL FROM funding_jobs WHERE wallet_id=?1",
        [wallet],
        |r| r.get(0),
    )?)
}
fn source_attempt_closed(db: &Connection, operation: &str) -> Result<bool> {
    Ok(db.query_row("SELECT (NOT EXISTS(SELECT 1 FROM outgoing WHERE id=?1) AND NOT EXISTS(SELECT 1 FROM treasury_operations WHERE id=?1) AND NOT EXISTS(SELECT 1 FROM operation_network WHERE operation_id=?1 AND preparation IS NOT NULL)) OR (EXISTS(SELECT 1 FROM outgoing WHERE id=?1 AND state='RESOLVED') AND (EXISTS(SELECT 1 FROM expired_operations WHERE id=?1) OR EXISTS(SELECT 1 FROM treasury_operations WHERE id=?1 AND submission='CONFIRMED')))",[operation],|r|r.get(0))?)
}
#[cfg(test)]
pub(crate) fn record_readiness(
    db: &Connection,
    wallet: &str,
    support: &super::evidence::BalanceSupport,
) -> Result<bool> {
    record_initial_credit(db, wallet, support.observation.available())
}

impl Store {
    /// Persist a current advisory read against the captured initial operation.
    /// A positive credit consumes that wallet's unique claim, including partial credit.
    #[cfg(test)]
    pub(crate) fn record_funding_balance(
        &mut self,
        job: &str,
        operation: &str,
        wallet: &str,
        recipient: &str,
        balance: U256,
    ) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (pool,address):(String,String)=tx.query_row("SELECT w.pool_id,w.address FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id JOIN funding_progress j ON j.job_id=f.id WHERE f.id=?1 AND j.operation_id=?2 AND w.id=?3",params![job,operation,wallet],|r|Ok((r.get(0)?,r.get(1)?)))?;
        ensure!(
            address.parse::<alloy_primitives::Address>()?
                == recipient.parse::<alloy_primitives::Address>()?,
            "initial credit recipient changed"
        );
        tx.execute("UPDATE wallets SET balance=?2,low_balance=refresh_pending,balance_refresh_error=NULL WHERE id=?1",params![wallet,balance.to_string()])?;
        if record_initial_credit(&tx, wallet, balance)? {
            publish_initial_credit(&tx, wallet)?;
        }
        tx.commit()?;
        self.notify_capacity_changed(&pool);
        Ok(())
    }
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
            if let Err(error) = self
                .replace_completed_reserve(&pool, kind)
                .and_then(|()| self.allocate_capacity_kind(&pool, kind, true))
            {
                failure.get_or_insert(error);
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub fn funding_facts(&self, job: &str, operation: &str) -> Result<FundingFacts> {
        let fulfilled = provider_succeeded(&self.db, operation)?;
        let (mut facts, flags, (submission, attempts)) = self.db.query_row(
            "SELECT f.readiness_requirement IS NOT NULL,j.quote IS NOT NULL,EXISTS(SELECT 1 FROM funding_recovery r WHERE r.operation_id=j.operation_id AND r.job_id=j.job_id),j.provider_success,j.provider_failed,j.provider_refunded,j.provider_incomplete,o.submission,COALESCE(o.attempts,0),EXISTS(SELECT 1 FROM wallets w JOIN pools p ON p.id=w.pool_id WHERE w.id=f.wallet_id AND p.enabled=1),EXISTS(SELECT 1 FROM outgoing x WHERE x.id=j.operation_id AND x.state='PREPARED'),EXISTS(SELECT 1 FROM operation_network n WHERE n.operation_id=j.operation_id AND n.preparation IS NOT NULL),o.id IS NOT NULL,EXISTS(SELECT 1 FROM expired_operations e WHERE e.id=j.operation_id) FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id LEFT JOIN treasury_operations o ON o.id=j.operation_id WHERE f.id=?1 AND j.operation_id=?2",
            params![job,operation], |r| Ok((FundingFacts {
                initial_credit_recorded:r.get(0)?, accepted_quote:r.get(1)?, archived:r.get(2)?, source_closed: false,
                provider_success:false, provider_failed:false, provider_refunded:false, provider_incomplete:false,
                source_submission:None, source_attempts:0,
                new_funding_allowed:r.get(9)?, source_pending:r.get(10)?,preparation_accepted:r.get(11)?,
            }, provider_flags(r, 3), observation_disclosure(r,7,8,r.get(12)?,r.get(13)?)?)),
        ).context("funding operation binding changed")?;
        facts.source_submission = submission;
        facts.source_attempts = u32::try_from(attempts).unwrap_or(1);
        (
            facts.provider_success,
            facts.provider_failed,
            facts.provider_refunded,
            facts.provider_incomplete,
        ) = flags.context(super::scheduling::FundingDecodeFailure)?;
        facts.provider_success |= fulfilled;
        facts.new_funding_allowed = facts.new_funding_allowed
            && !facts.provider_success
            && !facts.provider_failed
            && !facts.provider_refunded
            && !facts.provider_incomplete
            && !self.source_previously_closed(operation)?
            && facts.source_attempts == 0
            && facts.source_submission.as_deref() != Some("CONFIRMED")
            && !facts.initial_credit_recorded
            && self.funding_restriction == super::super::restriction::FundingRestriction::Ordinary
            && capacity::funding_claim_present(&self.db, job)?;
        facts.source_closed = facts.archived && source_attempt_closed(&self.db, operation)?;
        Ok(facts)
    }

    /// NEAR owns settlement of this exact accepted swap, not SDK source history.
    pub(crate) fn funding_operation_fulfilled(&mut self, operation: &str) -> Result<bool> {
        let tx = self.db.transaction()?;
        let fulfilled = close_fulfilled_source(&tx, operation)?;
        tx.commit()?;
        Ok(fulfilled)
    }

    /// Replacing disclosed work needs a terminal provider outcome, not source expiry alone.
    pub(crate) fn funding_retry_allowed(&self, job: &str, operation: &str) -> Result<bool> {
        let ((success, failed, refunded, incomplete), attempts, confirmed) = self.db.query_row(
            "SELECT j.provider_success,j.provider_failed,j.provider_refunded,j.provider_incomplete,COALESCE(o.attempts,0),COALESCE(o.submission='CONFIRMED',0),o.submission,o.id IS NOT NULL,EXISTS(SELECT 1 FROM expired_operations e WHERE e.id=j.operation_id) FROM funding_progress j LEFT JOIN treasury_operations o ON o.id=j.operation_id WHERE j.job_id=?1 AND j.operation_id=?2",
            params![job,operation], |r| Ok((provider_flags(r,0)?,observation_disclosure(r,6,4,r.get(7)?,r.get(8)?)?.1,r.get::<_,bool>(5)?)),
        )?;
        Ok(!success && (failed || refunded || (attempts == 0 && !confirmed && !incomplete)))
    }

    pub fn require_funding_attempt(&self, job: &str, max_attempts: u32) -> Result<()> {
        self.funding_restriction
            .require_new_funding("funding_attempt_start")?;
        self.check_job_permit(job, 1)?;
        ensure!(
            self.funding_recovery_count(job)? <= u64::from(max_attempts),
            crate::rotation::error::RecoveryAttemptLimit
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
            .then(|| self.observation_operation(&job.operation_id))
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

    pub fn funding_source_ready(&self) -> Result<bool> {
        Ok(!self.preparation_pending()?)
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
        let changed=self.db.execute("UPDATE funding_progress SET quote_target=?3,quote_ceiling=?4 WHERE job_id=?1 AND operation_id=?2 AND phase='\"ALLOCATED\"' AND quote IS NULL AND NOT EXISTS(SELECT 1 FROM outgoing WHERE id=?2) AND NOT EXISTS(SELECT 1 FROM operation_network WHERE operation_id=?2 AND preparation IS NOT NULL) AND EXISTS(SELECT 1 FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id JOIN pools p ON p.id=w.pool_id WHERE f.id=?1 AND f.state='QUEUED' AND f.readiness_requirement IS NULL AND w.role='ALLOCATED' AND p.enabled=1)",params![job,operation,target,ceiling])?;
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

    /// Settle existing work and read the bound current decision snapshot in one
    /// serialized Store job. The reused mutations retain their own transactions.
    pub(crate) fn settle_funding_source(
        &mut self,
        job: &str,
        operation: &str,
        phase_reconciliation_due: bool,
    ) -> Result<Option<(FundingJob, FundingFacts)>> {
        if phase_reconciliation_due {
            self.reconcile_funding_phase(job, operation)?;
        }
        if self.finish_funding_if_proven(job, operation)? {
            return Ok(None);
        }
        let current = self.funding_job(job)?.context("funding job missing")?;
        ensure!(
            current.operation_id == operation,
            "funding operation binding changed"
        );
        let facts = self.funding_facts(job, operation)?;
        if facts.initial_credit_recorded
            && !facts.preparation_accepted
            && !facts.source_pending
            && matches!(
                current.phase,
                FundingPhase::Allocated | FundingPhase::Quoted
            )
        {
            self.close_unprepared_funding_for_operation(job, operation)?;
            ensure!(
                self.finish_funding_if_proven(job, operation)?,
                "closed destination lacks completion proof"
            );
            return Ok(None);
        }
        Ok(Some((current, facts)))
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
        let (success, failed, refunded, incomplete) = tx.query_row("SELECT provider_success,provider_failed,provider_refunded,provider_incomplete FROM funding_progress WHERE job_id=?1 AND operation_id=?2 AND quote IS NOT NULL", params![job,operation], |r| provider_flags(r,0))?;
        let changed = tx.execute("UPDATE funding_progress SET provider_success=?3,provider_failed=?4,provider_refunded=?5,provider_incomplete=?6,provider_observed_at=MAX(COALESCE(provider_observed_at,0),?7) WHERE job_id=?1 AND operation_id=?2 AND quote IS NOT NULL", params![job,operation,success || matches!(status,SwapStatus::Success),failed || matches!(status,SwapStatus::Failed),refunded || matches!(status,SwapStatus::Refunded),incomplete || matches!(status,SwapStatus::IncompleteDeposit),i64::try_from(observed_at)?])?;
        ensure!(changed == 1, "funding operation correlation mismatch");
        derive_operation_phase(&tx, job, operation)?;
        tx.commit()?;
        Ok(())
    }
}

type ProviderFlags = (bool, bool, bool, bool);

fn provider_flag(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<bool> {
    Ok(matches!(row.get_ref(index)?, rusqlite::types::ValueRef::Integer(value) if value != 0))
}

fn provider_flags(row: &rusqlite::Row<'_>, start: usize) -> rusqlite::Result<ProviderFlags> {
    Ok((
        provider_flag(row, start)?,
        provider_flag(row, start + 1)?,
        provider_flag(row, start + 2)?,
        provider_flag(row, start + 3)?,
    ))
}

/// Existing current and archived exact-operation provider facts, not a new ledger.
pub(super) fn provider_succeeded(db: &Connection, operation: &str) -> Result<bool> {
    let mut query = db.prepare("SELECT provider_success FROM funding_progress WHERE operation_id=?1 AND quote IS NOT NULL UNION ALL SELECT provider_success FROM funding_recovery WHERE operation_id=?1 AND quote IS NOT NULL")?;
    for success in query.query_map([operation], |r| provider_flag(r, 0))? {
        if success? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Idempotently close local source work while retaining factual SDK submission status.
pub(super) fn close_fulfilled_source(db: &Connection, operation: &str) -> Result<bool> {
    let fulfilled = provider_succeeded(db, operation)?;
    if fulfilled {
        db.execute(
            "UPDATE outgoing SET state='RESOLVED' WHERE id=?1",
            [operation],
        )?;
    }
    Ok(fulfilled)
}

pub(super) fn finish_if_proven(tx: &Connection, job: &str, operation: &str) -> Result<bool> {
    let (ready, correlation, flags, attempts, confirmed):
            (bool, Option<String>, rusqlite::Result<ProviderFlags>, rusqlite::Result<i64>, bool) = tx.query_row(
            "SELECT f.readiness_requirement IS NOT NULL,f.readiness_operation,j.provider_success,j.provider_failed,j.provider_refunded,j.provider_incomplete,COALESCE(o.attempts,0),EXISTS(SELECT 1 FROM treasury_operations o WHERE o.id=j.operation_id AND o.submission='CONFIRMED'),o.submission,o.id IS NOT NULL,EXISTS(SELECT 1 FROM expired_operations e WHERE e.id=j.operation_id) FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id LEFT JOIN treasury_operations o ON o.id=j.operation_id WHERE f.id=?1 AND j.operation_id=?2", params![job,operation], |r| Ok((r.get(0)?,r.get(1)?,provider_flags(r,2),observation_disclosure(r,8,6,r.get(9)?,r.get(10)?).map(|(_,attempts)|attempts),r.get(7)?)))?;
    let (_success, failed, refunded, incomplete) =
        flags.context(super::scheduling::FundingDecodeFailure)?;
    let wallet: String = tx.query_row(
        "SELECT wallet_id FROM funding_jobs WHERE id=?1",
        [job],
        |r| r.get(0),
    )?;
    let ready = ready && initial_credit_recorded(tx, &wallet)?;
    let fulfilled = close_fulfilled_source(tx, operation)?;
    if fulfilled {
        tx.execute("UPDATE funding_progress SET phase='\"VERIFYING_CREDIT\"',last_error=NULL WHERE job_id=?1 AND operation_id=?2", params![job,operation])?;
    }
    let outcome = if ready && correlation.as_deref() == Some(operation) && fulfilled {
        Some("SUCCESS")
    } else if ready
        && (failed
            || refunded
            || (!confirmed && !incomplete && attempts.unwrap_or(1) == 0))
        && correlation.as_deref().is_none_or(|id| id == operation)
        && source_attempt_closed(tx, operation)?
    {
        let signed: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1)",
            [operation],
            |r| r.get(0),
        )?;
        Some(if signed { "RECOVERED" } else { "EXTERNAL" })
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
pub(crate) fn publish_initial_credit(db: &Connection, wallet: &str) -> Result<()> {
    let job: String = db.query_row(
        "SELECT id FROM funding_jobs WHERE wallet_id=?1 AND readiness_requirement IS NOT NULL",
        [wallet],
        |r| r.get(0),
    )?;
    super::capacity::publish_funded_role(db, &job)?;
    // Source completion is observed separately; it cannot roll back an initial
    // credit or leave that consumed claim available for a second deposit.
    Ok(())
}

fn derive_operation_phase(db: &Connection, job: &str, operation: &str) -> Result<()> {
    let matched: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM funding_progress WHERE job_id=?1 AND operation_id=?2)",
        params![job, operation],
        |r| r.get(0),
    )?;
    ensure!(matched, "funding operation correlation mismatch");
    // NEAR fulfillment and destination readiness are independently established.
    if finish_if_proven(db, job, operation)? || provider_succeeded(db, operation)? {
        return Ok(());
    }
    db.execute("UPDATE funding_progress SET phase=CASE WHEN provider_failed=1 OR provider_refunded=1 THEN '\"REFUND_PENDING\"' WHEN provider_incomplete=1 THEN '\"RECOVERY_REQUIRED\"' WHEN EXISTS(SELECT 1 FROM treasury_operations o WHERE o.id=operation_id AND o.submission='CONFIRMED') THEN '\"SWAPPING\"' WHEN EXISTS(SELECT 1 FROM treasury_operations o JOIN outgoing x ON x.id=o.id WHERE o.id=operation_id AND o.attempts>0 AND x.state='PREPARED') THEN '\"DEPOSIT_PENDING\"' ELSE phase END WHERE job_id=?1 AND operation_id=?2 AND phase!='\"COMPLETE\"'", params![job,operation])?;
    Ok(())
}

/// Structural historical D validity is distinct from whether its original support
/// remains undisputed. Fresh payment evidence can establish current capacity.
pub(crate) fn readiness_recorded_valid(db: &Connection, wallet: &str) -> Result<bool> {
    let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version >= 19 {
        return initial_credit_recorded(db, wallet);
    }
    legacy_readiness_recorded_valid(db, wallet)
}
fn legacy_readiness_recorded_valid(db: &Connection, wallet: &str) -> Result<bool> {
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

/// Source work and bridge delivery are independent original obligations.
/// NEAR terminal outcomes dispose of delivery; local source expiry alone does not.
pub fn destination_obligation_pending(db: &Connection, wallet: &str) -> Result<bool> {
    let credited = initial_credit_recorded(db, wallet)?;
    if !credited
        && db.query_row(
            "SELECT EXISTS(SELECT 1 FROM funding_jobs WHERE wallet_id=?1 AND state!='COMPLETE')",
            [wallet],
            |r| r.get::<_, bool>(0),
        )?
    {
        return Ok(true);
    }
    let mut query = db.prepare("WITH operations AS (
        SELECT j.operation_id,j.provider_success,j.provider_failed,j.provider_refunded,j.provider_incomplete,j.quote FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id WHERE f.wallet_id=?1
        UNION ALL SELECT r.operation_id,r.provider_success,r.provider_failed,r.provider_refunded,r.provider_incomplete,r.quote FROM funding_jobs f JOIN funding_recovery r ON r.job_id=f.id WHERE f.wallet_id=?1 AND NOT EXISTS(SELECT 1 FROM funding_progress j WHERE j.job_id=r.job_id AND j.operation_id=r.operation_id))
        SELECT o.provider_success,o.provider_failed,o.provider_refunded,o.provider_incomplete,o.quote IS NOT NULL,
        EXISTS(SELECT 1 FROM operation_network n WHERE n.operation_id=o.operation_id AND n.preparation IS NOT NULL),
        EXISTS(SELECT 1 FROM outgoing x WHERE x.id=o.operation_id AND x.state='PREPARED'),
        COALESCE(t.attempts,0),COALESCE(t.submission='CONFIRMED',0),
        t.id IS NULL AND EXISTS(SELECT 1 FROM outgoing x WHERE x.id=o.operation_id),t.submission,t.id IS NOT NULL,EXISTS(SELECT 1 FROM expired_operations e WHERE e.id=o.operation_id)
        FROM operations o LEFT JOIN treasury_operations t ON t.id=o.operation_id")?;
    let mut original = false;
    for obligation in query.query_map([wallet], |r| {
        Ok((
            provider_flags(r, 0)?,
            r.get::<_, bool>(4)?,
            r.get::<_, bool>(5)?,
            r.get::<_, bool>(6)?,
            observation_disclosure(r, 10, 7, r.get(11)?, r.get(12)?).map(|(_, attempts)| attempts),
            r.get::<_, bool>(8)?,
            r.get::<_, bool>(9)?,
        ))
    })? {
        let (
            (success, failed, refunded, incomplete),
            quoted,
            intent_pending,
            raw_pending,
            attempts,
            confirmed,
            missing_original,
        ) = obligation?;
        original = true;
        if raw_pending || (intent_pending && !(quoted && success)) {
            return Ok(true);
        }
        if quoted && (success || failed || refunded) {
            continue;
        }
        ensure!(
            !missing_original,
            "destination obligation original treasury operation missing"
        );
        if attempts.unwrap_or(1) > 0 || confirmed || incomplete {
            return Ok(true);
        }
    }
    Ok(!credited && !original)
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
        let touched: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1) OR EXISTS(SELECT 1 FROM operation_network WHERE operation_id=?1 AND preparation IS NOT NULL)", [&operation], |r| r.get(0))?;
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
        // Only accepted calculation/bytes prove work started; phase is diagnostic.
        let _ = phase;
        let touched: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1) OR EXISTS(SELECT 1 FROM operation_network WHERE operation_id=?1 AND preparation IS NOT NULL)",[&operation],|r|r.get(0))?;
        ensure!(!touched, "unprepared closure has source effects");
        tx.execute("INSERT INTO funding_recovery(operation_id,job_id,phase,quote,refund,provider_success,provider_failed,provider_refunded,provider_incomplete,provider_observed_at,quote_target,quote_ceiling,accepted_output,accepted_recipient) SELECT j.operation_id,j.job_id,j.phase,j.quote,r.address,j.provider_success,j.provider_failed,j.provider_refunded,j.provider_incomplete,j.provider_observed_at,j.quote_target,j.quote_ceiling,j.accepted_output,j.accepted_recipient FROM funding_progress j LEFT JOIN funding_refunds r ON r.job_id=j.job_id WHERE j.job_id=?1",[job])?;
        tx.execute("UPDATE funding_progress SET phase='\"RECOVERY_REQUIRED\"',last_error='funding_attempt_closed; current authority required for another attempt' WHERE job_id=?1",[job])?;
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
        self.funding_restriction
            .require_new_funding("funding_attempt_start")?;
        self.check_job_permit(job, 1)?;
        ensure!(
            self.funding_retry_allowed(job, expected_operation)?,
            "swap_outcome_pending"
        );
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure!(
            super::capacity::current_funding_claim(&tx, job)?.is_some(),
            "funding claim disabled or changed"
        );
        let operation: String = tx.query_row(
            "SELECT j.operation_id FROM funding_progress j JOIN funding_jobs f ON f.id=j.job_id JOIN wallets w ON w.id=f.wallet_id JOIN pools p ON p.id=w.pool_id WHERE f.id=?1 AND j.operation_id=?2 AND f.state!='COMPLETE' AND f.readiness_requirement IS NULL AND w.role='ALLOCATED' AND p.enabled=1 AND j.phase IN ('\"RECOVERY_REQUIRED\"','\"REFUND_PENDING\"')",
            params![job,expected_operation], |r| r.get(0))?;
        ensure!(
            source_attempt_closed(&tx, &operation)?,
            "funding closure proof missing"
        );
        tx.execute("INSERT OR IGNORE INTO funding_recovery(operation_id,job_id,phase,quote,refund,provider_success,provider_failed,provider_refunded,provider_incomplete,provider_observed_at,quote_target,quote_ceiling,accepted_output,accepted_recipient) SELECT j.operation_id,j.job_id,j.phase,j.quote,r.address,j.provider_success,j.provider_failed,j.provider_refunded,j.provider_incomplete,j.provider_observed_at,j.quote_target,j.quote_ceiling,j.accepted_output,j.accepted_recipient FROM funding_progress j LEFT JOIN funding_refunds r ON r.job_id=j.job_id WHERE j.job_id=?1",[job])?;
        let closed: u64 = tx.query_row(RECOVERY_COUNT_SQL, [job], |r| sql_u64(r, 0))?;
        ensure!(
            closed <= u64::from(max_attempts),
            crate::rotation::error::RecoveryAttemptLimit
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
        let (operation, phase): (String, String) = tx.query_row("SELECT j.operation_id,j.phase FROM funding_progress j JOIN funding_jobs f ON f.id=j.job_id JOIN wallets w ON w.id=f.wallet_id WHERE j.job_id=?1 AND j.operation_id=?2 AND f.state!='COMPLETE' AND f.readiness_requirement IS NULL AND w.role='ALLOCATED' AND EXISTS(SELECT 1 FROM pools p WHERE p.id=w.pool_id AND p.enabled=1)", params![job,expected_operation], |r| Ok((r.get(0)?,r.get(1)?)))?;
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
            "SELECT EXISTS(SELECT 1 FROM operation_network WHERE operation_id=?1 AND preparation IS NOT NULL)",
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
                "SELECT w.id,w.address,p.enabled,f.readiness_requirement IS NOT NULL,EXISTS(SELECT 1 FROM outgoing o WHERE o.id=j.operation_id) OR EXISTS(SELECT 1 FROM operation_network n WHERE n.operation_id=j.operation_id AND n.preparation IS NOT NULL) FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id JOIN pools p ON p.id=w.pool_id JOIN funding_progress j ON j.job_id=f.id WHERE f.id=?1 AND f.state='QUEUED' AND w.role='ALLOCATED'", [id],
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
        self.db.execute("INSERT INTO funding_health(job_id,timed_out) SELECT ?1,1 WHERE EXISTS(SELECT 1 FROM funding_progress j JOIN funding_jobs f ON f.id=j.job_id WHERE j.job_id=?1 AND j.operation_id=?2 AND f.state!='COMPLETE') ON CONFLICT(job_id) DO UPDATE SET timed_out=1",params![id,operation])?;
        Ok(())
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
        let error = if complete {
            completed_error(error)
        } else {
            error
        };
        ensure!(tx.execute("UPDATE funding_progress SET next_poll=?2,last_error=?3,attempts=attempts+?4 WHERE job_id=?1",params![id,i64::try_from(next_poll)?,error,u32::from(quote_attempt)])? == 1, "unknown funding job");
        tx.execute("INSERT INTO funding_health(job_id,error_streak) VALUES (?1,?2) ON CONFLICT(job_id) DO UPDATE SET error_streak=CASE WHEN excluded.error_streak=0 THEN 0 ELSE MIN(error_streak+1,32) END",params![id,u32::from(error.is_some())])?;
        tx.execute("UPDATE funding_health SET timed_out=0 WHERE job_id=?1 AND EXISTS(SELECT 1 FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id JOIN treasury_operations o ON o.id=j.operation_id WHERE f.id=?1 AND f.state='COMPLETE' AND o.submission='CONFIRMED')",[id])?;
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
        let last_error = if parsed_phase == FundingPhase::Complete {
            completed_error(last_error.as_deref()).map(str::to_owned)
        } else {
            last_error
        };
        let last_error = last_error.or_else(||timed_out.then(||"swap_timeout; slower reconciliation continues; inspect source and refund status".into()));
        Ok(FundingJob {
            started_at: started_at.map(u64::try_from).transpose().context(super::scheduling::FundingDecodeFailure)?,
            error_streak: if parsed_phase == FundingPhase::Complete && last_error.is_none() { 0 } else { error_streak },
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

    #[test]
    fn preparation_phase_and_input_owner_commit_together() {
        let (dir, mut store, job) = fixture();
        store
            .save_funding_quote(
                &job.id,
                &serde_json::to_vec(
                    &serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}}),
                )
                .unwrap(),
            )
            .unwrap();
        store.db.execute_batch("CREATE TRIGGER reject_preparing BEFORE UPDATE OF phase ON funding_progress WHEN NEW.phase='\"PREPARING\"' BEGIN SELECT RAISE(ABORT,'fixture phase publication failure'); END;").unwrap();
        assert!(
            store
                .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
                .is_err()
        );
        store
            .require_unstarted_preparation(&job.operation_id)
            .unwrap();
        assert_eq!(
            store.funding_job(&job.id).unwrap().unwrap().phase,
            FundingPhase::Quoted
        );
        assert_eq!(store.funding_recovery_count(&job.id).unwrap(), 0);
        store
            .db
            .execute_batch("DROP TRIGGER reject_preparing")
            .unwrap();
        store
            .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
            .unwrap();
        assert_eq!(
            store.funding_job(&job.id).unwrap().unwrap().phase,
            FundingPhase::Preparing
        );
        let treasury = store.id().to_owned();
        drop(store);
        let store = Store::open(
            &dir.path().join("state"),
            &dir.path().join("key"),
            &treasury,
        )
        .unwrap();
        assert_eq!(
            store.funding_job(&job.id).unwrap().unwrap().phase,
            FundingPhase::Preparing
        );
        assert!(
            store
                .require_unstarted_preparation(&job.operation_id)
                .is_err()
        );
    }

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
    fn obsolete_historical_readiness_annotations_do_not_veto_initial_credit() {
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
                    .unwrap()
            );
            let state: String = store
                .db
                .query_row(
                    "SELECT state FROM funding_jobs WHERE id=?1",
                    [&job.id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(state, "COMPLETE");
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
                    .require_preparation_authority(&job.operation_id, Some(&authority))
                    .unwrap();
                store
                    .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
                    .unwrap();
            }
            store
                .record_destination_readiness(&job.id, &job.target, "credit", 1)
                .unwrap();
            if credit_first {
                assert!(
                    store
                        .require_preparation_authority(&job.operation_id, Some(&authority))
                        .is_err()
                );
                let effects:bool=store.db.query_row("SELECT EXISTS(SELECT 1 FROM outgoing) OR EXISTS(SELECT 1 FROM operation_network WHERE preparation IS NOT NULL)",[],|r|r.get(0)).unwrap();
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
                            deadline: Some(u64::MAX),
                        }),
                    )
                    .unwrap();
                assert_eq!(
                    store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
                    b"exact signed bytes"
                );
                assert!(
                    store
                        .request_broadcast(&job.operation_id, 1, 1, false)
                        .is_err()
                );
                assert_eq!(store.operation(&job.operation_id).unwrap().attempts, 0);
                assert_eq!(
                    store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
                    b"exact signed bytes"
                );
            }
        }
    }

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
                "SELECT EXISTS(SELECT 1 FROM outgoing) OR EXISTS(SELECT 1 FROM operation_network WHERE preparation IS NOT NULL)",
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

    #[test]
    fn initial_credit_is_independent_of_source_accounting_and_historical_support() {
        let (_dir, mut store, job) = fixture();
        let quote = serde_json::to_vec(
            &serde_json::json!({"request":{"amount":"10","recipient":job.recipient}}),
        )
        .unwrap();
        store.save_funding_quote(&job.id, &quote).unwrap();
        assert!(destination_obligation_pending(&store.db, &job.wallet_id).unwrap());
        assert!(record_initial_credit(&store.db, &job.wallet_id, U256::from(9)).unwrap());
        store.db.execute("UPDATE funding_jobs SET readiness_support='malformed',readiness_disputed='old' WHERE id=?1",[&job.id]).unwrap();
        assert!(!destination_obligation_pending(&store.db, &job.wallet_id).unwrap());
        assert!(record_initial_credit(&store.db, &job.wallet_id, U256::ZERO).unwrap());
        assert_eq!(store.destination_requirement(&job.id).unwrap().amount, "9");
        assert!(
            store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        assert_eq!(
            store.funding_job(&job.id).unwrap().unwrap().operation_id,
            job.operation_id
        );
        assert!(store.status().unwrap().treasury_operations.is_empty());
        assert!(
            !store
                .funding_facts(&job.id, &job.operation_id)
                .unwrap()
                .provider_success
        );
        assert_eq!(
            store
                .db
                .query_row::<String, _, _>(
                    "SELECT completion_outcome FROM funding_jobs WHERE id=?1",
                    [&job.id],
                    |r| r.get(0)
                )
                .unwrap(),
            "EXTERNAL"
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
            store
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
        assert_eq!(requirement.amount, "10");
        assert_eq!(
            requirement.operation_id.as_deref(),
            Some(job.operation_id.as_str())
        );
        assert!(
            store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        assert!(store.status().unwrap().treasury_operations.is_empty());
        assert!(
            !store
                .funding_facts(&job.id, &job.operation_id)
                .unwrap()
                .provider_success
        );
        assert_eq!(
            store
                .db
                .query_row::<String, _, _>(
                    "SELECT completion_outcome FROM funding_jobs WHERE id=?1",
                    [&job.id],
                    |r| r.get(0)
                )
                .unwrap(),
            "EXTERNAL"
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
    fn source_settlement_closes_a_clean_partial_credit_once() {
        let (_dir, mut store, job) = fixture();
        let quote = serde_json::to_vec(
            &serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}}),
        )
        .unwrap();
        store.save_funding_quote(&job.id, &quote).unwrap();
        store
            .record_destination_readiness(&job.id, "1", "partial-credit", 1)
            .unwrap();
        for _ in 0..2 {
            assert!(
                store
                    .settle_funding_source(&job.id, &job.operation_id, false)
                    .unwrap()
                    .is_none()
            );
        }
        let current = store.funding_job(&job.id).unwrap().unwrap();
        assert_eq!(current.operation_id, job.operation_id);
        assert_eq!(current.phase, FundingPhase::Complete);
        assert_eq!(store.funding_recovery_count(&job.id).unwrap(), 1);
        assert!(!store.operation_pending(&job.operation_id).unwrap());
        assert!(store.operation(&job.operation_id).is_err());
        assert_eq!(store.destination_requirement(&job.id).unwrap().amount, "1");
    }

    #[test]
    fn source_settlement_preserves_accepted_and_prepared_work_after_credit() {
        use crate::rotation::transaction::{ManagedFundingAuthority, TransactionFacts};
        for with_bytes in [false, true] {
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
            store
                .require_preparation_authority(&job.operation_id, Some(&authority))
                .unwrap();
            store
                .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
                .unwrap();
            if with_bytes {
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
                            deadline: Some(u64::MAX),
                        }),
                    )
                    .unwrap();
            }
            store
                .record_destination_readiness(&job.id, "1", "partial-credit", 1)
                .unwrap();
            let (current, facts) = store
                .settle_funding_source(&job.id, &job.operation_id, false)
                .unwrap()
                .expect("destination credit cannot close accepted source work");
            assert_eq!(current.operation_id, job.operation_id);
            assert!(facts.initial_credit_recorded);
            assert!(!facts.new_funding_allowed);
            assert_eq!(facts.preparation_accepted, !with_bytes);
            assert_eq!(facts.source_pending, with_bytes);
            assert_eq!(store.funding_recovery_count(&job.id).unwrap(), 0);
            if with_bytes {
                assert_eq!(
                    store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
                    b"exact signed bytes"
                );
                assert_eq!(store.operation(&job.operation_id).unwrap().attempts, 0);
            }
        }
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
        for publication_due in [false, true] {
            assert!(
                store
                    .settle_funding_source(&job.id, &job.operation_id, publication_due)
                    .is_err()
            );
        }
        assert_eq!(
            store
                .funding_quote_for_operation(&current.id, &current.operation_id)
                .unwrap()
                .as_slice(),
            quote.as_slice()
        );
        let unchanged = store.funding_job(&job.id).unwrap().unwrap();
        assert_eq!(unchanged.operation_id, current.operation_id);
        assert_eq!(unchanged.phase, current.phase);
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

    #[test]
    fn bound_success_keeps_initial_credit_without_source_confirmation_or_replay() {
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
            .accept_preparation(&job.operation_id, Some(&job.pool_id), 100, None)
            .unwrap();
        let facts = TransactionFacts {
            txid: "source".into(),
            expiry_height: 100,
            amount_zatoshis: 90,
            fee_zatoshis: 10,
            deadline: Some(u64::MAX),
        };
        store
            .prepare_with_facts(&job.operation_id, 1, b"snapshot", b"source", Some(facts))
            .unwrap();
        store
            .record_destination_readiness(&job.id, "10", "block", 1)
            .unwrap();
        store
            .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Success, 1)
            .unwrap();
        assert!(
            store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        assert_eq!(
            store.operation(&job.operation_id).unwrap().submission,
            "PREPARED"
        );
        assert!(!store.operation_pending(&job.operation_id).unwrap());
        assert_eq!(store.operation(&job.operation_id).unwrap().attempts, 0);
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
            .require_preparation_authority(
                &job.operation_id,
                Some(&ManagedFundingAuthority {
                    job_id: job.id.clone(),
                    auto_fund: true,
                    max_attempts: 3,
                }),
            )
            .unwrap();
        store
            .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
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
                    deadline: Some(u64::MAX),
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

    #[test]
    fn incomplete_provider_deposit_without_a_local_attempt_keeps_observing_without_topup() {
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
            .record_provider_outcome(
                &job.id,
                &job.operation_id,
                &SwapStatus::IncompleteDeposit,
                1,
            )
            .unwrap();
        let facts = store.funding_facts(&job.id, &job.operation_id).unwrap();
        assert!(facts.status_needed());
        assert!(!facts.new_funding_allowed);
        assert!(
            !store
                .funding_retry_allowed(&job.id, &job.operation_id)
                .unwrap()
        );
        assert!(
            store
                .require_unstarted_preparation(&job.operation_id)
                .is_ok()
        );
        store
            .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Success, 2)
            .unwrap();
        assert!(
            store
                .funding_facts(&job.id, &job.operation_id)
                .unwrap()
                .credit_needed()
        );
        assert!(
            !store
                .funding_facts(&job.id, &job.operation_id)
                .unwrap()
                .status_needed()
        );
        assert!(
            store
                .funding_job(&job.id)
                .unwrap()
                .unwrap()
                .last_error
                .is_none()
        );
    }

    #[test]
    fn incomplete_deposit_keeps_observing_until_a_terminal_provider_fact() {
        use crate::rotation::near::SwapStatus;
        for terminal in [
            SwapStatus::Success,
            SwapStatus::Failed,
            SwapStatus::Refunded,
        ] {
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
            for (n, status) in [SwapStatus::Processing, SwapStatus::IncompleteDeposit]
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
                if matches!(terminal, SwapStatus::Success) {
                    FundingPhase::VerifyingCredit
                } else {
                    FundingPhase::RefundPending
                }
            );
            assert_eq!(store.operation(&job.operation_id).unwrap().attempts, 1);
        }
    }

    #[test]
    fn positive_completion_recovers_prior_provider_reports_after_restart() {
        use crate::rotation::near::SwapStatus;
        for negative in [
            SwapStatus::IncompleteDeposit,
            SwapStatus::Failed,
            SwapStatus::Refunded,
        ] {
            for credit_first in [true, false] {
                let (dir, mut store, job) = fixture();
                let identity = store.id().to_owned();
                let quote = serde_json::to_vec(
                    &serde_json::json!({"request":{"amount":"10","recipient":job.recipient}}),
                )
                .unwrap();
                store.save_funding_quote(&job.id, &quote).unwrap();
                submit_source_for_status(&mut store, &job);
                let bytes = store.prepared_bytes(&job.operation_id).unwrap();
                store
                    .record_provider_outcome(&job.id, &job.operation_id, &negative, 1)
                    .unwrap();
                if credit_first {
                    store
                        .record_destination_readiness(&job.id, "10", "block", 1)
                        .unwrap();
                }
                assert!(
                    !store
                        .finish_funding_if_proven(&job.id, &job.operation_id)
                        .unwrap(),
                    "credit and negative reporting cannot substitute for fulfillment"
                );
                store
                    .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Success, 2)
                    .unwrap();
                assert!(!store.operation_pending(&job.operation_id).unwrap());
                assert_eq!(
                    store.operation(&job.operation_id).unwrap().submission,
                    "BROADCAST_REQUESTED"
                );
                assert_eq!(
                    store
                        .finish_funding_if_proven(&job.id, &job.operation_id)
                        .unwrap(),
                    credit_first
                );
                if !credit_first {
                    assert!(
                        !store
                            .finish_funding_if_proven(&job.id, &job.operation_id)
                            .unwrap(),
                        "source confirmation and fulfillment cannot substitute for destination credit"
                    );
                }
                assert_eq!(
                    store.funding_job(&job.id).unwrap().unwrap().phase,
                    if credit_first {
                        FundingPhase::Complete
                    } else {
                        FundingPhase::VerifyingCredit
                    }
                );
                drop(store);
                store = Store::open(
                    &dir.path().join("state"),
                    &dir.path().join("key"),
                    &identity,
                )
                .unwrap();
                if !credit_first {
                    assert!(
                        store
                            .funding_facts(&job.id, &job.operation_id)
                            .unwrap()
                            .credit_needed()
                    );
                    store
                        .record_destination_readiness(&job.id, "10", "block", 1)
                        .unwrap();
                }
                assert!(
                    store
                        .settle_funding_source(&job.id, &job.operation_id, false)
                        .unwrap()
                        .is_none()
                );
                for status in [&negative, &SwapStatus::Processing] {
                    store
                        .record_provider_outcome(&job.id, &job.operation_id, status, 3)
                        .unwrap();
                    assert_eq!(
                        store.funding_job(&job.id).unwrap().unwrap().phase,
                        FundingPhase::Complete,
                        "a delayed report cannot revoke the same positive completion proof"
                    );
                }
                let facts = store.funding_facts(&job.id, &job.operation_id).unwrap();
                assert!(facts.provider_success);
                assert!(match negative {
                    SwapStatus::IncompleteDeposit => facts.provider_incomplete,
                    SwapStatus::Failed => facts.provider_failed,
                    SwapStatus::Refunded => facts.provider_refunded,
                    _ => unreachable!(),
                });
                assert_eq!(
                    store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
                    bytes.as_slice()
                );
                assert_eq!(store.operation(&job.operation_id).unwrap().attempts, 1);
                assert_eq!(
                    store.funding_quote(&job.id).unwrap().as_slice(),
                    quote.as_slice()
                );
                assert_eq!(store.funding_jobs().unwrap().len(), 2);
                assert_eq!(store.funding_recovery_count(&job.id).unwrap(), 0);
                assert!(
                    store
                        .record_provider_outcome(
                            &job.id,
                            "another-operation",
                            &SwapStatus::Success,
                            4
                        )
                        .is_err()
                );
            }
        }
    }

    #[test]
    fn fulfilled_intent_survives_restart_without_blocking_unrelated_work_or_late_journal() {
        use crate::rotation::near::SwapStatus;
        use crate::rotation::store::preparation::{CalculationInputs, InputPool, OwnedInput};
        let (dir, mut store, job) = fixture();
        let identity = store.id().to_owned();
        store
            .save_funding_quote(
                &job.id,
                &serde_json::to_vec(&serde_json::json!({
                    "request":{"amount":job.target,"recipient":job.recipient}
                }))
                .unwrap(),
            )
            .unwrap();
        let calculation = CalculationInputs {
            destination: "original deposit".into(),
            inputs: vec![OwnedInput {
                pool: InputPool::Orchard,
                txid: format!("{:064x}", 1),
                output_index: 2,
            }],
        };
        store
            .accept_preparation(
                &job.operation_id,
                Some(&job.pool_id),
                60,
                Some(calculation.clone()),
            )
            .unwrap();
        let accepted: Vec<u8> = store
            .db
            .query_row(
                "SELECT preparation FROM operation_network WHERE operation_id=?1",
                [&job.operation_id],
                |r| r.get(0),
            )
            .unwrap();
        store
            .record_destination_readiness(&job.id, &job.target, "block", 1)
            .unwrap();
        for status in [
            SwapStatus::Processing,
            SwapStatus::Failed,
            SwapStatus::Refunded,
        ] {
            store
                .record_provider_outcome(&job.id, &job.operation_id, &status, 2)
                .unwrap();
            assert!(store.require_preparation_available().is_err());
            assert!(destination_obligation_pending(&store.db, &job.wallet_id).unwrap());
        }
        store
            .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Success, 3)
            .unwrap();
        assert!(
            store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        assert!(!destination_obligation_pending(&store.db, &job.wallet_id).unwrap());
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        drop(store);
        store = Store::open(&state, &key, &identity).unwrap();
        assert!(!super::super::status(&state).unwrap().outgoing_pending);
        store.require_preparation_available().unwrap();
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT preparation FROM operation_network WHERE operation_id=?1",
                    [&job.operation_id],
                    |r| r.get::<_, Vec<u8>>(0)
                )
                .unwrap(),
            accepted
        );
        assert!(
            store
                .require_unstarted_preparation(&job.operation_id)
                .is_err()
        );
        assert!(
            store
                .request_broadcast(&job.operation_id, 4, 1, false)
                .is_err()
        );
        // Retrying acceptance is recognition of the same immutable owner, not
        // permission to repeat SDK calculation or disclose an original result.
        store
            .accept_preparation(
                &job.operation_id,
                Some(&job.pool_id),
                60,
                Some(calculation.clone()),
            )
            .unwrap();
        assert!(
            store
                .accept_preparation(
                    &job.operation_id,
                    Some(&job.pool_id),
                    61,
                    Some(calculation.clone())
                )
                .is_err()
        );
        let expected = store.snapshot().unwrap().0;
        store
            .accept_preparation("unrelated", None, 1, None)
            .unwrap();
        let unrelated: Vec<u8> = store
            .db
            .query_row(
                "SELECT preparation FROM operation_network WHERE operation_id='unrelated'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // An unrelated accepted intent alone cannot change the SDK revision or
        // consume this late result's preserved input/identity binding.
        assert_eq!(store.snapshot().unwrap().0, expected);
        let facts = TransactionFacts {
            txid: "original".into(),
            expiry_height: 100,
            amount_zatoshis: 50,
            fee_zatoshis: 10,
            deadline: Some(u64::MAX),
        };
        let revision = store
            .prepare_with_facts(
                &job.operation_id,
                expected,
                b"late accepted SDK",
                b"exact late signed bytes",
                Some(facts.clone()),
            )
            .unwrap();
        assert_eq!(
            store
                .prepare_with_facts(
                    &job.operation_id,
                    expected,
                    b"late accepted SDK",
                    b"exact late signed bytes",
                    Some(facts)
                )
                .unwrap(),
            revision
        );
        assert_eq!(
            store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
            b"exact late signed bytes"
        );
        assert_eq!(
            store.prepared_inputs(&job.operation_id).unwrap(),
            calculation.inputs
        );
        assert!(!store.operation_pending(&job.operation_id).unwrap());
        assert!(
            store
                .request_broadcast(&job.operation_id, 4, 1, false)
                .is_err()
        );
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT preparation FROM outgoing WHERE id=?1",
                    [&job.operation_id],
                    |r| r.get::<_, Vec<u8>>(0)
                )
                .unwrap(),
            accepted
        );
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT preparation FROM operation_network WHERE operation_id='unrelated'",
                    [],
                    |r| r.get::<_, Vec<u8>>(0)
                )
                .unwrap(),
            unrelated
        );
        assert!(store.require_preparation_available().is_err());
        assert!(store.status().unwrap().outgoing_pending);
        assert!(!destination_obligation_pending(&store.db, &job.wallet_id).unwrap());
    }

    #[test]
    fn fulfilled_operation_closes_before_acceptance_or_disclosure_and_retains_late_calculation() {
        use crate::rotation::near::SwapStatus;
        for stage in [
            "before_acceptance",
            "during_calculation",
            "before_disclosure",
            "after_disclosure",
        ] {
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
            if stage != "before_acceptance" {
                store
                    .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
                    .unwrap();
            }
            let facts = TransactionFacts {
                txid: "original".into(),
                expiry_height: 100,
                amount_zatoshis: 50,
                fee_zatoshis: 10,
                deadline: Some(u64::MAX),
            };
            if matches!(stage, "before_disclosure" | "after_disclosure") {
                store
                    .prepare_with_facts(
                        &job.operation_id,
                        store.snapshot().unwrap().0,
                        b"accepted SDK",
                        b"exact original",
                        Some(facts.clone()),
                    )
                    .unwrap();
            }
            if stage == "after_disclosure" {
                store
                    .request_broadcast(&job.operation_id, 1, 1, false)
                    .unwrap();
            }
            store
                .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Success, 2)
                .unwrap();
            let ready = store.funding_facts(&job.id, &job.operation_id).unwrap();
            assert!(ready.provider_success && ready.credit_needed());
            assert!(!ready.new_funding_allowed && !ready.status_needed());
            assert_eq!(
                store.funding_job(&job.id).unwrap().unwrap().phase,
                FundingPhase::VerifyingCredit
            );
            if stage == "before_acceptance" {
                assert!(
                    store
                        .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
                        .is_err()
                );
                assert!(
                    store
                        .require_unstarted_preparation(&job.operation_id)
                        .is_ok()
                );
                assert_eq!(store.snapshot().unwrap().0, 1);
            } else {
                if stage == "during_calculation" {
                    assert!(
                        store.require_preparation_available().is_ok(),
                        "fulfilled intent remains audit evidence while unrelated work can proceed"
                    );
                    store
                        .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
                        .unwrap();
                    store
                        .prepare_with_facts(
                            &job.operation_id,
                            store.snapshot().unwrap().0,
                            b"accepted SDK",
                            b"exact original",
                            Some(facts),
                        )
                        .unwrap();
                }
                assert_eq!(
                    store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
                    b"exact original"
                );
                assert!(!store.operation_pending(&job.operation_id).unwrap());
                assert!(
                    store
                        .request_broadcast(&job.operation_id, 3, 1, false)
                        .is_err()
                );
                assert!(
                    store
                        .request_broadcast(&job.operation_id, 3, 1, true)
                        .is_err()
                );
                if stage == "after_disclosure" {
                    store.broadcast_result(&job.operation_id, 1, true).unwrap();
                    assert_eq!(
                        store.operation(&job.operation_id).unwrap().submission,
                        "BROADCAST"
                    );
                } else {
                    assert_eq!(
                        store.operation(&job.operation_id).unwrap().submission,
                        "PREPARED"
                    );
                }
                assert_eq!(
                    store.operation(&job.operation_id).unwrap().attempts,
                    i64::from(stage == "after_disclosure")
                );
            }
            assert!(
                !store
                    .finish_funding_if_proven(&job.id, &job.operation_id)
                    .unwrap(),
                "provider success cannot invent Base credit"
            );
            store
                .record_destination_readiness(&job.id, "10", "block", 1)
                .unwrap();
            assert!(
                store
                    .finish_funding_if_proven(&job.id, &job.operation_id)
                    .unwrap()
            );
            assert_eq!(store.funding_recovery_count(&job.id).unwrap(), 0);
            assert_eq!(store.funding_jobs().unwrap().len(), 2);
        }
    }

    #[test]
    fn existing_provider_success_closes_legacy_pending_work_without_decoding_source() {
        let (dir, mut store, job) = fixture();
        let identity = store.id().to_owned();
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
        // Prior executable could retain source work after persisting provider success.
        store
            .db
            .execute(
                "UPDATE funding_progress SET provider_success=1 WHERE job_id=?1",
                [&job.id],
            )
            .unwrap();
        store
            .db
            .execute(
                "UPDATE treasury_operations SET facts='malformed original source',attempts='invalid historical counter' WHERE id=?1",
                [&job.operation_id],
            )
            .unwrap();
        let original = store.prepared_bytes(&job.operation_id).unwrap();
        drop(store);
        store = Store::open(
            &dir.path().join("state"),
            &dir.path().join("key"),
            &identity,
        )
        .unwrap();
        assert!(store.operation_pending(&job.operation_id).unwrap());
        assert!(
            store
                .funding_operation_fulfilled(&job.operation_id)
                .unwrap()
        );
        assert!(
            store
                .funding_operation_fulfilled(&job.operation_id)
                .unwrap()
        );
        assert!(!store.operation_pending(&job.operation_id).unwrap());
        assert_eq!(
            store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
            original.as_slice()
        );
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT submission FROM treasury_operations WHERE id=?1",
                    [&job.operation_id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "BROADCAST_REQUESTED"
        );
        assert!(
            !store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        assert!(
            store
                .funding_facts(&job.id, &job.operation_id)
                .unwrap()
                .credit_needed()
        );
        let work = store
            .funding_work_page("", store.funding_work_boundary().unwrap())
            .unwrap()
            .into_iter()
            .find(|(id, _)| id == &job.id)
            .unwrap()
            .1
            .unwrap();
        assert!(work.facts.provider_success && work.facts.credit_needed());
        assert!(!work.facts.new_funding_allowed && !work.facts.source_reconciliation_needed());
        store
            .record_destination_readiness(&job.id, "1", "late-credit", 1)
            .unwrap();
        assert!(
            store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT attempts FROM treasury_operations WHERE id=?1",
                    [&job.operation_id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "invalid historical counter"
        );
        assert!(!destination_obligation_pending(&store.db, &job.wallet_id).unwrap());
    }

    #[test]
    fn source_expiry_keeps_current_provider_status_and_original_quote_binding() {
        use crate::rotation::near::SwapStatus;
        for terminal in [
            SwapStatus::Success,
            SwapStatus::Failed,
            SwapStatus::Refunded,
        ] {
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
            let revision = store.snapshot().unwrap().0;
            let observation = SyncObservation {
                phase: SyncPhase::Ready,
                last_error: None,
                snapshot_revision: revision,
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
            store.resolve_expired(&job.operation_id, revision).unwrap();
            store
                .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Processing, 2)
                .unwrap();
            let facts = store.funding_facts(&job.id, &job.operation_id).unwrap();
            assert!(facts.archived && facts.status_needed());
            assert!(
                !store
                    .funding_retry_allowed(&job.id, &job.operation_id)
                    .unwrap()
            );
            assert!(
                store
                    .reauthorize_closed_funding_for_operation(&job.id, &job.operation_id, 3)
                    .is_err()
            );
            // A historical completion label cannot dispose of unresolved delivery.
            store
                .db
                .execute(
                    "UPDATE funding_jobs SET state='COMPLETE' WHERE id=?1",
                    [&job.id],
                )
                .unwrap();
            assert!(destination_obligation_pending(&store.db, &job.wallet_id).unwrap());
            store
                .record_provider_outcome(&job.id, &job.operation_id, &terminal, 3)
                .unwrap();
            assert!(
                !store
                    .funding_facts(&job.id, &job.operation_id)
                    .unwrap()
                    .status_needed()
            );
            assert!(
                !destination_obligation_pending(&store.db, &job.wallet_id).unwrap(),
                "current terminal report supersedes archived unknown flags for same operation"
            );
            store
                .record_destination_readiness(&job.id, "10", "block", 1)
                .unwrap();
            assert!(
                store
                    .finish_funding_if_proven(&job.id, &job.operation_id)
                    .unwrap()
            );
            assert_eq!(store.destination_requirement(&job.id).unwrap().amount, "10");
            assert_eq!(store.operation(&job.operation_id).unwrap().attempts, 1);
        }
    }

    #[test]
    fn never_disclosed_expired_original_can_complete_from_external_credit_without_status_polling() {
        let (_dir, mut store, job) = fixture();
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
            .accept_preparation(&job.operation_id, Some(&job.pool_id), 60, None)
            .unwrap();
        store
            .prepare_with_facts(
                &job.operation_id,
                1,
                b"prepared SDK",
                b"never disclosed original",
                Some(TransactionFacts {
                    txid: "original".into(),
                    expiry_height: 100,
                    amount_zatoshis: 50,
                    fee_zatoshis: 10,
                    deadline: None,
                }),
            )
            .unwrap();
        let revision = store.snapshot().unwrap().0;
        let revision = store
            .save_sync_snapshot(
                revision,
                b"canonical expired SDK",
                Some(SyncObservation {
                    phase: SyncPhase::Ready,
                    last_error: None,
                    snapshot_revision: revision,
                    checked_at: Some(1),
                    checkpoint_at: 1,
                    scanned_blocks: 102,
                    target_height: Some(102),
                    observed_tip_height: Some(102),
                    height: Some(102),
                    confirmations: 2,
                    confirmed_pool_balances_zatoshis: None,
                    confirmed_shielded_zatoshis: 1000,
                    spendable_shielded_zatoshis: 1000,
                }),
            )
            .unwrap();
        store.resolve_expired(&job.operation_id, revision).unwrap();
        let observed = store.observation_operation(&job.operation_id).unwrap();
        assert_eq!(
            (observed.submission.as_str(), observed.attempts),
            ("EXPIRED", 0)
        );
        let facts = store.funding_facts(&job.id, &job.operation_id).unwrap();
        assert_eq!(facts.source_attempts, 0);
        assert!(
            !facts.status_needed(),
            "native PREPARED0 never disclosed a deposit"
        );
        store
            .record_destination_readiness(&job.id, "1", "external credit", 1)
            .unwrap();
        assert!(
            store
                .finish_funding_if_proven(&job.id, &job.operation_id)
                .unwrap()
        );
        assert!(!destination_obligation_pending(&store.db, &job.wallet_id).unwrap());
        let native: (String, i64) = store
            .db
            .query_row(
                "SELECT submission,attempts FROM treasury_operations WHERE id=?1",
                [&job.operation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            native,
            ("PREPARED".into(), 0),
            "closure keeps actual disclosure history unchanged"
        );
        assert_eq!(
            store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
            b"never disclosed original"
        );
    }

    #[test]
    fn pending_bridge_survives_partial_credit_source_closure_and_zero_attempt_label() {
        use crate::rotation::near::SwapStatus;
        for expired in [false, true] {
            let (_dir, mut store, job) = fixture();
            let quote = serde_json::to_vec(
                &serde_json::json!({"request":{"amount":"10","recipient":job.recipient}}),
            )
            .unwrap();
            store.save_funding_quote(&job.id, &quote).unwrap();
            submit_source_for_status(&mut store, &job);
            if expired {
                let revision = store.snapshot().unwrap().0;
                let observation = SyncObservation {
                    phase: SyncPhase::Ready,
                    last_error: None,
                    snapshot_revision: revision,
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
                store.resolve_expired(&job.operation_id, revision).unwrap();
                // A stale zero counter/UNKNOWN label cannot make disclosed
                // expired work look never sent while delivery remains pending.
                store.db.execute("UPDATE treasury_operations SET attempts=0,submission='UNKNOWN' WHERE id=?1",[&job.operation_id]).unwrap();
            } else {
                store.confirm_spend(&job.operation_id).unwrap();
                // Canonical inclusion is stronger than an old local disclosure counter.
                store
                    .db
                    .execute(
                        "UPDATE treasury_operations SET attempts=0 WHERE id=?1",
                        [&job.operation_id],
                    )
                    .unwrap();
            }
            store
                .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Processing, 1)
                .unwrap();
            store
                .record_destination_readiness(&job.id, "1", "partial-credit", 1)
                .unwrap();
            record_initial_credit(&store.db, &job.wallet_id, U256::ZERO).unwrap();
            assert!(
                store
                    .funding_facts(&job.id, &job.operation_id)
                    .unwrap()
                    .status_needed()
            );
            assert!(
                !store
                    .finish_funding_if_proven(&job.id, &job.operation_id)
                    .unwrap()
            );
            assert!(destination_obligation_pending(&store.db, &job.wallet_id).unwrap());
            store
                .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Failed, 2)
                .unwrap();
            assert!(!destination_obligation_pending(&store.db, &job.wallet_id).unwrap());
            assert!(
                store
                    .finish_funding_if_proven(&job.id, &job.operation_id)
                    .unwrap()
            );
        }
    }

    #[test]
    fn destination_guard_does_not_invent_missing_original_provenance() {
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
        store.confirm_spend(&job.operation_id).unwrap();
        record_initial_credit(&store.db, &job.wallet_id, U256::ONE).unwrap();
        store
            .db
            .execute(
                "DELETE FROM treasury_operations WHERE id=?1",
                [&job.operation_id],
            )
            .unwrap();
        assert!(
            destination_obligation_pending(&store.db, &job.wallet_id)
                .unwrap_err()
                .to_string()
                .contains("original treasury operation missing")
        );
        assert!(store.prepared_bytes(&job.operation_id).is_ok());
    }

    #[test]
    fn provider_status_repairs_optional_flags_without_vetoing_known_success() {
        use crate::rotation::near::SwapStatus;
        for known_success in [false, true] {
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
            store.db.execute("UPDATE funding_progress SET provider_success=?2,provider_failed='broken',provider_refunded=x'00',provider_incomplete='broken' WHERE job_id=?1", params![job.id, if known_success { rusqlite::types::Value::Integer(1) } else { rusqlite::types::Value::Text("broken".into()) }]).unwrap();
            assert_eq!(
                store
                    .funding_facts(&job.id, &job.operation_id)
                    .unwrap()
                    .provider_success,
                known_success
            );
            if known_success {
                store
                    .record_destination_readiness(&job.id, "1", "credit", 1)
                    .unwrap();
                assert!(
                    store
                        .finish_funding_if_proven(&job.id, &job.operation_id)
                        .unwrap()
                );
            }
            store
                .record_provider_outcome(&job.id, &job.operation_id, &SwapStatus::Success, 1)
                .unwrap();
            let normalized: (bool,bool,bool,bool) = store.db.query_row("SELECT provider_success,provider_failed,provider_refunded,provider_incomplete FROM funding_progress WHERE job_id=?1", [&job.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
            assert_eq!(normalized, (true, false, false, false));
            assert!(!store.operation_pending(&job.operation_id).unwrap());
            assert_eq!(
                store.operation(&job.operation_id).unwrap().submission,
                "BROADCAST_REQUESTED"
            );
        }
    }

    #[test]
    fn negative_terminal_confirmed_source_can_reauthorize_under_current_claim() {
        use crate::rotation::near::SwapStatus;
        for terminal in [SwapStatus::Failed, SwapStatus::Refunded] {
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
            let raw = store.prepared_bytes(&job.operation_id).unwrap();
            store.confirm_spend(&job.operation_id).unwrap();
            store
                .record_provider_outcome(&job.id, &job.operation_id, &terminal, 1)
                .unwrap();
            assert!(
                store
                    .funding_retry_allowed(&job.id, &job.operation_id)
                    .unwrap()
            );
            store
                .reauthorize_closed_funding_for_operation(&job.id, &job.operation_id, 3)
                .unwrap();
            let replacement = store.funding_job(&job.id).unwrap().unwrap();
            assert_ne!(replacement.operation_id, job.operation_id);
            assert_eq!(replacement.phase, FundingPhase::Allocated);
            assert_eq!(store.prepared_bytes(&job.operation_id).unwrap(), raw);
            assert_eq!(
                store.operation(&job.operation_id).unwrap().submission,
                "CONFIRMED"
            );
            assert_eq!(store.funding_recovery_count(&job.id).unwrap(), 1);
            assert!(!store.preparation_pending().unwrap());
        }
    }

    #[test]
    fn later_negative_reports_preserve_facts_without_vetoing_success() {
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
            FundingPhase::VerifyingCredit
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
        store.accept_preparation(&unknown.operation_id, Some(&unknown.pool_id), 10, None)?;
        store.advance_funding(
            &unknown.id,
            FundingPhase::Preparing,
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
            assert!(!can_recover_facts(None, &facts));
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
            assert!(facts.source_closed && can_recover_facts(None, &facts));
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
        let bytes = seal(
            &store.key,
            &format!(
                "v1:{}:{}:preparation:{}",
                store.id,
                store.network.name(),
                job.operation_id
            ),
            &serde_json::to_vec(&super::preparation::PreparationIntent {
                pool: Some(job.pool_id.clone()),
                cost_zatoshis: 10,
                calculation: None,
            })?,
        )?;
        store.db.execute(
            "UPDATE operation_network SET operation_id=?1,preparation=?2 WHERE operation_id=?3",
            params![job.operation_id, bytes, unknown.operation_id],
        )?;
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
            "DELETE FROM operation_network WHERE operation_id=?1",
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
