//! Actual API ownership across scope replacement, cover and retained HTTP/2 work.
use super::{PaidClient, Payer, SpendPolicy};
use crate::{
    catalog::RoutedRequest,
    network::*,
    test_socks::{Fault, Socks},
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Semaphore, mpsc};

const INITIAL_SCOPE: &str = "TREAZURY_API_FIXTURE_INITIAL_SCOPE ";

#[derive(Clone)]
struct Fixture {
    calls: Arc<Mutex<HashMap<String, usize>>>,
    events: mpsc::UnboundedSender<String>,
    held: Arc<Semaphore>,
    late: Arc<Semaphore>,
    evicted: Arc<Semaphore>,
}
async fn endpoint(State(f): State<Fixture>, uri: Uri, headers: HeaderMap) -> Response {
    assert!(!headers.contains_key("payment-signature"));
    let path = uri.path().to_owned();
    let ordinal = {
        let mut calls = f.calls.lock().unwrap();
        let count = calls.entry(path.clone()).or_default();
        *count += 1;
        *count
    };
    if path == "/cover" {
        let raw = headers["range"]
            .to_str()
            .unwrap()
            .strip_prefix("bytes=")
            .unwrap();
        let (a, b) = raw.split_once('-').unwrap();
        let (a, b) = (a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap());
        f.events.send("cover".into()).unwrap();
        return (
            StatusCode::PARTIAL_CONTENT,
            [
                ("content-range", format!("bytes {a}-{b}/16384")),
                ("etag", "\"v1\"".into()),
            ],
            vec![b'a'; b - a + 1],
        )
            .into_response();
    }
    if path == "/hold" || ordinal == 2 {
        assert!(
            headers.contains_key("x-example-padding"),
            "initial and fresh candidate scopes each receive their own padding"
        );
    }
    f.events.send(format!("{path}:{ordinal}")).unwrap();
    if matches!(path.as_str(), "/recover" | "/first" | "/late" | "/evicted") && ordinal == 1 {
        return StatusCode::FORBIDDEN.into_response();
    }
    if path == "/hold" {
        let gate = f.held;
        let stream = futures_util::stream::once(async move {
            gate.acquire().await.unwrap().forget();
            Ok::<_, std::convert::Infallible>(axum::body::Bytes::from_static(
                b"old-stream-finished",
            ))
        });
        return (
            [("content-type", "text/plain")],
            axum::body::Body::from_stream(stream),
        )
            .into_response();
    }
    if path == "/recover" {
        let calls = f.calls;
        let stream = futures_util::stream::once(async move {
            while calls.lock().unwrap().get("/cover").copied().unwrap_or(0) < 4 {
                tokio::task::yield_now().await;
            }
            Ok::<_, std::convert::Infallible>(axum::body::Bytes::from_static(b"ok"))
        });
        return (
            [("content-type", "text/plain")],
            axum::body::Body::from_stream(stream),
        )
            .into_response();
    }
    if path == "/late" {
        f.late.acquire().await.unwrap().forget();
    }
    if path == "/evicted" {
        f.evicted.acquire().await.unwrap().forget();
    }
    ([("content-type", "text/plain")], "ok").into_response()
}
fn route(path: &str) -> RoutedRequest {
    RoutedRequest {
        headers: Default::default(),
        method: "GET".into(),
        url: format!("https://api.example.com{path}"),
        query: BTreeMap::new(),
        body: None,
    }
}
fn selected(id: &IsolationId) -> Arc<HttpTransport> {
    global()
        .http_participating(
            id,
            "https://api.example.com",
            None,
            false,
            Default::default(),
        )
        .unwrap()
}
async fn event(events: &mut mpsc::UnboundedReceiver<String>, wanted: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if events.recv().await.as_deref() == Some(wanted) {
                break;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("missing fixture event {wanted}"));
}
fn fixed(unit: &str, n: usize) -> crate::cover::sampling::Distribution {
    toml::from_str(&format!(
        "distribution='uniform'\nmin_{unit}={n}\nmax_{unit}={n}"
    ))
    .unwrap()
}
#[test]
fn api_cover_http2_races_in_isolated_process() {
    if std::env::var_os("TREAZURY_API_COVER_CHILD").is_none() {
        let mut initial_scopes = Vec::<(String, String)>::new();
        for _ in 0..2 {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .env_clear()
                .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
                .env("TREAZURY_API_COVER_CHILD", "1")
                .args([
                    "--exact",
                    "payment::tor_cover_tests::api_cover_http2_races_in_isolated_process",
                    "--nocapture",
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            let report = stderr
                .lines()
                .find_map(|line| line.strip_prefix(crate::cover::metrics::REPORT_PREFIX))
                .expect("cover report");
            let report: serde_json::Value = serde_json::from_str(report).unwrap();
            assert!(
                report["metrics"]["qualified_ranges"].as_u64().unwrap() >= 2,
                "both original and replacement scopes successfully qualify cover: {report}"
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            let initial_scope = stdout
                .lines()
                .find_map(|line| line.split_once(INITIAL_SCOPE).map(|(_, value)| value))
                .expect("initial API SOCKS scope evidence");
            initial_scopes.push(serde_json::from_str(initial_scope).unwrap());
        }
        assert_eq!(
            initial_scopes[0].0, initial_scopes[1].0,
            "process restart preserves canonical payer identity"
        );
        assert_ne!(
            initial_scopes[0].1, initial_scopes[1].1,
            "first actual API SOCKS acquisition after restart has a fresh scope"
        );
        return;
    }
    tokio::runtime::Runtime::new().unwrap().block_on(fixture());
}
async fn fixture() {
    let (tx, mut events) = mpsc::unbounded_channel();
    let f = Fixture {
        calls: Default::default(),
        events: tx,
        held: Arc::new(Semaphore::new(0)),
        late: Arc::new(Semaphore::new(0)),
        evicted: Arc::new(Semaphore::new(0)),
    };
    let app = axum::Router::new()
        .route("/{*path}", get(endpoint))
        .with_state(f.clone());
    let (address, server) =
        crate::test_tls::serve_with(app, &[&rustls::version::TLS13], &[b"h2"]).await;
    let socks = Socks::start(
        BTreeMap::from([("api.example.com".into(), address)]),
        Fault::None,
    )
    .await;
    install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(socks.address),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let example = include_str!("../../docs/cover-traffic.md")
        .split("```toml\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let table: toml::Table = toml::from_str(example).unwrap();
    let mut config: crate::cover::Config = table["cover_traffic"].clone().try_into().unwrap();
    config.url = "https://api.example.com/cover".into();
    config.start_delay = fixed("ms", 0);
    config.request_gap = fixed("ms", 0);
    config.tail = fixed("ms", 500);
    config.concurrency = 1;
    config.volume = fixed("bytes", 2048);
    config.ranges = fixed("bytes", 1024);
    config.padding.as_mut().unwrap().size = fixed("bytes", 128);
    config.validate().unwrap();
    let payer = Payer::new(&format!("{:064x}", 991), SpendPolicy::dollars("1").unwrap()).unwrap();
    let id = IsolationId::evm(&payer.address).unwrap();
    let client = PaidClient::new(payer).with_cover(
        Some(config),
        crate::cover::Scope {
            listener: "fixture".into(),
            source: "api".into(),
        },
    );
    let original = selected(&id);
    assert_ne!(
        global().credentials(&original.scope),
        global().credentials(&id),
        "API starts with a fresh owner-bound scope"
    );
    let hold = {
        let client = client.clone();
        tokio::spawn(async move { client.execute_response(route("/hold")).await })
    };
    event(&mut events, "/hold:1").await;
    // Public deterministic fixture payer only. Capture the real SOCKS handshake,
    // rather than merely comparing factory metadata across process restarts.
    {
        let records = socks.records.lock().unwrap();
        let first = records.first().unwrap();
        assert_eq!(
            (first.user.clone(), first.password.clone()),
            global().credentials(&original.scope)
        );
        println!(
            "{INITIAL_SCOPE}{}",
            serde_json::to_string(&(&first.user, &first.password)).unwrap()
        );
    }
    // The second range is dispatched only after this scope's qualification was
    // validated. Both scopes have a two-range volume and one stream.
    tokio::time::timeout(Duration::from_secs(5), async {
        while f.calls.lock().unwrap().get("/cover").copied().unwrap_or(0) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("primary scope cover qualification and second range");
    assert_eq!(
        client
            .execute_response(route("/recover"))
            .await
            .unwrap()
            .bytes,
        b"ok"
    );
    let winner = selected(&id);
    assert!(!Arc::ptr_eq(&original, &winner));
    assert!(
        !hold.is_finished(),
        "the old HTTP/2 stream remains active during promotion"
    );
    let records = socks.records.lock().unwrap().clone();
    assert_eq!(
        records.len(),
        2,
        "primary and candidate have distinct physical pools, multiplexing their own cover"
    );
    assert_ne!(records[0].password, records[1].password);
    assert!(
        f.calls.lock().unwrap().get("/cover").copied().unwrap_or(0) >= 2,
        "new scope independently qualifies cover"
    );
    assert_eq!(
        client.execute_response(route("/next")).await.unwrap().bytes,
        b"ok"
    );
    assert!(Arc::ptr_eq(&winner, &selected(&id)));
    assert_eq!(
        socks.records.lock().unwrap().len(),
        2,
        "next API call reuses winner's pool"
    );
    f.held.add_permits(1);
    assert_eq!(hold.await.unwrap().unwrap().bytes, b"old-stream-finished");

    // Both calls start from the same selected entry; late success loses CAS but
    // completes through its own transport with exactly two unsigned requests.
    let late = {
        let client = client.clone();
        tokio::spawn(async move { client.execute_response(route("/late")).await })
    };
    event(&mut events, "/late:2").await;
    assert_eq!(
        client
            .execute_response(route("/first"))
            .await
            .unwrap()
            .bytes,
        b"ok"
    );
    let race_winner = selected(&id);
    assert!(!Arc::ptr_eq(&winner, &race_winner));
    f.late.add_permits(1);
    assert_eq!(late.await.unwrap().unwrap().bytes, b"ok");
    assert!(
        Arc::ptr_eq(&race_winner, &selected(&id)),
        "late API success cannot overwrite winner"
    );
    assert_eq!(f.calls.lock().unwrap()["/late"], 2);
    assert_eq!(f.calls.lock().unwrap()["/first"], 2);

    let evicted = {
        let client = client.clone();
        tokio::spawn(async move { client.execute_response(route("/evicted")).await })
    };
    event(&mut events, "/evicted:2").await;
    for n in 0..260 {
        global()
            .http_transport(
                &IsolationId::treasury(&format!("api-evict-{n}")),
                &format!("http://127.0.0.1:{}", 20000 + n),
                None,
                false,
                Default::default(),
                SelectionPolicy::Legacy,
            )
            .unwrap();
    }
    assert_eq!(
        client
            .execute_response(route("/reinsert"))
            .await
            .unwrap()
            .bytes,
        b"ok"
    );
    let reinserted = selected(&id);
    assert!(!Arc::ptr_eq(&race_winner, &reinserted));
    assert_ne!(
        global().credentials(&race_winner.scope),
        global().credentials(&reinserted.scope)
    );
    f.evicted.add_permits(1);
    assert_eq!(evicted.await.unwrap().unwrap().bytes, b"ok");
    assert!(
        Arc::ptr_eq(&reinserted, &selected(&id)),
        "old API candidate cannot replace reinserted entry"
    );
    assert_eq!(f.calls.lock().unwrap()["/evicted"], 2);
    assert_eq!(
        client
            .execute_response(route("/after-eviction"))
            .await
            .unwrap()
            .bytes,
        b"ok"
    );
    assert!(Arc::ptr_eq(&reinserted, &selected(&id)));
    global().cover.as_ref().unwrap().shutdown().await;
    server.abort();
}
