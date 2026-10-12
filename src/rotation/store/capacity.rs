//! Capacity roles and exact reserve ownership. All mutation helpers run inside
//! the serialized store owner's transaction; none performs network I/O.
use super::*;
use crate::rotation::config::ReservesConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AllocationKind {
    Small,
    Large,
}
impl AllocationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Large => "large",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaymentOwnership {
    pub role: String,
    pub slot: Option<String>,
    pub claim_generation: Option<i64>,
    pub payment_limit: U256,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FundingClaim {
    pub target: U256,
    pub maximum_output: U256,
}

/// Current authority for accepting a quote or entering preparation. Accepted
/// prepared work uses its original capability instead of this new-work predicate.
pub(super) fn current_funding_claim(db: &Connection, job: &str) -> Result<Option<FundingClaim>> {
    // Existing initial jobs retain recovery authority under the small settings;
    // new allocations always own an exact reserve claim.
    let row: Option<(String, String)> = db.query_row(
        "SELECT c.target,c.maximum_output FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id AND w.pool_id=f.pool_id JOIN pools p ON p.id=w.pool_id JOIN reserve_claims c ON c.pool_id=p.id AND c.slot=CASE f.allocation_kind WHEN 'initial' THEN 'small' ELSE f.allocation_kind END WHERE f.id=?1 AND p.enabled=1 AND p.payment_limit IS NOT NULL AND c.enabled=1 AND f.readiness_requirement IS NULL AND (f.allocation_kind='initial' OR (c.wallet_id=f.wallet_id AND c.generation=f.claim_generation))",
        [job], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    row.map(|(target, maximum)| {
        Ok(FundingClaim {
            target: amount(&target)?,
            maximum_output: amount(&maximum)?,
        })
    })
    .transpose()
}

/// Shared eligibility predicate for selection and the final signing commit.
/// Stored configuration is authoritative, including an explicitly unlimited cap.
pub(super) fn payment_ownership(
    db: &Connection,
    pool: &str,
    wallet: &str,
) -> Result<Option<PaymentOwnership>> {
    type OwnershipRow = (bool, Option<String>, String, Option<String>, Option<i64>);
    let row: Option<OwnershipRow> = db.query_row(
        "SELECT p.enabled,p.payment_limit,w.role,c.slot,c.generation FROM wallets w JOIN pools p ON p.id=w.pool_id LEFT JOIN funding_jobs f ON f.wallet_id=w.id LEFT JOIN reserve_claims c ON c.pool_id=w.pool_id AND c.wallet_id=w.id AND c.enabled=1 AND c.slot=f.allocation_kind AND c.generation=f.claim_generation WHERE w.id=?1 AND p.id=?2",
        params![wallet,pool], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    let Some((enabled, limit, role, slot, generation)) = row else {
        return Ok(None);
    };
    let Some(limit) = limit else { return Ok(None) };
    if !enabled
        || !matches!(role.as_str(), "ACTIVE" | "RETAINED" | "READY")
        || (role == "READY" && slot.is_none())
    {
        return Ok(None);
    }
    // Historical D establishes the immutable funded role binding, but its
    // discrepancy cannot permanently veto fresh payment evidence (plan failure
    // scopes). This structural check deliberately does not require undisputed D.
    // Admission separately checks current evidence repair, exposure and capacity.
    if !funding::readiness_recorded_valid(db, wallet)? {
        return Ok(None);
    }
    Ok(Some(PaymentOwnership {
        role,
        slot,
        claim_generation: generation,
        payment_limit: amount(&limit)?,
    }))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ConsumedReserve {
    pub slot: String,
    pub generation: i64,
    pub next_generation: i64,
}

/// Invoke only inside the authorization journal transaction, after its admission
/// predicates. An unsigned selection must never call this helper.
pub(super) fn consume_reserve(
    db: &Connection,
    pool: &str,
    wallet: &str,
) -> Result<Option<ConsumedReserve>> {
    let ownership = payment_ownership(db, pool, wallet)?.context("payment_wallet_ineligible")?;
    if ownership.role != "READY" {
        return Ok(None);
    }
    let slot = ownership.slot.context("reserve_claim_missing")?;
    let generation = ownership
        .claim_generation
        .context("reserve_claim_missing")?;
    let next_generation = generation
        .checked_add(1)
        .context("reserve_generation_overflow")?;
    ensure!(db.execute("UPDATE reserve_claims SET wallet_id=NULL,generation=?5 WHERE pool_id=?1 AND slot=?2 AND wallet_id=?3 AND generation=?4 AND enabled=1",params![pool,slot,wallet,generation,next_generation])? == 1,"reserve_claim_conflict");
    db.execute(
        "UPDATE wallets SET role='RETAINED' WHERE pool_id=?1 AND role='ACTIVE'",
        [pool],
    )?;
    ensure!(
        db.execute(
            "UPDATE wallets SET role='ACTIVE' WHERE id=?1 AND pool_id=?2 AND role='READY'",
            params![wallet, pool]
        )? == 1,
        "reserve_role_conflict"
    );
    Ok(Some(ConsumedReserve {
        slot,
        generation,
        next_generation,
    }))
}

/// Shared role publication after immutable destination readiness has been
/// recorded in this same transaction. Financial completion is independent.
pub(super) fn publish_funded_role(db: &Connection, job: &str) -> Result<()> {
    let (wallet,pool,kind,generation,role,ready,enabled): (String,String,String,Option<i64>,String,bool,bool) = db.query_row(
        "SELECT w.id,w.pool_id,f.allocation_kind,f.claim_generation,w.role,f.readiness_requirement IS NOT NULL,p.enabled FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id JOIN pools p ON p.id=w.pool_id WHERE f.id=?1",
        [job], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)))?;
    ensure!(ready, "destination_readiness_missing");
    // Repeat completion cannot restore consumed inventory or overwrite retirement.
    if role != "ALLOCATED" {
        return Ok(());
    }
    let next_role = if kind == "initial" {
        // Completion of historical jobs only; new groups have no initial allocation.
        let active: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM wallets WHERE pool_id=?1 AND role='ACTIVE')",
            [&pool],
            |r| r.get(0),
        )?;
        if active || !enabled {
            "RETAINED"
        } else {
            "ACTIVE"
        }
    } else {
        let claim: Option<(bool,i64)> = db.query_row("SELECT c.enabled=1 AND p.enabled=1,c.generation FROM reserve_claims c JOIN pools p ON p.id=c.pool_id WHERE c.pool_id=?1 AND c.slot=?2 AND c.wallet_id=?3",params![pool,kind,wallet],|r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (enabled, current) = claim.context("funded_reserve_claim_missing")?;
        ensure!(Some(current) == generation, "funded_reserve_claim_conflict");
        if enabled {
            "READY"
        } else {
            let next = current
                .checked_add(1)
                .context("reserve_generation_overflow")?;
            db.execute("UPDATE reserve_claims SET wallet_id=NULL,generation=?4 WHERE pool_id=?1 AND slot=?2 AND wallet_id=?3",params![pool,kind,wallet,next])?;
            "RETAINED"
        }
    };
    db.execute(
        "UPDATE wallets SET role=?2 WHERE id=?1 AND role='ALLOCATED'",
        params![wallet, next_role],
    )?;
    Ok(())
}

/// Reopen-only slot configuration changes affect new work. Incomplete allocations
/// preserve their wallet, generation, original target and recovery history.
pub(super) fn configure_reserves(
    db: &Connection,
    pool: &str,
    reserves: &ReservesConfig,
) -> Result<()> {
    reserves.validate()?;
    for (slot, config) in [
        ("small", Some(&reserves.small)),
        ("large", reserves.large.as_ref()),
    ] {
        if let Some(config) = config {
            db.execute("INSERT INTO reserve_claims(pool_id,slot,enabled,target,maximum_output) VALUES (?1,?2,1,?3,?4) ON CONFLICT(pool_id,slot) DO UPDATE SET enabled=1,target=excluded.target,maximum_output=excluded.maximum_output",params![pool,slot,config.target_atomic()?.to_string(),config.maximum_atomic()?.to_string()])?;
        } else {
            let ready: Option<(String,i64)>=db.query_row("SELECT c.wallet_id,c.generation FROM reserve_claims c JOIN wallets w ON w.id=c.wallet_id WHERE c.pool_id=?1 AND c.slot=?2 AND w.role='READY'",params![pool,slot],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            if let Some((wallet, generation)) = ready {
                let next = generation
                    .checked_add(1)
                    .context("reserve_generation_overflow")?;
                db.execute("UPDATE wallets SET role='RETAINED' WHERE id=?1", [&wallet])?;
                db.execute("UPDATE reserve_claims SET wallet_id=NULL,generation=?3 WHERE pool_id=?1 AND slot=?2",params![pool,slot,next])?;
            }
            db.execute(
                "UPDATE reserve_claims SET enabled=0 WHERE pool_id=?1 AND slot=?2",
                params![pool, slot],
            )?;
        }
    }
    Ok(())
}

/// Disputes remain on the historical retirement record. Neither a changed proof
/// nor a repair annotation reactivates a retired signer or authorizes funding.
pub(super) fn dispute_support(
    db: &Connection,
    wallet: &str,
    hash: &str,
    reason: &str,
) -> Result<()> {
    ensure!(
        matches!(reason, "chain_repair_required" | "evidence_repair_required"),
        "invalid retirement dispute reason"
    );
    db.execute("UPDATE wallet_retirements SET disputed=?3 WHERE wallet_id=?1 AND (confirmed_hash=?2 OR latest_hash=?2 OR checkpoint_hash=?2)",params![wallet,hash,reason])?;
    Ok(())
}
pub(super) fn dispute_endpoint_support(db: &Connection, endpoint: &str) -> Result<()> {
    db.execute(
        "UPDATE wallet_retirements SET disputed='evidence_repair_required' WHERE endpoint=?1",
        [endpoint],
    )?;
    Ok(())
}

impl Store {
    /// Configuration creates the group and its declared empty slots only. The
    /// separate funding owner checks allocation authority for every vacancy.
    pub fn ensure_capacity_group(
        &mut self,
        name: &str,
        reserves: &ReservesConfig,
        limit: Option<U256>,
        retirement: u64,
    ) -> Result<String> {
        ensure!(
            regex::Regex::new("^[a-z][a-z0-9_]*$")?.is_match(name),
            "invalid pool name"
        );
        reserves.validate()?;
        ensure!(
            retirement <= i64::MAX as u64,
            "retirement tolerance overflow"
        );
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing: Option<String> = tx
            .query_row("SELECT id FROM pools WHERE name=?1", [name], |r| r.get(0))
            .optional()?;
        let id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
        tx.execute("INSERT INTO pools(id,name,payment_limit,retirement_tolerance) VALUES (?1,?2,?3,?4) ON CONFLICT(name) DO UPDATE SET enabled=1,payment_limit=excluded.payment_limit,retirement_tolerance=excluded.retirement_tolerance",params![id,name,limit.unwrap_or(U256::MAX).to_string(),retirement.to_string()])?;
        configure_reserves(&tx, &id, reserves)?;
        tx.commit()?;
        Ok(id)
    }
    /// Resolve unsigned identity under the group gate, reconstructing an absent
    /// active role only from eligible retained capacity.
    /// Balance freshness and liabilities belong to later payment admission.
    pub(crate) fn resolve_payment_identity(&mut self, pool: &str) -> Result<Option<String>> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let chosen = resolve_identity(&tx, pool)?;
        tx.commit()?;
        Ok(chosen)
    }

    /// Initialize authority explicitly; migration never derives a payment cap
    /// from historical targets. None here is explicit unlimited, persisted as MAX.
    pub fn configure_capacity_group(
        &mut self,
        pool: &str,
        limit: Option<U256>,
        retirement: u64,
        reserves: &ReservesConfig,
    ) -> Result<()> {
        ensure!(
            retirement <= i64::MAX as u64,
            "retirement tolerance overflow"
        );
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure!(
            tx.execute(
                "UPDATE pools SET enabled=1,payment_limit=?2,retirement_tolerance=?3 WHERE id=?1",
                params![
                    pool,
                    limit.unwrap_or(U256::MAX).to_string(),
                    retirement.to_string()
                ]
            )? == 1,
            "missing capacity group"
        );
        configure_reserves(&tx, pool, reserves)?;
        tx.commit()?;
        Ok(())
    }

    /// Fill one current vacancy. The funding owner calls small, large
    /// in that order, with independent commits so an obstructed slot cannot undo
    /// another allocation. Holdings and treasury forecasts are absent here.
    pub fn allocate_capacity_kind(
        &mut self,
        pool: &str,
        kind: AllocationKind,
        auto_fund: bool,
    ) -> Result<Option<String>> {
        ensure!(auto_fund, "automatic_funding_disabled");
        self.funding_restriction
            .require_new_funding("capacity_allocation")?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let enabled: bool = tx.query_row(
            "SELECT enabled=1 AND payment_limit IS NOT NULL FROM pools WHERE id=?1",
            [pool],
            |r| r.get(0),
        )?;
        ensure!(enabled, "capacity_group_disabled");

        let slot = kind.as_str();
        let claim:Option<(i64,String)>=tx.query_row("SELECT generation,target FROM reserve_claims WHERE pool_id=?1 AND slot=?2 AND enabled=1 AND wallet_id IS NULL",params![pool,slot],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((generation, target)) = claim else {
            return Ok(None);
        };
        let sequence: i64 = tx.query_row(
            "SELECT COALESCE(MAX(sequence),-1)+1 FROM wallets WHERE pool_id=?1",
            [pool],
            |r| r.get(0),
        )?;
        let job = allocation_jobs(
            self.funding_permits.as_ref(),
            &tx,
            &self.id,
            pool,
            &[sequence],
        )?
        .remove(0);
        let wallet = allocate_claim(
            &tx,
            &self.key,
            &self.id,
            self.network,
            pool,
            sequence,
            &job,
            kind.as_str(),
            Some(generation),
            &target,
        )?;
        ensure!(tx.execute("UPDATE reserve_claims SET wallet_id=?4 WHERE pool_id=?1 AND slot=?2 AND generation=?3 AND wallet_id IS NULL AND enabled=1",params![pool,slot,generation,wallet])?==1,"reserve_claim_conflict");
        tx.commit()?;
        Ok(Some(job))
    }
}

const ROLE_IDENTITY_SQL: &str = "SELECT w.id,w.role FROM wallets w JOIN pools p ON p.id=w.pool_id WHERE p.id=?1 AND p.enabled=1 AND p.payment_limit IS NOT NULL AND w.role=?2 ORDER BY w.sequence";
const READY_IDENTITY_SQL: &str = "SELECT w.id,w.role FROM wallets w JOIN reserve_claims c ON c.wallet_id=w.id WHERE w.pool_id=?1 AND w.role=?2 AND c.enabled=1 ORDER BY CASE c.slot WHEN 'small' THEN 0 ELSE 1 END,w.sequence";

/// Read-only default identity selection, also used by startup. A malformed
/// wallet cannot hide independent funded capacity; retain its diagnostic when
/// there is no eligible alternative. No balance acquisition or role change.
pub(super) fn default_identity(db: &Connection, pool: &str) -> Result<Option<(String, String)>> {
    let mut failure = None;
    for role in ["ACTIVE", "RETAINED", "READY"] {
        let sql = if role == "READY" {
            READY_IDENTITY_SQL
        } else {
            ROLE_IDENTITY_SQL
        };
        let mut statement = db.prepare(sql)?;
        let mut rows = statement.query(params![pool, role])?;
        while let Some(row) = rows.next()? {
            let wallet: String = row.get(0)?;
            match payment_ownership(db, pool, &wallet) {
                Ok(Some(_)) => return Ok(Some((wallet, role.to_owned()))),
                Ok(None) => {}
                Err(error)
                    if error.downcast_ref::<super::super::base::evidence::EvidenceFailure>()
                        == Some(
                            &super::super::base::evidence::EvidenceFailure::MalformedSupport,
                        ) =>
                {
                    if failure.is_none() {
                        tracing::warn!(
                            category = "malformed_durable_evidence_support",
                            "Default wallet readiness rejected; checking independent funded identities"
                        );
                        failure = Some(error);
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }
    failure.map_or(Ok(None), Err)
}

fn resolve_identity(db: &Connection, pool: &str) -> Result<Option<String>> {
    let Some((wallet, role)) = default_identity(db, pool)? else {
        return Ok(None);
    };
    // A rejected active identity retains its role and history. Choosing a healthy
    // fallback does not promote it or consume its reserve claim.
    if role == "RETAINED" {
        db.execute("UPDATE wallets SET role='ACTIVE' WHERE id=?1 AND pool_id=?2 AND role='RETAINED' AND NOT EXISTS(SELECT 1 FROM wallets WHERE pool_id=?2 AND role='ACTIVE')",params![wallet,pool])?;
    }
    Ok(Some(wallet))
}

pub(super) fn retire_with_support(
    db: &Connection,
    pool: &str,
    wallet: &str,
    support: &evidence::BalanceSupport,
    now: u64,
) -> Result<bool> {
    let observation = &support.observation;
    ensure!(
        observation.wallet == wallet,
        "retirement_evidence_binding_mismatch"
    );
    observation.validate()?;
    if now > observation.valid_until {
        return Ok(false);
    }
    let (enabled,role,tolerance):(bool,String,Option<String>)=db.query_row("SELECT p.enabled,w.role,p.retirement_tolerance FROM wallets w JOIN pools p ON p.id=w.pool_id WHERE w.id=?1 AND p.id=?2",params![wallet,pool],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    let Some(tolerance) = tolerance else {
        return Ok(false);
    };
    let tolerance = amount(&tolerance)?;
    if !enabled || role != "RETAINED" || observation.remainder() > tolerance {
        return Ok(false);
    }
    let pending: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM payment_attempts WHERE wallet_id=?1 AND state!='RESOLVED')",
        [wallet],
        |r| r.get(0),
    )?;
    let owns_claim: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM reserve_claims WHERE wallet_id=?1 AND enabled=1)",
        [wallet],
        |r| r.get(0),
    )?;
    if pending
        || owns_claim
        || evidence::exposure(db, wallet)? != U256::ZERO
        || funding::destination_obligation_pending(db, wallet)?
    {
        return Ok(false);
    }
    ensure!(
        db.execute(
            "UPDATE wallets SET role='RETIRED' WHERE id=?1 AND pool_id=?2 AND role='RETAINED'",
            params![wallet, pool]
        )? == 1,
        "retirement_role_conflict"
    );
    db.execute("INSERT INTO wallet_retirements(wallet_id,pool_id,confirmed_amount,latest_amount,confirmed_height,confirmed_hash,latest_height,latest_hash,accepted_maximum,observed_bound,support,endpoint,checkpoint_hash,reason,recorded_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'automatic',?14)",params![wallet,pool,observation.confirmed.amount.to_string(),observation.latest.amount.to_string(),i64::try_from(observation.confirmed.anchor.height)?,observation.confirmed.anchor.hash,i64::try_from(observation.latest.anchor.height)?,observation.latest.anchor.hash,tolerance.to_string(),observation.remainder().to_string(),serde_json::to_string(support)?,observation.endpoint,support.checkpoint.as_ref().map(|anchor|anchor.hash.as_str()),i64::try_from(now)?])?;
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn allocate_claim(
    db: &Connection,
    key: &[u8; 32],
    treasury: &str,
    network: TreasuryNetwork,
    pool: &str,
    sequence: i64,
    job: &str,
    kind: &str,
    generation: Option<i64>,
    target: &str,
) -> Result<String> {
    let signer = PrivateKeySigner::random();
    let wallet = Uuid::new_v4().to_string();
    let secret = Zeroizing::new(signer.to_bytes().to_vec());
    let encrypted = seal(
        key,
        &format!("v1:{treasury}:{}:{pool}:evm:{wallet}", network.name()),
        &secret,
    )?;
    db.execute("INSERT INTO wallets(id,pool_id,sequence,address,key,target,role) VALUES (?1,?2,?3,?4,?5,?6,'ALLOCATED')",params![wallet,pool,sequence,signer.address().to_string(),encrypted,target])?;
    db.execute("INSERT INTO funding_jobs(id,wallet_id,state,target,pool_id,allocation_kind,claim_generation) VALUES (?1,?2,'QUEUED',?3,?4,?5,?6)",params![job,wallet,target,pool,kind,generation])?;
    db.execute(
        "INSERT INTO funding_progress(job_id,operation_id,phase) VALUES (?1,?2,'\"ALLOCATED\"')",
        params![job, Uuid::new_v4().to_string()],
    )?;
    Ok(wallet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::base::{
        Anchor,
        evidence::{BalanceObservation, BalancePoint},
    };
    use crate::rotation::config::ReserveConfig;

    fn reserves(large: bool) -> ReservesConfig {
        ReservesConfig {
            small: ReserveConfig {
                funding_amount_usdc: "2".into(),
                max_funding_amount_usdc: None,
            },
            large: large.then(|| ReserveConfig {
                funding_amount_usdc: "10".into(),
                max_funding_amount_usdc: None,
            }),
        }
    }
    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        db.execute_batch(include_str!("schema.sql")).unwrap();
        db.execute("INSERT INTO pools(id,name,payment_limit,retirement_tolerance) VALUES ('pool','pool','100000000','0')",[]).unwrap();
        configure_reserves(&db, "pool", &reserves(true)).unwrap();
        db
    }
    fn allocation(db: &Connection, kind: &str) -> String {
        let seq: i64 = db
            .query_row("SELECT COUNT(*) FROM wallets", [], |r| r.get(0))
            .unwrap();
        let job = format!("job-{seq}");
        let target = if kind == "large" {
            "10000000"
        } else {
            "2000000"
        };
        let generation: Option<i64> = if kind == "initial" {
            None
        } else {
            Some(
                db.query_row(
                    "SELECT generation FROM reserve_claims WHERE pool_id='pool' AND slot=?1",
                    [kind],
                    |r| r.get(0),
                )
                .unwrap(),
            )
        };
        let wallet = allocate_claim(
            db,
            &[0; 32],
            "test",
            TreasuryNetwork::Mainnet,
            "pool",
            seq,
            &job,
            kind,
            generation,
            target,
        )
        .unwrap();
        if kind != "initial" {
            assert_eq!(db.execute("UPDATE reserve_claims SET wallet_id=?2 WHERE pool_id='pool' AND slot=?1 AND wallet_id IS NULL",params![kind,wallet]).unwrap(),1);
        }
        job
    }
    fn wallet(db: &Connection, job: &str) -> String {
        db.query_row(
            "SELECT wallet_id FROM funding_jobs WHERE id=?1",
            [job],
            |r| r.get(0),
        )
        .unwrap()
    }
    fn role(db: &Connection, job: &str) -> String {
        db.query_row(
            "SELECT w.role FROM wallets w JOIN funding_jobs f ON f.wallet_id=w.id WHERE f.id=?1",
            [job],
            |r| r.get(0),
        )
        .unwrap()
    }
    fn support(
        db: &Connection,
        job: &str,
        confirmed: u64,
        latest: u64,
    ) -> evidence::BalanceSupport {
        let wallet = wallet(db, job);
        let address = db
            .query_row("SELECT address FROM wallets WHERE id=?1", [&wallet], |r| {
                r.get(0)
            })
            .unwrap();
        evidence::BalanceSupport {
            observation: BalanceObservation {
                wallet,
                address,
                network: "eip155:8453".into(),
                asset: crate::payment::USDC.into(),
                endpoint: "a".repeat(64),
                confirmations: 12,
                confirmed: BalancePoint {
                    anchor: Anchor {
                        height: 100,
                        hash: format!("0x{:064x}", 100),
                    },
                    amount: U256::from(confirmed),
                },
                latest: BalancePoint {
                    anchor: Anchor {
                        height: 112,
                        hash: format!("0x{:064x}", 112),
                    },
                    amount: U256::from(latest),
                },
                valid_until: 200,
            },
            checkpoint: None,
        }
    }
    fn funded(db: &Connection, job: &str) {
        let support = support(db, job, 10_000_000, 10_000_000);
        funding::record_readiness(db, &wallet(db, job), &support).unwrap();
        publish_funded_role(db, job).unwrap();
    }
    fn pending_payment(db: &Connection, id: &str, wallet: &str) {
        db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES (?1,'pool',?2,0,'1','fixture','POSSIBLY_SUBMITTED')",params![id,wallet]).unwrap();
    }

    #[test]
    fn historical_initial_completion_orders_preserve_exact_jobs_and_inventory() {
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let mut db = database();
            let tx = db.transaction().unwrap();
            let jobs = [
                allocation(&tx, "initial"),
                allocation(&tx, "small"),
                allocation(&tx, "large"),
            ];
            for i in order {
                funded(&tx, &jobs[i]);
            }
            assert_eq!(role(&tx, &jobs[0]), "ACTIVE");
            assert_eq!(role(&tx, &jobs[1]), "READY");
            assert_eq!(role(&tx, &jobs[2]), "READY");
            assert_eq!(
                tx.query_row("SELECT COUNT(*) FROM funding_jobs", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                3
            );
            assert_eq!(tx.query_row("SELECT COUNT(*) FROM reserve_claims WHERE wallet_id IS NOT NULL AND generation=0",[],|r|r.get::<_,i64>(0)).unwrap(),2);
            tx.commit().unwrap();
        }
    }
    #[test]
    fn unsigned_reserve_selection_is_read_only_and_payment_consumes_atomically() {
        let mut db = database();
        let initial = allocation(&db, "initial");
        let small = allocation(&db, "small");
        funded(&db, &small);
        let payer = wallet(&db, &small);
        for _ in 0..3 {
            assert_eq!(
                payment_ownership(&db, "pool", &payer)
                    .unwrap()
                    .unwrap()
                    .role,
                "READY"
            )
        }
        assert_eq!(role(&db, &small), "READY");
        {
            let tx = db.transaction().unwrap();
            consume_reserve(&tx, "pool", &payer).unwrap().unwrap();
            // A later journal failure rolls back the role and claim too.
        }
        assert_eq!(role(&db, &small), "READY");
        let tx = db.transaction().unwrap();
        let transition = consume_reserve(&tx, "pool", &payer).unwrap().unwrap();
        assert_eq!((transition.generation, transition.next_generation), (0, 1));
        pending_payment(&tx, "attempt", &payer);
        tx.commit().unwrap();
        funded(&db, &initial);
        assert_eq!(role(&db, &initial), "RETAINED");
        assert_eq!(role(&db, &small), "ACTIVE");
        let replacement = allocation(&db, "small");
        funded(&db, &replacement);
        funded(&db, &small);
        assert_eq!(role(&db, &small), "ACTIVE");
        assert_eq!(role(&db, &replacement), "READY");
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn disable_preserves_partial_claim_and_detaches_only_funded_reserve() {
        let mut db = database();
        let large = allocation(&db, "large");
        let payer = wallet(&db, &large);
        db.execute("UPDATE wallets SET balance='3000000' WHERE id=?1", [&payer])
            .unwrap();
        configure_reserves(&db, "pool", &reserves(false)).unwrap();
        assert_eq!(role(&db, &large), "ALLOCATED");
        assert!(payment_ownership(&db, "pool", &payer).unwrap().is_none());
        assert_eq!(
            db.query_row(
                "SELECT wallet_id FROM reserve_claims WHERE slot='large'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            payer
        );
        configure_reserves(&db, "pool", &reserves(true)).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT generation FROM reserve_claims WHERE slot='large'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        funded(&db, &large);
        let tx = db.transaction().unwrap();
        configure_reserves(&tx, "pool", &reserves(false)).unwrap();
        tx.commit().unwrap();
        assert_eq!(role(&db, &large), "RETAINED");
        assert!(
            db.query_row(
                "SELECT wallet_id IS NULL AND generation=1 FROM reserve_claims WHERE slot='large'",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
        configure_reserves(&db, "pool", &reserves(true)).unwrap();
        let next = allocation(&db, "large");
        assert_ne!(next, large);
        funded(&db, &large);
        assert_eq!(role(&db, &large), "RETAINED");
    }
    #[test]
    fn disabled_completion_retains_and_unconfigured_authority_never_pays() {
        let db = database();
        let large = allocation(&db, "large");
        configure_reserves(&db, "pool", &reserves(false)).unwrap();
        funded(&db, &large);
        assert_eq!(role(&db, &large), "RETAINED");
        let payer = wallet(&db, &large);
        db.execute("UPDATE pools SET payment_limit=NULL", [])
            .unwrap();
        assert!(payment_ownership(&db, "pool", &payer).unwrap().is_none());
        db.execute("UPDATE pools SET payment_limit=?1", [U256::MAX.to_string()])
            .unwrap();
        assert_eq!(
            payment_ownership(&db, "pool", &payer)
                .unwrap()
                .unwrap()
                .payment_limit,
            U256::MAX
        );
        db.execute("UPDATE pools SET enabled=0", []).unwrap();
        assert!(payment_ownership(&db, "pool", &payer).unwrap().is_none());
    }
    #[test]
    fn retirement_uses_maximum_dual_zero_and_exact_destination_obligations() {
        let mut db = database();
        let initial = allocation(&db, "initial");
        let small = allocation(&db, "small");
        funded(&db, &initial);
        funded(&db, &small);
        let tx = db.transaction().unwrap();
        consume_reserve(&tx, "pool", &wallet(&tx, &small)).unwrap();
        tx.commit().unwrap();
        let retired = wallet(&db, &initial);
        let one = support(&db, &initial, 0, 1);
        assert!(!retire_with_support(&db, "pool", &retired, &one, 100).unwrap());
        db.execute("UPDATE pools SET retirement_tolerance='1'", [])
            .unwrap();
        db.execute(
            "UPDATE funding_progress SET quote=x'00' WHERE job_id=?1",
            [&initial],
        )
        .unwrap();
        assert!(!retire_with_support(&db, "pool", &retired, &one, 100).unwrap());
        db.execute("UPDATE funding_jobs SET readiness_operation=(SELECT operation_id FROM funding_progress WHERE job_id=?1) WHERE id=?1",[&initial]).unwrap();
        db.execute(
            "UPDATE funding_progress SET provider_success=1,accepted_output='2000000',accepted_recipient=(SELECT address FROM wallets WHERE id=?2) WHERE job_id=?1",
            params![initial,retired],
        )
        .unwrap();
        pending_payment(&db, "pending", &retired);
        assert!(!retire_with_support(&db, "pool", &retired, &one, 100).unwrap());
        db.execute("UPDATE payment_attempts SET state='RESOLVED'", [])
            .unwrap();
        // Success closes destination while source work remains scheduled.
        let tx = db.transaction().unwrap();
        assert!(retire_with_support(&tx, "pool", &retired, &one, 100).unwrap());
        tx.commit().unwrap();
        assert_eq!(role(&db, &initial), "RETIRED");
        assert!(!retire_with_support(&db, "pool", &retired, &one, 100).unwrap());
        let record:(String,String,String)=db.query_row("SELECT confirmed_amount,latest_amount,observed_bound FROM wallet_retirements WHERE wallet_id=?1",[&retired],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(record, ("0".into(), "1".into(), "1".into()));
        assert_eq!(
            db.query_row(
                "SELECT state FROM funding_jobs WHERE id=?1",
                [&initial],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "QUEUED"
        );
        assert!(
            db.query_row(
                "SELECT LENGTH(key)>0 FROM wallets WHERE id=?1",
                [&retired],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
        assert!(payment_ownership(&db, "pool", &retired).unwrap().is_none());
        funded(&db, &initial);
        assert_eq!(role(&db, &initial), "RETIRED");
    }
    #[test]
    fn retirement_rechecks_role_enablement_freshness_and_both_exact_amounts() {
        for (confirmed, latest, tolerance, role, enabled, now, expected) in [
            (0, 0, 0, "RETAINED", true, 100, true),
            (0, 1, 0, "RETAINED", true, 100, false),
            (1, 0, 0, "RETAINED", true, 100, false),
            (2, 1, 1, "RETAINED", true, 100, false),
            (1, 2, 1, "RETAINED", true, 100, false),
            (1, 1, 1, "RETAINED", true, 100, true),
            (0, 0, 0, "ACTIVE", true, 100, false),
            (0, 0, 0, "READY", true, 100, false),
            (0, 0, 0, "RETAINED", false, 100, false),
            (0, 0, 0, "RETAINED", true, 201, false),
        ] {
            let mut db = database();
            let job = allocation(&db, "initial");
            funded(&db, &job);
            let wallet = wallet(&db, &job);
            db.execute(
                "UPDATE wallets SET role=?1 WHERE id=?2",
                params![role, wallet],
            )
            .unwrap();
            db.execute(
                "UPDATE pools SET enabled=?1,retirement_tolerance=?2",
                params![enabled, tolerance.to_string()],
            )
            .unwrap();
            let support = support(&db, &job, confirmed, latest);
            let tx = db.transaction().unwrap();
            assert_eq!(
                retire_with_support(&tx, "pool", &wallet, &support, now).unwrap(),
                expected
            );
            tx.commit().unwrap();
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM wallet_retirements", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                i64::from(expected)
            );
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM funding_jobs", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
    }
    #[test]
    fn allocation_identity_is_immutable_and_initial_is_unique() {
        let mut db = database();
        let job = allocation(&db, "initial");
        assert!(
            db.execute(
                "UPDATE funding_jobs SET allocation_kind='small',claim_generation=0 WHERE id=?1",
                [&job]
            )
            .is_err()
        );
        assert!(
            db.execute("UPDATE funding_jobs SET target='999' WHERE id=?1", [&job])
                .is_err()
        );
        {
            let tx = db.transaction().unwrap();
            assert!(
                allocate_claim(
                    &tx,
                    &[0; 32],
                    "test",
                    TreasuryNetwork::Mainnet,
                    "pool",
                    1,
                    "duplicate",
                    "initial",
                    None,
                    "2"
                )
                .is_err()
            );
        }
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM wallets", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn current_funding_authority_preserves_original_target_and_exact_claim() {
        let db = database();
        let initial = allocation(&db, "initial");
        let large = allocation(&db, "large");
        assert_eq!(
            current_funding_claim(&db, &initial)
                .unwrap()
                .unwrap()
                .target,
            U256::from(2_000_000)
        );
        assert_eq!(
            current_funding_claim(&db, &large).unwrap().unwrap().target,
            U256::from(10_000_000)
        );
        configure_reserves(&db, "pool", &reserves(false)).unwrap();
        assert!(current_funding_claim(&db, &large).unwrap().is_none());
        let mut changed = reserves(true);
        changed.large.as_mut().unwrap().funding_amount_usdc = "20".into();
        configure_reserves(&db, "pool", &changed).unwrap();
        assert_eq!(
            current_funding_claim(&db, &large).unwrap().unwrap().target,
            U256::from(20_000_000)
        );
        assert_eq!(
            db.query_row(
                "SELECT target FROM funding_jobs WHERE id=?1",
                [&large],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "10000000"
        );
        funded(&db, &large);
        assert!(current_funding_claim(&db, &large).unwrap().is_none());
        db.execute("UPDATE pools SET enabled=0", []).unwrap();
        assert!(current_funding_claim(&db, &initial).unwrap().is_none());
    }
    #[test]
    fn independent_allocations_keep_committed_siblings_when_another_slot_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = store
            .ensure_capacity_group("capacity", &reserves(true), Some(U256::from(100)), 0)
            .unwrap();
        let _small = store
            .allocate_capacity_kind(&pool, AllocationKind::Small, true)
            .unwrap()
            .unwrap();
        store.db.execute_batch("CREATE TRIGGER reject_large BEFORE INSERT ON funding_jobs WHEN NEW.allocation_kind='large' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(
            store
                .allocate_capacity_kind(&pool, AllocationKind::Large, true)
                .is_err()
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT COUNT(*) FROM wallets", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT COUNT(*) FROM funding_progress", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(
            store
                .allocate_capacity_kind(&pool, AllocationKind::Small, true)
                .unwrap()
                .is_none()
        );
        store.db.execute_batch("DROP TRIGGER reject_large").unwrap();
        assert!(
            store
                .allocate_capacity_kind(&pool, AllocationKind::Large, false)
                .is_err()
        );
        assert!(
            store
                .allocate_capacity_kind(&pool, AllocationKind::Large, true)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT COUNT(*) FROM wallets", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT COUNT(*) FROM budget_entries", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn disabled_group_late_credit_detaches_only_the_fulfilled_reserve() {
        let db = database();
        let initial = allocation(&db, "initial");
        let small = allocation(&db, "small");
        let large = allocation(&db, "large");
        let large_wallet = wallet(&db, &large);
        db.execute("UPDATE pools SET enabled=0", []).unwrap();
        let partial = support(&db, &large, 1, 1);
        assert!(!funding::record_readiness(&db, &large_wallet, &partial).unwrap());
        funded(&db, &initial);
        funded(&db, &small);
        assert_eq!(role(&db, &initial), "RETAINED");
        assert_eq!(role(&db, &small), "RETAINED");
        assert_eq!(role(&db, &large), "ALLOCATED");
        assert_eq!(
            db.query_row(
                "SELECT generation FROM reserve_claims WHERE slot='small'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT wallet_id FROM reserve_claims WHERE slot='large'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            large_wallet
        );
        assert!(current_funding_claim(&db, &large).unwrap().is_none());
        db.execute("UPDATE pools SET enabled=1", []).unwrap();
        assert_eq!(
            current_funding_claim(&db, &large).unwrap().unwrap().target,
            U256::from(10_000_000)
        );
        assert_eq!(role(&db, &small), "RETAINED");
    }

    #[test]
    fn retirement_rejects_exposure_disputed_readiness_and_corrupted_support() {
        let db = database();
        let job = allocation(&db, "initial");
        funded(&db, &job);
        let wallet = wallet(&db, &job);
        db.execute(
            "UPDATE wallets SET role='RETAINED',unresolved_exposure='1' WHERE id=?1",
            [&wallet],
        )
        .unwrap();
        let zero = support(&db, &job, 0, 0);
        assert!(!retire_with_support(&db, "pool", &wallet, &zero, 100).unwrap());
        db.execute(
            "UPDATE wallets SET unresolved_exposure='0' WHERE id=?1",
            [&wallet],
        )
        .unwrap();
        db.execute(
            "UPDATE funding_jobs SET readiness_disputed='fixture' WHERE id=?1",
            [&job],
        )
        .unwrap();
        assert!(!retire_with_support(&db, "pool", &wallet, &zero, 100).unwrap());
        db.execute("UPDATE funding_jobs SET readiness_disputed=NULL,readiness_support='malformed' WHERE id=?1", [&job]).unwrap();
        assert!(retire_with_support(&db, "pool", &wallet, &zero, 100).is_err());
        assert_eq!(role(&db, &job), "RETAINED");
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM wallet_retirements", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn payment_authority_is_explicit_and_independent_of_allocation_target() {
        let db = database();
        let job = allocation(&db, "initial");
        funded(&db, &job);
        let wallet = wallet(&db, &job);
        for cap in [U256::ZERO, U256::from(1), U256::from(50_000_000), U256::MAX] {
            db.execute("UPDATE pools SET payment_limit=?1", [cap.to_string()])
                .unwrap();
            assert_eq!(
                payment_ownership(&db, "pool", &wallet)
                    .unwrap()
                    .unwrap()
                    .payment_limit,
                cap
            );
            assert_eq!(
                db.query_row("SELECT target FROM funding_jobs WHERE id=?1", [&job], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                "2000000"
            );
        }
        db.execute("UPDATE pools SET payment_limit=NULL", [])
            .unwrap();
        assert!(payment_ownership(&db, "pool", &wallet).unwrap().is_none());
    }
    #[test]
    fn active_reconstruction_prefers_sequence_and_never_consumes_ready() {
        let mut db = database();
        let initial = allocation(&db, "initial");
        let small = allocation(&db, "small");
        let large = allocation(&db, "large");
        for job in [&initial, &small, &large] {
            funded(&db, job);
        }
        let first = wallet(&db, &initial);
        let second = wallet(&db, &small);
        db.execute(
            "UPDATE wallets SET role='RETAINED' WHERE id IN (?1,?2)",
            params![first, second],
        )
        .unwrap();
        {
            let tx = db.transaction().unwrap();
            assert_eq!(resolve_identity(&tx, "pool").unwrap(), Some(first.clone()));
            // Cancellation/failure of the enclosing owner commit preserves roles.
        }
        assert_eq!(role(&db, &initial), "RETAINED");
        let tx = db.transaction().unwrap();
        assert_eq!(resolve_identity(&tx, "pool").unwrap(), Some(first.clone()));
        tx.commit().unwrap();
        assert_eq!(role(&db, &large), "READY");
        assert_eq!(
            db.query_row(
                "SELECT generation FROM reserve_claims WHERE slot='large'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        // No cached observation exists; zero balance and unresolved liabilities
        // do not change the unsigned active identity.
        pending_payment(&db, "pending-active", &first);
        db.execute(
            "UPDATE wallets SET balance='0',unresolved_exposure='1' WHERE id=?1",
            [&first],
        )
        .unwrap();
        let tx = db.transaction().unwrap();
        assert_eq!(resolve_identity(&tx, "pool").unwrap(), Some(first.clone()));
        tx.commit().unwrap();
        assert_eq!(role(&db, &initial), "ACTIVE");
        assert_eq!(role(&db, &small), "RETAINED");
        db.execute("UPDATE pools SET payment_limit='0'", [])
            .unwrap();
        assert_eq!(resolve_identity(&db, "pool").unwrap(), Some(first));
        db.execute("UPDATE pools SET enabled=0", []).unwrap();
        assert!(resolve_identity(&db, "pool").unwrap().is_none());
    }
    #[test]
    fn identity_lookup_work_is_independent_of_retired_and_retained_history() {
        use rusqlite::StatementStatus;
        let mut db = database();
        let initial = allocation(&db, "initial");
        funded(&db, &initial);
        let first = wallet(&db, &initial);
        let inspect = |db: &Connection, role: &str| {
            let mut stmt = db.prepare(ROLE_IDENTITY_SQL).unwrap();
            let chosen: Option<(String, String)> = stmt
                .query_row(params!["pool", role], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()
                .unwrap();
            assert_eq!(stmt.get_status(StatementStatus::FullscanStep), 0);
            assert_eq!(stmt.get_status(StatementStatus::Sort), 0);
            (chosen, stmt.get_status(StatementStatus::VmStep))
        };
        let before = inspect(&db, "ACTIVE");
        let tx = db.transaction().unwrap();
        for n in 1000..21000 {
            tx.execute("INSERT INTO wallets(id,pool_id,sequence,address,key,target,role) VALUES (?1,'pool',?2,?1,X'00','1',?3)",params![format!("history-{n}"),n,if n%2==0 {"RETIRED"} else {"RETAINED"}]).unwrap();
        }
        tx.commit().unwrap();
        let after = inspect(&db, "ACTIVE");
        assert_eq!(before, after);
        assert_eq!(after.0.unwrap().0, first);
        db.execute("UPDATE wallets SET role='RETAINED' WHERE id=?1", [&first])
            .unwrap();
        assert!(inspect(&db, "ACTIVE").0.is_none());
        let retained = inspect(&db, "RETAINED");
        assert_eq!(retained.0.unwrap().0, first);
        assert!(retained.1 < 100, "retained fallback scanned history");
        assert_eq!(resolve_identity(&db, "pool").unwrap(), Some(first));
    }

    #[test]
    fn reopening_schema16_adds_lookup_indexes_without_changing_inventory() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"fixture").unwrap();
        let id = store.id().to_owned();
        let pool = store
            .ensure_capacity_group("index_reopen", &reserves(true), Some(U256::from(100)), 0)
            .unwrap();
        store
            .allocate_capacity_kind(&pool, AllocationKind::Small, true)
            .unwrap();
        let before = serde_json::to_value(store.status().unwrap()).unwrap();
        store
            .db
            .execute_batch("DROP INDEX wallet_role_sequence; DROP INDEX wallet_live_sequence; DROP INDEX funding_claim_lookup; DROP INDEX funding_recovery_job")
            .unwrap();
        drop(store);
        for _ in 0..2 {
            let reopened = Store::open(&state, &key, &id).unwrap();
            for index in [
                "wallet_role_sequence",
                "wallet_live_sequence",
                "funding_claim_lookup",
                "funding_recovery_job",
            ] {
                let exists: bool = reopened
                    .db
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)",
                        [index],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert!(exists, "missing {index}");
            }
            assert_eq!(
                serde_json::to_value(reopened.status().unwrap()).unwrap(),
                before
            );
        }
    }
}

#[cfg(test)]
mod reserve_bootstrap_tests {
    use super::*;

    #[test]
    fn fresh_reserves_need_no_active_wallet_and_bootstrap_keeps_captured_generations() {
        for large in [false, true] {
            for reverse in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let state = dir.path().join("state");
                let key = dir.path().join("key");
                let mut store = Store::create(&state, &key, 1, b"fixture").unwrap();
                let mut config = super::super::test_support::reserves("2");
                if large {
                    config.large = Some(config.small.clone());
                }
                let pool = store
                    .ensure_capacity_group("fresh", &config, None, 0)
                    .unwrap();
                for _ in 0..3 {
                    store.fill_funding_vacancies("fresh").unwrap();
                }
                let count = if large { 2 } else { 1 };
                let mut jobs = store.funding_jobs().unwrap();
                assert_eq!(jobs.len(), count);
                assert_eq!(
                    store
                        .db
                        .query_row(
                            "SELECT COUNT(*) FROM funding_jobs WHERE allocation_kind='initial'",
                            [],
                            |r| r.get::<_, i64>(0)
                        )
                        .unwrap(),
                    0
                );
                let captured = store
                    .startup_groups(&["fresh".into()])
                    .unwrap()
                    .remove(0)
                    .claims;
                assert_eq!(captured.len(), count);
                if reverse {
                    jobs.reverse();
                }
                for (i, job) in jobs.iter().enumerate() {
                    store
                        .record_credit(&job.wallet_id, &job.target, "fixture", 100)
                        .unwrap();
                    let startup = store.startup_groups(&["fresh".into()]).unwrap().remove(0);
                    assert!(startup.serving_ready);
                    assert_eq!(startup.inventory_ready, i + 1 == count);
                }
                let id = store.id().to_owned();
                drop(store);
                let mut store = Store::open(&state, &key, &id).unwrap();
                assert!(store.status().unwrap().pools[0].inventory_ready);
                let before = store.status().unwrap();
                for _ in 0..3 {
                    assert!(store.payment_candidate(&pool).is_ok());
                    store.fill_funding_vacancies("fresh").unwrap();
                }
                assert_eq!(store.funding_jobs().unwrap().len(), count);
                assert!(
                    store.status().unwrap().pools[0]
                        .addresses
                        .iter()
                        .all(|w| w.role == "READY")
                );
                assert!(before.pools[0].addresses.iter().all(|w| w.role == "READY"));
                let payer = jobs[0].wallet_id.clone();
                let transition = {
                    let tx = store.db.transaction().unwrap();
                    let consumed = consume_reserve(&tx, &pool, &payer).unwrap().unwrap();
                    // A concurrent second admission on this payer cannot consume
                    // another generation; actual signing/journal races are in manager tests.
                    assert!(consume_reserve(&tx, &pool, &payer).unwrap().is_none());
                    tx.commit().unwrap();
                    consumed
                };
                assert!(!store.status().unwrap().pools[0].inventory_ready);
                for _ in 0..3 {
                    store.fill_funding_vacancies("fresh").unwrap();
                }
                assert_eq!(store.funding_jobs().unwrap().len(), count + 1);
                let replacement = store.db.query_row("SELECT wallet_id,generation FROM reserve_claims WHERE pool_id=?1 AND slot=?2", params![pool,transition.slot], |r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?))).unwrap();
                assert_ne!(replacement.0, payer);
                assert_eq!(replacement.1, transition.next_generation);
                let paths = store.startup_claim_paths(&pool, &captured).unwrap();
                assert_eq!(paths.len(), count);
                assert!(paths.iter().all(|path| path.destination_ready));
                assert!(
                    store
                        .startup_obstruction("fresh", 3, Some(&captured))
                        .unwrap()
                        .is_none()
                );
                drop(store);
                let mut store = Store::open(&state, &key, &id).unwrap();
                store.fill_funding_vacancies("fresh").unwrap();
                assert_eq!(store.funding_jobs().unwrap().len(), count + 1);
            }
        }
    }

    #[test]
    fn historical_prepared_initial_job_survives_reopen_without_becoming_a_bootstrap_claim() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"fixture").unwrap();
        let pool = store
            .ensure_capacity_group(
                "legacy",
                &super::super::test_support::reserves("2"),
                None,
                0,
            )
            .unwrap();
        let id = store.id().to_owned();
        // Reproduce a persisted job from the previous release, without a runtime
        // entry point that can create new initial jobs.
        let wallet = allocate_claim(
            &store.db,
            &store.key,
            &id,
            store.network,
            &pool,
            0,
            "historical",
            "initial",
            None,
            "2000000",
        )
        .unwrap();
        let job = store.funding_job("historical").unwrap().unwrap();
        store
            .save_funding_quote(
                &job.id,
                &crate::test_funding_quote::bound(&job.target, &job.recipient, "historical"),
            )
            .unwrap();
        store
            .advance_funding(
                &job.id,
                funding::FundingPhase::Quoted,
                funding::FundingPhase::Preparing,
            )
            .unwrap();
        store
            .reserve(&job.operation_id, Some(&pool), 1, 100)
            .unwrap();
        let revision = store.snapshot().unwrap().0;
        store
            .prepare_with_facts(
                &job.operation_id,
                revision,
                b"prepared",
                b"signed",
                Some(crate::rotation::transaction::TransactionFacts {
                    txid: "historical-tx".into(),
                    expiry_height: 100,
                    amount_zatoshis: 80,
                    fee_zatoshis: 20,
                    deadline: 1000,
                }),
            )
            .unwrap();
        drop(store);
        let mut store = Store::open(&state, &key, &id).unwrap();
        assert_eq!(
            store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
            b"signed"
        );
        assert!(current_funding_claim(&store.db, &job.id).unwrap().is_some());
        store.fill_funding_vacancies("legacy").unwrap();
        let group = store.startup_groups(&["legacy".into()]).unwrap().remove(0);
        assert_eq!(group.claims.len(), 1);
        assert_eq!(group.claims[0].slot, "small");
        let reserve = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.id != job.id)
            .unwrap();
        store
            .record_credit(&reserve.wallet_id, &reserve.target, "fixture", 100)
            .unwrap();
        assert!(
            store
                .startup_claim_paths(&pool, &group.claims)
                .unwrap()
                .iter()
                .all(|p| p.destination_ready)
        );
        {
            let tx = store.db.transaction().unwrap();
            consume_reserve(&tx, &pool, &reserve.wallet_id)
                .unwrap()
                .unwrap();
            tx.commit().unwrap();
        }
        store
            .record_credit(&wallet, &job.target, "fixture", 100)
            .unwrap();
        let status = store.status().unwrap();
        assert_eq!(
            status.pools[0]
                .addresses
                .iter()
                .find(|w| w.id == wallet)
                .unwrap()
                .role,
            "RETAINED"
        );
        assert_eq!(
            status.pools[0]
                .addresses
                .iter()
                .find(|w| w.id == reserve.wallet_id)
                .unwrap()
                .role,
            "ACTIVE"
        );
        assert_eq!(
            store.prepared_bytes(&job.operation_id).unwrap().as_slice(),
            b"signed"
        );
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT reserved FROM budget_entries WHERE id=?1",
                    [&job.operation_id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            100
        );
    }

    #[tokio::test]
    async fn concurrent_vacancy_passes_allocate_one_job_per_enabled_slot() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let mut config = super::super::test_support::reserves("2");
        config.large = Some(config.small.clone());
        store
            .ensure_capacity_group("fresh", &config, None, 0)
            .unwrap();
        let (handle, worker) = StoreHandle::spawn(store);
        let results = futures_util::future::join_all(
            (0..16).map(|_| handle.call(|s| s.fill_funding_vacancies("fresh"))),
        )
        .await;
        assert!(results.into_iter().all(|result| result.is_ok()));
        handle
            .call(|s| {
                assert_eq!(s.funding_jobs()?.len(), 2);
                assert!(
                    s.status()?.pools[0]
                        .addresses
                        .iter()
                        .all(|wallet| wallet.role == "ALLOCATED")
                );
                Ok(())
            })
            .await
            .unwrap();
        drop(handle);
        worker.await.unwrap();
    }
}
