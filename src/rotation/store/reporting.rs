//! Read-only reporting from existing durable facts and the current owner cache.
use super::*;

#[derive(Serialize)]
pub struct RuntimeBalanceEvidence {
    pub confirmed: BalancePointStatus,
    pub latest: BalancePointStatus,
    pub valid_until: u64,
    pub network: String,
    pub asset: String,
    pub endpoint: String,
    pub confirmations: u64,
    pub fresh: bool,
    pub compatible: bool,
    pub unavailable_reason: Option<String>,
}
#[derive(Serialize)]
pub struct BalancePointStatus {
    pub amount_atomic: String,
    pub height: u64,
    pub hash: String,
    /// Block timestamps are not retained in the existing balance certificate.
    pub block_timestamp: Option<u64>,
}
impl From<&super::super::base::evidence::BalancePoint> for BalancePointStatus {
    fn from(point: &super::super::base::evidence::BalancePoint) -> Self {
        Self {
            amount_atomic: point.amount.to_string(),
            height: point.anchor.height,
            hash: point.anchor.hash.clone(),
            block_timestamp: None,
        }
    }
}
#[derive(Default, Serialize)]
pub struct RetirementAccounting {
    pub automatic_recorded_confirmed_atomic: Option<String>,
    pub automatic_count: Option<usize>,
    pub manual_recorded_confirmed_atomic: Option<String>,
    pub manual_count: Option<usize>,
    pub historical_unknown_count: usize,
}
impl RetirementAccounting {
    pub(super) fn from_wallets(wallets: &[AddressStatus], schema: u32) -> Result<Self> {
        let mut total = U256::ZERO;
        let mut automatic = 0;
        let mut unknown = 0;
        for wallet in wallets {
            if let Some(retirement) = &wallet.retirement {
                total = total
                    .checked_add(amount(&retirement.confirmed_amount)?)
                    .context("retirement accounting total overflow")?;
                automatic += 1;
            } else if wallet.role == "RETIRED" {
                unknown += 1;
            }
        }
        Ok(Self {
            automatic_recorded_confirmed_atomic: (matches!(schema, 16..=22))
                .then(|| total.to_string()),
            automatic_count: (matches!(schema, 16..=22)).then_some(automatic),
            manual_recorded_confirmed_atomic: (matches!(schema, 16..=22)).then(|| "0".into()),
            manual_count: (matches!(schema, 16..=22)).then_some(0),
            historical_unknown_count: unknown,
        })
    }
}

pub(super) fn runtime_status(store: &Store, checked_at: u64) -> Result<Status> {
    let mut status = read_status(&store.db)?;
    for wallet in status.pools.iter_mut().flat_map(|pool| &mut pool.addresses) {
        let Some((_, certificate)) = store.evidence_cache.get(&wallet.id) else {
            continue;
        };
        let observation = certificate.observation();
        let fresh = certificate.is_fresh(checked_at)?;
        let (compatible, unavailable_reason) =
            match evidence::check_payment_balance(&store.db, store.evidence_namespace, certificate)
            {
                Ok(_) => (true, (!fresh).then(|| "evidence_expired".into())),
                Err(error) => {
                    match error.downcast_ref::<super::super::base::evidence::EvidenceFailure>() {
                        Some(reason) => (false, Some(reason.to_string())),
                        None => return Err(error),
                    }
                }
            };
        wallet.runtime_balance_evidence = Some(RuntimeBalanceEvidence {
            confirmed: (&observation.confirmed).into(),
            latest: (&observation.latest).into(),
            valid_until: observation.valid_until,
            network: observation.network.clone(),
            asset: observation.asset.clone(),
            endpoint: observation.endpoint.clone(),
            confirmations: observation.confirmations,
            fresh,
            compatible,
            unavailable_reason,
        });
    }
    Ok(status)
}

// Persisted projections use the caller's connection and preserve its snapshot.
pub(super) fn read_pools(db: &Connection, version: u32) -> Result<Vec<PoolStatus>> {
    let mut stmt = db.prepare("SELECT id,name,generation,enabled FROM pools ORDER BY name")?;
    let rows = stmt.query_map([], |r| {
        Ok(PoolStatus {
            id: r.get(0)?,
            name: r.get(1)?,
            generation: r.get(2)?,
            enabled: r.get(3)?,
            payment_limit_atomic: None,
            retirement_tolerance_atomic: None,
            retirement_accounting: RetirementAccounting::default(),
            serving_ready: false,
            inventory_ready: false,
            reserve_claims: Vec::new(),
            retained_wallets: 0,
            retired_wallets: 0,
            funding_degraded: false,
            addresses: Vec::new(),
        })
    })?;
    let mut pools = Vec::new();
    for row in rows {
        let mut pool = row?;
        pool.addresses = read_addresses(db, &pool.id)?;
        if matches!(version, 19..=22) {
            populate_current_pool(db, &mut pool, version)?;
        } else if matches!(version, 16..=18) {
            // Historical accounting is independent of current admission fields.
            for wallet in &mut pool.addresses {
                wallet.retirement = read_retirement(db, &wallet.id)?;
                pool.retained_wallets += usize::from(wallet.role == "RETAINED");
                pool.retired_wallets += usize::from(wallet.role == "RETIRED");
            }
        }
        pool.retirement_accounting = RetirementAccounting::from_wallets(&pool.addresses, version)?;
        pools.push(pool);
    }
    Ok(pools)
}

fn read_addresses(db: &Connection, pool: &str) -> Result<Vec<AddressStatus>> {
    let mut stmt = db.prepare("SELECT id,address,role,target,balance,block_height,block_hash FROM wallets WHERE pool_id=?1 ORDER BY sequence")?;
    stmt.query_map([pool], |r| {
        Ok(AddressStatus {
            id: r.get(0)?,
            address: r.get(1)?,
            role: r.get(2)?,
            target: r.get(3)?,
            confirmed_balance: r.get(4)?,
            confirmed_height: r.get(5)?,
            confirmed_hash: r.get(6)?,
            signed_exposure_atomic: None,
            financial_repair: None,
            destination_recorded: false,
            destination_disputed: None,
            retirement: None,
            runtime_balance_evidence: None,
        })
    })?
    .collect::<rusqlite::Result<_>>()
    .map_err(Into::into)
}

fn read_retirement(db: &Connection, wallet: &str) -> Result<Option<RetirementStatus>> {
    Ok(db.query_row("SELECT confirmed_amount,latest_amount,accepted_maximum,observed_bound,confirmed_height,confirmed_hash,latest_height,latest_hash,disputed,recorded_at FROM wallet_retirements WHERE wallet_id=?1",[wallet],|r|Ok(RetirementStatus {
            confirmed_amount:r.get(0)?,latest_amount:r.get(1)?,accepted_maximum:r.get(2)?,observed_bound:r.get(3)?,confirmed_height:r.get(4)?,confirmed_hash:r.get(5)?,latest_height:r.get(6)?,latest_hash:r.get(7)?,disputed:r.get(8)?,recorded_at:r.get(9)?,
        })).optional()?)
}

fn populate_current_pool(db: &Connection, pool: &mut PoolStatus, version: u32) -> Result<()> {
    (pool.payment_limit_atomic, pool.retirement_tolerance_atomic) = db.query_row(
        "SELECT payment_limit,retirement_tolerance FROM pools WHERE id=?1",
        [&pool.id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    for wallet in &mut pool.addresses {
        wallet.signed_exposure_atomic = Some(
            if version >= 21 {
                evidence::exposure(db, &wallet.id)?
            } else {
                evidence::historical_exposure(db, &wallet.id)?
            }
            .to_string(),
        );
        wallet.financial_repair = db.query_row(
            "SELECT evidence_repair FROM wallets WHERE id=?1",
            [&wallet.id],
            |r| r.get(0),
        )?;
        wallet.destination_recorded =
            reported_readiness(funding::readiness_recorded_valid(db, &wallet.id))?;
        wallet.destination_disputed = db.query_row(
            "SELECT readiness_disputed FROM funding_jobs WHERE wallet_id=?1",
            [&wallet.id],
            |r| r.get(0),
        )?;
        wallet.retirement = read_retirement(db, &wallet.id)?;
        pool.retained_wallets += usize::from(wallet.role == "RETAINED");
        pool.retired_wallets += usize::from(wallet.role == "RETIRED");
        pool.serving_ready |= capacity::payment_ownership(db, &pool.id, &wallet.id)?.is_some();
    }
    let mut claims = db.prepare("SELECT slot,enabled,generation,wallet_id,target,maximum_output FROM reserve_claims WHERE pool_id=?1 ORDER BY CASE slot WHEN 'small' THEN 0 ELSE 1 END")?;
    pool.reserve_claims = claims
        .query_map([&pool.id], |r| {
            Ok(ReserveClaimStatus {
                slot: r.get(0)?,
                enabled: r.get(1)?,
                generation: r.get(2)?,
                wallet_id: r.get(3)?,
                target_atomic: r.get(4)?,
                maximum_output_atomic: r.get(5)?,
                destination_ready: false,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    for claim in &mut pool.reserve_claims {
        if let Some(wallet) = &claim.wallet_id {
            claim.destination_ready = reported_readiness(
                capacity::inventory_ownership(db, &pool.id, wallet).map(|owner| {
                    owner.is_some_and(|ownership| {
                        ownership.role == "READY"
                            && ownership.slot.as_deref() == Some(&claim.slot)
                            && ownership.claim_generation == Some(claim.generation)
                    })
                }),
            )?;
        }
    }
    pool.inventory_ready = pool.enabled
        && pool.reserve_claims.iter().any(|c| c.enabled)
        && pool
            .reserve_claims
            .iter()
            .filter(|c| c.enabled)
            .all(|c| c.destination_ready);
    Ok(())
}

// Malformed archival proof means incomplete inventory, while shared store
// failures remain errors. This does not change current payment eligibility.
fn reported_readiness(result: Result<bool>) -> Result<bool> {
    match result {
        Err(error)
            if error.downcast_ref::<super::super::base::evidence::EvidenceFailure>()
                == Some(&super::super::base::evidence::EvidenceFailure::MalformedSupport) =>
        {
            tracing::warn!(
                category = "malformed_durable_evidence_support",
                "Historical destination readiness is malformed; inventory completion is unavailable"
            );
            Ok(false)
        }
        other => other,
    }
}

#[cfg(test)]
#[test]
fn readiness_reporting_preserves_shared_store_failures() {
    let db = Connection::open_in_memory().unwrap();
    let failure = funding::readiness_recorded_valid(&db, "wallet");
    let error = reported_readiness(failure).unwrap_err();
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
}

pub(super) fn read_operations(db: &Connection, version: u32) -> Result<Vec<OperationStatus>> {
    if version >= 3 {
        let mut stmt =
            db.prepare("SELECT id,facts,submission,attempts FROM treasury_operations ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        rows.map(|row| -> Result<OperationStatus> {
            let (operation_id, facts, mut submission, attempts) = row?;
            if version >= 8
                && db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM expired_operations WHERE id=?1)",
                    [&operation_id],
                    |r| r.get::<_, bool>(0),
                )?
            {
                submission = "EXPIRED".into();
            }
            Ok(OperationStatus {
                operation_id,
                facts: serde_json::from_str(&facts)?,
                submission,
                attempts,
            })
        })
        .collect::<Result<Vec<_>>>()
    } else {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn retired(confirmed: Option<&str>, latest: &str) -> AddressStatus {
        AddressStatus {
            id: "fixture".into(),
            address: "fixture".into(),
            role: "RETIRED".into(),
            target: "1".into(),
            confirmed_balance: "999".into(),
            confirmed_height: None,
            confirmed_hash: None,
            signed_exposure_atomic: Some("0".into()),
            financial_repair: None,
            destination_recorded: true,
            destination_disputed: None,
            runtime_balance_evidence: None,
            retirement: confirmed.map(|confirmed| RetirementStatus {
                confirmed_amount: confirmed.into(),
                latest_amount: latest.into(),
                accepted_maximum: "99".into(),
                observed_bound: latest.into(),
                confirmed_height: 10,
                confirmed_hash: "confirmed".into(),
                latest_height: 11,
                latest_hash: "latest".into(),
                disputed: None,
                recorded_at: 1,
            }),
        }
    }
    #[test]
    fn retirement_totals_count_only_recorded_confirmed_amounts_and_keep_unknowns() {
        let wallets = [
            retired(Some("3"), "7"),
            retired(Some("5"), "9"),
            retired(None, "0"),
        ];
        let totals = RetirementAccounting::from_wallets(&wallets, 16).unwrap();
        assert_eq!(
            totals.automatic_recorded_confirmed_atomic.as_deref(),
            Some("8")
        );
        assert_eq!(totals.automatic_count, Some(2));
        assert_eq!(totals.manual_count, Some(0));
        assert_eq!(
            totals.manual_recorded_confirmed_atomic.as_deref(),
            Some("0")
        );
        assert_eq!(totals.historical_unknown_count, 1);
        let historical = RetirementAccounting::from_wallets(&[retired(None, "0")], 12).unwrap();
        assert!(historical.automatic_recorded_confirmed_atomic.is_none());
        assert!(historical.automatic_count.is_none());
        assert!(historical.manual_recorded_confirmed_atomic.is_none());
        assert!(historical.manual_count.is_none());
        assert_eq!(historical.historical_unknown_count, 1);
        assert!(
            RetirementAccounting::from_wallets(
                &[
                    retired(Some(&U256::MAX.to_string()), "0"),
                    retired(Some("1"), "1")
                ],
                16
            )
            .is_err()
        );
    }
}

#[cfg(test)]
#[path = "reporting_tests.rs"]
mod status_tests;
