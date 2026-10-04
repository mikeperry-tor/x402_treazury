//! Opt-in, conservative funded MCP driver. Never starts funding or retries a call.
#[path = "qualification/catalogs.rs"]
mod catalogs;
#[path = "qualification/identities.rs"]
mod identities;
#[path = "qualification/ledger.rs"]
mod ledger;
#[cfg(test)]
#[path = "qualification/manifest_tests.rs"]
mod manifest_tests;
#[path = "qualification/renewal.rs"]
mod renewal;
#[path = "qualification/timeouts.rs"]
mod timeouts;
use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use x402_treazury::{
    deployment::Deployment,
    network::{NetworkContext, NetworkPolicy},
    rotation::{
        base::now,
        config::{positive_usdc, zatoshis},
        store,
    },
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    manifest: PathBuf,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Fetch catalogs without keys/payments, pin definitions/config, create immutable budget ledger.
    Prepare,
    /// Once, before any activity: audited 120s/240s Tor and 900s driver timeout amendment.
    AmendTimeouts,
    /// Once, explicitly renew an expired window within the same UTC day; preserve all budgets/cases.
    RenewWindow {
        #[arg(long)]
        expires_at: u64,
    },
    /// Capture independent catalogs with explicit reviewed qualification fallbacks; no wallet access.
    FreezeCatalogs {
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        evidence_dir: PathBuf,
    },
    /// One selected batch (at most two calls). Never auto-resumes a previously attempted case.
    Execute {
        #[arg(long, required = true)]
        case: Vec<String>,
        #[arg(long)]
        allow_paid: bool,
    },
    /// Report reservations and read-only treasury observations, without network calls.
    Report,
    /// Read-only Base chain/freshness check through the deployment's network policy.
    CheckBase,
    /// Export private Tor identity bindings from public state/config; no keys or network access.
    Identities {
        #[arg(long)]
        output: PathBuf,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    run_id: String,
    deployment: PathBuf,
    expires_at: u64,
    api_budget_usdc: String,
    max_source_zec: String,
    max_funding_jobs: usize,
    #[serde(default = "timeout")]
    timeout_seconds: u64,
    cases: Vec<Case>,
}
fn timeout() -> u64 {
    120
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    server: String,
    tool: String,
    arguments: serde_json::Map<String, Value>,
    /// Must cover the entire effective wallet per-payment cap, not just the expected price.
    reserve_usdc: String,
    /// Explicitly reviewed read-only API operation and input semantics; not JSON Schema certification.
    reviewed_read_only: bool,
}
fn hash(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}
fn atomic(s: &str) -> Result<u64> {
    Ok(u64::try_from(positive_usdc(s)?)?)
}
fn id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
impl Manifest {
    fn validate(&self, instant: u64) -> Result<()> {
        ensure!(
            self.version == 1 && id(&self.run_id),
            "unsupported version or invalid run ID"
        );
        ensure!(
            self.expires_at > instant && self.expires_at / 86400 == instant / 86400,
            "run must expire later in this UTC day; no daily-budget rollover"
        );
        ensure!(
            (1..=1800).contains(&self.timeout_seconds)
                && (1..=100).contains(&self.max_funding_jobs),
            "invalid time/funding limits"
        );
        ensure!(
            atomic(&self.api_budget_usdc)? <= i64::MAX as u64,
            "API budget too large"
        );
        zatoshis(&self.max_source_zec)?;
        let mut ids = BTreeSet::new();
        ensure!(!self.cases.is_empty(), "empty manifest");
        for c in &self.cases {
            ensure!(
                id(&c.id) && ids.insert(&c.id) && c.reviewed_read_only,
                "case IDs must be unique and inputs explicitly reviewed"
            );
            atomic(&c.reserve_usdc)?;
        }
        Ok(())
    }
}
async fn settings(manifest: &Manifest) -> Result<Value> {
    let shown = Deployment::show_config(&manifest.deployment).await?;
    ensure!(shown["network"]["mode"] == "tor", "deployment must use Tor");
    ensure!(
        shown["funding"]["confidentiality"] == "public",
        "only public test swaps supported"
    );
    let daily = shown["treasury"]["daily_input_zec"]
        .as_str()
        .context("treasury daily limit missing")?;
    ensure!(
        zatoshis(daily)? <= zatoshis(&manifest.max_source_zec)?,
        "configured source cap exceeds experiment budget"
    );
    // A fixed catalog is part of the experiment; reject dynamic configuration changes.
    let raw: Value =
        serde_json::to_value(x402_treazury::config::read_table(&manifest.deployment).await?)?;
    ensure!(
        raw.get("source_management").is_none(),
        "dynamic source management must be disabled"
    );
    for server in raw["servers"]
        .as_object()
        .context("missing listeners")?
        .values()
    {
        ensure!(
            server.get("source_management").is_none(),
            "dynamic server grants must be disabled"
        );
    }
    Ok(shown)
}
fn directory(shown: &Value) -> Result<PathBuf> {
    Ok(PathBuf::from(
        shown["treasury"]["state_dir"]
            .as_str()
            .context("missing treasury state")?,
    )
    .join("qualification"))
}
fn observe(shown: &Value, manifest: &Manifest, possible_new_jobs: usize) -> Result<Value> {
    let state = store::status(Path::new(
        shown["treasury"]["state_dir"]
            .as_str()
            .context("missing state path")?,
    ))?;
    ensure!(
        state.treasury_id
            == shown["treasury"]["id"]
                .as_str()
                .context("missing treasury ID")?,
        "treasury identity mismatch"
    );
    ensure!(
        state
            .funding_jobs
            .len()
            .checked_add(possible_new_jobs)
            .is_some_and(|n| n <= manifest.max_funding_jobs),
        "funding-job ceiling reached; no further API calls admitted"
    );
    let wallets = shown["resolved_wallets"]
        .as_object()
        .context("missing resolved wallets")?;
    let worst_input = wallets
        .values()
        .filter(|w| w["mode"] == "zcash_rotation")
        .map(|w| {
            zatoshis(
                w["max_input_zec"]
                    .as_str()
                    .context("missing pool source cap")?,
            )
            .map(|n| n as u64)
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max()
        .unwrap_or(0);
    let mut source_exposure = 0u64;
    for op in &state.treasury_operations {
        // EXPIRED is written only by guarded recovery after canonical absence,
        // synced invalidation and proof that the original inputs are unspent.
        // A passed wall-clock deadline alone must never release exposure.
        source_exposure = source_exposure
            .checked_add(operation_exposure(
                &op.submission,
                op.facts.amount_zatoshis,
                op.facts.fee_zatoshis,
            )?)
            .context("source exposure overflow")?;
    }
    for job in &state.funding_jobs {
        if !state
            .treasury_operations
            .iter()
            .any(|op| op.operation_id == job.operation_id)
        {
            let input = zatoshis(
                wallets
                    .get(&job.pool_name)
                    .context("unknown existing pool")?["max_input_zec"]
                    .as_str()
                    .context("missing input limit")?,
            )? as u64;
            source_exposure = source_exposure
                .checked_add(input)
                .context("source exposure overflow")?;
        }
    }
    let projected = source_exposure
        .checked_add(
            worst_input
                .checked_mul(possible_new_jobs as u64)
                .context("source exposure overflow")?,
        )
        .context("source exposure overflow")?;
    ensure!(
        projected <= zatoshis(&manifest.max_source_zec)? as u64,
        "lifetime source exposure budget exceeded; no further calls admitted"
    );
    Ok(
        json!({"source_exposure_bound_zatoshis":source_exposure,"funding_jobs":state.funding_jobs.len(),"outgoing_pending":state.outgoing_pending,"sync_fresh":state.sync_fresh,
        "pools":state.pools.iter().map(|p|json!({"name":p.name,"generation":p.generation,"bootstrapped":p.bootstrapped,"degraded":p.funding_degraded,
        "roles":p.addresses.iter().map(|a|json!({"role":a.role,"target":a.target,"balance":a.confirmed_balance})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "note":"Balances and job states are observations, not settlement certification"}),
    )
}
fn operation_exposure(submission: &str, amount: u64, fee: u64) -> Result<u64> {
    if submission == "EXPIRED" {
        Ok(0)
    } else {
        amount.checked_add(fee).context("source exposure overflow")
    }
}
async fn prepare(manifest: &Manifest, manifest_hash: &str) -> Result<()> {
    manifest.validate(now()?)?;
    let shown = settings(manifest).await?;
    let dir = directory(&shown)?;
    ensure!(
        !dir.exists(),
        "this treasury already has a qualification ledger; no new budget via another run ID"
    );
    let pool_count = shown["resolved_wallets"]
        .as_object()
        .context("missing wallets")?
        .values()
        .filter(|w| w["mode"] == "zcash_rotation")
        .count();
    ensure!(
        pool_count
            .checked_mul(2)
            .is_some_and(|n| n <= manifest.max_funding_jobs),
        "bootstrap alone exceeds funding job limit"
    );
    let before = observe(&shown, manifest, 0)?;
    let inventory = Deployment::load(&manifest.deployment).await?.inventory();
    let mut cases = serde_json::Map::new();
    for c in &manifest.cases {
        let listener = inventory
            .iter()
            .find(|s| s.server == c.server)
            .context("case listener missing")?;
        ensure!(
            listener.listen.ip().is_loopback() && listener.listen.port() != 0,
            "driver requires a fixed loopback listener"
        );
        let entry = listener
            .tools
            .iter()
            .find(|t| t.tool.name == c.tool)
            .context("case tool excluded or missing")?;
        let binding = listener
            .wallet_bindings
            .get(&entry.source)
            .context("case has no wallet binding")?;
        let wallet = &shown["resolved_wallets"][&binding.wallet];
        ensure!(
            wallet["mode"] == "zcash_rotation",
            "funded test requires managed wallets"
        );
        ensure!(
            atomic(
                wallet["max_price_usd"]
                    .as_str()
                    .context("wallet needs a finite payment cap")?
            )? <= atomic(&c.reserve_usdc)?,
            "case reservation does not cover effective wallet payment cap"
        );
        // Required properties and basic shape catch manifest mistakes; complex vendor schema
        // semantics still require the explicit review attestation above.
        let schema = &entry.tool.input_schema;
        for name in schema["required"].as_array().into_iter().flatten() {
            ensure!(
                c.arguments
                    .contains_key(name.as_str().context("invalid required property")?),
                "case {} missing required argument {}",
                c.id,
                name
            );
        }
        ensure!(
            entry.tool.help_url.is_none(),
            "paid case cannot be a help tool; use discovery phase for help"
        );
        ensure!(
            matches!(entry.tool.method.as_str(), "GET" | "POST"),
            "only reviewed read APIs supported"
        );
        cases.insert(c.id.clone(),json!({"endpoint":format!("http://{}/mcp",listener.listen),"token_env":shown["servers"][&c.server]["bearer_token_env"],
            "schema":schema,"wallet":binding.wallet,"source":entry.source,"method":entry.tool.method,"path":entry.tool.path}));
    }
    let prepared = json!({"manifest_hash":manifest_hash,"manifest":manifest,"resolved_config_hash":hash(serde_json::to_vec(&shown)?),"cases":cases,"prepared_at":now()?});
    let ledger = ledger::Ledger::create(&dir, &prepared, atomic(&manifest.api_budget_usdc)?)?;
    ledger.event("prepared", &before)?;
    println!(
        "{}",
        json!({"prepared":true,"run_id":manifest.run_id,"ledger":dir,"cases":manifest.cases.len(),"api_budget_usdc":manifest.api_budget_usdc,"max_source_zec":manifest.max_source_zec,"max_funding_jobs":manifest.max_funding_jobs,"note":"No funding or API calls started. Run only the pinned deployment with no other callers."})
    );
    Ok(())
}
async fn rpc(
    http: &reqwest::Client,
    endpoint: &str,
    token: &str,
    method: &str,
    params: Value,
) -> Result<Value> {
    let mut response = http
        .post(endpoint)
        .bearer_auth(token)
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;
    ensure!(
        response.status().is_success(),
        "MCP HTTP status {}",
        response.status().as_u16()
    );
    ensure!(
        response.headers().get("mcp-session-id").is_none(),
        "driver requires stateless MCP; session transport unsupported"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        ensure!(
            chunk.len() <= 16_777_216usize.saturating_sub(bytes.len()),
            "MCP response exceeds 16777216-byte limit; rejected, possible payment remains reserved"
        );
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).context("MCP response is not JSON")?;
    ensure!(
        value["jsonrpc"] == "2.0" && value["id"] == 1,
        "MCP response identity mismatch"
    );
    ensure!(
        value.get("error").is_none(),
        "MCP protocol error (request may have executed)"
    );
    value.get("result").cloned().context("missing MCP result")
}
async fn check_tool(
    http: &reqwest::Client,
    endpoint: &str,
    token: &str,
    c: &Case,
    schema: &Value,
) -> Result<()> {
    rpc(http,endpoint,token,"initialize",json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"treazury-qualification","version":"1"}})).await?;
    let mut cursor = Value::Null;
    for _ in 0..100 {
        let page = rpc(
            http,
            endpoint,
            token,
            "tools/list",
            if cursor.is_null() {
                json!({})
            } else {
                json!({"cursor":cursor})
            },
        )
        .await?;
        for tool in page["tools"].as_array().context("invalid tools/list")? {
            if tool["name"] == c.tool {
                ensure!(
                    &tool["inputSchema"] == schema,
                    "tool input schema changed since preparation"
                );
                return Ok(());
            }
        }
        cursor = page["nextCursor"].clone();
        if cursor.is_null() {
            anyhow::bail!("selected tool is not exposed by this listener");
        }
    }
    anyhow::bail!("tools/list exceeds 100-page limit; inventory incomplete, nothing paid")
}
async fn execute(
    manifest: &Manifest,
    manifest_hash: &str,
    ids: &[String],
    allowed: bool,
) -> Result<()> {
    ensure!(allowed, "execute requires --allow-paid");
    manifest.validate(now()?)?;
    ensure!(
        (1..=2).contains(&ids.len()) && ids.iter().collect::<BTreeSet<_>>().len() == ids.len(),
        "choose one or two distinct cases"
    );
    let shown = settings(manifest).await?;
    let dir = directory(&shown)?;
    let mut ledger = ledger::Ledger::open(&dir)?;
    let prepared = ledger.prepared()?;
    ensure!(
        prepared["manifest_hash"] == manifest_hash,
        "manifest changed; cannot reset or amend an existing experiment budget"
    );
    ensure!(
        prepared["resolved_config_hash"] == hash(serde_json::to_vec(&shown)?),
        "resolved deployment changed since preparation"
    );
    ledger.event("before_batch", &observe(&shown, manifest, ids.len())?)?;
    // This factory belongs to the separate local MCP driver. The application still
    // uses its pinned Tor policy; no provider URL can be passed to this client.
    let local = NetworkContext::new(NetworkPolicy::default())?;
    let mut calls = Vec::new();
    for id in ids {
        let case = manifest
            .cases
            .iter()
            .find(|c| &c.id == id)
            .context("unknown case")?
            .clone();
        let pin = &prepared["cases"][id];
        let status = store::status(Path::new(
            shown["treasury"]["state_dir"]
                .as_str()
                .context("missing state")?,
        ))?;
        ensure!(
            status
                .pools
                .iter()
                .any(|p| Some(p.name.as_str()) == pin["wallet"].as_str()
                    && p.enabled
                    && p.bootstrapped),
            "case pool has not bootstrapped; no API call sent"
        );

        let endpoint = pin["endpoint"]
            .as_str()
            .context("missing endpoint")?
            .to_owned();
        let url = reqwest::Url::parse(&endpoint)?;
        ensure!(
            url.scheme() == "http"
                && url
                    .host_str()
                    .and_then(|h| h.parse::<std::net::IpAddr>().ok())
                    .is_some_and(|ip| ip.is_loopback()),
            "MCP driver refuses non-loopback endpoint"
        );
        let token = std::env::var(pin["token_env"].as_str().context("missing bearer env")?)
            .context("missing MCP bearer token")?;
        let http = local.discovery(&endpoint, Duration::from_secs(manifest.timeout_seconds))?;
        check_tool(&http, &endpoint, &token, &case, &pin["schema"]).await?;
        calls.push((case, http, endpoint, token));
    }
    manifest.validate(now()?)?;
    observe(&shown, manifest, calls.len())?;
    for (c, _, _, _) in &calls {
        ledger.reserve(&c.id, atomic(&c.reserve_usdc)?)?;
    }
    let results = futures_util::future::join_all(calls.into_iter().map(
        |(c, http, endpoint, token)| async move {
            let result = rpc(
                &http,
                &endpoint,
                &token,
                "tools/call",
                json!({"name":c.tool,"arguments":c.arguments}),
            )
            .await;
            (c.id, result)
        },
    ))
    .await;
    for (id, result) in results {
        match result {
            Ok(value) => {
                let bytes = serde_json::to_vec_pretty(&value)?;
                let mut file = ledger::private_file(&dir.join(format!("result-{id}.json")))?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                std::fs::File::open(&dir)?.sync_all()?;
                let outcome = if value["isError"] == true {
                    "tool_error"
                } else {
                    "response_received"
                };
                ledger.finish(&id,outcome,&json!({"response_sha256":hash(&bytes),"bytes":bytes.len(),"settlement":"unverified"}))?;
                eprintln!("{id}: {outcome}; reservation retained, no automatic retry");
            }
            Err(error) => {
                ledger.finish(&id, "ambiguous", &json!({"error":format!("{error:#}")}))?;
                eprintln!("{id}: ambiguous result; reservation retained, no automatic retry");
            }
        }
    }
    ledger.event("after_batch", &observe(&shown, manifest, 0)?)?;
    println!("{}", serde_json::to_string_pretty(&ledger.report()?)?);
    Ok(())
}
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let args = Args::parse();
    let bytes = std::fs::read(&args.manifest)?;
    let digest = hash(&bytes);
    let mut manifest: Manifest = toml::from_str(std::str::from_utf8(&bytes)?)?;
    if manifest.deployment.is_relative() {
        manifest.deployment = args
            .manifest
            .canonicalize()?
            .parent()
            .unwrap()
            .join(&manifest.deployment);
    }
    match args.command {
        Command::FreezeCatalogs {
            output,
            evidence_dir,
        } => catalogs::freeze(&manifest, &output, &evidence_dir).await?,
        Command::CheckBase => {
            let shown = settings(&manifest).await?;
            let table = x402_treazury::config::read_table(&manifest.deployment).await?;
            let policy: NetworkPolicy = table
                .get("network")
                .context("missing network")?
                .clone()
                .try_into()?;
            x402_treazury::network::install(policy)?;
            let name = shown["funding"]["base_rpc_url_env"]
                .as_str()
                .context("missing RPC environment name")?;
            let url =
                std::env::var(name).context("set the configured Base RPC environment variable")?;
            let rpc = x402_treazury::rotation::base::BaseRpc::new(
                &url,
                shown["funding"]["base_confirmations"]
                    .as_u64()
                    .context("missing confirmations")?,
                shown["funding"]["base_max_block_age_seconds"]
                    .as_u64()
                    .context("missing block freshness limit")?,
            )?;
            let view = rpc
                .view(x402_treazury::rotation::base::ChainQuery {
                    wallets: vec![],
                    pending: vec![],
                    anchor: None,
                })
                .await?;
            println!(
                "{}",
                json!({"chain_id":8453,"confirmed_height":view.anchor.height,"confirmed_hash":view.anchor.hash,"read_only":true})
            );
        }
        Command::Identities { output } => identities::export(&manifest, &output).await?,
        Command::Prepare => prepare(&manifest, &digest).await?,
        Command::AmendTimeouts => timeouts::amend(&args.manifest, &manifest, &digest).await?,
        Command::RenewWindow { expires_at } => {
            renewal::renew(&args.manifest, &manifest, &digest, expires_at).await?
        }
        Command::Execute { case, allow_paid } => {
            execute(&manifest, &digest, &case, allow_paid).await?
        }
        Command::Report => {
            // Reporting remains possible after expiry/config edits; it never admits work.
            let shown = Deployment::show_config(&manifest.deployment).await?;
            let l = ledger::Ledger::open(&directory(&shown)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"ledger":l.report()?,"pinned":l.prepared()?,"treasury":observe(&shown,&manifest,0).map_err(|e|e.to_string())})
                )?
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod driver_tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[tokio::test]
    async fn mcp_auth_scope_schema_and_concurrent_calls_use_actual_wire() {
        let count = Arc::new(AtomicUsize::new(0));
        let captured = count.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener,Router::new().route("/mcp",post(move |headers:axum::http::HeaderMap,Json(v):Json<Value>| {let count=captured.clone();async move {
            if headers.get("authorization").and_then(|h|h.to_str().ok())!=Some("Bearer test") {return (axum::http::StatusCode::UNAUTHORIZED,Json(json!({})));}
            let result=match v["method"].as_str().unwrap(){
                "initialize"=>json!({"protocolVersion":"2025-03-26","capabilities":{}}),
                "tools/list"=>json!({"tools":[{"name":"read","inputSchema":{"type":"object"}}]}),
                "tools/call"=>{count.fetch_add(1,Ordering::SeqCst);json!({"content":[{"type":"text","text":"ok"}]})},_=>panic!()};
            (axum::http::StatusCode::OK,Json(json!({"jsonrpc":"2.0","id":1,"result":result})))
        }}))).await.unwrap();
        });
        let context = NetworkContext::new(NetworkPolicy::default()).unwrap();
        let http = context
            .discovery(&endpoint, Duration::from_secs(2))
            .unwrap();
        let case = Case {
            id: "one".into(),
            server: "a".into(),
            tool: "read".into(),
            arguments: Default::default(),
            reserve_usdc: "0.20".into(),
            reviewed_read_only: true,
        };
        assert!(
            check_tool(&http, &endpoint, "bad", &case, &json!({"type":"object"}))
                .await
                .is_err()
        );
        assert!(
            check_tool(&http, &endpoint, "test", &case, &json!({"type":"string"}))
                .await
                .is_err()
        );
        let mut absent = case.clone();
        absent.tool = "missing".into();
        assert!(
            check_tool(&http, &endpoint, "test", &absent, &json!({}))
                .await
                .is_err()
        );
        assert_eq!(count.load(Ordering::SeqCst), 0);
        check_tool(&http, &endpoint, "test", &case, &json!({"type":"object"}))
            .await
            .unwrap();
        let (a, b) = tokio::join!(
            rpc(
                &http,
                &endpoint,
                "test",
                "tools/call",
                json!({"name":"read"})
            ),
            rpc(
                &http,
                &endpoint,
                "test",
                "tools/call",
                json!({"name":"read"})
            )
        );
        assert!(a.is_ok() && b.is_ok());
        assert_eq!(count.load(Ordering::SeqCst), 2);
        server.abort();
    }
    #[tokio::test]
    async fn timeout_after_server_receives_call_is_not_replayed_on_resume() {
        let count = Arc::new(AtomicUsize::new(0));
        let captured = count.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/mcp",
                    post(move || {
                        let count = captured.clone();
                        async move {
                            count.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_secs(3)).await;
                            Json(json!({}))
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("ledger");
        let mut ledger = ledger::Ledger::create(&dir, &json!({}), 200000).unwrap();
        ledger.reserve("call", 200000).unwrap();
        let local = NetworkContext::new(NetworkPolicy::default()).unwrap();
        let http = local
            .discovery(&endpoint, Duration::from_millis(200))
            .unwrap();
        assert!(
            rpc(&http, &endpoint, "test", "tools/call", json!({}))
                .await
                .is_err()
        );
        drop(ledger);
        let mut reopened = ledger::Ledger::open(&dir).unwrap();
        assert!(reopened.reserve("call", 200000).is_err());
        assert!(reopened.reserve("new", 1).is_err());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        server.abort();
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    #[tokio::test]
    async fn example_profiles_resolve_shared_and_isolated_bindings_offline() {
        let shown = Deployment::show_config(Path::new("examples/live-qualification.toml"))
            .await
            .unwrap();
        assert_eq!(
            shown["wallet_bindings"]["a"]["shared_social"]["wallet"],
            "coverage_a"
        );
        assert_eq!(
            shown["wallet_bindings"]["b"]["shared_social"]["wallet"],
            "coverage_a"
        );
        assert_eq!(
            shown["wallet_bindings"]["b"]["isolated_social"]["wallet"],
            "coverage_b"
        );
        assert_eq!(
            shown["wallet_bindings"]["c"]["isolated_social"]["wallet"],
            "coverage_b"
        );
        let mut m: Manifest =
            toml::from_str(include_str!("../tests/live/driver.example.toml")).unwrap();
        assert!(m.validate(1000).is_err());
        m.expires_at = 2000;
        m.validate(1000).unwrap();
        assert_eq!(shown["funding"]["auto_fund"], false);
    }
    #[test]
    fn manifest_rejects_rollover_duplicates_unreviewed_and_unbounded_prices() {
        let mut m = Manifest {
            version: 1,
            run_id: "run".into(),
            deployment: "unused".into(),
            expires_at: 2000,
            api_budget_usdc: "1".into(),
            max_source_zec: "0.01".into(),
            max_funding_jobs: 6,
            timeout_seconds: 30,
            cases: vec![Case {
                id: "one".into(),
                server: "a".into(),
                tool: "read".into(),
                arguments: Default::default(),
                reserve_usdc: "0.2".into(),
                reviewed_read_only: true,
            }],
        };
        m.validate(1000).unwrap();
        m.expires_at = 86401;
        assert!(m.validate(1000).is_err());
        m.expires_at = 2000;
        m.cases.push(m.cases[0].clone());
        assert!(m.validate(1000).is_err());
        m.cases.pop();
        m.cases[0].reviewed_read_only = false;
        assert!(m.validate(1000).is_err());
        m.cases[0].reviewed_read_only = true;
        m.cases[0].reserve_usdc = "off".into();
        assert!(m.validate(1000).is_err());
    }
    #[tokio::test]
    async fn source_and_allocation_limits_use_existing_treasury_history() {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        let key = tmp.path().join("key");
        let mut store = store::Store::create(&state, &key, 1, b"test encrypted snapshot").unwrap();
        let id = store.id().to_owned();
        store.ensure_pool("pool", "1.717124").unwrap();
        drop(store);
        let shown = json!({"treasury":{"id":id,"state_dir":state},"resolved_wallets":{"pool":{"mode":"zcash_rotation","max_input_zec":"0.002"}}});
        let mut m = Manifest {
            version: 1,
            run_id: "run".into(),
            deployment: "unused".into(),
            expires_at: 2000,
            api_budget_usdc: "1".into(),
            max_source_zec: "0.006".into(),
            max_funding_jobs: 3,
            timeout_seconds: 30,
            cases: vec![],
        };
        let status = observe(&shown, &m, 1).unwrap();
        assert_eq!(status["funding_jobs"], 2);
        assert_eq!(status["source_exposure_bound_zatoshis"], 400000);
        assert!(observe(&shown, &m, 2).is_err());
        m.max_source_zec = "0.00599999".into();
        assert!(observe(&shown, &m, 1).is_err());
        let mut wrong = shown.clone();
        wrong["treasury"]["id"] = json!("wrong");
        assert!(observe(&wrong, &m, 0).is_err());
    }
    #[test]
    fn only_proven_expired_operations_release_source_exposure() {
        // Confirmed history remains spent; prepared/unknown broadcasts remain
        // liabilities. Unknown future states must also fail conservatively.
        for phase in [
            "PREPARED",
            "BROADCAST_REQUESTED",
            "BROADCAST",
            "UNKNOWN",
            "CONFIRMED",
            "unrecognized",
        ] {
            assert_eq!(operation_exposure(phase, 152316, 15000).unwrap(), 167316);
            assert!(operation_exposure(phase, u64::MAX, 1).is_err());
        }
        assert_eq!(operation_exposure("EXPIRED", 152316, 15000).unwrap(), 0);
        // The live scenario: four settled deposits, one pending refill and
        // one archived unspent transaction must fit the original cap including
        // a potential next job, without raising that cap.
        let settled = 670210u64;
        let pending = operation_exposure("PREPARED", 152000, 15000).unwrap();
        let archived = operation_exposure("EXPIRED", 152316, 15000).unwrap();
        assert!(settled + pending + archived + 200000 <= 1050000);
        assert!(settled + pending + archived + 400000 > 1050000);
    }
    #[tokio::test]
    async fn real_mcp_paid_handshake_stays_single_after_driver_crash() {
        use axum::{Router, routing::get};
        use base64::{Engine, engine::general_purpose::STANDARD};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use x402_treazury::{
            catalog::{Config, build_tools},
            payment::{PaidClient, Payer, SpendPolicy},
            server::{Server, http_app},
        };
        let vendor = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", vendor.local_addr().unwrap());
        let challenge=STANDARD.encode(json!({"x402Version":2,"resource":{"url":format!("{base}/read"),"description":"test","mimeType":"text/plain"},"accepts":[{"scheme":"exact","network":"eip155:8453","amount":"1000","asset":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913","payTo":"0x0000000000000000000000000000000000000002","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]}).to_string());
        let signed = Arc::new(AtomicUsize::new(0));
        let count = signed.clone();
        let vendor_task = tokio::spawn(async move {
            axum::serve(
                vendor,
                Router::new().route(
                    "/read",
                    get(move |h: axum::http::HeaderMap| {
                        let challenge = challenge.clone();
                        let count = count.clone();
                        async move {
                            if h.contains_key("payment-signature") {
                                count.fetch_add(1, Ordering::SeqCst);
                                (
                                    axum::http::StatusCode::OK,
                                    [("content-type", "text/plain".to_owned())],
                                    "paid",
                                )
                            } else {
                                (
                                    axum::http::StatusCode::PAYMENT_REQUIRED,
                                    [("payment-required", challenge)],
                                    "pay",
                                )
                            }
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let tools = build_tools(
            &Config::default(),
            &json!({"paths":{"/read":{"get":{}}}}),
            "test",
        )
        .unwrap();
        let client = PaidClient::new(
            Payer::new(
                &format!("{:064x}", 1),
                SpendPolicy::dollars("0.20").unwrap(),
            )
            .unwrap(),
        );
        let server = Server::new(tools, client, base, None, None);
        let mcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", mcp.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(mcp, http_app(server, "test".into()))
                .await
                .unwrap();
        });
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("ledger");
        let mut l = ledger::Ledger::create(&dir, &json!({}), 200000).unwrap();
        l.reserve("paid", 200000).unwrap();
        let local = NetworkContext::new(NetworkPolicy::default()).unwrap();
        let http = local.discovery(&endpoint, Duration::from_secs(5)).unwrap();
        check_tool(
            &http,
            &endpoint,
            "test",
            &Case {
                id: "paid".into(),
                server: "a".into(),
                tool: "test_read".into(),
                arguments: Default::default(),
                reserve_usdc: "0.20".into(),
                reviewed_read_only: true,
            },
            &json!({"type":"object","properties":{}}),
        )
        .await
        .unwrap();
        let result = rpc(
            &http,
            &endpoint,
            "test",
            "tools/call",
            json!({"name":"test_read","arguments":{}}),
        )
        .await
        .unwrap();
        assert_ne!(result["isError"], true);
        assert_eq!(signed.load(Ordering::SeqCst), 1);
        drop(l);
        let mut resumed = ledger::Ledger::open(&dir).unwrap();
        assert!(resumed.reserve("paid", 200000).is_err());
        assert_eq!(signed.load(Ordering::SeqCst), 1);
        task.abort();
        vendor_task.abort();
    }
}
