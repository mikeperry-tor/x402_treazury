//! Local SOCKS/TLS evidence for actual Base consumers. No live chain or funds.
use super::*;
use crate::network::{self, HttpPolicy, IsolationId, Mode, NetworkContext, NetworkPolicy};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::post,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;

const INITIAL_SCOPE: &str = "TREAZURY_BASE_FIXTURE_INITIAL_SCOPE ";
const ORIGIN: &str = "https://api.example.com";
const OTHER: &str = "https://spec.example.com";
#[derive(Clone)]
enum Step {
    Status(u16),
    Result(Value),
    InvalidJson,
    InvalidEnvelope,
    Held(Value, Arc<Semaphore>),
    Interrupted(Arc<Semaphore>, Arc<Semaphore>),
}
type Plans = HashMap<(String, String), VecDeque<Step>>;
#[derive(Clone)]
struct Fixture {
    plans: Arc<Mutex<Plans>>,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    timestamp: u64,
}
impl Fixture {
    fn plan(&self, path: &str, method: &str, steps: impl IntoIterator<Item = Step>) {
        self.plans
            .lock()
            .unwrap()
            .insert((path.into(), method.into()), steps.into_iter().collect());
    }
    fn reset(&self) {
        self.plans.lock().unwrap().clear();
        self.requests.lock().unwrap().clear();
    }
    fn requests(&self) -> Vec<(String, Value)> {
        self.requests.lock().unwrap().clone()
    }
    async fn wait(&self, path: &str, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _)| p == path)
                .count()
                < count
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("Base fixture dispatch");
    }
}
async fn handler(
    State(f): State<Fixture>,
    uri: Uri,
    headers: HeaderMap,
    axum::Json(payload): axum::Json<Value>,
) -> Response {
    assert!(!headers.contains_key("payment-signature") && !headers.contains_key("x-payment"));
    assert!(!headers.contains_key("x-example-padding"));
    assert_eq!(payload["id"], 1);
    let path = uri.path().to_owned();
    let method = payload["method"].as_str().unwrap();
    assert!(matches!(
        method,
        "eth_chainId" | "eth_getBlockByNumber" | "eth_getTransactionReceipt" | "eth_call"
    ));
    f.requests
        .lock()
        .unwrap()
        .push((uri.path().to_owned(), payload.clone()));
    let step = f
        .plans
        .lock()
        .unwrap()
        .get_mut(&(path, method.into()))
        .and_then(VecDeque::pop_front);
    let result = || match method {
        "eth_chainId" => json!("0x2105"),
        "eth_getBlockByNumber" => {
            let height = if matches!(payload["params"][0].as_str(), Some("latest" | "finalized")) {
                100
            } else {
                u64::from_str_radix(
                    payload["params"][0]
                        .as_str()
                        .unwrap()
                        .trim_start_matches("0x"),
                    16,
                )
                .unwrap()
            };
            json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",f.timestamp)})
        }
        "eth_call" => json!(format!("0x{:064x}", 7)),
        "eth_getTransactionReceipt" => Value::Null,
        _ => unreachable!(),
    };
    let value = match step {
        Some(Step::Status(code)) => {
            return (StatusCode::from_u16(code).unwrap(), "unused upstream body").into_response();
        }
        Some(Step::InvalidJson) => return (StatusCode::OK, "not JSON").into_response(),
        Some(Step::InvalidEnvelope) => {
            return axum::Json(json!({"jsonrpc":"2.0","id":999,"result":"0x2105"})).into_response();
        }
        Some(Step::Held(value, gate)) => {
            let bytes =
                serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"result":value})).unwrap();
            let stream = futures_util::stream::once(async move {
                gate.acquire().await.unwrap().forget();
                Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(bytes))
            });
            return (
                [("content-type", "application/json")],
                axum::body::Body::from_stream(stream),
            )
                .into_response();
        }
        Some(Step::Interrupted(started, fail)) => {
            use futures_util::StreamExt;
            let prefix = futures_util::stream::once(async move {
                started.add_permits(1);
                Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"{\"jsonrpc\":\"2.0\""))
            });
            let failure = futures_util::stream::once(async move {
                fail.acquire().await.unwrap().forget();
                Err::<axum::body::Bytes, _>(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "fixture interrupted RPC body",
                ))
            });
            return (
                [("content-type", "application/json")],
                axum::body::Body::from_stream(prefix.chain(failure)),
            )
                .into_response();
        }
        Some(Step::Result(value)) => value,
        None => result(),
    };
    axum::Json(json!({"jsonrpc":"2.0","id":1,"result":value})).into_response()
}
fn url(path: &str) -> String {
    format!("{ORIGIN}{path}")
}
fn address(n: usize) -> String {
    format!("0x{n:040x}")
}
fn selected(identity: &IsolationId, endpoint: &str) -> Arc<network::HttpTransport> {
    network::global()
        .http_participating(identity, endpoint, None, false, HttpPolicy::COMPATIBLE)
        .unwrap()
}
fn query() -> ChainQuery {
    ChainQuery {
        wallets: vec![],
        pending: vec![],
        anchor: None,
        captures: Default::default(),
    }
}
fn expected() -> TransferExpectation {
    TransferExpectation {
        transaction: B256::repeat_byte(1),
        payer: address(900).parse().unwrap(),
        payee: address(901).parse().unwrap(),
        amount: U256::from(1),
        nonce: B256::repeat_byte(2),
    }
}
#[test]
fn base_tor_consumers_in_isolated_process() {
    if std::env::var_os("TREAZURY_BASE_TOR_CHILD").is_none() {
        let mut scopes = Vec::<(String, String)>::new();
        for _ in 0..2 {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .env_clear()
                .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
                .env("TREAZURY_BASE_TOR_CHILD", "1")
                .args([
                    "--exact",
                    "rotation::base::tor_tests::base_tor_consumers_in_isolated_process",
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
            let stdout = String::from_utf8_lossy(&output.stdout);
            let scope = stdout
                .lines()
                .find_map(|line| line.split_once(INITIAL_SCOPE).map(|(_, v)| v))
                .expect("first actual Base SOCKS handshake");
            scopes.push(serde_json::from_str(scope).unwrap());
        }
        assert_eq!(scopes[0].0, scopes[1].0);
        assert_ne!(
            scopes[0].1, scopes[1].1,
            "actual Base consumer starts fresh after process restart"
        );
        return;
    }
    tokio::runtime::Runtime::new().unwrap().block_on(fixture());
}
async fn fixture() {
    let f = Fixture {
        plans: Default::default(),
        requests: Default::default(),
        timestamp: now().unwrap(),
    };
    let app = axum::Router::new()
        .route("/{*path}", post(handler))
        .with_state(f.clone());
    let (server_address, server) =
        crate::test_tls::serve_with(app, &[&rustls::version::TLS13], &[b"h2"]).await;
    let socks = crate::test_socks::Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), server_address),
            ("spec.example.com".into(), server_address),
        ]),
        crate::test_socks::Fault::None,
    )
    .await;
    network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(socks.address),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let rpc = BaseRpc::new(&url("/matrix"), 12).unwrap();
    let mut wallet = 1000;
    // Every eligible status, success at all three positions and finite exhaustion
    // through the real advisory balance consumer; all payload bytes stay equal.
    for status in [400, 403, 408, 429, 500, 502, 503, 504] {
        for failures in 0..=3 {
            f.reset();
            f.plan(
                "/matrix",
                "eth_call",
                std::iter::repeat_n(Step::Status(status), failures),
            );
            let payer = address(wallet);
            wallet += 1;
            let owner = IsolationId::evm(&payer).unwrap();
            let before = selected(&owner, &url("/matrix"));
            let physical_before = socks.records.lock().unwrap().len();
            let result = rpc.current_balance(&payer).await;
            if failures == 3 {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap(), U256::from(7));
            }
            let requests = f.requests();
            let attempts = (failures + 1).min(3);
            assert_eq!(
                requests.len(),
                attempts,
                "status{status} finite isolated sequence"
            );
            assert!(requests.windows(2).all(|pair| pair[0] == pair[1]));
            assert_eq!(requests[0].1["params"][1], "latest");
            assert_eq!(
                requests[0].1["params"][0]["data"],
                format!("0x70a08231{:0>64}", &payer[2..])
            );
            let after = selected(&owner, &url("/matrix"));
            assert_eq!(Arc::ptr_eq(&before, &after), failures == 0 || failures == 3);
            let records = socks.records.lock().unwrap();
            let records = &records[physical_before..];
            assert_eq!(records.len(), attempts, "no nested same-client retry loop");
            let tokens: std::collections::BTreeSet<_> =
                records.iter().map(|r| &r.password).collect();
            assert_eq!(tokens.len(), attempts);
            assert_eq!(
                (records[0].user.clone(), records[0].password.clone()),
                network::global().credentials(&before.scope)
            );
            if failures < 3 {
                assert_eq!(
                    (
                        records.last().unwrap().user.clone(),
                        records.last().unwrap().password.clone()
                    ),
                    network::global().credentials(&after.scope),
                    "successful result pins and retains final scope"
                );
            }
            if wallet == 1001 {
                println!(
                    "{INITIAL_SCOPE}{}",
                    serde_json::to_string(&(&records[0].user, &records[0].password)).unwrap()
                );
            }
        }
    }
    for status in [401, 402, 404, 405, 410, 422, 501, 505, 520] {
        f.reset();
        let payer = address(wallet);
        wallet += 1;
        let owner = IsolationId::evm(&payer).unwrap();
        let before = selected(&owner, &url("/matrix"));
        let physical = socks.records.lock().unwrap().len();
        f.plan("/matrix", "eth_call", [Step::Status(status)]);
        assert!(rpc.current_balance(&payer).await.is_err());
        assert_eq!(f.requests().len(), 1, "ineligible status {status}");
        assert_eq!(socks.records.lock().unwrap().len(), physical + 1);
        assert!(Arc::ptr_eq(&before, &selected(&owner, &url("/matrix"))));
    }
    method_consumers(&f).await;
    interrupted_read(&f).await;
    fallback_and_validation(&f).await;
    races_and_eviction(&f, &socks).await;
    server.abort();
}

async fn interrupted_read(f: &Fixture) {
    f.reset();
    let endpoint = url("/interrupted");
    let rpc = BaseRpc::new(&endpoint, 12).unwrap();
    let payer = address(1800);
    let owner = IsolationId::evm(&payer).unwrap();
    let before = selected(&owner, &endpoint);
    let started = Arc::new(Semaphore::new(0));
    let fail = Arc::new(Semaphore::new(0));
    f.plan(
        "/interrupted",
        "eth_call",
        [Step::Interrupted(started.clone(), fail.clone())],
    );
    let task = tokio::spawn(async move { rpc.current_balance(&payer).await });
    tokio::time::timeout(Duration::from_secs(5), started.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert!(!task.is_finished());
    fail.add_permits(1);
    assert_eq!(task.await.unwrap().unwrap(), U256::from(7));
    let requests = f.requests();
    assert_eq!(
        requests.len(),
        2,
        "typed interrupted unsigned read gets one fresh scope before success"
    );
    assert_eq!(requests[0], requests[1]);
    assert!(!Arc::ptr_eq(&before, &selected(&owner, &endpoint)));
}

async fn method_consumers(f: &Fixture) {
    for status in [400, 403, 408, 429, 500, 502, 503, 504] {
        for method in [
            "eth_chainId",
            "eth_getBlockByNumber",
            "eth_getTransactionReceipt",
        ] {
            f.reset();
            let endpoint = url("/methods");
            f.plan("/methods", method, [Step::Status(status)]);
            let rpc = BaseRpc::new(&endpoint, 12).unwrap();
            match method {
                "eth_chainId" => {
                    rpc.view(query()).await.unwrap();
                }
                "eth_getBlockByNumber" => {
                    assert_eq!(rpc.block("latest").await.unwrap().0.height, 100);
                }
                "eth_getTransactionReceipt" => {
                    assert!(rpc.verify_transfer(&expected()).await.unwrap().is_none());
                }
                _ => unreachable!(),
            }
            let all = f.requests();
            let attempts: Vec<_> = all.iter().filter(|(_, v)| v["method"] == method).collect();
            assert!(attempts.len() >= 2);
            assert_eq!(
                attempts[0], attempts[1],
                "same logical read preserves exact payload"
            );
            if method == "eth_getTransactionReceipt" {
                assert_eq!(attempts.len(), 2);
                assert_eq!(
                    all.len(),
                    4,
                    "null receipt stops after chain/header/receipt and its one retry"
                );
            }
            if method == "eth_getBlockByNumber" {
                assert_eq!(attempts.len(), 2);
            }
        }
    }
    f.reset();
    let rpc = BaseRpc::new(&url("/forbidden"), 12).unwrap();
    let result: Result<Value> = rpc
        .rpc("eth_sendRawTransaction", json!(["0x00"]), |value| {
            Ok((value, true))
        })
        .await;
    assert!(result.is_err());
    assert!(
        f.requests().is_empty(),
        "write RPC is rejected before provider I/O"
    );
}

async fn fallback_and_validation(f: &Fixture) {
    // The whole finite sequence on endpoint A completes before endpoint B.
    for status in [400, 403, 408, 429, 500, 502, 503, 504] {
        f.reset();
        f.plan(
            "/a",
            "eth_call",
            std::iter::repeat_n(Step::Status(status), 3),
        );
        let rpc = BaseRpc::with_fallbacks(&[url("/a"), format!("{OTHER}/b")], 12).unwrap();
        assert_eq!(
            rpc.current_balance(&address(2000 + status as usize))
                .await
                .unwrap(),
            U256::from(7)
        );
        let requests = f.requests();
        assert_eq!(
            requests
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            ["/a", "/a", "/a", "/b"]
        );
        assert!(requests.iter().all(|(_, v)| v == &requests[0].1));
    }
    f.reset();
    f.plan(
        "/strict-a",
        "eth_chainId",
        std::iter::repeat_n(Step::Status(400), 3),
    );
    let strict =
        BaseRpc::with_fallbacks(&[url("/strict-a"), format!("{OTHER}/strict-b")], 12).unwrap();
    assert!(strict.view(query()).await.is_err());
    assert_eq!(
        f.requests()
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        ["/strict-a", "/strict-a", "/strict-a"],
        "strict HTTP400 does not gain endpoint fallback"
    );
    // A failed pinned balance read invalidates the partial view; fallback starts
    // again at chain ID and publishes only the full configured URL's evidence.
    f.reset();
    f.plan(
        "/view-a",
        "eth_call",
        std::iter::repeat_n(Step::Status(503), 3),
    );
    let view_b = BaseRpc::new(&url("/view-b"), 12).unwrap();
    let view_rpc = BaseRpc::with_fallbacks(&[url("/view-a"), url("/view-b")], 12).unwrap();
    let mut chain_query = query();
    chain_query.wallets.push(("wallet".into(), address(2500)));
    let view = view_rpc.view(chain_query).await.unwrap();
    assert_eq!(view.anchor.height, 88);
    let observation = &view.observations["wallet"];
    assert_eq!(observation.endpoint, view_b.endpoint_id());
    assert_eq!(observation.confirmed.amount, U256::from(7));
    assert_eq!(observation.latest.amount, U256::from(7));
    assert_eq!(observation.confirmed.anchor.height, 88);
    assert_eq!(observation.latest.anchor.height, 100);
    let requests = f.requests();
    let boundary = requests
        .iter()
        .position(|(path, _)| path == "/view-b")
        .unwrap();
    assert_eq!(
        boundary, 7,
        "A chain and three headers precede its three isolated balance failures"
    );
    assert_eq!(requests[boundary].1["method"], "eth_chainId");
    assert!(
        requests[..boundary]
            .iter()
            .all(|(path, _)| path == "/view-a")
    );
    assert!(
        requests[boundary..]
            .iter()
            .all(|(path, _)| path == "/view-b")
    );
    let balance_reads: Vec<_> = requests
        .iter()
        .filter(|(_, value)| value["method"] == "eth_call")
        .collect();
    assert_eq!(balance_reads.len(), 5);
    assert!(
        balance_reads
            .iter()
            .all(|(_, value)| value["params"][1]["requireCanonical"] == true)
    );
    assert_eq!(
        balance_reads[3].1["params"][1]["blockHash"],
        format!("0x{:064x}", 88)
    );
    assert_eq!(
        balance_reads[4].1["params"][1]["blockHash"],
        format!("0x{:064x}", 100)
    );
    // Wrong explicit height retains the caller's terminal validation error, but
    // cannot promote a candidate or broaden strict endpoint fallback.
    f.reset();
    let endpoint = url("/wrong-height");
    let owner = IsolationId::discovery(&endpoint).unwrap();
    let before = selected(&owner, &endpoint);
    f.plan(
        "/wrong-height",
        "eth_getBlockByNumber",
        [
            Step::Result(
                json!({"number":"0x64","timestamp":"0x2","hash":format!("0x{:064x}",100)}),
            ),
            Step::Result(
                json!({"number":"0x64","timestamp":"0x2","hash":format!("0x{:064x}",100)}),
            ),
            Step::Status(403),
            Step::Result(json!({"number":"0x59","timestamp":"0x1","hash":format!("0x{:064x}",89)})),
        ],
    );
    let error = BaseRpc::with_fallbacks(&[endpoint.clone(), url("/forbidden-height-fallback")], 12)
        .unwrap()
        .view(query())
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("confirmed block"), "{error}");
    assert!(error.downcast_ref::<RpcFailure>().is_none());
    assert!(Arc::ptr_eq(&before, &selected(&owner, &endpoint)));
    assert_eq!(f.requests().len(), 5); // chain ID, latest, finalized, rejected confirmed, wrong confirmed
    f.reset();
    let endpoint = url("/invalid-nonce");
    let payer = address(3010);
    let owner = IsolationId::evm(&payer).unwrap();
    let before = selected(&owner, &endpoint);
    f.plan(
        "/invalid-nonce",
        "eth_call",
        [
            Step::Status(503),
            Step::Result(json!(format!("0x{:064x}", 2))),
        ],
    );
    let mut nonce_query = query();
    nonce_query.pending.push(PendingAuthorization {
        id: "auth".into(),
        wallet: "wallet".into(),
        payer,
        nonce: B256::repeat_byte(8).to_string(),
        valid_before: u64::MAX,
    });
    assert!(
        BaseRpc::new(&endpoint, 12)
            .unwrap()
            .view(nonce_query)
            .await
            .is_err()
    );
    assert_eq!(
        f.requests()
            .iter()
            .filter(|(_, value)| value["method"] == "eth_call")
            .count(),
        2
    );
    assert!(
        Arc::ptr_eq(&before, &selected(&owner, &endpoint)),
        "invalid authorizationState cannot promote"
    );
    // Validation failure after an eligible rejection never promotes or acquires
    // a third scope merely because the JSON-RPC envelope was valid.
    for step in [
        Step::Result(json!("0x1")),
        Step::Result(json!("invalid quantity")),
        Step::InvalidJson,
        Step::InvalidEnvelope,
    ] {
        f.reset();
        let endpoint = url("/wrong-chain");
        let owner = IsolationId::discovery(&endpoint).unwrap();
        let before = selected(&owner, &endpoint);
        f.plan("/wrong-chain", "eth_chainId", [Step::Status(403), step]);
        assert!(
            BaseRpc::new(&endpoint, 12)
                .unwrap()
                .view(query())
                .await
                .is_err()
        );
        assert_eq!(f.requests().len(), 2);
        assert!(Arc::ptr_eq(&before, &selected(&owner, &endpoint)));
    }
    for (method, value) in [
        (
            "eth_getBlockByNumber",
            json!({"number":"0x64","timestamp":"0x1","hash":"broken"}),
        ),
        ("eth_call", json!("0x01")),
        (
            "eth_getTransactionReceipt",
            json!({"blockNumber":"not-a-quantity"}),
        ),
    ] {
        f.reset();
        let endpoint = url("/invalid-result");
        let rpc = BaseRpc::new(&endpoint, 12).unwrap();
        let owner = if method == "eth_getBlockByNumber" {
            IsolationId::discovery(&endpoint).unwrap()
        } else {
            let payer = if method == "eth_call" {
                address(3000)
            } else {
                expected().payer.to_string()
            };
            IsolationId::evm(&payer).unwrap()
        };
        let before = selected(&owner, &endpoint);
        f.plan(
            "/invalid-result",
            method,
            [Step::Status(503), Step::Result(value)],
        );
        match method {
            "eth_getBlockByNumber" => assert!(rpc.block("latest").await.is_err()),
            "eth_call" => assert!(rpc.current_balance(&address(3000)).await.is_err()),
            _ => assert!(rpc.verify_transfer(&expected()).await.is_err()),
        }
        assert_eq!(
            f.requests()
                .iter()
                .filter(|(_, v)| v["method"] == method)
                .count(),
            2
        );
        assert!(Arc::ptr_eq(&before, &selected(&owner, &endpoint)));
    }
}

async fn races_and_eviction(f: &Fixture, socks: &crate::test_socks::Socks) {
    f.reset();
    let payer = address(4000);
    let owner = IsolationId::evm(&payer).unwrap();
    let primary = BaseRpc::new(&url("/held"), 12).unwrap();
    let recover = BaseRpc::new(&url("/recover"), 12).unwrap();
    assert_ne!(
        primary.endpoint_id(),
        recover.endpoint_id(),
        "endpoint evidence preserves full URL despite origin-shared selection"
    );
    let original = selected(&owner, &url("/held"));
    let discovery_owner = IsolationId::discovery(&url("/held")).unwrap();
    let discovery_before = selected(&discovery_owner, &url("/held"));
    let independent = BaseRpc::new(&format!("{OTHER}/independent"), 12).unwrap();
    assert_eq!(
        independent.current_balance(&payer).await.unwrap(),
        U256::from(7)
    );
    let independent_before = selected(&owner, &format!("{OTHER}/independent"));
    let gate = Arc::new(Semaphore::new(0));
    f.plan(
        "/held",
        "eth_call",
        [Step::Held(json!(format!("0x{:064x}", 5)), gate.clone())],
    );
    let task = {
        let payer = payer.clone();
        tokio::spawn(async move { primary.current_balance(&payer).await })
    };
    f.wait("/held", 1).await;
    let physical = socks.records.lock().unwrap().len();
    f.plan("/recover", "eth_call", [Step::Status(403)]);
    assert_eq!(
        recover.current_balance(&payer).await.unwrap(),
        U256::from(7)
    );
    let winner = selected(&owner, &url("/recover"));
    assert!(!Arc::ptr_eq(&original, &winner));
    assert!(
        Arc::ptr_eq(
            &independent_before,
            &selected(&owner, &format!("{OTHER}/independent"))
        ),
        "same-wallet other-origin transport does not migrate"
    );
    assert!(
        Arc::ptr_eq(
            &discovery_before,
            &selected(&discovery_owner, &url("/recover"))
        ),
        "wallet read success does not migrate discovery/header scope"
    );
    assert!(
        !task.is_finished(),
        "old HTTP2 response survives isolated recovery"
    );
    assert_eq!(
        socks.records.lock().unwrap().len(),
        physical + 1,
        "recovery opens one new physical scope while old stream lives"
    );
    assert_eq!(
        recover.current_balance(&payer).await.unwrap(),
        U256::from(7)
    );
    assert_eq!(
        socks.records.lock().unwrap().len(),
        physical + 1,
        "new actual read reacquires retained winner pool"
    );
    gate.add_permits(1);
    assert_eq!(task.await.unwrap().unwrap(), U256::from(5));
    assert!(
        Arc::ptr_eq(&winner, &selected(&owner, &url("/held"))),
        "late original result cannot roll back winner"
    );
    // A private candidate loses promotion to another actual consumer.
    f.reset();
    let gate = Arc::new(Semaphore::new(0));
    f.plan(
        "/late",
        "eth_call",
        [
            Step::Status(403),
            Step::Held(json!(format!("0x{:064x}", 6)), gate.clone()),
        ],
    );
    let late = BaseRpc::new(&url("/late"), 12).unwrap();
    let task = {
        let payer = payer.clone();
        tokio::spawn(async move { late.current_balance(&payer).await })
    };
    f.wait("/late", 2).await;
    f.plan("/recover", "eth_call", [Step::Status(503)]);
    assert_eq!(
        recover.current_balance(&payer).await.unwrap(),
        U256::from(7)
    );
    let race_winner = selected(&owner, &url("/recover"));
    gate.add_permits(1);
    assert_eq!(task.await.unwrap().unwrap(), U256::from(6));
    assert!(Arc::ptr_eq(&race_winner, &selected(&owner, &url("/late"))));
    assert_eq!(
        f.requests().len(),
        4,
        "lost promotion race grants no additional I/O"
    );
    f.reset();
    let gate = Arc::new(Semaphore::new(0));
    f.plan(
        "/evicted",
        "eth_call",
        [
            Step::Status(403),
            Step::Held(json!(format!("0x{:064x}", 8)), gate.clone()),
        ],
    );
    let evicted = BaseRpc::new(&url("/evicted"), 12).unwrap();
    let task = {
        let payer = payer.clone();
        tokio::spawn(async move { evicted.current_balance(&payer).await })
    };
    f.wait("/evicted", 2).await;
    for n in 0..260 {
        network::global()
            .http_transport(
                &IsolationId::treasury(&format!("base-evict-{n}")),
                &format!("http://127.0.0.1:{}", 20000 + n),
                None,
                false,
                HttpPolicy::COMPATIBLE,
                network::SelectionPolicy::Legacy,
            )
            .unwrap();
    }
    assert_eq!(
        recover.current_balance(&payer).await.unwrap(),
        U256::from(7)
    );
    let reinserted = selected(&owner, &url("/recover"));
    assert_ne!(
        network::global().credentials(&race_winner.scope),
        network::global().credentials(&reinserted.scope)
    );
    gate.add_permits(1);
    assert_eq!(task.await.unwrap().unwrap(), U256::from(8));
    assert!(
        Arc::ptr_eq(&reinserted, &selected(&owner, &url("/evicted"))),
        "old candidate cannot replace reinserted Base selection"
    );
    assert_eq!(f.requests().len(), 3);
}
