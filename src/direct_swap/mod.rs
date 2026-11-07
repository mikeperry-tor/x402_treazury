//! First-party deposit-funded swaps. Provider settlement and SDK funds are separate facts.
pub(crate) mod route;
mod send;
#[cfg(test)]
mod tests;

use crate::rotation::{
    config::FundingConfig,
    near::NearClient,
    store::{DirectRoute, StoreHandle, direct_swap::DirectOperation},
};
use crate::treasury::actor::TreasuryHandle;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub receive: bool,
    #[serde(default)]
    pub send: bool,
    #[serde(default)]
    pub max_send_zec: Option<String>,
}
impl Config {
    pub(crate) fn max_debit_zatoshis(&self) -> Result<Option<u64>> {
        match self.max_send_zec.as_deref() {
            None | Some("none") => Ok(None),
            Some(value) => Ok(Some(u64::try_from(
                crate::rotation::config::zatoshis_allow_zero(value)?,
            )?)),
        }
    }
    pub(crate) fn validate(&self) -> Result<()> {
        self.max_debit_zatoshis().map(|_| ())
    }
    pub fn enabled(&self) -> bool {
        self.receive || self.send
    }
}

#[derive(Clone)]
pub struct Service {
    store: StoreHandle,
    backend: Option<Arc<Backend>>,
    wake: Arc<tokio::sync::Notify>,
}

#[derive(Clone)]
struct Backend {
    treasury: TreasuryHandle,
    near: Option<NearSource>,
    poll_seconds: u64,
    max_fee: u64,
}
#[derive(Clone)]
struct NearSource {
    client: NearClient,
    funding: FundingConfig,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receive {
    asset_id: String,
    chain: String,
    amount: String,
    refund_address: Option<String>,
    request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Status {
    operation_id: Option<String>,
    request_id: Option<String>,
    kind: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Assets {
    query: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
}

pub fn definitions(config: &Config) -> Vec<rmcp::model::Tool> {
    let text = |description: &str| json!({"type":"string","minLength":1,"description":description});
    let mut specs = vec![(
        "x402_treazury_swap_status",
        "Inspect your saved native Zcash payment or swap and its delivery/receipt evidence. Recover a lost response using kind and original request_id. Creates no payment.",
        json!({"operation_id":text("Saved operation ID; omit when using kind and request_id."),"kind":{"type":"string","enum":["receive","send"],"description":"Operation kind when looking up request_id."},"request_id":text("Original caller request ID; requires kind and no operation_id.")}),
        vec![],
    )];
    if config.enabled() {
        specs.push(("x402_treazury_swap_assets", "Search native Ironwood Zcash (ZEC) and supported foreign assets by symbol, chain, asset ID or available names. Case-insensitive literal matches rank before BM25 keyword matches; token support does not guarantee liquidity.", json!({"query":text("Symbol, chain, asset ID or available name keywords; separate words with spaces."),"offset":{"type":"integer","minimum":0,"description":"Result offset; keep the same query."},"limit":{"type":"integer","minimum":1,"default":25,"description":"Number of assets to return."}}), vec![]));
    }
    if config.receive {
        specs.push(("x402_treazury_payment_request_create", "Receive native Ironwood Zcash (ZEC) or foreign assets into this treasury. Native ZEC gets a fresh Ironwood address per new request_id with no NEAR call. Foreign assets use one exact-input swap; ZEC value is estimated until settlement. Identical retries return the original request.", json!({"asset_id":text("Full source assetId from swap_assets."),"chain":text("Exact source blockchain from swap_assets."),"amount":text("Positive atomic amount as decimal digits; native ZEC uses zatoshis."),"refund_address":text("Required for foreign assets: payer-owned source-chain refund address. Omit for native ZEC; destination tags are unsupported."),"request_id":text("Unique caller request ID for these exact terms.")}), vec!["asset_id","chain","amount","request_id"]));
    }
    if config.send {
        specs.push(("x402_treazury_swap_preview", "Preview a native Ironwood Zcash (ZEC) payment or an exact-output foreign swap. Creates no payment authority. Inspect configured total-debit and network-fee limits, quoted principal and advisory SDK fees. Foreign fees estimate a standard NEAR deposit output; actual fees are checked during preparation.", json!({"asset_id":text("Full destination assetId from swap_assets."),"chain":text("Exact destination blockchain from swap_assets."),"amount":text("Positive destination atomic amount; native ZEC uses zatoshis."),"recipient":text("Destination address; native ZEC requires an Ironwood-capable unified address without a transparent receiver. Memos and destination tags are unsupported.")}), vec!["asset_id","chain","amount","recipient"]));
        specs.push(("x402_treazury_swap_send", "Send native Ironwood Zcash or swap treasury ZEC for an exact foreign amount. Accepted work continues after disconnect. Identical request_id retries return the original operation; inspect swap_status for settlement.", json!({"asset_id":text("Full destination assetId from swap_assets."),"chain":text("Exact destination blockchain from swap_assets."),"amount":text("Positive destination atomic amount; native ZEC uses zatoshis."),"recipient":text("Destination address; native ZEC requires an Ironwood-capable unified address without a transparent receiver. Memos and destination tags are unsupported."),"max_source_zatoshis":text("Required for foreign swaps: maximum ZEC principal in zatoshis. Optional for native ZEC. Operator total-debit limit also includes network fee."),"request_id":text("Unique caller request ID for these exact terms; reuse only for identical retries.")}), vec!["asset_id","chain","amount","recipient","request_id"]));
    }
    specs.into_iter().map(|(name,description,properties,required)| rmcp::model::Tool::new(name,description,Arc::new(json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}).as_object().unwrap().clone()))).collect()
}

impl Service {
    pub fn new(
        store: StoreHandle,
        treasury: TreasuryHandle,
        near: NearClient,
        funding: FundingConfig,
        max_fee: u64,
    ) -> Self {
        let wake = store.direct_changes();
        Self {
            store,
            backend: Some(Arc::new(Backend {
                treasury,
                max_fee,
                poll_seconds: funding.poll_seconds,
                near: Some(NearSource {
                    client: near,
                    funding,
                }),
            })),
            wake,
        }
    }
    pub fn native(
        store: StoreHandle,
        treasury: TreasuryHandle,
        max_fee: u64,
        poll_seconds: u64,
    ) -> Self {
        let wake = store.direct_changes();
        Self {
            store,
            backend: Some(Arc::new(Backend {
                treasury,
                max_fee,
                near: None,
                poll_seconds,
            })),
            wake,
        }
    }
    pub fn status_only(store: StoreHandle) -> Self {
        let wake = store.direct_changes();
        Self {
            store,
            backend: None,
            wake,
        }
    }
    fn backend(&self) -> Result<&Backend> {
        self.backend.as_deref().context("Direct payment admission unavailable; treasury SDK requires correction and restart. Saved status remains available")
    }
    fn near(&self) -> Result<&NearSource> {
        self.backend()?.near.as_ref().context("Foreign swaps unavailable; correct NEAR configuration and restart. Native ZEC remains available")
    }
    async fn identity(&self, id: &str) -> Result<crate::network::IsolationId> {
        let treasury = self.store.call(|s| Ok(s.id().to_owned())).await?;
        Ok(crate::network::IsolationId::direct_swap(&treasury, id))
    }
    pub async fn invoke(
        &self,
        owner: &str,
        config: &Config,
        name: &str,
        args: Value,
    ) -> Result<Value> {
        ensure!(
            definitions(config).iter().any(|tool| tool.name == name),
            "swap capability disabled or unknown tool"
        );
        if name != "x402_treazury_swap_status" {
            self.backend()?;
        }
        match name {
            "x402_treazury_swap_assets" => {
                let args: Assets = serde_json::from_value(args)?;
                let mut assets = vec![native_asset()];
                let mut foreign_available = false;
                if let Some(near) = &self.backend()?.near {
                    let identity = self.identity("assets").await?;
                    if let Ok(remote) = route::assets(&near.client, &identity).await {
                        assets.extend(
                            remote
                                .into_iter()
                                .filter(|asset| asset.asset_id != route::ZEC),
                        );
                        foreign_available = true;
                    }
                }
                let assets = route::rank_assets(assets, args.query.as_deref().unwrap_or_default());
                let offset = args.offset.unwrap_or(0);
                let limit = args.limit.unwrap_or(25);
                ensure!(limit > 0 && offset <= assets.len(), "invalid asset page");
                let end = offset.saturating_add(limit).min(assets.len());
                Ok(
                    json!({"assets":&assets[offset..end],"next_offset":(end<assets.len()).then_some(end),"total":assets.len(),"complete":foreign_available,"foreign_assets_available":foreign_available}),
                )
            }
            "x402_treazury_payment_request_create" => {
                self.receive(owner, serde_json::from_value(args)?).await
            }
            "x402_treazury_swap_preview" => {
                self.preview(owner, config, serde_json::from_value(args)?)
                    .await
            }
            "x402_treazury_swap_send" => {
                self.send(owner, config, serde_json::from_value(args)?)
                    .await
            }
            "x402_treazury_swap_status" => {
                let args: Status = serde_json::from_value(args)?;
                self.status_reference(owner, args).await
            }
            _ => anyhow::bail!("unknown swap tool"),
        }
    }
    async fn receive(&self, owner: &str, args: Receive) -> Result<Value> {
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
        let terms = serde_json::to_value(&args)?;
        let owner_owned = owner.to_owned();
        let request = args.request_id.clone();
        let (operation, created) = self
            .store
            .call(move |s| {
                if native {
                    s.reserve_native(&owner_owned, "receive", &request, terms)
                } else {
                    s.reserve_direct(&owner_owned, "receive", &request, terms)
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
        let deadline = now
            .checked_add(near.funding.quote_deadline_seconds)
            .context("invalid quote deadline")?;
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
    async fn status_reference(&self, owner: &str, reference: Status) -> Result<Value> {
        match (reference.operation_id, reference.kind, reference.request_id) {
            (Some(id), None, None) => self.status(owner, &id).await,
            (None, Some(kind), Some(request_id)) if matches!(kind.as_str(), "receive" | "send") => {
                let owner_owned = owner.to_owned();
                let operation = self
                    .store
                    .call(move |s| s.direct_request(&owner_owned, &kind, &request_id))
                    .await?;
                self.status(owner, &operation.id).await
            }
            _ => anyhow::bail!("provide operation_id or kind plus request_id"),
        }
    }
    async fn status(&self, owner: &str, id: &str) -> Result<Value> {
        let owner_owned = owner.to_owned();
        let id_owned = id.to_owned();
        let operation = self
            .store
            .call(move |s| s.direct_operation(&owner_owned, &id_owned))
            .await?;
        let native_receive = operation.route == DirectRoute::Native && operation.kind == "receive";
        let refresh_unavailable = if native_receive && self.backend.is_some() {
            if let Err(error) = self.refresh_native(&operation).await {
                tracing::debug!(operation_id=%operation.id,category=%crate::rotation::base::safe_diagnostic(&error),"Native receipt refresh unavailable; returning saved snapshot evidence");
                true
            } else {
                false
            }
        } else {
            native_receive && self.backend.is_none()
        };
        let owner = owner.to_owned();
        let id = id.to_owned();
        self.store.call(move |s| {
            let operation = s.direct_operation(&owner,&id)?;
            let saved = s.treasury_sync_state()?;
            let receipt = if operation.route == DirectRoute::Native { operation.status.as_ref().map(|s| s["receipt"].clone()).unwrap_or(Value::Null) } else { json!({"attribution":"unknown"}) };
            let matches_snapshot = receipt["snapshot_revision"].as_i64() == Some(saved.snapshot_revision);
            let instructions = native_receive.then(|| json!({"asset_id":route::ZEC,"chain":"zec","amount_zatoshis":operation.terms["amount"],"recipient":operation.address}));
            let source = if operation.kind == "send" { s.direct_source_status(&id).unwrap_or_else(|_| json!({"unavailable":true})) } else { Value::Null };
            Ok(json!({"source":source,"operation":operation,"zec_receipt":receipt,"receipt_matches_saved_snapshot":native_receive.then_some(matches_snapshot),"receipt_refresh_unavailable":native_receive.then_some(refresh_unavailable && !matches_snapshot),"payment_instructions":instructions,"treasury_observation":saved.sync}))
        }).await
    }
    async fn refresh_native(&self, operation: &DirectOperation) -> Result<DirectOperation> {
        let owner = operation.owner.clone();
        let id = operation.id.clone();
        let (saved, revision) = self
            .store
            .call(move |s| {
                Ok((
                    s.direct_operation(&owner, &id)?,
                    s.treasury_sync_state()?.snapshot_revision,
                ))
            })
            .await?;
        ensure!(
            saved.route == DirectRoute::Native
                && saved.kind == "receive"
                && saved.address == operation.address
                && saved.terms == operation.terms,
            "native receive binding changed"
        );
        if saved
            .status
            .as_ref()
            .and_then(|s| s["receipt"]["snapshot_revision"].as_i64())
            == Some(revision)
        {
            return Ok(saved);
        }
        let receipt = self
            .backend()?
            .treasury
            .native_receipt(operation.id.clone())
            .await?;
        let owner = operation.owner.clone();
        let id = operation.id.clone();
        self.store
            .call(move |s| {
                s.save_native_status(&id, receipt)?;
                s.direct_operation(&owner, &id)
            })
            .await
    }
    pub async fn run(&self, stop: CancellationToken) -> Result<()> {
        if self.backend.is_none() {
            stop.cancelled().await;
            return Ok(());
        }
        let mut active = HashSet::new();
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            let changed = self.wake.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let pending = self.store.call(|s| s.direct_pending()).await?;
            for operation in pending {
                if operation.route == DirectRoute::Near
                    && self.backend()?.near.is_none()
                    && (operation.kind != "send"
                        || matches!(
                            self.source_action(&operation.id).await,
                            Ok(crate::rotation::store::DirectSendAction::NoAutomaticAction)
                        ))
                {
                    continue;
                }
                if active.insert(operation.id.clone()) {
                    let service = self.clone();
                    let stopped = stop.clone();
                    tasks.spawn(async move {
                        let id = operation.id.clone();
                        service.run_operation(operation, stopped).await;
                        id
                    });
                }
            }
            tokio::select! {
                _ = stop.cancelled() => break,
                _ = &mut changed => {},
                joined = tasks.join_next(), if !tasks.is_empty() => {
                    match joined { Some(Ok(id)) => { active.remove(&id); }, Some(Err(_)) => anyhow::bail!("direct payment processing stopped; accepted work remains durable"), None => {} }
                }
            }
        }
        while tasks.join_next().await.is_some() {}
        Ok(())
    }
    async fn observe(&self, operation: DirectOperation, stop: CancellationToken) {
        let Some(backend) = &self.backend else {
            return;
        };
        let mut streak = 0u32;
        let jitter = operation.id.bytes().map(u64::from).sum::<u64>()
            % backend.poll_seconds.saturating_add(1);
        loop {
            let result = self.observe_once(&operation).await;
            let delay = match result {
                Ok(true) => return,
                Ok(false) => {
                    streak = 0;
                    backend.poll_seconds
                }
                Err(_) => {
                    tracing::debug!(operation_id=%operation.id,"Swap status observation unavailable; saved work remains pending");
                    streak = streak.saturating_add(1);
                    backend
                        .poll_seconds
                        .saturating_mul(1u64 << streak.min(6))
                        .clamp(60, 300)
                }
            };
            tokio::select! { _ = stop.cancelled() => return, _ = tokio::time::sleep(Duration::from_secs(delay.saturating_add(jitter))) => {} }
        }
    }
    async fn observe_once(&self, operation: &DirectOperation) -> Result<bool> {
        if operation.route == DirectRoute::Native {
            return Ok(self.refresh_native(operation).await?.state == "confirmed");
        }
        let quote: route::DirectQuote =
            serde_json::from_value(operation.quote.clone().context("missing saved quote")?)?;
        let identity = self.identity(&operation.id).await?;
        let status = route::status(&self.near()?.client, &identity, &quote).await?;
        let terminal = matches!(
            status["status"].as_str(),
            Some("SUCCESS" | "FAILED" | "REFUNDED")
        );
        let id = operation.id.clone();
        let status = json!({"provider":status,"observed_at":crate::rotation::base::now().ok()});
        self.store
            .call(move |s| s.save_direct_status(&id, status))
            .await?;
        Ok(terminal)
    }
}

fn native_asset() -> route::Asset {
    route::Asset {
        asset_id: route::ZEC.into(),
        blockchain: "zec".into(),
        decimals: 8,
        symbol: "ZEC".into(),
        metadata: serde_json::Map::from_iter([
            ("native".into(), json!(true)),
            ("name".into(), json!("Zcash")),
        ]),
    }
}
fn native_amount(amount: &str) -> Result<u64> {
    let value = amount
        .parse::<u64>()
        .context("native ZEC amount must be atomic decimal digits")?;
    ensure!(value > 0, "amount must be positive");
    zcash_protocol::value::Zatoshis::from_u64(value)?;
    Ok(value)
}
