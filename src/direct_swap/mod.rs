//! First-party deposit-funded swaps. Provider settlement and SDK funds are separate facts.
pub(crate) mod route;
#[cfg(test)]
mod tests;

use crate::rotation::{
    config::FundingConfig,
    near::NearClient,
    store::{StoreHandle, direct_swap::DirectOperation},
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
}
impl Config {
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
    near: NearClient,
    funding: FundingConfig,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receive {
    asset_id: String,
    chain: String,
    amount: String,
    refund_address: String,
    request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Status {
    operation_id: String,
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
        "Inspect your saved payment request or send, provider delivery, and separate Zcash wallet observations. Creates no swap.",
        json!({"operation_id":text("Saved operation ID.")}),
        vec!["operation_id"],
    )];
    if config.enabled() {
        specs.push(("x402_treazury_swap_assets", "Find supported swap assets by full asset ID and chain. Token support does not guarantee route liquidity.", json!({"query":text("Asset ID, chain or symbol to match."),"offset":{"type":"integer","minimum":0,"description":"Result offset; keep the same query."},"limit":{"type":"integer","minimum":1,"default":25,"description":"Number of assets to return."}}), vec![]));
    }
    if config.receive {
        specs.push(("x402_treazury_payment_request_create", "Create one exact-input payment request delivered as shielded ZEC to this treasury. Pay the saved deposit instructions once; value is an estimate until settlement. Reuse request_id to retrieve the same operation.", json!({"asset_id":text("Full source asset ID from swap_assets."),"chain":text("Exact source chain from swap_assets."),"amount":text("Positive source amount in atomic units, as decimal digits."),"refund_address":text("Payer-owned source-chain refund address; destination tags are unsupported."),"request_id":text("Unique caller request ID for these exact terms.")}), vec!["asset_id","chain","amount","refund_address","request_id"]));
    }
    specs.into_iter().map(|(name,description,properties,required)| rmcp::model::Tool::new(name,description,Arc::new(json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}).as_object().unwrap().clone()))).collect()
}

impl Service {
    pub fn new(
        store: StoreHandle,
        treasury: TreasuryHandle,
        near: NearClient,
        funding: FundingConfig,
        _max_fee: u64,
    ) -> Self {
        let wake = store.direct_changes();
        Self {
            store,
            backend: Some(Arc::new(Backend {
                treasury,
                near,
                funding,
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
        self.backend.as_deref().context("Direct swap admission unavailable; treasury SDK or source configuration requires correction and restart. Saved status remains available")
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
                let identity = self.identity("assets").await?;
                let assets = route::assets(&self.backend()?.near, &identity).await?;
                let query = args.query.unwrap_or_default().to_lowercase();
                let assets: Vec<_> = assets
                    .into_iter()
                    .filter(|asset| {
                        serde_json::to_string(asset)
                            .is_ok_and(|text| text.to_lowercase().contains(&query))
                    })
                    .collect();
                let offset = args.offset.unwrap_or(0);
                let limit = args.limit.unwrap_or(25);
                ensure!(limit > 0 && offset <= assets.len(), "invalid asset page");
                let end = offset.saturating_add(limit).min(assets.len());
                Ok(
                    json!({"assets":&assets[offset..end],"next_offset":(end<assets.len()).then_some(end),"total":assets.len()}),
                )
            }
            "x402_treazury_payment_request_create" => {
                self.receive(owner, serde_json::from_value(args)?).await
            }
            "x402_treazury_swap_status" => {
                let args: Status = serde_json::from_value(args)?;
                self.status(owner, &args.operation_id).await
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
        let terms = serde_json::to_value(&args)?;
        let owner_owned = owner.to_owned();
        let request = args.request_id.clone();
        let (operation, created) = self
            .store
            .call(move |s| s.reserve_direct(&owner_owned, "receive", &request, terms))
            .await?;
        if !created {
            return self.status(owner, &operation.id).await;
        }
        // Any interruption from this point retains the original operation; no new quote is minted on retry.
        let address = self
            .backend()?
            .treasury
            .direct_address(operation.id.clone())
            .await?;
        let identity = self.identity(&operation.id).await?;
        let assets = route::assets(&self.backend()?.near, &identity).await?;
        let origin = route::select_asset(&assets, &args.asset_id, &args.chain)?;
        let destination = assets
            .iter()
            .find(|a| a.asset_id == route::ZEC)
            .context("ZEC asset unavailable")?;
        let now = crate::rotation::base::now()?;
        let deadline = now
            .checked_add(self.backend()?.funding.quote_deadline_seconds)
            .context("invalid quote deadline")?;
        let mut request = route::request_on(
            &self.backend()?.near.chain(),
            origin,
            destination,
            &address,
            &args.refund_address,
            &args.amount,
            route::SwapType::ExactInput,
            self.backend()?.funding.slippage_bps,
            deadline,
            false,
        )?;
        request["confidentiality"] = json!(self.backend()?.funding.confidentiality);
        let quote = route::quote(&self.backend()?.near, &identity, request, now).await?;
        let id = operation.id.clone();
        self.store
            .call(move |s| s.save_direct_quote(&id, serde_json::to_value(quote)?))
            .await?;
        self.status(owner, &operation.id).await
    }
    async fn status(&self, owner: &str, id: &str) -> Result<Value> {
        let owner = owner.to_owned();
        let id = id.to_owned();
        self.store.call(move |s| {
            let operation = s.direct_operation(&owner,&id)?;
            let sync = s.treasury_sync_state()?.sync;
            Ok(json!({"operation":operation,"zec_receipt":{"attribution":"unknown"},"treasury_observation":sync}))
        }).await
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
                if active.insert(operation.id.clone()) {
                    let service = self.clone();
                    let stopped = stop.clone();
                    tasks.spawn(async move {
                        let id = operation.id.clone();
                        service.observe(operation, stopped).await;
                        id
                    });
                }
            }
            tokio::select! {
                _ = stop.cancelled() => break,
                _ = &mut changed => {},
                joined = tasks.join_next(), if !tasks.is_empty() => {
                    match joined { Some(Ok(id)) => { active.remove(&id); }, Some(Err(_)) => anyhow::bail!("direct swap observation owner stopped; saved work remains durable"), None => {} }
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
            % backend.funding.poll_seconds.saturating_add(1);
        loop {
            let result = self.observe_once(&operation).await;
            let delay = match result {
                Ok(true) => return,
                Ok(false) => {
                    streak = 0;
                    backend.funding.poll_seconds
                }
                Err(_) => {
                    tracing::debug!(operation_id=%operation.id,"Swap status observation unavailable; saved work remains pending");
                    streak = streak.saturating_add(1);
                    backend
                        .funding
                        .poll_seconds
                        .saturating_mul(1u64 << streak.min(6))
                        .clamp(60, 300)
                }
            };
            tokio::select! { _ = stop.cancelled() => return, _ = tokio::time::sleep(Duration::from_secs(delay.saturating_add(jitter))) => {} }
        }
    }
    async fn observe_once(&self, operation: &DirectOperation) -> Result<bool> {
        let quote: route::DirectQuote =
            serde_json::from_value(operation.quote.clone().context("missing saved quote")?)?;
        let identity = self.identity(&operation.id).await?;
        let status = route::status(&self.backend()?.near, &identity, &quote).await?;
        let terminal = matches!(
            status["status"].as_str(),
            Some("SUCCESS" | "FAILED" | "REFUNDED")
        );
        let id = operation.id.clone();
        let status = json!({"provider":status,"observed_at":crate::rotation::base::now()?});
        self.store
            .call(move |s| s.save_direct_status(&id, status))
            .await?;
        Ok(terminal)
    }
}
