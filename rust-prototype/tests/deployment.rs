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
use x402_mcp_prototype::{deployment::Deployment, payment::USDC};

#[derive(Clone)]
struct Vendor {
    name: &'static str,
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
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_x402-mcp-prototype"))
        .args(["--meta-config", path.to_str().unwrap(), "--list-tools"])
        .env_clear()
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
            "only static",
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
async fn imported_configs_keep_overrides_and_use_explicit_root() {
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
        dir.path().join("vendor.json"),
        json!({
            "spec":"alpha.json","prefix":"alpha","instructions_text":"Use alpha_pay",
            "overrides":{"alpha_pay":{"description":"Authored instructions"}},
            "help_url":"http://127.0.0.1:1/llms.txt","probe_pricing":false
        })
        .to_string(),
    )
    .unwrap();
    let text = configuration()
        .replace("version = 1", "version = 1\nroot = \"..\"")
        .replacen("spec = \"alpha.json\"", "config = \"vendor.json\"", 1);
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
    let invalid = text.replace(
        "config = \"vendor.json\"",
        "config = \"vendor.json\"\nspec = \"alpha.json\"",
    );
    assert!(
        Deployment::load(&write_config(&dir.path().join("config"), &invalid))
            .await
            .is_err()
    );
}
