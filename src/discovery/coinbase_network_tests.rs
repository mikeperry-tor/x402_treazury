//! The public-origin and paid-fallback consumer chain, through fixture SOCKS/TLS.
use super::*;
use axum::{
    body::Bytes,
    extract::State as AxumState,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::atomic::Ordering;
use tokio_util::sync::CancellationToken;

const ORIGIN: &str = "https://api.example.com/platform/v2/x402/discovery/search";
const RELAY: &str = "https://spec.example.com/curl";

#[derive(Clone, Default)]
struct Fixture {
    // Sequence records distinguish complete workflow replay from signing retries.
    calls: Arc<Mutex<Vec<(String, String)>>>,
    payers: Arc<Mutex<Vec<(String, String)>>>,
    held: CancellationToken,
    release: CancellationToken,
    terminal_retry: Arc<Mutex<Option<Instant>>>,
}

fn search_response(query: &str) -> Value {
    json!({"resources":if query == "empty" { vec![] } else { vec![json!({
        "url":"https://merchant.example/path?exact=1", "method":"POST",
        "protocol":"http", "unknown":{"retained":true}
    })]}, "partialResults":query == "partial", "unknownTopLevel":"retained"})
}

fn query(url: &str) -> String {
    reqwest::Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "query")
        .unwrap()
        .1
        .into_owned()
}

async fn origin(
    AxumState(f): AxumState<Fixture>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    assert_eq!(method, Method::GET);
    for forbidden in ["payment-signature", "x-payment", "authorization"] {
        assert!(!headers.contains_key(forbidden));
    }
    let target = format!("https://api.example.com{uri}");
    let q = query(&target);
    let parsed = reqwest::Url::parse(&target).unwrap();
    let fields: Vec<_> = parsed.query_pairs().collect();
    assert!(
        fields
            .iter()
            .any(|(k, v)| k == "curatedOnly" && v == "true")
    );
    assert!(
        fields
            .iter()
            .any(|(k, v)| k == "network" && v == "eip155:8453")
    );
    assert!(fields.iter().any(|(k, v)| k == "scheme" && v == "exact"));
    f.calls.lock().unwrap().push(("origin".into(), q.clone()));
    match q.as_str() {
        "range-candidate" if sequence(&f, &q).len() == 1 => StatusCode::FORBIDDEN.into_response(),
        "range-initial" | "range-candidate" => (
            [("content-range", "bytes 0-100/999")],
            axum::Json(search_response(&q)),
        )
            .into_response(),
        "empty" | "partial" | "healthy" => axum::Json(search_response(&q)).into_response(),
        "promotion" if sequence(&f, &q).len() > 1 => {
            axum::Json(search_response(&q)).into_response()
        }
        "retry-after" => {
            let terminal = sequence(&f, &q).len() == 3;
            if terminal {
                *f.terminal_retry.lock().unwrap() = Some(Instant::now());
            }
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [("retry-after", if terminal { "1" } else { "0" })],
                "temporarily unavailable",
            )
                .into_response()
        }
        "malformed" => (StatusCode::OK, "not search JSON").into_response(),
        "nonretry" => StatusCode::NOT_FOUND.into_response(),
        "large" => (StatusCode::OK, "x".repeat(32768)).into_response(),
        _ => (
            StatusCode::FORBIDDEN,
            [("cf-mitigated", "challenge")],
            "origin fixture challenge",
        )
            .into_response(),
    }
}

async fn relay(
    AxumState(f): AxumState<Fixture>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    assert_eq!(method, Method::POST);
    assert!(!headers.contains_key("authorization"));
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["method"], "GET");
    let target = body["url"].as_str().unwrap();
    assert!(target.starts_with(ORIGIN));
    let q = query(target);
    if q == "retry-after" {
        assert!(
            f.terminal_retry.lock().unwrap().unwrap().elapsed() >= Duration::from_secs(1),
            "terminal origin Retry-After must elapse before any paid relay request"
        );
    }
    let Some(signature) = headers.get("payment-signature") else {
        f.calls.lock().unwrap().push(("challenge".into(), q));
        let challenge = json!({"x402Version":2,"resource":{"url":RELAY},"accepts":[{
            "scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,
            "amount":"1","payTo":"0x0000000000000000000000000000000000000003",
            "maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}
        }]});
        return (
            StatusCode::PAYMENT_REQUIRED,
            [(
                "payment-required",
                STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
            )],
            "",
        )
            .into_response();
    };
    let signature: Value =
        serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
    f.payers.lock().unwrap().push((
        q.clone(),
        signature["payload"]["authorization"]["from"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase(),
    ));
    f.calls.lock().unwrap().push(("signed".into(), q.clone()));
    if q == "held" {
        f.held.cancel();
        f.release.cancelled().await;
    }
    if q == "uncertain" {
        return (StatusCode::BAD_GATEWAY, "PRIVATE_RELAY_FAILURE").into_response();
    }
    if q == "outer_large" {
        return (StatusCode::OK, "PRIVATE_OUTER_RELAY_BODY".repeat(2048)).into_response();
    }
    let headers = if q == "relay-range" {
        json!({"content-type":"application/json", "Content-Range":"bytes 0-100/999"})
    } else {
        json!({"content-type":"application/json"})
    };
    axum::Json(json!({
        "url":target,"method":"GET","statusCode":200,"headers":headers,
        "body":search_response(&q).to_string(),"redirectChain":[],"error":null
    }))
    .into_response()
}

#[test]
fn coinbase_search_retries_fresh_scopes_and_retains_endpoint_paid_work() {
    // The network factory is process-global; keep test injection in its own process.
    const CHILD: &str = "TREAZURY_COINBASE_NETWORK_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Runtime::new().unwrap().block_on(fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "discovery::tests::coinbase_network_tests::coinbase_search_retries_fresh_scopes_and_retains_endpoint_paid_work",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn invoke(m: &Arc<Manager>, owner: &str, q: &str) -> anyhow::Result<Value> {
    let output = server(m, owner)
        .invoke(
            directory::SEARCH,
            json!({"directory":"coinbase","q":q}).as_object().unwrap(),
        )
        .await?;
    Ok(serde_json::from_str(&output)?)
}

fn sequence(f: &Fixture, q: &str) -> Vec<String> {
    f.calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, query)| query == q)
        .map(|(stage, _)| stage.clone())
        .collect()
}

async fn fixture() {
    use crate::{
        discovery_relay::SearchRelay,
        network::{IsolationId, Mode, NetworkContext, NetworkPolicy, ScopeObservation},
        test_socks::{Fault, Socks},
    };
    let f = Fixture::default();
    let app = axum::Router::new()
        .route("/platform/v2/x402/discovery/search", any(origin))
        .route("/curl", any(relay))
        .with_state(f.clone());
    let (address, task) = crate::test_tls::serve(app).await;
    let proxy = Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), address),
            ("spec.example.com".into(), address),
        ]),
        Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let observations = Arc::new(Mutex::new(Vec::<ScopeObservation>::new()));
    let captured = observations.clone();
    crate::network::install_http_scope_observer(move |observation| {
        captured.lock().unwrap().push(observation.clone());
        Ok(())
    })
    .unwrap();
    let mut p = policy(None);
    p.allowed_origins = vec!["https://api.example.com".into()];
    p.max_response_bytes = 16384;
    let mut ls = listeners(false);
    ls.get_mut("reader").unwrap().wallet = Some("split".into());
    let payers = [(1, "shared"), (2, "split")]
        .into_iter()
        .map(|(key, name)| {
            (
                name.into(),
                PaidClient::new(
                    Payer::new(
                        &format!("{key:064x}"),
                        SpendPolicy::dollars("0.01").unwrap(),
                    )
                    .unwrap(),
                ),
            )
        })
        .collect();
    let m = Manager::new_with_search(
        p,
        ls,
        payers,
        Arc::new(CatalogState::default()),
        vec![],
        Some(Arc::new(SearchRelay::fixture(
            RELAY.into(),
            crate::catalog::Config::default(),
        ))),
    )
    .await
    .unwrap();
    *m.fixture_coinbase.lock().unwrap() = Some(ORIGIN.into());

    // Invalid selectors, listener permission and origin policy cannot spend or connect.
    let before = proxy.accepted.load(Ordering::SeqCst);
    for args in [
        json!({"directory":"coinbase","mode":"browse"}),
        json!({"directory":"coinbase","local_only":true}),
        json!({"directory":"coinbase","page":1}),
        json!({"directory":"coinbase","wallet":"split"}),
        json!({"directory":"coinbase","limit":21}),
        json!({"directory":"coinbase","curated_only":true}),
        json!({"directory":"coinbase","curated_only":false}),
        json!({"directory":"coinbase","tags":["Weather"]}),
        json!({"directory":"coinbase","bundle_slugs":["weather-kit"]}),
        json!({"directory":"coinbase","extensions":["bazaar"]}),
        json!({"directory":"coinbase","pay_to":"merchant"}),
        json!({"directory":"coinbase","url_substring":"api/weather"}),
        json!({"directory":"coinbase","max_price_usd":-0.05}),
        json!({"directory":"coinbase","max_price_usd":"0.05"}),
        json!({"directory":"coinbase","q":"é".repeat(401)}),
    ] {
        assert!(
            server(&m, "writer")
                .invoke(directory::SEARCH, args.as_object().unwrap())
                .await
                .is_err()
        );
    }
    assert!(invoke(&m, "hidden", "retry").await.is_err());
    *m.fixture_coinbase.lock().unwrap() = Some("https://other.example.com/search".into());
    assert!(invoke(&m, "writer", "retry").await.is_err());
    *m.fixture_coinbase.lock().unwrap() = Some(ORIGIN.into());
    assert_eq!(proxy.accepted.load(Ordering::SeqCst), before);
    assert!(f.calls.lock().unwrap().is_empty());
    assert!(
        f.payers.lock().unwrap().is_empty(),
        "rejected inputs must not disclose payment signatures"
    );
    assert!(
        m.inner.lock().unwrap().state.records.is_empty(),
        "rejected inputs must not register sources"
    );
    assert!(
        observations.lock().unwrap().is_empty(),
        "rejected inputs must not even select an HTTP scope"
    );

    let unsigned = Manager::new(
        m.policy.clone(),
        listeners(false),
        BTreeMap::from([("shared".into(), PaidClient::unsigned())]),
        Arc::new(CatalogState::default()),
        vec![],
    )
    .await
    .unwrap();
    *unsigned.fixture_coinbase.lock().unwrap() = Some(ORIGIN.into());
    assert!(
        invoke(&unsigned, "writer", "identity-unavailable")
            .await
            .is_err()
    );
    assert_eq!(
        proxy.accepted.load(Ordering::SeqCst),
        before,
        "unavailable public identity cannot allocate a wallet or use Curl"
    );

    let first_record = proxy.records.lock().unwrap().len();
    let first = invoke(&m, "writer", "retry").await.unwrap();
    assert_eq!(first["directory"], "coinbase");
    assert_eq!(first["acquisition"]["via"], "curl");
    assert_eq!(first["response"], search_response("retry"));
    eprintln!("Actual Coinbase fixture output: {first}");
    assert_eq!(
        sequence(&f, "retry"),
        ["origin", "origin", "origin", "challenge", "signed"]
    );
    {
        let records = proxy.records.lock().unwrap();
        let scopes: BTreeSet<_> = records[first_record..]
            .iter()
            .filter(|r| r.host == "api.example.com")
            .map(|r| (r.user.clone(), r.password.clone()))
            .collect();
        assert_eq!(
            scopes.len(),
            3,
            "initial scope plus two fresh Tor isolation attempts"
        );
        assert!(records.iter().any(|r| r.host == "spec.example.com"));
        assert!(
            records.iter().all(|r| r.address_type == 3),
            "SOCKS must resolve public hostnames remotely"
        );
    }
    let disabled = Manager::new(
        m.policy.clone(),
        m.listeners.clone(),
        m.payers.clone(),
        Arc::new(CatalogState::default()),
        vec![],
    )
    .await
    .unwrap();
    *disabled.fixture_coinbase.lock().unwrap() = Some(ORIGIN.into());
    assert!(invoke(&disabled, "writer", "disabled").await.is_err());
    assert_eq!(
        sequence(&f, "disabled"),
        ["origin", "origin", "origin"],
        "ordinary deployments have no implied paid fallback authority"
    );
    invoke(&m, "writer", "retry").await.unwrap();
    assert_eq!(
        sequence(&f, "retry").len(),
        10,
        "repeated search must perform fresh origin and paid workflows"
    );
    invoke(&m, "writer", "retry-after").await.unwrap();
    assert_eq!(
        sequence(&f, "retry-after"),
        ["origin", "origin", "origin", "challenge", "signed"],
        "the terminal eligible response still delays the first relay request"
    );

    let client = m.source_client("writer").unwrap();
    let identity = client.documentation_identity().await.unwrap();
    let selected = || {
        crate::network::global()
            .http_participating(
                &identity,
                ORIGIN,
                import::read_timeout(&m.policy, None),
                true,
                client.transport(),
            )
            .unwrap()
    };
    let original = selected();
    let promoted = invoke(&m, "writer", "promotion").await.unwrap();
    assert_eq!(promoted["acquisition"]["via"], "origin");
    assert_eq!(sequence(&f, "promotion"), ["origin", "origin"]);
    let winner = selected();
    assert!(
        !Arc::ptr_eq(&original, &winner),
        "complete valid candidate response promotes its scope"
    );
    let winner_credentials = crate::network::global().credentials(&winner.scope);
    assert!(
        proxy
            .records
            .lock()
            .unwrap()
            .iter()
            .any(|record| record.host == "api.example.com"
                && (record.user.clone(), record.password.clone()) == winner_credentials)
    );
    let scopes_before = observations.lock().unwrap().len();
    invoke(&m, "writer", "promotion").await.unwrap();
    assert!(
        Arc::ptr_eq(&winner, &selected()),
        "the next invocation selects the promoted candidate"
    );
    assert_eq!(
        observations.lock().unwrap().len(),
        scopes_before,
        "selected candidate is reused without creating another isolation attempt"
    );
    assert_eq!(sequence(&f, "promotion"), ["origin", "origin", "origin"]);

    for (q, attempts) in [("range-initial", 1), ("range-candidate", 2)] {
        let result = invoke(&m, "writer", q).await.unwrap();
        assert_eq!(result["acquisition"]["via"], "curl");
        assert_eq!(result["response"], search_response(q));
        let mut expected = vec!["origin"; attempts];
        expected.extend(["challenge", "signed"]);
        assert_eq!(sequence(&f, q), expected);
        assert!(
            Arc::ptr_eq(&winner, &selected()),
            "ranged origin JSON cannot promote its candidate; Curl cannot promote it either"
        );
    }
    let error = format!(
        "{:#}",
        invoke(&m, "writer", "relay-range").await.unwrap_err()
    );
    assert!(error.contains("response_rejected"));
    assert!(error.contains("do not automatically repeat"));
    assert_eq!(
        sequence(&f, "relay-range"),
        ["origin", "origin", "origin", "challenge", "signed"],
        "explicitly ranged target content fails without replaying paid work"
    );
    assert!(Arc::ptr_eq(&winner, &selected()));

    for q in ["empty", "partial"] {
        let result = invoke(&m, "writer", q).await.unwrap();
        assert_eq!(result["acquisition"]["via"], "origin");
        assert_eq!(result["response"], search_response(q));
        assert_eq!(
            sequence(&f, q),
            ["origin"],
            "valid empty/partial is successful"
        );
    }
    for q in ["malformed", "nonretry"] {
        let result = invoke(&m, "writer", q).await.unwrap();
        assert_eq!(result["acquisition"]["via"], "curl");
        assert_eq!(
            sequence(&f, q),
            ["origin", "challenge", "signed"],
            "complete invalid JSON and nonretryable statuses do not earn isolation retries"
        );
    }
    assert!(invoke(&m, "writer", "large").await.is_err());
    assert_eq!(
        sequence(&f, "large"),
        ["origin"],
        "download policy rejection must never authorize Curl"
    );
    let outer_error = format!(
        "{:#}",
        invoke(&m, "writer", "outer_large").await.unwrap_err()
    );
    assert!(outer_error.contains("max_response_bytes"), "{outer_error}");
    assert!(outer_error.contains("16384"), "{outer_error}");
    assert!(
        outer_error.contains("do not automatically repeat"),
        "{outer_error}"
    );
    assert!(!outer_error.contains("PRIVATE_OUTER_RELAY_BODY"));
    assert_eq!(
        sequence(&f, "outer_large"),
        ["origin", "origin", "origin", "challenge", "signed"],
        "a signed oversized outer response must fail visibly without another authorization"
    );

    let error = invoke(&m, "reader", "uncertain")
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("PRIVATE_RELAY_FAILURE"));
    assert!(error.contains("do not automatically repeat"));
    assert_eq!(
        sequence(&f, "uncertain"),
        ["origin", "origin", "origin", "challenge", "signed"],
        "uncertain signed fallback has no enclosing replay"
    );
    invoke(&m, "reader", "after-failure").await.unwrap();
    assert_eq!(
        sequence(&f, "after-failure"),
        ["origin", "origin", "origin", "challenge", "signed"],
        "runtime failures cannot poison later searches"
    );

    let held = tokio::spawn({
        let m = m.clone();
        async move { invoke(&m, "writer", "held").await }
    });
    tokio::time::timeout(Duration::from_secs(10), f.held.cancelled())
        .await
        .unwrap();
    held.abort();
    assert!(held.await.unwrap_err().is_cancelled());
    // An accepted signed request retains ownership after its waiter disappears,
    // while independent public and paid searches remain able to make progress.
    tokio::time::timeout(Duration::from_secs(10), invoke(&m, "reader", "healthy"))
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), invoke(&m, "reader", "concurrent"))
        .await
        .unwrap()
        .unwrap();
    let mut drain = tokio::spawn({
        let client = m.source_client("writer").unwrap();
        async move { client.drain().await }
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut drain)
            .await
            .is_err(),
        "drain must retain cancelled signed work"
    );
    f.release.cancel();
    tokio::time::timeout(Duration::from_secs(10), drain)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        sequence(&f, "held"),
        ["origin", "origin", "origin", "challenge", "signed"]
    );

    let address = |key| {
        let signer: alloy_signer_local::PrivateKeySigner = format!("{key:064x}").parse().unwrap();
        signer.address().to_string().to_ascii_lowercase()
    };
    for (q, payer) in f.payers.lock().unwrap().iter() {
        assert_eq!(
            *payer,
            address(
                if ["uncertain", "after-failure", "concurrent"].contains(&q.as_str()) {
                    2
                } else {
                    1
                }
            )
        );
    }
    for key in 1..=2 {
        let canonical =
            crate::network::global().credentials(&IsolationId::evm(&address(key)).unwrap());
        assert!(
            observations
                .lock()
                .unwrap()
                .iter()
                .any(|o| o.origin == "https://api.example.com" && o.canonical == canonical)
        );
    }
    assert!(
        m.inner.lock().unwrap().state.records.is_empty(),
        "discovery never registers or allocates a source"
    );
    m.source_client("reader").unwrap().drain().await;
    task.abort();
}
