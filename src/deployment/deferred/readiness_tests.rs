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

#[tokio::test]
async fn selector_warnings_wait_for_own_catalogs_and_do_not_repeat_on_unrelated_results() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _logging = tracing::subscriber::set_default(subscriber);
    let fixtures = Fixture::new(
        &(listener(
            "waiting",
            "['local','slow']",
            "include_tools=['local_*','slow_*','slow_missing*']",
        ) + &listener(
            "eager",
            "['local','slow']",
            "wait_for_catalogs=false\ninclude_tools=['local_*','slow_*','slow_missing*']",
        ) + &listener(
            "failed",
            "['local','optional']",
            "include_tools=['local_*','optional_read','optional_missing*']",
        ) + &listener(
            "local",
            "['local']",
            "include_tools=['local_*','local_missing*']",
        )),
        true,
    )
    .await;
    let initial = std::fs::read_to_string(log.path()).unwrap();
    assert!(!initial.contains("selector=\"slow_missing*\""), "{initial}");
    assert!(
        !initial.contains("selector=\"optional_missing*\""),
        "{initial}"
    );
    assert_eq!(
        initial.matches("selector=\"local_missing*\"").count(),
        1,
        "{initial}"
    );
    let addresses: BTreeMap<_, _> = fixtures.running.addresses().into_iter().collect();
    let stop = CancellationToken::new();
    let serving = tokio::spawn(fixtures.running.serve(stop.clone()));
    // failure/optional are aliases of one download; slow owns the other slot.
    tokio::time::timeout(Duration::from_secs(5), fixtures.arrived.acquire_many(2))
        .await
        .unwrap()
        .unwrap()
        .forget();
    fixtures.slow.add_permits(1);
    tokio::time::timeout(
        Duration::from_secs(5),
        rpc(addresses["waiting"], "initialize"),
    )
    .await
    .unwrap();
    let after_slow = std::fs::read_to_string(log.path()).unwrap();
    assert_eq!(
        after_slow.matches("selector=\"slow_missing*\"").count(),
        2,
        "{after_slow}"
    );
    assert!(!after_slow.contains("selector=\"optional_missing*\""));
    fixtures.failure.add_permits(1);
    tokio::time::timeout(
        Duration::from_secs(5),
        rpc(addresses["failed"], "initialize"),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let logs = std::fs::read_to_string(log.path()).unwrap();
            if logs.lines().any(|line| {
                line.contains("catalog_unavailable") && line.contains("source=\"failure\"")
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    finish(stop, serving).await;
    let logs = std::fs::read_to_string(log.path()).unwrap();
    assert_eq!(
        logs.matches("selector=\"slow_missing*\"").count(),
        2,
        "{logs}"
    );
    assert_eq!(
        logs.matches("selector=\"optional_missing*\"").count(),
        1,
        "{logs}"
    );
    assert_eq!(
        logs.matches("selector=\"local_missing*\"").count(),
        1,
        "{logs}"
    );
    assert!(!logs.contains("selector=\"optional_read\""), "{logs}");
    assert!(!logs.contains("Tool pattern matches nothing"), "{logs}");
    fixtures.origin.abort();
}
impl Fixture {
    async fn new(listeners: &str, remote: bool) -> Self {
        Self::with_failure_document(listeners, remote, None).await
    }

    async fn with_failure_document(listeners: &str, remote: bool, document: Option<Value>) -> Self {
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
                let document = document.clone();
                async move {
                    assert!(!headers.contains_key("payment-signature"));
                    assert!(!headers.contains_key("x-payment"));
                    arrived.add_permits(1);
                    gate.acquire().await.unwrap().forget();
                    if uri.path() == "/slow" {
                        Json(json!({"operations":[{"method":"GET","path":"/read"}]}))
                            .into_response()
                    } else if let Some(document) = document {
                        Json(document).into_response()
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
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _logging = tracing::subscriber::set_default(subscriber);
    let fixtures = Fixture::new(
        &(listener("wait", "['local','slow']", "")
            + &listener("eager", "['local','slow']", "wait_for_catalogs=false")
            + &listener("local", "['local']", "")),
        true,
    )
    .await;
    let initial_logs = std::fs::read_to_string(log.path()).unwrap();
    assert!(initial_logs.contains("Catalogs are still loading; each waiting MCP endpoint will start when its required catalogs are ready"), "{initial_logs}");
    assert!(
        !initial_logs.contains("Initial catalog loading finished"),
        "{initial_logs}"
    );
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
    assert!(logs.contains("failure_stage=\"headers\""));
    assert!(logs.contains("http_status=502"));
    assert!(logs.contains("The catalog server returned an unsuccessful HTTP status"));
    assert!(
        !logs
            .lines()
            .any(|line| line.contains("MCP listening") && line.contains("server=\"broken\""))
    );
    fixtures.origin.abort();
}

#[tokio::test]
async fn generation_and_listener_failures_explain_why_a_readable_catalog_is_unavailable() {
    for (document, settings, stage, reason) in [
        (
            json!({"paths":{"/read":{"get":{
                "description":"private upstream credential text",
                "parameters":[{"name":"Cookie","in":"header"}]
            }}}}),
            "",
            "generation",
            "API header is reserved",
        ),
        (
            json!({"operations":[{"path":"/read","method":"GET"}]}),
            "include_tools=['failure_missing']",
            "selection",
            "configured listeners",
        ),
    ] {
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        let _logging = tracing::subscriber::set_default(subscriber);
        let fixtures = Fixture::with_failure_document(
            &(listener("broken", "['failure']", settings) + &listener("healthy", "['local']", "")),
            true,
            Some(document),
        )
        .await;
        let addresses: BTreeMap<_, _> = fixtures.running.addresses().into_iter().collect();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(fixtures.running.serve(stop.clone()));
        fixtures.failure.add_permits(1);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let logs = std::fs::read_to_string(log.path()).unwrap();
                if logs
                    .lines()
                    .any(|line| line.contains("ERROR") && line.contains("server=\"broken\""))
                {
                    assert!(
                        logs.contains(&format!("failure_stage=\"{stage}\"")),
                        "{logs}"
                    );
                    assert!(logs.contains(reason), "{logs}");
                    assert!(!logs.contains("private upstream credential text"));
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(TcpListener::bind(addresses["broken"]).await.unwrap());
        let listed = rpc(addresses["healthy"], "tools/list").await;
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 1);
        finish(stop, serving).await;
        fixtures.origin.abort();
    }
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
async fn independent_capability_endpoints_start_without_pending_catalogs() {
    let fixtures = Fixture::new(
        &(listener("local", "['local']", "")
            + &listener("management", "[]", "provider_management_remote=true")
            + &listener("bundled", "[]", "provider_management_local=true")
            + &listener("raw", "[]", "raw_url_calls=true")
            + &listener("discovery", "[]", "discover_on_demand=true")),
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
            let expected = match name.as_str() {
                "management" | "bundled" => 4,
                "raw" => 1,
                "discovery" => 2,
                _ => panic!("unexpected fixture listener"),
            };
            assert_eq!(tools.len(), expected);
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
        if !panic {
            let logs = std::fs::read_to_string(log.path()).unwrap();
            assert!(logs.contains("failure_stage=\"publication\""), "{logs}");
            assert!(logs.contains("previous tool inventory"), "{logs}");
        }
        fixtures.origin.abort();
    }
}
