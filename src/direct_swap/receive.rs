//! Receive requests and display-only payer instructions share the original saved terms.
use super::*;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receive {
    asset_id: String,
    chain: String,
    amount: String,
    refund_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refund_after_seconds: Option<u64>,
    request_id: String,
}

impl Service {
    pub(super) async fn receive(&self, owner: &str, args: Receive) -> Result<Value> {
        ensure!(!args.request_id.trim().is_empty(), "request_id is required");
        ensure!(
            route::atomic(&args.amount)? > alloy_primitives::U256::ZERO,
            "amount must be positive"
        );
        let native = args.asset_id == route::ZEC;
        if native {
            ensure!(args.chain == "zec", "native ZEC requires chain zec");
            native_amount(&args.amount)?;
            ensure!(
                args.refund_address.is_none(),
                "native ZEC does not use a refund_address"
            );
        } else {
            self.near()?;
            ensure!(
                args.refund_address
                    .as_deref()
                    .is_some_and(|a| !a.trim().is_empty()),
                "foreign payment requests require a payer-owned refund_address"
            );
        }
        if native {
            ensure!(
                args.refund_after_seconds.is_none(),
                "native ZEC does not use refund_after_seconds"
            );
        }
        let after = args.refund_after_seconds;
        let terms = serde_json::to_value(&args)?;
        let owner_owned = owner.to_owned();
        let request = args.request_id.clone();
        let (operation, created) = self
            .store
            .call(move |s| {
                if native {
                    s.reserve_native(&owner_owned, "receive", &request, terms)
                } else {
                    s.reserve_direct_checked(&owner_owned, "receive", &request, terms, || {
                        refund_deadline(crate::rotation::base::now()?, after).map(|_| ())
                    })
                }
            })
            .await?;
        if native {
            // A native reservation has no uncertain remote creation: finish its original derivation.
            if operation.address.is_none() {
                self.backend()?
                    .treasury
                    .direct_address(operation.id.clone())
                    .await?;
            }
            return self.status(owner, &operation.id).await;
        }
        if !created {
            return self.status(owner, &operation.id).await;
        }
        self.receive_swap(owner, operation, args).await
    }
    async fn receive_swap(
        &self,
        owner: &str,
        operation: DirectOperation,
        args: Receive,
    ) -> Result<Value> {
        let near = self.near()?;
        // Any interruption retains this reservation; an uncertain provider quote is never replaced.
        let address = self
            .backend()?
            .treasury
            .direct_address(operation.id.clone())
            .await?;
        let identity = self.identity(&operation.id).await?;
        let assets = route::assets(&near.client, &identity).await?;
        let origin = route::select_asset(&assets, &args.asset_id, &args.chain)?;
        let destination = assets
            .iter()
            .find(|a| a.asset_id == route::ZEC)
            .context("ZEC asset unavailable")?;
        let now = crate::rotation::base::now()?;
        let deadline = refund_deadline(now, args.refund_after_seconds)?;
        let mut request = route::request_on(
            &near.client.chain(),
            origin,
            destination,
            &address,
            args.refund_address
                .as_deref()
                .context("missing refund address")?,
            &args.amount,
            route::SwapType::ExactInput,
            near.funding.slippage_bps,
            deadline,
            false,
        )?;
        request["confidentiality"] = json!(near.funding.confidentiality);
        let quote = route::quote(&near.client, &identity, request, now).await?;
        let id = operation.id.clone();
        self.store
            .call(move |s| s.save_direct_quote(&id, serde_json::to_value(quote)?))
            .await?;
        self.status(owner, &operation.id).await
    }
}

/// A caller-relative refund preference, independently of managed/send quote windows.
pub(super) fn refund_deadline(now: u64, after: Option<u64>) -> Result<u64> {
    let after = after.unwrap_or(7200);
    ensure!(after > 0, "refund_after_seconds must be positive");
    let deadline = now
        .checked_add(after)
        .context("invalid receive refund time")?;
    utc(deadline).context("receive refund time cannot be represented")?;
    Ok(deadline)
}

pub(super) fn utc(seconds: u64) -> Option<String> {
    chrono::DateTime::from_timestamp(i64::try_from(seconds).ok()?, 0)
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

fn timestamp(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        u64::try_from(
            chrono::DateTime::parse_from_rfc3339(value.as_str()?)
                .ok()?
                .timestamp(),
        )
        .ok()
    })
}

fn timing(value: &Value, now: Option<u64>) -> (Option<String>, Option<u64>) {
    let Some(seconds) = timestamp(value) else {
        return (None, None);
    };
    let Some(display) = utc(seconds) else {
        return (None, None);
    };
    (Some(display), now.map(|now| seconds.saturating_sub(now)))
}

/// Presentation only: countdowns never change saved terms, settlement or replay authority.
pub(super) fn instructions(operation: &DirectOperation, now: Option<u64>) -> Value {
    if operation.kind != "receive" {
        return Value::Null;
    }
    if operation.route == DirectRoute::Native {
        return json!({
            "asset_id":route::ZEC,"chain":"zec","amount_zatoshis":operation.terms["amount"],
            "recipient":operation.address,"current_time_utc":now.and_then(utc),
            "exact_amount_required":false,"partial_payments_accepted":true,"address_reusable":true,
            "requested_refund_start_utc":null,"seconds_until_requested_refund_start":null,
            "deposit_address_expires_at_utc":null,"seconds_until_deposit_address_expiry":null,
            "deposit_address_cold_at_utc":null,"seconds_until_deposit_address_cold":null,
            "payer_instruction":"Send native ZEC to this address. Partial payments accumulate toward the requested amount; this request has no provider expiry and its native ZEC address may be reused."
        });
    }
    let Some(quote) = &operation.quote else {
        return Value::Null;
    };
    if quote["deposit"]
        .as_str()
        .is_none_or(|value| value.trim().is_empty())
        || operation.terms["amount"]
            .as_str()
            .is_none_or(|value| value.trim().is_empty())
    {
        return Value::Null;
    }
    let (refund, until_refund) = timing(&quote["request"]["deadline"], now);
    let (expires, until_expiry) = timing(&quote["deadline"], now);
    let (cold, until_cold) = timing(&quote["response"]["quote"]["timeWhenInactive"], now);
    json!({
        "asset_id":operation.terms["asset_id"],"chain":operation.terms["chain"],
        "amount_atomic":operation.terms["amount"],"recipient":quote["deposit"],
        "deposit_memo":quote["deposit_memo"],"current_time_utc":now.and_then(utc),
        "exact_amount_required":true,"partial_payments_accepted":false,"address_reusable":false,
        "network_fees_paid_separately":true,
        "requested_refund_start_utc":refund,"seconds_until_requested_refund_start":until_refund,
        "deposit_address_expires_at_utc":expires,"seconds_until_deposit_address_expiry":until_expiry,
        "deposit_address_cold_at_utc":cold,"seconds_until_deposit_address_cold":until_cold,
        "payer_instruction":format!("This request is for one deposit of exactly {} atomic units of {} on {} to recipient, with network fees paid separately so they do not reduce the deposit and deposit_memo included when present. Never reuse this swap receive address. Another payment requires a new request_id and fresh quote/address; an identical retry only retrieves this original request. A different deposited amount can fail or be refunded. Refund-start time does not guarantee refund completion.",
            operation.terms["amount"].as_str().unwrap_or("the stated amount"),
            operation.terms["asset_id"].as_str().unwrap_or("the source asset"),
            operation.terms["chain"].as_str().unwrap_or("the source chain"))
    })
}
