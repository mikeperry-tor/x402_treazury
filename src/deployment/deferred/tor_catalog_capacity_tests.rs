//! Exercise the actual catalog schedulers, rather than bypassing their gates.
use super::*;
use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse};
use std::{sync::Mutex, time::Duration};

#[derive(Clone)]
struct Origin {
    calls: Arc<Mutex<BTreeMap<String, usize>>>,
    rejected: Arc<tokio::sync::Barrier>,
    changed: Arc<tokio::sync::Notify>,
}
impl Origin {
    async fn seen(&self, path: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self.calls.lock().unwrap().contains_key(path) {
                    return;
                }
                changed.await;
            }
        })
        .await
        .expect("healthy catalog queued behind Retry-After sleepers");
    }
}
async fn origin(
    State(state): State<Origin>,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    assert!(!headers.contains_key("payment-signature"));
    assert!(!headers.contains_key("x-payment"));
    let path = uri.path().to_owned();
    *state.calls.lock().unwrap().entry(path.clone()).or_default() += 1;
    state.changed.notify_waiters();
    if path.ends_with("/a") || path.ends_with("/b") {
        // Both distinct isolation slots are occupied before either rejected
        // response begins its indefinite server-directed wait.
        state.rejected.wait().await;
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [("retry-after", "18446744073709551615")],
            "unused",
        )
            .into_response();
    }
    if path == "/read" {
        return "healthy".into_response();
    }
    Json(serde_json::json!({"operations":[{"method":"GET","path":"/read"}]})).into_response()
}
fn config(mode: &str, socks: std::net::SocketAddr) -> String {
    // Isolate catalog scheduling: an invoked tool's optional Tor cover ranges
    // can otherwise use the catalog URL and inflate these request counts.
    let mut config = format!(
        "version=1\n[network]\nmode='tor'\nsocks_endpoint='{socks}'\ncover_traffic_enabled=false\n[startup]\ncatalog_concurrency=2\n[wallets.w]\nmode='static'\nprivate_key_env='KEY'\n"
    );
    for id in ["a", "b", "z"] {
        config.push_str(&format!(
            "[sources.{id}]\nspec='https://{id}.example.com/{mode}/{id}'\nbase_url='https://{id}.example.com'\n"
        ));
    }
    config.push_str("[servers.one]\nlisten='127.0.0.1:0'\nbearer_token_env='TOKEN'\nwallet='w'\nsources=['a','b','z']\nwait_for_catalogs=false\n");
    config
}

#[tokio::test]
async fn retry_waits_release_catalog_slots_in_serving_inspection_and_warming() {
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args([
            "--ignored",
            "--exact",
            "deployment::deferred::tor_catalog_capacity_tests::catalog_capacity_child",
            "--nocapture",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[tokio::test]
#[ignore = "isolated process network policy; invoked by parent"]
async fn catalog_capacity_child() {
    let state = Origin {
        calls: Default::default(),
        rejected: Arc::new(tokio::sync::Barrier::new(2)),
        changed: Default::default(),
    };
    let listener = crate::test_tls::listener().await;
    let address = listener.local_addr().unwrap();
    let app = Router::new().fallback(origin).with_state(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let socks = crate::test_socks::Socks::start(
        BTreeMap::from([
            ("a.example.com".into(), address),
            ("b.example.com".into(), address),
            ("z.example.com".into(), address),
        ]),
        crate::test_socks::Fault::None,
    )
    .await;
    let env = BTreeMap::from([
        ("KEY".into(), format!("{:064x}", 2)),
        ("TOKEN".into(), "fixture".into()),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");

    // Ordinary serving publishes the queued healthy source while both rejected
    // catalogs retain their operation owners. Its tool is actually callable.
    std::fs::write(&path, config("serving", socks.address)).unwrap();
    let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
        .await
        .unwrap();
    assert_eq!(deployment.pending_sources.len(), 3);
    let running = deployment.bind(&env).await.unwrap();
    let catalog = running.deferred.as_ref().unwrap().catalog.clone();
    let stop = CancellationToken::new();
    let serving = tokio::spawn(running.serve(stop.clone()));
    state.seen("/serving/z").await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while catalog.read().views["one"].len() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let snapshot = catalog.read();
    assert!(
        snapshot.instructions["one"]
            .as_ref()
            .unwrap()
            .contains("PENDING")
    );
    assert!(snapshot.views["one"][0].tool.name.starts_with("z_"));
    assert_eq!(
        snapshot.views["one"][0]
            .invoke(&Default::default())
            .await
            .unwrap(),
        "healthy"
    );
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Strict inspection still waits for completeness, but its logical queue must
    // let the healthy source download. Warming uses the same scope capacity.
    for mode in ["inspection", "warm"] {
        let text = config(mode, socks.address);
        std::fs::write(&path, &text).unwrap();
        let operation = if mode == "inspection" {
            let path = path.clone();
            tokio::spawn(async move { Deployment::load(&path).await.map(|_| ()) })
        } else {
            let cfg = toml::from_str(&text).unwrap();
            let path = path.clone();
            tokio::spawn(async move {
                startup::warm(
                    &cfg,
                    &path,
                    &BTreeSet::from(["a".into(), "b".into(), "z".into()]),
                    false,
                    Default::default(),
                )
                .await
                .map(|_| ())
            })
        };
        state.seen(&format!("/{mode}/z")).await;
        assert!(
            !operation.is_finished(),
            "strict work still requires delayed catalogs"
        );
        operation.abort();
        assert!(operation.await.unwrap_err().is_cancelled());
    }
    for mode in ["serving", "inspection", "warm"] {
        for id in ["a", "b", "z"] {
            assert_eq!(
                state.calls.lock().unwrap()[&format!("/{mode}/{id}")],
                1,
                "server waits and cancellation grant no later dispatch"
            );
        }
    }
    server.abort();
}

#[derive(Clone, Default)]
struct HeldBody {
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

impl HeldBody {
    fn response(&self, document: String) -> axum::body::Body {
        use axum::body::Bytes;
        use futures_util::stream;
        let (started, release) = (self.started.clone(), self.release.clone());
        // Headers and a real body chunk arrive before the remainder is held.
        let chunks = stream::once(async move {
            started.notify_one();
            Ok::<_, std::io::Error>(Bytes::from_static(b" "))
        })
        .chain(stream::once(async move {
            release.notified().await;
            Ok::<_, std::io::Error>(Bytes::from(document))
        }));
        axum::body::Body::from_stream(chunks)
    }
}

#[tokio::test]
async fn pricing_and_deferred_catalogs_share_actual_socks_scope_capacity() {
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args([
            "--ignored",
            "--exact",
            "deployment::deferred::tor_catalog_capacity_tests::shared_scope_capacity_child",
            "--nocapture",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[tokio::test]
#[ignore = "isolated process network policy; invoked by parent"]
async fn shared_scope_capacity_child() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let bodies: BTreeMap<_, _> = ["/prices", "/same-spec", "/other-spec"]
            .into_iter()
            .map(|path| (path.to_owned(), HeldBody::default()))
            .collect();
        let calls = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
        let schema = serde_json::json!({"operations":[{"method":"GET","path":"/read"}]});
        let prices = serde_json::json!({"x402Version":2,"items":[{
            "resource":"https://catalog.example.com/read",
            "accepts":[{"scheme":"exact","network":"eip155:8453",
                "asset":crate::payment::USDC,"amount":"1000"}]
        }]})
        .to_string();
        let app = Router::new().fallback({
            let (bodies, calls, schema) = (bodies.clone(), calls.clone(), schema.clone());
            move |uri: axum::http::Uri, headers: axum::http::HeaderMap| {
                let (bodies, calls, schema, prices) = (
                    bodies.clone(),
                    calls.clone(),
                    schema.clone(),
                    prices.clone(),
                );
                async move {
                    assert!(!headers.contains_key("payment-signature"));
                    assert!(!headers.contains_key("x-payment"));
                    let path = uri.path();
                    *calls.lock().unwrap().entry(path.into()).or_default() += 1;
                    bodies[path].response(if path == "/prices" {
                        prices
                    } else {
                        schema.to_string()
                    })
                }
            }
        });
        // HTTP/1 forces the concurrent held bodies onto separate connections,
        // making both actual SOCKS credential disclosures observable.
        let (address, server) = crate::test_tls::serve_with(
            app,
            &[&rustls::version::TLS13, &rustls::version::TLS12],
            &[b"http/1.1"],
        )
        .await;
        let socks = crate::test_socks::Socks::start(
            BTreeMap::from([
                ("catalog.example.com".into(), address),
                ("other.example.com".into(), address),
            ]),
            crate::test_socks::Fault::None,
        )
        .await;
        let env = BTreeMap::from([
            ("KEY".into(), format!("{:064x}", 2)),
            ("TOKEN".into(), "fixture".into()),
        ]);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("local.json"), schema.to_string()).unwrap();
        let path = dir.path().join("deployment.toml");
        std::fs::write(
            &path,
            format!(
                r#"
version=1
[network]
mode='tor'
socks_endpoint='{}'
cover_traffic_enabled=false
[startup]
catalog_concurrency=1
[wallets.w]
mode='static'
private_key_env='KEY'
[sources.a]
allow_http1=true
spec='local.json'
base_url='https://catalog.example.com'
[sources.a.pricing]
source='https://catalog.example.com/prices'
format='x402-discovery'
[sources.b]
allow_http1=true
spec='https://catalog.example.com/same-spec'
base_url='https://catalog.example.com'
[sources.c]
allow_http1=true
spec='https://other.example.com/other-spec'
base_url='https://other.example.com'
[servers.one]
listen='127.0.0.1:0'
bearer_token_env='TOKEN'
wallet='w'
sources=['a','b','c']
wait_for_catalogs=false
"#,
                socks.address
            ),
        )
        .unwrap();
        let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
            .await
            .unwrap();
        assert_eq!(deployment.pending_sources.len(), 2);
        let price = deployment.sources["a"].tools[0].advertised_pricing.clone();
        let gate = deployment.catalog_capacity.clone();
        bodies["/prices"].started.notified().await;
        assert!(price.amount().is_none());
        assert_eq!(gate.available_permits(), 0);
        let running = deployment.bind(&env).await.unwrap();
        let catalog = running.deferred.as_ref().unwrap().catalog.clone();
        let stop = CancellationToken::new();
        let serving = tokio::spawn(running.serve(stop.clone()));
        bodies["/same-spec"].started.notified().await;
        // Observe the distinct scope's actual pending admission, without a sleep
        // or an assumption about when the deferred worker was scheduled.
        while gate.live_scope_count() != 2 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            *calls.lock().unwrap(),
            BTreeMap::from([("/prices".into(), 1), ("/same-spec".into(), 1),])
        );
        assert_eq!(socks.records.lock().unwrap().len(), 2);
        bodies["/prices"].release.notify_one();
        while price.amount() != Some(alloy_primitives::U256::from(1000)) {
            tokio::task::yield_now().await;
        }
        // Finishing optional metadata cannot release B's full-body ownership.
        assert_eq!(gate.available_permits(), 0);
        assert_eq!(gate.live_scope_count(), 2);
        assert!(!calls.lock().unwrap().contains_key("/other-spec"));
        assert_eq!(socks.records.lock().unwrap().len(), 2);
        bodies["/same-spec"].release.notify_one();
        bodies["/other-spec"].started.notified().await;
        assert_eq!(gate.available_permits(), 0);
        {
            let records = socks.records.lock().unwrap();
            assert_eq!(records.len(), 3);
            assert_eq!(records[0].host, "catalog.example.com");
            assert_eq!(records[1].host, "catalog.example.com");
            assert_eq!(records[2].host, "other.example.com");
            assert_eq!(records[0].user, records[1].user);
            assert_eq!(records[0].password, records[1].password);
            assert_ne!(
                (&records[0].user, &records[0].password),
                (&records[2].user, &records[2].password)
            );
            assert!(records.iter().all(|record| record.address_type == 3));
        }
        bodies["/other-spec"].release.notify_one();
        while catalog.read().views["one"].len() != 3 {
            tokio::task::yield_now().await;
        }
        assert!(
            !catalog.read().instructions["one"]
                .as_ref()
                .is_some_and(|instructions| instructions.contains("PENDING"))
        );
        assert_eq!(gate.available_permits(), 1);
        assert_eq!(gate.live_scope_count(), 0);
        assert_eq!(
            *calls.lock().unwrap(),
            BTreeMap::from([
                ("/prices".into(), 1),
                ("/same-spec".into(), 1),
                ("/other-spec".into(), 1),
            ])
        );
        stop.cancel();
        serving.await.unwrap().unwrap();
        server.abort();
    })
    .await
    .expect("Tor discovery scope fixture did not make progress");
}
