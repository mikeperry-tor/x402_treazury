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
            automatic_recorded_confirmed_atomic: (schema == 16).then(|| total.to_string()),
            automatic_count: (schema == 16).then_some(automatic),
            manual_recorded_confirmed_atomic: (schema == 16).then(|| "0".into()),
            manual_count: (schema == 16).then_some(0),
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
        let fresh = checked_at <= observation.valid_until;
        let (compatible, unavailable_reason) =
            match evidence::check_balance(&store.db, store.evidence_namespace, certificate) {
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
