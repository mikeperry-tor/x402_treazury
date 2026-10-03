use axum::{
    Router,
    extract::{OriginalUri, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use x402_treazury::{deployment::Deployment, payment::USDC};

#[derive(Clone)]
struct Vendor {
    name: &'static str,
    unsigned: Arc<AtomicUsize>,
    signed: Arc<AtomicUsize>,
    hold: Arc<AtomicBool>,
    arrived: Arc<Notify>,
    release: Arc<Notify>,
}
async fn endpoint(
    State(v): State<Vendor>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    if let Some(h) = headers.get("payment-signature") {
        let p: Value = serde_json::from_slice(&STANDARD.decode(h.as_bytes()).unwrap()).unwrap();
        v.signed.fetch_add(1, Ordering::SeqCst);
        if v.hold.swap(false, Ordering::SeqCst) {
            v.arrived.notify_one();
            v.release.notified().await;
        }
        return axum::Json(json!({"vendor":v.name,"query":uri.query(),"payer":p["payload"]["authorization"]["from"]})).into_response();
    }
    v.unsigned.fetch_add(1, Ordering::SeqCst);
    let amount = if uri.path().ends_with("expensive") {
        "50000"
    } else {
        "14000"
    };
    let challenge = json!({"x402Version":2,"resource":{"url":"http://localhost/pay","description":"paid","mimeType":"application/json"},
        "accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":amount,
        "payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
    (
        StatusCode::PAYMENT_REQUIRED,
        [(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
        )],
    )
        .into_response()
}
async fn vendor(name: &'static str, prefix: &str) -> (String, Vendor, tokio::task::JoinHandle<()>) {
    let state = Vendor {
        name,
        unsigned: Arc::default(),
        signed: Arc::default(),
        hold: Arc::default(),
        arrived: Arc::default(),
        release: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}{prefix}", listener.local_addr().unwrap());
    let app = Router::new()
        .route(&format!("{prefix}/pay"), get(endpoint))
        .route(&format!("{prefix}/expensive"), get(endpoint))
        .with_state(state.clone());
    (
        base,
        state,
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }),
    )
}
fn spec(base: &str) -> Value {
    json!({"servers":[{"url":base}],"paths":{
        "/pay":{"get":{"parameters":[{"name":"q","in":"query","schema":{"type":"string"}}]}},
        "/hidden":{"get":{}},"/expensive":{"get":{}}}})
}
fn configuration() -> String {
    r#"
version = 1
[sources.alpha]
spec = "alpha.json"
[sources.beta]
spec = "beta.json"
[wallets.shared]
mode = "static"
private_key_env = "TEST_KEY"
max_price_usd = "0.02"
[servers.research]
listen = "127.0.0.1:0"
bearer_token_env = "RESEARCH_TOKEN"
wallet = "shared"
sources = ["alpha", "beta"]
include_tools = ["alpha_*", "beta_pay"]
exclude_tools = ["alpha_hidden"]
[servers.beta_only]
listen = "127.0.0.1:0"
bearer_token_env = "BETA_TOKEN"
wallet = "shared"
sources = ["beta"]
include_tools = ["beta_pay"]
"#
    .into()
}
fn write_config(dir: &std::path::Path, text: &str) -> std::path::PathBuf {
    let path = dir.join("servers.toml");
    std::fs::write(&path, text).unwrap();
    path
}
fn env() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("TEST_KEY".into(), format!("{:064x}", 1)),
        ("RESEARCH_TOKEN".into(), "research-secret".into()),
        ("BETA_TOKEN".into(), "beta-secret".into()),
    ])
}
async fn rpc(
    http: &reqwest::Client,
    address: std::net::SocketAddr,
    token: &str,
    method: &str,
    params: Value,
) -> Value {
    let response = http
        .post(format!("http://{address}/mcp"))
        .bearer_auth(token)
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.json().await.unwrap()
}
#[tokio::test]
async fn listeners_filter_authenticate_route_pay_and_drain_together() {
    let (a, state, va) = vendor("alpha", "/api").await;
    let (b, beta, vb) = vendor("beta", "/gateway").await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alpha.json"), spec(&a).to_string()).unwrap();
    std::fs::write(dir.path().join("beta.json"), spec(&b).to_string()).unwrap();
    let path = write_config(dir.path(), &configuration());
    let deployment = Deployment::load(&path).await.unwrap();
    let inventory = deployment.inventory();
    assert_eq!(inventory[0].tools.len(), 1);
    assert_eq!(inventory[1].tools.len(), 3);
    let running = deployment.bind(&env()).await.unwrap();
    let addresses: BTreeMap<_, _> = running.addresses().into_iter().collect();
    let research = addresses["research"];
    let limited = addresses["beta_only"];
    let stop = CancellationToken::new();
    let task = tokio::spawn(running.serve(stop.clone()));
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    for (address, wrong) in [
        (research, "beta-secret"),
        (limited, "research-secret"),
        (research, ""),
    ] {
        let r = http
            .post(format!("http://{address}/mcp"))
            .bearer_auth(wrong)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401);
        assert_eq!(r.headers()["www-authenticate"], "Bearer");
    }
    let listed = rpc(&http, limited, "beta-secret", "tools/list", json!({})).await;
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 1);
    for name in ["alpha_pay", "beta_hidden"] {
        let r = rpc(
            &http,
            limited,
            "beta-secret",
            "tools/call",
            json!({"name":name,"arguments":{}}),
        )
        .await;
        assert_eq!(r["result"]["isError"], true);
    }
    let (first, second) = tokio::join!(
        rpc(
            &http,
            research,
            "research-secret",
            "tools/call",
            json!({"name":"alpha_pay","arguments":{"q":"one two"}})
        ),
        rpc(
            &http,
            limited,
            "beta-secret",
            "tools/call",
            json!({"name":"beta_pay","arguments":{"q":"other"}})
        )
    );
    let body = |r: &Value| {
        serde_json::from_str::<Value>(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    assert_eq!(body(&first)["vendor"], "alpha");
    assert_eq!(body(&first)["query"], "q=one+two");
    assert_eq!(body(&second)["vendor"], "beta");
    assert_eq!(body(&first)["payer"], body(&second)["payer"]);
    assert_eq!(
        body(&first)["payer"].as_str().unwrap().to_lowercase(),
        "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
    );
    let across = rpc(
        &http,
        research,
        "research-secret",
        "tools/call",
        json!({"name":"beta_pay","arguments":{}}),
    )
    .await;
    assert_eq!(body(&across)["vendor"], "beta");
    let rejected = rpc(
        &http,
        research,
        "research-secret",
        "tools/call",
        json!({"name":"alpha_expensive","arguments":{}}),
    )
    .await;
    assert_eq!(rejected["result"]["isError"], true);
    assert_eq!(state.signed.load(Ordering::SeqCst), 1);
    assert_eq!(beta.signed.load(Ordering::SeqCst), 2);
    // Shutdown must drain an already submitted payment instead of dropping its response.
    state.hold.store(true, Ordering::SeqCst);
    let call = tokio::spawn({
        let http = http.clone();
        async move {
            rpc(
                &http,
                research,
                "research-secret",
                "tools/call",
                json!({"name":"alpha_pay","arguments":{}}),
            )
            .await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), state.arrived.notified())
        .await
        .unwrap();
    let healthy = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        rpc(
            &http,
            limited,
            "beta-secret",
            "tools/call",
            json!({"name":"beta_pay","arguments":{}}),
        ),
    )
    .await
    .expect("slow alpha must not block healthy beta");
    assert_eq!(body(&healthy)["vendor"], "beta");
    assert!(!call.is_finished());
    stop.cancel();
    assert!(!task.is_finished());
    state.release.notify_one();
    assert_eq!(body(&call.await.unwrap())["vendor"], "alpha");
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    for address in addresses.values() {
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
    }
    va.abort();
    vb.abort();
}

#[tokio::test]
async fn validation_and_inventory_need_no_wallet_credentials() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["alpha", "beta"] {
        std::fs::write(
            dir.path().join(format!("{name}.json")),
            spec("http://127.0.0.1:1").to_string(),
        )
        .unwrap();
    }
    let path = write_config(dir.path(), &configuration());
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_treazury"))
        .args(["--meta-config", path.to_str().unwrap(), "--list-tools"])
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let invalid = [
        (
            configuration().replace("version = 1", "version = 2"),
            "version",
        ),
        (
            configuration().replace("wallet = \"shared\"", "wallet = \"missing\""),
            "unknown wallet",
        ),
        (
            configuration().replace("sources = [\"beta\"]", "sources = [\"missing\"]"),
            "unknown source",
        ),
        (
            configuration().replace(
                "exclude_tools = [\"alpha_hidden\"]",
                "exclude_tools = [\"alpha_typo\"]",
            ),
            "unknown tool selector",
        ),
        (
            configuration().replace(
                "include_tools = [\"beta_pay\"]",
                "include_tools = [\"missing_*\"]",
            ),
            "no tools selected",
        ),
        (
            configuration().replace("mode = \"static\"", "mode = \"zcash_rotation\""),
            "unknown field `private_key_env`",
        ),
        (
            configuration().replace("127.0.0.1:0", "127.0.0.1:8173"),
            "duplicate listener",
        ),
        (
            configuration().replace("max_price_usd = \"0.02\"", "max_price_usd = \"-1\""),
            "invalid max_price",
        ),
        (
            format!("unknown = true\n{}", configuration()),
            "unknown field",
        ),
    ];
    for (text, message) in invalid {
        let error = match Deployment::load(&write_config(dir.path(), &text)).await {
            Err(e) => format!("{e:#}"),
            Ok(_) => panic!("accepted {message}"),
        };
        assert!(error.contains(message), "{error}");
    }
}

#[tokio::test]
async fn binding_is_atomic_and_missing_auth_opens_no_port() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["alpha", "beta"] {
        std::fs::write(
            dir.path().join(format!("{name}.json")),
            spec("http://127.0.0.1:1").to_string(),
        )
        .unwrap();
    }
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let occupied = reserved.local_addr().unwrap();
    let free = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = free.local_addr().unwrap();
    drop(free);
    // BTree order binds beta_only first, then fails research. Both must close.
    let text = configuration()
        .replacen("127.0.0.1:0", &occupied.to_string(), 1)
        .replacen("127.0.0.1:0", &address.to_string(), 1);
    let path = write_config(dir.path(), &text);
    assert!(
        Deployment::load(&path)
            .await
            .unwrap()
            .bind(&BTreeMap::new())
            .await
            .is_err()
    );
    assert!(
        Deployment::load(&path)
            .await
            .unwrap()
            .bind(&env())
            .await
            .is_err()
    );
    let rebound = tokio::net::TcpListener::bind(address).await.unwrap();
    drop(rebound);
}

#[tokio::test]
async fn extended_providers_keep_overrides_and_resolve_declaring_paths() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("config")).unwrap();
    for name in ["alpha", "beta"] {
        std::fs::write(
            dir.path().join(format!("{name}.json")),
            spec("http://127.0.0.1:1").to_string(),
        )
        .unwrap();
    }
    std::fs::write(
        dir.path().join("vendor.toml"),
        r#"spec = "alpha.json"
prefix = "alpha"
instructions_text = "Use alpha_pay"
help_url = "http://127.0.0.1:1/llms.txt"
probe_pricing = false
[overrides.alpha_pay]
description = "Authored instructions"
"#,
    )
    .unwrap();
    let text = configuration()
        .replacen("spec = \"alpha.json\"", "extends = \"../vendor.toml\"", 1)
        .replace("spec = \"beta.json\"", "spec = \"../beta.json\"");
    let path = write_config(&dir.path().join("config"), &text);
    let deployment = Deployment::load(&path).await.unwrap();
    let inventory = deployment.inventory();
    let pay = inventory[1]
        .tools
        .iter()
        .find(|t| t.tool.name == "alpha_pay")
        .unwrap();
    assert_eq!(pay.tool.description, "Authored instructions");
    assert!(
        inventory[1]
            .tools
            .iter()
            .any(|t| t.tool.name == "alpha_help")
    );
    std::fs::write(
        dir.path().join("vendor.toml"),
        "extends = \"other.toml\"\nspec = \"alpha.json\"\n",
    )
    .unwrap();
    assert!(Deployment::load(&path).await.is_err());
}

#[tokio::test]
async fn pricing_is_discovered_once_across_sources_and_listeners() {
    let (base, state, vendor) = vendor("shared", "/api").await;
    let dir = tempfile::tempdir().unwrap();
    for file in ["alpha.json", "beta.json"] {
        std::fs::write(dir.path().join(file), spec(&base).to_string()).unwrap();
    }
    let config = configuration()
        .replace("spec = ", "probe_pricing = true\nspec = ")
        .replace(
            "include_tools = [\"alpha_*\", \"beta_pay\"]",
            "include_tools = [\"alpha_pay\", \"beta_pay\"]",
        );
    let path = write_config(dir.path(), &config);
    let deployment = Deployment::load(&path).await.unwrap();
    let _ = deployment.inventory();
    assert_eq!(state.unsigned.load(Ordering::SeqCst), 0);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_treazury"))
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .args(["--meta-config", path.to_str().unwrap(), "--list-tools"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(state.unsigned.load(Ordering::SeqCst), 0);
    let running = deployment.bind(&env()).await.unwrap();
    assert_eq!(state.unsigned.load(Ordering::SeqCst), 1);
    let addresses = running.addresses();
    let stop = CancellationToken::new();
    let task = tokio::spawn(running.serve(stop.clone()));
    let http = reqwest::Client::new();
    for (name, address) in addresses {
        for _ in 0..3 {
            let result = rpc(
                &http,
                address,
                if name == "research" {
                    "research-secret"
                } else {
                    "beta-secret"
                },
                "tools/list",
                json!({}),
            )
            .await;
            for tool in result["result"]["tools"].as_array().unwrap() {
                assert!(tool["description"].as_str().unwrap().contains("$0.014"));
            }
        }
    }
    assert_eq!(state.unsigned.load(Ordering::SeqCst), 1);
    assert_eq!(state.signed.load(Ordering::SeqCst), 0);
    stop.cancel();
    task.await.unwrap().unwrap();
    vendor.abort();
}

#[tokio::test]
async fn wallet_bindings_are_explicit_offline_and_provider_files_cannot_assign_them() {
    let dir = tempfile::tempdir().unwrap();
    let text = configuration().replace("[sources.beta]", "[sources.beta]\nwallet = 'specific'")
        + "\n[wallets.specific]\nmode='static'\nprivate_key_env='SOURCE_KEY'\n";
    let path = write_config(dir.path(), &text);
    // Specs intentionally absent: showing configuration must not load catalogs.
    let shown = Deployment::show_config(&path).await.unwrap();
    assert_eq!(shown["sources"]["beta"]["wallet"], "specific");
    assert!(shown["sources"]["beta"]["settings"].get("wallet").is_none());
    assert_eq!(
        shown["wallet_bindings"]["research"]["alpha"],
        json!({"wallet":"shared","origin":"servers.research.wallet"})
    );
    assert_eq!(
        shown["wallet_bindings"]["research"]["beta"],
        json!({"wallet":"specific","origin":"sources.beta.wallet"})
    );
    assert_eq!(
        shown["wallet_bindings"]["beta_only"]["beta"]["wallet"],
        "specific"
    );
    for (bad, expected) in [
        (
            text.replace("wallet = 'specific'", "wallet = 'missing'"),
            "unknown wallet",
        ),
        (
            text.replace("wallet = 'specific'", "wallet = ''"),
            "unknown wallet",
        ),
        (text.replace("wallet = 'specific'", "wallet = 1"), "string"),
        (
            text.replace("wallet = \"shared\"\n", ""),
            "wallet required on source or server",
        ),
        (
            text.replace("wallet = \"shared\"", "wallet = 'missing'"),
            "unknown wallet",
        ),
    ] {
        write_config(dir.path(), &bad);
        let error = Deployment::show_config(&path).await.unwrap_err();
        assert!(format!("{error:#}").contains(expected), "{error:#}");
    }
    let source_only = text
        .replace("wallet = \"shared\"\n", "")
        .replace("[sources.alpha]", "[sources.alpha]\nwallet='shared'");
    write_config(dir.path(), &source_only);
    assert!(
        Deployment::show_config(&path).await.unwrap()["servers"]["research"]["wallet"].is_null()
    );
    for file in ["alpha.json", "beta.json"] {
        std::fs::write(
            dir.path().join(file),
            spec("https://example.invalid").to_string(),
        )
        .unwrap();
    }
    let deployment = Deployment::load(&path).await.unwrap();
    let inventory = deployment.inventory();
    assert!(inventory[1].default_wallet.is_none());
    assert_eq!(inventory[1].wallet_bindings["beta"].wallet, "specific");
    // Provider files cannot set identities, even when the source overrides them.
    std::fs::write(
        dir.path().join("provider.toml"),
        "spec='alpha.json'\nwallet='shared'\n",
    )
    .unwrap();
    write_config(
        dir.path(),
        &source_only.replace("spec = \"alpha.json\"", "extends='provider.toml'"),
    );
    assert!(Deployment::show_config(&path).await.is_err());
    assert!(
        x402_treazury::config::load(&dir.path().join("provider.toml"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn source_wallet_overrides_route_real_signatures_across_server_defaults() {
    let (base, _, vendor) = vendor("api", "/api").await;
    let dir = tempfile::tempdir().unwrap();
    for file in ["alpha.json", "beta.json"] {
        std::fs::write(dir.path().join(file), spec(&base).to_string()).unwrap();
    }
    let text = configuration()
        .replace("[sources.beta]", "[sources.beta]\nwallet='specific'")
        .replace("sources = [\"beta\"]", "sources = [\"alpha\", \"beta\"]")
        .replace(
            "include_tools = [\"beta_pay\"]",
            "include_tools = [\"alpha_pay\", \"beta_pay\"]",
        );
    let (research, beta) = text.split_once("[servers.beta_only]").unwrap();
    let text = format!(
        "{research}[servers.beta_only]{}",
        beta.replace("wallet = \"shared\"", "wallet='other'")
    ) + r#"
[wallets.specific]
mode='static'
private_key_env='SOURCE_KEY'
[wallets.other]
mode='static'
private_key_env='OTHER_KEY'
[wallets.unused]
mode='static'
private_key_env='DO_NOT_READ'
[servers.source_only]
listen='127.0.0.1:0'
bearer_token_env='BETA_TOKEN'
sources=['beta']
include_tools=['beta_pay']
[servers.overridden]
listen='127.0.0.1:0'
bearer_token_env='BETA_TOKEN'
wallet='unused'
sources=['beta']
include_tools=['beta_pay']
"#;
    let path = write_config(dir.path(), &text);
    let deployment = Deployment::load(&path).await.unwrap();
    let mut env = env();
    env.insert("SOURCE_KEY".into(), format!("{:064x}", 2));
    env.insert("OTHER_KEY".into(), format!("{:064x}", 3));
    // The overridden default has no environment secret and must not load a signer.
    let running = deployment.bind(&env).await.unwrap();
    let addresses: BTreeMap<_, _> = running.addresses().into_iter().collect();
    let stop = CancellationToken::new();
    let task = tokio::spawn(running.serve(stop.clone()));
    let http = reqwest::Client::new();
    for (server, tool, key) in [
        ("research", "alpha_pay", 1),
        ("research", "beta_pay", 2),
        ("beta_only", "alpha_pay", 3),
        ("beta_only", "beta_pay", 2),
        ("source_only", "beta_pay", 2),
        ("overridden", "beta_pay", 2),
    ] {
        let reply = rpc(
            &http,
            addresses[server],
            if server == "research" {
                "research-secret"
            } else {
                "beta-secret"
            },
            "tools/call",
            json!({"name":tool,"arguments":{}}),
        )
        .await;
        let body: Value =
            serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        let signer: alloy_signer_local::PrivateKeySigner = format!("{key:064x}").parse().unwrap();
        assert_eq!(
            body["payer"].as_str().unwrap().to_lowercase(),
            signer.address().to_string().to_lowercase(),
            "{server}/{tool}"
        );
    }
    stop.cancel();
    task.await.unwrap().unwrap();
    vendor.abort();
}
