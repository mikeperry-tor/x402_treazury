//! Native payouts and executable swaps share the treasury's original source owner.
use super::*;
use crate::rotation::store::direct_swap::{
    DirectSendAction, DirectSendAuthority, DirectSourceCondition,
};
use crate::treasury::{FeePreview, FeePreviewUnavailable, FeeTarget};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Destination {
    asset_id: String,
    chain: String,
    amount: String,
    recipient: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Send {
    asset_id: String,
    chain: String,
    amount: String,
    recipient: String,
    request_id: String,
}

impl Service {
    async fn validate_destination(&self, destination: &Destination) -> Result<bool> {
        ensure!(
            route::atomic(&destination.amount)? > alloy_primitives::U256::ZERO,
            "amount must be positive"
        );
        ensure!(
            !destination.recipient.trim().is_empty(),
            "recipient is required"
        );
        let native = destination.asset_id == route::ZEC;
        if native {
            ensure!(destination.chain == "zec", "native ZEC requires chain zec");
            native_amount(&destination.amount)?;
            let chain = self.store.call(|s| Ok(s.network().chain())).await?;
            crate::treasury::native_recipient(&chain, &destination.recipient)?;
        } else {
            self.near()?;
        }
        Ok(native)
    }
    pub(super) async fn preview(
        &self,
        owner: &str,
        config: &Config,
        destination: Destination,
    ) -> Result<Value> {
        let native = self.validate_destination(&destination).await?;
        let cap = config.max_debit_zatoshis()?;
        if native {
            let principal = native_amount(&destination.amount)?;
            let estimate = self
                .fee_estimate(
                    FeeTarget::Native(destination.recipient.clone()),
                    principal,
                    cap,
                )
                .await?;
            return Ok(
                json!({"executable":false,"route":"native","asset_id":route::ZEC,"chain":"zec","amount_zatoshis":principal.to_string(),"recipient":destination.recipient,"fee_estimate":estimate,"max_total_debit_zatoshis":cap.map(|value|value.to_string())}),
            );
        }
        let refund = self.backend()?.treasury.preview_address().await?;
        let identity = self.identity(&format!("preview:{owner}")).await?;
        let quote = self
            .output_quote(&destination, &refund, &identity, true)
            .await?;
        let principal = native_amount(&quote.input)?;
        let estimate = self
            .fee_estimate(FeeTarget::NearDeposit, principal, cap)
            .await?;
        Ok(
            json!({"executable":false,"route":"near","quote":quote,"fee_estimate":estimate,"max_total_debit_zatoshis":cap.map(|value|value.to_string())}),
        )
    }
    async fn fee_estimate(
        &self,
        target: FeeTarget,
        principal: u64,
        cap: Option<u64>,
    ) -> Result<Value> {
        let backend = self.backend()?;
        let basis = match &target {
            FeeTarget::Native(_) => "native_transaction",
            FeeTarget::NearDeposit => "near_deposit_output_shape",
        };
        let fee = backend.treasury.preview_fee(target, principal).await;
        Ok(advisory_fee(fee, principal, cap, basis))
    }
    pub(super) async fn send(&self, owner: &str, config: &Config, request: Send) -> Result<Value> {
        ensure!(
            !request.request_id.trim().is_empty(),
            "request_id is required"
        );
        let destination = Destination {
            asset_id: request.asset_id.clone(),
            chain: request.chain.clone(),
            amount: request.amount.clone(),
            recipient: request.recipient.clone(),
        };
        let native = self.validate_destination(&destination).await?;
        let owner_owned = owner.to_owned();
        let request_id = request.request_id.clone();
        let terms = serde_json::to_value(&request)?;
        let (operation, created) = self
            .store
            .call(move |s| {
                if native {
                    s.reserve_native(&owner_owned, "send", &request_id, terms)
                } else {
                    s.reserve_direct(&owner_owned, "send", &request_id, terms)
                }
            })
            .await?;
        if operation.authority.is_some() || (!native && !created) {
            return self.status(owner, &operation.id).await;
        }
        let maximum = config.max_debit_zatoshis()?;
        ensure!(maximum != Some(0), "direct send total-debit limit is zero");
        let quote = if native {
            let principal = native_amount(&request.amount)?;
            ensure!(
                maximum.is_none_or(|limit| principal <= limit),
                "native principal exceeds direct send total-debit limit"
            );
            None
        } else {
            let refund = self
                .backend()?
                .treasury
                .direct_address(operation.id.clone())
                .await?;
            let identity = self.identity(&operation.id).await?;
            Some(serde_json::to_value(
                self.output_quote(&destination, &refund, &identity, false)
                    .await?,
            )?)
        };
        let authority = DirectSendAuthority {
            max_debit_zatoshis: maximum,
        };
        let id = operation.id.clone();
        self.store
            .call(move |s| s.accept_direct_send(&id, quote, authority))
            .await?;
        self.status(owner, &operation.id).await
    }
    async fn output_quote(
        &self,
        destination: &Destination,
        refund: &str,
        identity: &crate::network::IsolationId,
        dry: bool,
    ) -> Result<route::DirectQuote> {
        let near = self.near()?;
        let assets = route::assets(&near.client, identity).await?;
        let origin = assets
            .iter()
            .find(|asset| asset.asset_id == route::ZEC)
            .context("ZEC asset unavailable")?;
        let target = route::select_asset(&assets, &destination.asset_id, &destination.chain)?;
        let now = crate::rotation::base::now()?;
        let deadline = now
            .checked_add(near.funding.quote_deadline_seconds)
            .context("invalid quote deadline")?;
        let mut request = route::request_on(
            &near.client.chain(),
            origin,
            target,
            &destination.recipient,
            refund,
            &destination.amount,
            route::SwapType::ExactOutput,
            near.funding.slippage_bps,
            deadline,
            dry,
        )?;
        request["confidentiality"] = json!(near.funding.confidentiality);
        route::quote(&near.client, identity, request, now).await
    }
    pub(super) async fn run_operation(&self, operation: DirectOperation, stop: CancellationToken) {
        if operation.kind == "send" {
            let provider = async {
                if operation.route == DirectRoute::Near
                    && self
                        .backend
                        .as_ref()
                        .is_some_and(|backend| backend.near.is_some())
                {
                    self.observe(operation.clone(), stop.clone()).await;
                }
            };
            tokio::join!(provider, self.progress_source(&operation, stop.clone()));
        } else {
            self.observe(operation, stop).await;
        }
    }
    async fn progress_source(&self, operation: &DirectOperation, stop: CancellationToken) {
        let Some(backend) = &self.backend else {
            return;
        };
        let jitter = operation.id.bytes().map(u64::from).sum::<u64>()
            % backend.poll_seconds.saturating_add(1);
        let mut streak = 0u32;
        loop {
            if stop.is_cancelled() {
                return;
            }
            let action = self.source_action(&operation.id).await;
            let result = match action {
                Ok(DirectSendAction::NoAutomaticAction) => return,
                Ok(DirectSendAction::Prepare(request)) => {
                    backend.treasury.prepare(request).await.map(|_| ())
                }
                Ok(DirectSendAction::Submit) => backend
                    .treasury
                    .submit(operation.id.clone(), false)
                    .await
                    .map(|_| ()),
                Err(error) => Err(error),
            };
            match result {
                Ok(()) => {
                    streak = 0;
                    self.source_diagnostic(&operation.id, None).await;
                    // Local preparation/publication/submission handoffs do not acquire a polling delay.
                    continue;
                }
                Err(error) => {
                    if matches!(
                        self.source_action(&operation.id).await,
                        Ok(DirectSendAction::NoAutomaticAction)
                    ) {
                        return;
                    }
                    let condition =
                        match error.downcast_ref::<crate::rotation::error::FundingCondition>() {
                            Some(
                                crate::rotation::error::FundingCondition::CostLimit
                                | crate::rotation::error::FundingCondition::AmountLimit,
                            ) => DirectSourceCondition::CostLimit,
                            Some(crate::rotation::error::FundingCondition::InsufficientFunds) => {
                                DirectSourceCondition::InsufficientFunds
                            }
                            _ => DirectSourceCondition::Unavailable,
                        };
                    self.source_diagnostic(&operation.id, Some(condition)).await;
                    tracing::debug!(operation_id=%operation.id,category=%crate::rotation::base::safe_diagnostic(&error),"Direct payment source work is pending; original terms and transaction ownership are retained");
                    streak = streak.saturating_add(1);
                    let delay = backend
                        .poll_seconds
                        .saturating_mul(1u64 << streak.min(6))
                        .clamp(60, 300)
                        .saturating_add(jitter);
                    tokio::select! {_=stop.cancelled()=>return,_=tokio::time::sleep(Duration::from_secs(delay))=>{}}
                }
            }
        }
    }
    async fn source_diagnostic(&self, id: &str, condition: Option<DirectSourceCondition>) {
        let id = id.to_owned();
        if self
            .store
            .call(move |s| s.direct_source_error(&id, condition))
            .await
            .is_err()
        {
            tracing::debug!(
                "Direct payment diagnostic could not be saved; source ownership and progress are unchanged"
            );
        }
    }
    pub(super) async fn source_action(&self, id: &str) -> Result<DirectSendAction> {
        let id = id.to_owned();
        let now = crate::rotation::base::now().ok();
        self.store
            .call(move |s| s.direct_send_action(&id, now))
            .await
    }
}

// Wallet state and quote prices may change before execution; previews reserve nothing.
fn advisory_fee(
    observation: Result<FeePreview>,
    principal: u64,
    cap: Option<u64>,
    basis: &str,
) -> Value {
    let observation = observation.unwrap_or_else(|error| FeePreview {
        spendable_zatoshis: None,
        fee: Err(error
            .downcast_ref::<FeePreviewUnavailable>()
            .copied()
            .unwrap_or(FeePreviewUnavailable::Unavailable)),
    });
    let available = observation
        .spendable_zatoshis
        .map(|balance| cap.map_or(balance, |cap| cap.min(balance)));
    let fee = observation.fee.as_ref().ok().copied();
    let reason = observation.fee.err();
    let total = fee.and_then(|fee| principal.checked_add(fee));
    let known_debit = total.unwrap_or(principal);
    let fits = if reason == Some(FeePreviewUnavailable::InsufficientFunds)
        || cap.is_some_and(|limit| known_debit > limit)
        || available.is_some_and(|limit| known_debit > limit)
    {
        Some(false)
    } else {
        total
            .zip(available)
            .map(|(total, available)| total <= available)
    };
    json!({"advisory":true,"basis":basis,"network_fee_zatoshis":fee.map(|v|v.to_string()),
        "total_debit_zatoshis":total.map(|v|v.to_string()),"unavailable_reason":reason.map(|v|v.to_string()),
        "spendable_zatoshis":observation.spendable_zatoshis.map(|v|v.to_string()),
        "available_debit_zatoshis":available.map(|v|v.to_string()),"fits":fits})
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    fn observation(
        balance: Option<u64>,
        fee: std::result::Result<u64, FeePreviewUnavailable>,
    ) -> Result<FeePreview> {
        Ok(FeePreview {
            spendable_zatoshis: balance,
            fee,
        })
    }
    #[test]
    fn preview_fits_uses_sdk_proposal_and_smaller_balance_or_operator_limit() {
        for (balance, cap, fits, available) in [
            (200_000, Some(110_000), false, 110_000),
            (110_000, Some(200_000), false, 110_000),
            (115_000, None, true, 115_000),
            (200_000, Some(115_000), true, 115_000),
        ] {
            let result = advisory_fee(
                observation(Some(balance), Ok(15_000)),
                100_000,
                cap,
                "native_transaction",
            );
            assert_eq!(result["total_debit_zatoshis"], "115000");
            assert_eq!(result["available_debit_zatoshis"], available.to_string());
            assert_eq!(result["fits"], fits);
        }
        let locked = advisory_fee(
            observation(Some(200_000), Err(FeePreviewUnavailable::InsufficientFunds)),
            100_000,
            None,
            "near_deposit_output_shape",
        );
        assert_eq!(
            locked["fits"], false,
            "proposal eligibility overrides advisory balance"
        );
        let unknown = advisory_fee(
            observation(None, Ok(15_000)),
            100_000,
            None,
            "native_transaction",
        );
        assert!(unknown["fits"].is_null());
        let over_cap = advisory_fee(
            observation(None, Ok(15_000)),
            100_000,
            Some(110_000),
            "native_transaction",
        );
        assert_eq!(
            over_cap["fits"], false,
            "known total exceeds operator authority even without balance data"
        );
        let missing = advisory_fee(
            observation(Some(200_000), Err(FeePreviewUnavailable::ScanRequired)),
            100_000,
            None,
            "native_transaction",
        );
        assert!(missing["fits"].is_null());
        assert_eq!(missing["unavailable_reason"], "scan_required");
        let unexpected = advisory_fee(
            Err(anyhow::anyhow!("private upstream diagnostic")),
            100_000,
            None,
            "native_transaction",
        );
        assert_eq!(unexpected["unavailable_reason"], "unavailable");
        assert!(!unexpected.to_string().contains("private"));
    }
}
