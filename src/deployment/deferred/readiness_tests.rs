//! Per-listener startup uses real HTTP, controlled catalogs and unfunded keys.
use super::*;
use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::get};
use serde_json::{Value, json};
use tokio::sync::Semaphore;

struct Fixture {
    _dir: tempfile::TempDir,
    running: RunningDeployment,
    slow: Arc<Semaphore>,
    failure: Arc<Semaphore>,
    arrived: Arc<Semaphore>,
    origin: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn new(listeners: &str, remote: bool) -> Self {
        let slow = Arc::new(Semaphore::new(0));
        let failure = Arc::new(Semaphore::new(0));
        let arrived = Arc::new(Semaphore::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().fallback(get({
            let slow = slow.clone();
            let failure = failure.clone();
            let arrived = arrived.clone();
            move |uri: axum::http::Uri, headers: axum::http::HeaderMap| {
                let gate = if uri.path() == "/slow" {
                    slow.clone()
                } else {
                    failure.clone()
                };
                let arrived = arrived.clone();
                async move {
                    assert!(!headers.contains_key("payment-signature"));
                    assert!(!headers.contains_key("x-payment"));
                    arrived.add_permits(1);
                    gate.acquire().await.unwrap().forget();
                    if uri.path() == "/slow" {
                        Json(json!({"operations":[{"method":"GET","path":"/read"}]}))
                            .into_response()
                    } else {
                        (StatusCode::BAD_GATEWAY, "private upstream credential text")
                            .into_response()
                    }
                }
            }
        }));
        let origin = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("local.json"),
            r#"{"operations":[{"method":"GET","path":"/read"}]}"#,
        )
        .unwrap();
        let mut config = format!(
            "version=1\n[wallets.w]\nmode='static'\nprivate_key_env='KEY'\n[sources.local]\nspec='local.json'\nbase_url='{base}'\n"
        );
        if remote {
            for (id, path, optional) in [
                ("slow", "slow", false),
                ("failure", "failure", false),
                ("optional", "failure", true),
            ] {
                config.push_str(&format!(
                    "[sources.{id}]\nspec='{base}/{path}'\nbase_url='{base}'\noptional_startup={optional}\n"
                ));
            }
        }
        config.push_str(listeners);
        let path = dir.path().join("deployment.toml");
        std::fs::write(&path, config).unwrap();
        let env = BTreeMap::from([("KEY".into(), format!("{:064x}", 2))]);
        let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
            .await
            .unwrap();
        let running = deployment.bind(&env).await.unwrap();
        Self {
            _dir: dir,
            running,
            slow,
            failure,
            arrived,
            origin,
        }
    }
}

fn listener(name: &str, sources: &str, settings: &str) -> String {
    format!(
        "[servers.{name}]\nlisten='127.0.0.1:0'\nauth=false\nwallet='w'\nsources={sources}\n{settings}\n"
    )
}

async fn rpc(address: SocketAddr, method: &str) -> Value {
    let url = format!("http://{address}/mcp");
    let params = if method == "initialize" {
        json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}})
    } else {
        json!({})
    };
    let response = crate::network::global()
        .http(&crate::network::IsolationId::bootstrap(), &url, None)
        .unwrap()
        .post(url)
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    response.mcp_json().await.unwrap()
}

async fn finish(stop: CancellationToken, serving: tokio::task::JoinHandle<Result<()>>) {
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn default_waits_for_own_catalog_publication_while_peers_serve() {
    let fixtures = Fixture::new(
        &(listener("wait", "['local','slow']", "")
            + &listener("eager", "['local','slow']", "wait_for_catalogs=false")
            + &listener("local", "['local']", "")),
        true,
    )
    .await;
    let addresses: BTreeMap<_, _> = fixtures.running.addresses().into_iter().collect();
    assert_eq!(fixtures.running.catalog_readiness.len(), 2);
    let stop = CancellationToken::new();
    let serving = tokio::spawn(fixtures.running.serve(stop.clone()));
    tokio::time::timeout(Duration::from_secs(5), fixtures.arrived.acquire_many(2))
        .await
        .unwrap()
        .unwrap()
        .forget();
    for name in ["eager", "local"] {
        let listed =
            tokio::time::timeout(Duration::from_secs(5), rpc(addresses[name], "tools/list"))
                .await
                .unwrap();
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 1);
    }
    let mut initializing = tokio::spawn(rpc(addresses["wait"], "initialize"));
    let mut listing = tokio::spawn(rpc(addresses["wait"], "tools/list"));
    assert!(
        tokio::time::timeout(Duration::from_millis(75), &mut initializing)
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(75), &mut listing)
            .await
            .is_err()
    );
    fixtures.slow.add_permits(1);
    let initialized = tokio::time::timeout(Duration::from_secs(5), initializing)
        .await
        .unwrap()
        .unwrap();
    assert!(initialized.get("error").is_none(), "{initialized}");
    let listed = tokio::time::timeout(Duration::from_secs(5), listing)
        .await
        .unwrap()
        .unwrap();
    let names: Vec<_> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["local_read", "slow_read"]);
    assert_eq!(
        fixtures.failure.available_permits(),
        0,
        "unused source must not gate listener"
    );
    finish(stop, serving).await;
    fixtures.origin.abort();
}

#[tokio::test]
async fn required_failure_logs_error_and_closes_only_dependent_endpoint() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _logging = tracing::subscriber::set_default(subscriber);
    let fixtures = Fixture::new(
        &(listener("broken", "['local','failure']", "")
            + &listener("optional", "['local','optional']", "")
            + &listener("eager", "['local','failure']", "wait_for_catalogs=false")
            + &listener("healthy", "['local']", "")),
        true,
    )
    .await;
    let addresses: BTreeMap<_, _> = fixtures.running.addresses().into_iter().collect();
    let stop = CancellationToken::new();
    let serving = tokio::spawn(fixtures.running.serve(stop.clone()));
    fixtures.failure.add_permits(1);
    let optional = tokio::time::timeout(
        Duration::from_secs(5),
        rpc(addresses["optional"], "initialize"),
    )
    .await
    .unwrap();
    assert!(
        optional["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("Source optional: UNAVAILABLE")
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let logs = std::fs::read_to_string(log.path()).unwrap();
            if logs.lines().any(|line| {
                line.contains("ERROR")
                    && line.contains("server=\"broken\"")
                    && line.contains("required catalog unavailable: failure")
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(TcpListener::bind(addresses["broken"]).await.unwrap());
    for name in ["healthy", "eager", "optional"] {
        let listed =
            tokio::time::timeout(Duration::from_secs(5), rpc(addresses[name], "tools/list"))
                .await
                .unwrap();
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 1);
    }
    assert!(!serving.is_finished());
    finish(stop, serving).await;
    let logs = std::fs::read_to_string(log.path()).unwrap();
    assert!(!logs.contains("private upstream credential text"));
    assert!(
        !logs
            .lines()
            .any(|line| line.contains("MCP listening") && line.contains("server=\"broken\""))
    );
    fixtures.origin.abort();
}

#[tokio::test]
async fn shutdown_while_waiting_releases_sockets_without_readiness_errors() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _logging = tracing::subscriber::set_default(subscriber);
    let fixtures = Fixture::new(&listener("wait", "['slow']", ""), true).await;
    let address = fixtures.running.addresses()[0].1;
    let stop = CancellationToken::new();
    let serving = tokio::spawn(fixtures.running.serve(stop.clone()));
    tokio::time::timeout(Duration::from_secs(5), fixtures.arrived.acquire_many(2))
        .await
        .unwrap()
        .unwrap()
        .forget();
    finish(stop, serving).await;
    assert!(
        !std::fs::read_to_string(log.path())
            .unwrap()
            .contains("ERROR")
    );
    drop(TcpListener::bind(address).await.unwrap());
    fixtures.origin.abort();
}

#[tokio::test]
async fn local_only_and_management_only_endpoints_start_without_pending_catalogs() {
    let fixtures = Fixture::new(
        &(listener("local", "['local']", "")
            + &listener("management", "[]", "source_management=true")),
        false,
    )
    .await;
    let addresses = fixtures.running.addresses();
    let stop = CancellationToken::new();
    let serving = tokio::spawn(fixtures.running.serve(stop.clone()));
    for (name, address) in addresses {
        let listed = tokio::time::timeout(Duration::from_secs(5), rpc(address, "tools/list"))
            .await
            .unwrap();
        let tools = listed["result"]["tools"].as_array().unwrap();
        if name == "local" {
            assert_eq!(tools.len(), 1);
            assert_eq!(tools[0]["name"], "local_read");
        } else {
            assert_eq!(tools.len(), 7);
            assert!(
                tools
                    .iter()
                    .all(|tool| tool["name"].as_str().unwrap().starts_with("x402_treazury_"))
            );
        }
    }
    finish(stop, serving).await;
    fixtures.origin.abort();
}

#[tokio::test]
async fn worker_or_publication_failure_closes_waiters_and_keeps_ready_peers() {
    for panic in [false, true] {
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        let _logging = tracing::subscriber::set_default(subscriber);
        let mut fixtures = Fixture::new(
            &(listener("wait", "['slow']", "") + &listener("healthy", "['local']", "")),
            true,
        )
        .await;
        let runtime = fixtures.running.deferred.as_mut().unwrap();
        let failed_catalog = if panic {
            runtime.catalog.clone()
        } else {
            let mut snapshot = (*runtime.catalog.read()).clone();
            snapshot.generation = u64::MAX;
            runtime.catalog = Arc::new(crate::catalog_state::CatalogState::new(snapshot));
            runtime.catalog.clone()
        };
        let (fail, failed) = tokio::sync::oneshot::channel();
        if panic {
            runtime.failure = Some(failed);
        }
        let addresses: BTreeMap<_, _> = fixtures.running.addresses().into_iter().collect();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(fixtures.running.serve(stop.clone()));
        tokio::time::timeout(
            Duration::from_secs(5),
            rpc(addresses["healthy"], "initialize"),
        )
        .await
        .unwrap();
        if panic {
            fail.send(()).unwrap();
        } else {
            fixtures.slow.add_permits(1);
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let logs = std::fs::read_to_string(log.path()).unwrap();
                if logs
                    .lines()
                    .any(|line| line.contains("ERROR") && line.contains("server=\"wait\""))
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(failed_catalog.publication_failed());
        drop(TcpListener::bind(addresses["wait"]).await.unwrap());
        let listed = tokio::time::timeout(
            Duration::from_secs(5),
            rpc(addresses["healthy"], "tools/list"),
        )
        .await
        .unwrap();
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 1);
        stop.cancel();
        let result = tokio::time::timeout(Duration::from_secs(5), serving)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.is_err(), panic);
        fixtures.origin.abort();
    }
}
