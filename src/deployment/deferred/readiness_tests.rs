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
        .with_max_level(tracing::Level::DEBUG)
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
        Self::with_distribution(listeners, remote, document, &BTreeMap::new()).await
    }

    async fn with_distribution(
        listeners: &str,
        remote: bool,
        document: Option<Value>,
        distribution: &BTreeMap<&str, &str>,
    ) -> Self {
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
        if let Some(send_tools) = distribution.get("local") {
            config.push_str(&format!("send_tools={send_tools}\n"));
        }
        if remote {
            for (id, path, optional) in [
                ("slow", "slow", false),
                ("failure", "failure", false),
                ("optional", "failure", true),
            ] {
                config.push_str(&format!(
                    "[sources.{id}]\nspec='{base}/{path}'\nbase_url='{base}'\noptional_startup={optional}\n"
                ));
                if let Some(send_tools) = distribution.get(id) {
                    config.push_str(&format!("send_tools={send_tools}\n"));
                }
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
    let params = if method == "initialize" {
        json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}})
    } else {
        json!({})
    };
    rpc_params(address, method, params).await
}

async fn rpc_params(address: SocketAddr, method: &str, params: Value) -> Value {
    let url = format!("http://{address}/mcp");
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

#[tokio::test]
async fn routed_shadowed_destination_waits_without_blocking_unrelated_listener() {
    let fixtures = Fixture::with_distribution(
        &(listener("waiting", "['local']", "")
            + &listener("eager", "['local']", "wait_for_catalogs=false")
            + &listener("healthy", "['local']", "")),
        true,
        None,
        &BTreeMap::from([(
            "slow",
            "[{to='eager',tools=['*']},{to='waiting',tools=['future_*']},{to='exclude'}]",
        )]),
    )
    .await;
    let addresses: BTreeMap<_, _> = fixtures.running.addresses().into_iter().collect();
    let stop = CancellationToken::new();
    let serving = tokio::spawn(fixtures.running.serve(stop.clone()));
    tokio::time::timeout(Duration::from_secs(5), fixtures.arrived.acquire_many(2))
        .await
        .unwrap()
        .unwrap()
        .forget();
    for name in ["eager", "healthy"] {
        let listed =
            tokio::time::timeout(Duration::from_secs(5), rpc(addresses[name], "tools/list"))
                .await
                .unwrap();
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 1);
    }
    let mut waiting = tokio::spawn(rpc(addresses["waiting"], "tools/list"));
    assert!(
        tokio::time::timeout(Duration::from_millis(75), &mut waiting)
            .await
            .is_err()
    );
    fixtures.slow.add_permits(1);
    let listed = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap();
    let names = |value: &Value| {
        value["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&listed), ["local_read"]);
    assert_eq!(
        names(&rpc(addresses["eager"], "tools/list").await),
        ["local_read", "slow_read"]
    );
    for name in ["waiting", "healthy"] {
        let rejected = rpc_params(
            addresses[name],
            "tools/call",
            json!({"name":"slow_read","arguments":{}}),
        )
        .await;
        assert!(
            rejected.get("error").is_some() || rejected["result"]["isError"] == true,
            "{rejected}"
        );
    }
    finish(stop, serving).await;
    fixtures.origin.abort();
}

#[tokio::test]
async fn empty_routed_destinations_do_not_discard_a_siblings_catalog() {
    for remote in [false, true] {
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        let _logging = tracing::subscriber::set_default(subscriber);
        let source = if remote { "slow" } else { "local" };
        let fixtures = Fixture::with_distribution(
            &(listener("broken", "[]", "")
                + &listener("eager", "[]", "wait_for_catalogs=false")
                + &listener("healthy", "[]", "")),
            remote,
            None,
            &BTreeMap::from([(
                source,
                "[{to=['broken','eager'],tools=['future_*']},{to='healthy'}]",
            )]),
        )
        .await;
        if !remote {
            let strict_error = Deployment::load(&fixtures._dir.path().join("deployment.toml"))
                .await
                .err()
                .unwrap();
            assert!(format!("{strict_error:#}").contains("no tools selected"));
        }
        let catalog = fixtures.running.deferred.as_ref().unwrap().catalog.clone();
        let addresses: BTreeMap<_, _> = fixtures.running.addresses().into_iter().collect();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(fixtures.running.serve(stop.clone()));
        if remote {
            fixtures.slow.add_permits(1);
        }
        let listed = tokio::time::timeout(
            Duration::from_secs(5),
            rpc(addresses["healthy"], "tools/list"),
        )
        .await
        .unwrap();
        assert_eq!(
            listed["result"]["tools"][0]["name"],
            format!("{source}_read")
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let initialized = rpc(addresses["eager"], "initialize").await;
                if initialized["result"]["instructions"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("The endpoint has no selected tools or discovery capabilities")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(catalog.read().views["eager"].is_empty());
        assert_eq!(catalog.read().views["healthy"].len(), 1);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(socket) = TcpListener::bind(addresses["broken"]).await {
                    drop(socket);
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // A different pending source's failure must not repeat the eager failure.
        if remote {
            let generation = catalog.read().generation;
            fixtures.failure.add_permits(1);
            tokio::time::timeout(Duration::from_secs(5), async {
                while catalog.read().generation == generation {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        finish(stop, serving).await;
        let logs = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(
            logs.lines()
                .filter(|line| line.contains("MCP endpoint tools are unavailable")
                    && line.contains("server=\"eager\""))
                .count(),
            1,
            "{logs}"
        );
        assert!(
            !logs.contains("required catalog unavailable: slow"),
            "{logs}"
        );
        fixtures.origin.abort();
    }
}

#[tokio::test]
async fn legacy_listener_typo_does_not_reject_a_shared_catalog() {
    let fixtures = Fixture::with_failure_document(
        &(listener("broken", "['failure']", "include_tools=['failure_missing']")
            + &listener("healthy", "['failure']", "")),
        true,
        Some(json!({"operations":[{"method":"GET","path":"/read"}]})),
    )
    .await;
    let catalog = fixtures.running.deferred.as_ref().unwrap().catalog.clone();
    let addresses: BTreeMap<_, _> = fixtures.running.addresses().into_iter().collect();
    let stop = CancellationToken::new();
    let serving = tokio::spawn(fixtures.running.serve(stop.clone()));
    fixtures.failure.add_permits(1);
    let listed = tokio::time::timeout(
        Duration::from_secs(5),
        rpc(addresses["healthy"], "tools/list"),
    )
    .await
    .unwrap();
    assert_eq!(listed["result"]["tools"][0]["name"], "failure_read");
    assert_eq!(catalog.read().views["healthy"].len(), 1);
    assert!(catalog.read().views["broken"].is_empty());
    finish(stop, serving).await;
    fixtures.origin.abort();
}

#[tokio::test]
async fn deferred_send_typo_reports_selection_failure_without_upstream_prose() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _logging = tracing::subscriber::set_default(subscriber);
    let fixtures = Fixture::with_distribution(
        &(listener("broken", "[]", "") + &listener("healthy", "['local']", "")),
        true,
        Some(json!({"operations":[{"method":"GET","path":"/read",
            "description":"private upstream credential text"}]})),
        &BTreeMap::from([(
            "failure",
            "[{to='broken',tools=['private_typo']},{to='exclude'}]",
        )]),
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
                assert!(logs.contains("failure_stage=\"selection\""), "{logs}");
                assert!(
                    logs.contains("Source distribution tool selectors are invalid"),
                    "{logs}"
                );
                assert!(
                    !logs.contains("Could not obtain the catalog document"),
                    "{logs}"
                );
                assert!(
                    !logs.contains("private upstream credential text")
                        && !logs.contains("private_typo"),
                    "{logs}"
                );
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        rpc(addresses["healthy"], "tools/list").await["result"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    finish(stop, serving).await;
    fixtures.origin.abort();
}

#[tokio::test]
async fn routed_catalog_failures_disable_only_required_potential_destinations() {
    let fixtures = Fixture::with_distribution(
        &(listener("broken", "['local']", "")
            + &listener("optional", "['local']", "")
            + &listener("eager", "['local']", "wait_for_catalogs=false")
            + &listener("healthy", "['local']", "")),
        true,
        None,
        &BTreeMap::from([
            ("failure", "[{to='eager',tools=['*']},{to='broken'}]"),
            ("optional", "[{to='optional'}]"),
        ]),
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
            if let Ok(released) = TcpListener::bind(addresses["broken"]).await {
                drop(released);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    for name in ["healthy", "eager", "optional"] {
        let listed =
            tokio::time::timeout(Duration::from_secs(5), rpc(addresses[name], "tools/list"))
                .await
                .unwrap();
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 1);
    }
    assert!(!serving.is_finished());
    finish(stop, serving).await;
    fixtures.origin.abort();
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
            "A listener tool selector matches no assigned tools",
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
            &(listener("wait", "['slow']", "")
                + &listener("healthy", "['local']", "")
                + &listener(
                    "eager",
                    "['slow']",
                    "wait_for_catalogs=false\ninclude_tools=['slow_missing']",
                )),
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
        assert!(
            !std::fs::read_to_string(log.path())
                .unwrap()
                .contains("MCP endpoint tools are unavailable"),
            "an unpublished selection must not announce tool withdrawal"
        );
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

#[tokio::test]
async fn aggregate_catalog_signal_waits_for_eager_sources_and_respects_failures() {
    for (source, optional) in [("slow", false), ("failure", false), ("optional", true)] {
        let mut fixture = Fixture::new(
            &(listener("local", "['local']", "")
                + &listener("eager", &format!("['{source}']"), "wait_for_catalogs=false")),
            true,
        )
        .await;
        let (_, replacement) = tokio::sync::oneshot::channel();
        let mut catalogs = std::mem::replace(&mut fixture.running.startup_catalogs, replacement);
        let addresses: BTreeMap<_, _> = fixture.running.addresses().into_iter().collect();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(fixture.running.serve(stop.clone()));
        tokio::time::timeout(
            Duration::from_secs(5),
            rpc(addresses["eager"], "initialize"),
        )
        .await
        .unwrap();
        assert!(matches!(
            catalogs.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        if source == "slow" {
            fixture.slow.add_permits(1);
        } else {
            fixture.failure.add_permits(1);
        }
        let outcome = tokio::time::timeout(Duration::from_secs(5), catalogs)
            .await
            .unwrap();
        assert_eq!(outcome.is_ok(), source == "slow" || optional);
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), serving)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        fixture.origin.abort();
    }
}
