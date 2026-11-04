//! Learn only from normal unsigned HTTP challenges, after full SDK validation.
use super::*;
use crate::{
    pricing::runtime::OperationKey,
    rotation::{
        base::BaseRpc,
        manager::ManagedPool,
        store::{Store, StoreHandle},
    },
};
use axum::serve::Listener as _;
use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use futures_util::StreamExt;
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

enum Reply {
    Challenge(Value),
    Malformed,
    Free,
    Held(Value, Arc<tokio::sync::Notify>, Arc<tokio::sync::Semaphore>),
}
struct Fixture {
    _dir: tempfile::TempDir,
    client: PaidClient,
    hints: Arc<crate::pricing::runtime::CatalogHints>,
    store: StoreHandle,
    worker: tokio::task::JoinHandle<()>,
    server: tokio::task::JoinHandle<()>,
    base: String,
    replies: Arc<Mutex<VecDeque<Reply>>>,
    unsigned: Arc<AtomicUsize>,
    signed: Arc<AtomicUsize>,
    signed_output: Arc<AtomicUsize>,
    rpc: Arc<AtomicUsize>,
}
fn challenge(amounts: &[u64]) -> Value {
    json!({"x402Version":2,"resource":{"url":"https://fixture.example/items","description":"fixture","mimeType":"text/plain"},"accepts":amounts.iter().map(|amount|json!({"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":amount.to_string(),"payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}})).collect::<Vec<_>>()})
}
async fn fixture() -> Fixture {
    let replies = Arc::new(Mutex::new(VecDeque::<Reply>::new()));
    let unsigned = Arc::new(AtomicUsize::new(0));
    let signed = Arc::new(AtomicUsize::new(0));
    let signed_output = Arc::new(AtomicUsize::new(500));
    let rpc = Arc::new(AtomicUsize::new(0));
    let balance = Arc::new(AtomicUsize::new(10_000));
    let timestamp = crate::rotation::base::now().unwrap();
    let app=Router::new().route("/items",get({
        let (replies,unsigned,signed,signed_output)=(replies.clone(),unsigned.clone(),signed.clone(),signed_output.clone());
        move |headers:HeaderMap| {
            let (replies,unsigned,signed,signed_output)=(replies.clone(),unsigned.clone(),signed.clone(),signed_output.clone());
            async move {
                if headers.contains_key("payment-signature") {
                    signed.fetch_add(1,Ordering::SeqCst);
                    return match signed_output.load(Ordering::SeqCst) {
                        200 => ([("content-type","application/json")], "{\"temperature\":18}").into_response(),
                        201 => ([("content-type","application/json")], vec![0xffu8]).into_response(),
                        202 => ([("content-type","image/png")], STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aFh0AAAAASUVORK5CYII=").unwrap()).into_response(),
                        203 => ([("content-type","application/json")], vec![b'x'; 2048]).into_response(),
                        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                    };
                }
                unsigned.fetch_add(1,Ordering::SeqCst);
                let reply=replies.lock().unwrap().pop_front().expect("unexpected extra HTTP/probe");
                let (header,body)=match reply {
                    Reply::Free=>return (StatusCode::OK,"free").into_response(),
                    Reply::Malformed=>("%%%invalid".to_owned(),axum::body::Body::empty()),
                    Reply::Challenge(value)=>(STANDARD.encode(serde_json::to_vec(&value).unwrap()),axum::body::Body::empty()),
                    Reply::Held(value,started,release)=>{
                        let chunks=futures_util::stream::iter([Ok::<_,std::convert::Infallible>(axum::body::Bytes::from_static(b"first"))]).chain(futures_util::stream::once(async move {
                            started.notify_one();
                            release.acquire().await.unwrap().forget();
                            Ok(axum::body::Bytes::from_static(b"last"))
                        }));
                        (STANDARD.encode(serde_json::to_vec(&value).unwrap()),axum::body::Body::from_stream(chunks))
                    }
                };
                (StatusCode::PAYMENT_REQUIRED,[("payment-required",header)],body).into_response()
            }
        }
    })).route("/rpc",post({
        let rpc=rpc.clone();
        let balance=balance.clone();
        move |Json(v):Json<Value>| {let rpc=rpc.clone();let balance=balance.clone();async move {
            rpc.fetch_add(1,Ordering::SeqCst);
            let result=match v["method"].as_str().unwrap() {
                "eth_chainId"=>json!("0x2105"),
                "eth_getBlockByNumber"=>{
                    let tag=v["params"][0].as_str().unwrap();
                    let height=if matches!(tag,"latest"|"finalized"){if balance.load(Ordering::SeqCst)>0 {100} else {124}}else{u64::from_str_radix(&tag[2..],16).unwrap()};
                    json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{timestamp:x}")})
                },
                "eth_call"=>{
                    let amount=if v["params"][1]=="latest" {balance.load(Ordering::SeqCst)} else {10_000};
                    json!(format!("0x{amount:064x}"))
                },
                method=>panic!("unexpected RPC mutation {method}"),
            };
            Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
        }}
    }));
    let listener = crate::test_tls::listener().await;
    let base = format!("https://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let group = crate::rotation::test_capacity::declare_and_allocate!(
        state,
        "hints",
        crate::rotation::test_capacity::two_reserves("0.01"),
        Some(crate::rotation::config::positive_usdc("0.01").unwrap()),
        0
    )
    .unwrap();
    for job in state.funding_jobs().unwrap() {
        state
            .record_credit(&job.wallet_id, "10000", "fixture", 100)
            .unwrap();
    }
    let (store, worker) = StoreHandle::spawn(state);
    let manager = ManagedPool::new(
        store.clone(),
        group,
        BaseRpc::new(&format!("{base}/rpc"), 12).unwrap(),
        SpendPolicy::dollars("0.01").unwrap(),
    )
    .unwrap();
    // Initial credit already established the positive estimate and claim. The
    // first scalar observation below establishes this fixture's zero estimates.
    balance.store(0, Ordering::SeqCst);
    manager.reconcile().await.unwrap();
    rpc.store(0, Ordering::SeqCst);
    let client = PaidClient::managed(Arc::new(manager)).with_source_identity("hint-fixture");
    Fixture {
        hints: crate::pricing::runtime::fixture(dir.path()).await,
        _dir: dir,
        client,
        store,
        worker,
        server,
        base,
        replies,
        unsigned,
        signed,
        signed_output,
        rpc,
    }
}
impl Fixture {
    fn key(&self, template: &str) -> OperationKey {
        self.hints.operation("GET", template)
    }
    fn route(&self) -> RoutedRequest {
        RoutedRequest {
            headers: Default::default(),
            method: "GET".into(),
            url: format!("{}/items", self.base),
            query: Default::default(),
            body: None,
        }
    }
    async fn call(&self, key: OperationKey, reply: Reply) -> Result<crate::output::HttpOutput> {
        self.replies.lock().unwrap().push_back(reply);
        self.client
            .execute_response_scoped(
                self.route(),
                &SubmissionProgress::default(),
                Some(key.clone()),
            )
            .await
    }
    async fn close(self) {
        self.client.drain().await;
        let db = rusqlite::Connection::open(self._dir.path().join("state/state.sqlite")).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            self.signed.load(Ordering::SeqCst) as i64
        );
        drop(db);
        drop(self.client);
        drop(self.store);
        self.worker.await.unwrap();
        self.server.abort();
    }
}

#[tokio::test]
async fn valid_selected_lowest_price_learns_before_financial_failure_and_updates_downward() {
    let f = fixture().await;
    let key = f.key("/items/{id}");
    let error = f
        .call(key.clone(), Reply::Challenge(challenge(&[9000, 7000])))
        .await
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<HttpFailure>()
            .is_some_and(|e| e.status == StatusCode::INTERNAL_SERVER_ERROR),
        "{error:#}"
    );
    assert_eq!(
        crate::output::ObservedPrice::from_error(&error)
            .unwrap()
            .amount,
        U256::from(7000)
    );
    assert_eq!(key.get(), Some(U256::from(7000)));
    assert_eq!(
        f.rpc.load(Ordering::SeqCst),
        0,
        "advisory insufficiency cannot require a balance RPC"
    );
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
    let original = key.saved_receipt().unwrap();
    assert!(
        f.call(key.clone(), Reply::Challenge(challenge(&[3000])))
            .await
            .is_err()
    );
    assert_eq!(key.get(), Some(U256::from(3000)));
    assert!(key.saved_receipt().unwrap() != original);
    assert_eq!(f.unsigned.load(Ordering::SeqCst), 2);
    f.close().await;
}

#[tokio::test]
async fn invalid_unsupported_overcap_incomplete_and_free_responses_never_teach_or_renew() {
    let f = fixture().await;
    let learned = f.key("/learned");
    let absent = f.key("/absent");
    assert!(
        f.call(learned.clone(), Reply::Challenge(challenge(&[7000])))
            .await
            .is_err()
    );
    let receipt = learned.saved_receipt().unwrap();
    let rpc = f.rpc.load(Ordering::SeqCst);
    for mode in 0..5 {
        for key in [learned.clone(), absent.clone()] {
            let reply = match mode {
                0 => Reply::Malformed,
                1 => {
                    let mut value = challenge(&[3000]);
                    value["accepts"][0]["network"] = json!("eip155:1");
                    Reply::Challenge(value)
                }
                2 => Reply::Challenge(challenge(&[10001])),
                3 => {
                    let mut value = challenge(&[3000]);
                    value["resource"].as_object_mut().unwrap().remove("url");
                    Reply::Challenge(value)
                }
                _ => Reply::Free,
            };
            let result = f.call(key.clone(), reply).await;
            assert_eq!(result.is_ok(), mode == 4);
            assert!(match &result {
                Ok(output) => output.observed_price.is_none(),
                Err(error) => crate::output::ObservedPrice::from_error(error).is_none(),
            });
            assert_eq!(learned.get(), Some(U256::from(7000)));
            assert!(learned.saved_receipt() == Some(receipt));
            assert!(absent.get().is_none());
        }
    }
    assert_eq!(
        f.rpc.load(Ordering::SeqCst),
        rpc,
        "invalid/free responses cannot reach financial acquisition"
    );
    assert_eq!(f.unsigned.load(Ordering::SeqCst), 11);
    f.close().await;
}

#[tokio::test]
async fn cached_price_above_a_changed_cap_does_not_veto_a_cheaper_current_offer() {
    let f = fixture().await;
    let key = f.key("/items/{id}");
    assert!(
        f.call(key.clone(), Reply::Challenge(challenge(&[7000])))
            .await
            .is_err()
    );
    assert_eq!(key.get(), Some(U256::from(7000)));
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
    f.store
        .call(|s| {
            let pool = s
                .status()?
                .pools
                .into_iter()
                .find(|p| p.name == "hints")
                .unwrap()
                .id;
            s.configure_capacity_group(
                &pool,
                Some(U256::from(5000)),
                0,
                &crate::rotation::test_capacity::two_reserves("0.01"),
            )
        })
        .await
        .unwrap();
    // A valid observed offer teaches a hint even when current Store authority
    // refuses it. The hint cannot authorize signing or create a journal entry.
    let error = f
        .call(key.clone(), Reply::Challenge(challenge(&[6000])))
        .await
        .unwrap_err();
    assert!(
        matches!(
            error.downcast_ref::<crate::rotation::error::AdmissionError>(),
            Some(crate::rotation::error::AdmissionError::PriceLimit(_))
        ),
        "{error:#}"
    );
    assert_eq!(f.unsigned.load(Ordering::SeqCst), 2);
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
    assert_eq!(
        crate::output::ObservedPrice::from_error(&error)
            .unwrap()
            .amount,
        U256::from(6000)
    );
    assert_eq!(key.get(), Some(U256::from(6000)));
    let db = rusqlite::Connection::open(f._dir.path().join("state/state.sqlite")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM payment_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(db);
    let error = f
        .call(key.clone(), Reply::Challenge(challenge(&[3000])))
        .await
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<HttpFailure>()
            .is_some_and(|e| e.status == StatusCode::INTERNAL_SERVER_ERROR),
        "{error:#}"
    );
    assert_eq!(f.unsigned.load(Ordering::SeqCst), 3);
    assert_eq!(f.signed.load(Ordering::SeqCst), 2);
    assert_eq!(key.get(), Some(U256::from(3000)));
    assert_eq!(f.rpc.load(Ordering::SeqCst), 0);
    f.close().await;
}

#[tokio::test]
async fn earlier_headers_with_late_body_cannot_overwrite_a_newer_lower_price() {
    let f = fixture().await;
    let key = f.key("/items/{id}");
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    f.replies.lock().unwrap().push_back(Reply::Held(
        challenge(&[9000]),
        started.clone(),
        release.clone(),
    ));
    let (client, route) = (f.client.clone(), f.route());
    let first_key = key.clone();
    let first = tokio::spawn(async move {
        client
            .execute_response_scoped(route, &SubmissionProgress::default(), Some(first_key))
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), started.notified())
        .await
        .unwrap();
    assert!(
        key.get().is_none(),
        "unvalidated envelope cannot teach from headers alone"
    );
    let second = f
        .call(key.clone(), Reply::Challenge(challenge(&[3000])))
        .await
        .unwrap_err();
    assert_eq!(
        crate::output::ObservedPrice::from_error(&second)
            .unwrap()
            .amount,
        U256::from(3000)
    );
    let newer = key.saved_receipt().unwrap();
    release.add_permits(1);
    let first = first.await.unwrap().unwrap_err();
    assert_eq!(
        crate::output::ObservedPrice::from_error(&first)
            .unwrap()
            .amount,
        U256::from(9000)
    );
    assert_eq!(key.get(), Some(U256::from(3000)));
    assert!(key.saved_receipt() == Some(newer));
    let mut tools = crate::catalog::build_tools(&Default::default(), &json!({"paths":{
        "/items/{id}":{"get":{"summary":"Read item.","parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}}]}}
    }}), "priced").unwrap();
    for tool in &mut tools {
        tool.catalog_hints = Some(f.hints.clone());
    }
    let name = tools[0].name.clone();
    let server = crate::server::Server::new(tools, f.client.clone(), f.base.clone(), None);
    use rmcp::ServerHandler;
    assert_eq!(
        server.get_tool(&name).unwrap().description.as_deref(),
        Some("Read item. Observed cost: ~$0.003/call.")
    );
    drop(server);
    assert_eq!(f.unsigned.load(Ordering::SeqCst), 2);
    f.close().await;
}

#[tokio::test]
async fn managed_mcp_outputs_keep_their_offer_across_success_media_and_errors() {
    use crate::mcp_wire::McpResponse;
    let f = fixture().await;
    let mut tools = crate::catalog::build_tools(
        &Default::default(),
        &json!({"paths":{"/items":{"get":{}}}}),
        "priced",
    )
    .unwrap();
    for tool in &mut tools {
        tool.catalog_hints = Some(f.hints.clone());
    }
    let mut calls = 0;
    let mut signatures = 0;
    for discovered in [false, true] {
        let mut server = crate::server::Server::new(
            tools.clone(),
            f.client
                .clone()
                .with_download_limits(1024, crate::limits::HELP_BYTES),
            f.base.clone(),
            None,
        );
        server.discover_on_demand = discovered;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let http = crate::network::NetworkContext::new(Default::default())
            .unwrap()
            .discovery(&endpoint, None)
            .unwrap();
        let serving = tokio::spawn(async move {
            axum::serve(listener, crate::server::http_app(server, "fixture".into()))
                .await
                .unwrap()
        });
        for (mode, amount, is_error, blocks) in [
            (200, 5000, false, 2),
            (202, 1, false, 3),
            (201, 7000, true, 2),
            (203, 7000, true, 2),
            (500, 9000, true, 2),
        ] {
            f.signed_output.store(mode, Ordering::SeqCst);
            f.replies
                .lock()
                .unwrap()
                .push_back(Reply::Challenge(challenge(&[amount])));
            let params = if discovered {
                json!({"name":"x402_treazury_tool_call","arguments":{"tool_id":"priced_items","arguments":{}}})
            } else {
                json!({"name":"priced_items","arguments":{}})
            };
            let result: Value = http
                .post(&endpoint)
                .bearer_auth("fixture")
                .header("accept", "application/json, text/event-stream")
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":params}))
                .send()
                .await
                .unwrap()
                .mcp_json()
                .await
                .unwrap();
            assert_eq!(
                result["result"]["isError"].as_bool().unwrap_or(false),
                is_error,
                "{result}"
            );
            let content = result["result"]["content"].as_array().unwrap();
            assert_eq!(content.len(), blocks, "{result}");
            let price: Value =
                serde_json::from_str(content.last().unwrap()["text"].as_str().unwrap()).unwrap();
            assert_eq!(
                price,
                json!({"x402_pricing":{"amount":crate::output::ObservedPrice { amount: U256::from(amount) }.to_string(),"currency":"USDC","basis":"observed_offer"}})
            );
            assert!(result["result"].get("structuredContent").is_none());
            if mode == 200 {
                assert_eq!(content[0]["text"], "{\"temperature\":18}");
            }
            if mode == 202 {
                assert_eq!(content[1]["type"], "image");
            }
            if is_error {
                assert!(
                    content[0]["text"]
                        .as_str()
                        .unwrap()
                        .contains("Do not retry automatically.")
                );
            }
            let listing_request = if discovered {
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"x402_treazury_tools_search","arguments":{"query":"priced_items"}}})
            } else {
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
            };
            let listing: Value = http
                .post(&endpoint)
                .bearer_auth("fixture")
                .header("accept", "application/json, text/event-stream")
                .json(&listing_request)
                .send()
                .await
                .unwrap()
                .mcp_json()
                .await
                .unwrap();
            let description = if discovered {
                assert_eq!(listing["result"]["content"].as_array().unwrap().len(), 1);
                let search: Value =
                    serde_json::from_str(listing["result"]["content"][0]["text"].as_str().unwrap())
                        .unwrap();
                assert_eq!(search["items"][0]["tool_id"], "priced_items");
                assert_eq!(listing["result"]["structuredContent"], search);
                search["items"][0]["description"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            } else {
                let tool = listing["result"]["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|tool| tool["name"] == "priced_items")
                    .unwrap();
                tool["description"].as_str().unwrap().to_owned()
            };
            assert_eq!(
                description,
                format!(
                    "GET /items Observed cost: ~${}/call.",
                    crate::output::ObservedPrice {
                        amount: U256::from(amount)
                    }
                )
            );
            calls += 1;
            signatures += 1;
        }
        serving.abort();
    }
    assert_eq!(f.unsigned.load(Ordering::SeqCst), calls);
    assert_eq!(f.signed.load(Ordering::SeqCst), signatures);
    assert_eq!(f.rpc.load(Ordering::SeqCst), 0);
    f.close().await;
}

#[tokio::test]
async fn managed_body_rejection_retains_offer_status_and_liability_without_replay() {
    let f = fixture().await;
    f.signed_output.store(203, Ordering::SeqCst);
    f.replies
        .lock()
        .unwrap()
        .push_back(Reply::Challenge(challenge(&[17])));
    let error = f
        .client
        .clone()
        .with_download_limits(1024, crate::limits::HELP_BYTES)
        .execute_response_scoped(
            f.route(),
            &SubmissionProgress::default(),
            Some(f.key("/items")),
        )
        .await
        .unwrap_err();
    assert_eq!(
        crate::output::ObservedPrice::from_error(&error)
            .unwrap()
            .amount,
        U256::from(17)
    );
    assert_eq!(
        super::ResponseFacts::from_error(&error).unwrap().provider,
        Some(reqwest::StatusCode::OK)
    );
    assert!(error.is::<crate::limits::DownloadLimit>());
    assert!(error.is::<super::PossiblePayment>());
    let presented = super::present_api_error(&error).unwrap();
    assert!(presented.contains("max_response_bytes=1024"));
    assert_eq!(presented.matches("Do not retry automatically.").count(), 1);
    assert_eq!(f.unsigned.load(Ordering::SeqCst), 1);
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
    assert_eq!(f.rpc.load(Ordering::SeqCst), 0);
    f.close().await;
}

#[tokio::test]
async fn raw_calls_capture_without_a_hint_key_and_zero_is_not_synthesized_free() {
    let f = fixture().await;
    for amount in [0, 1] {
        f.replies
            .lock()
            .unwrap()
            .push_back(Reply::Challenge(challenge(&[amount])));
        let result = f
            .client
            .execute_manual_raw(f.route(), Default::default())
            .await;
        let price = match result {
            Ok(output) => output.observed_price,
            Err(error) => crate::output::ObservedPrice::from_error(&error),
        };
        assert_eq!(
            price.map(|price| price.amount),
            (amount > 0).then(|| U256::from(amount))
        );
        assert!(f.key("/items").get().is_none());
    }
    assert_eq!(f.unsigned.load(Ordering::SeqCst), 2);
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
    assert_eq!(f.rpc.load(Ordering::SeqCst), 0);
    f.close().await;
}

#[tokio::test]
async fn manual_authorization_keeps_captured_managed_payer_after_active_rotation_and_cancel() {
    let f = fixture().await;
    // Publish the first ACTIVE through a real authorization transaction.
    assert!(
        f.call(
            f.key("/rotation/bootstrap"),
            Reply::Challenge(challenge(&[100]))
        )
        .await
        .is_err()
    );
    let db = rusqlite::Connection::open(f._dir.path().join("state/state.sqlite")).unwrap();
    let original: String = db
        .query_row("SELECT id FROM wallets WHERE role='ACTIVE'", [], |r| {
            r.get(0)
        })
        .unwrap();
    // Synthetic advisory observations make a larger concurrent call prefer the
    // other existing reserve, while the small accepted call remains affordable.
    db.execute(
        "UPDATE wallets SET balance=CASE WHEN id=?1 THEN '1000' ELSE '10000' END",
        [&original],
    )
    .unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    f.replies.lock().unwrap().push_back(Reply::Held(
        challenge(&[100]),
        started.clone(),
        release.clone(),
    ));
    let (client, route) = (f.client.clone(), f.route());
    let headers = crate::discovery::raw::ManualRawHeaders::parse(Some(
        &json!({"Authorization":"Bearer manual-secret"}),
    ))
    .unwrap();
    let waiter = tokio::spawn(async move { client.execute_manual_raw(route, headers).await });
    tokio::time::timeout(std::time::Duration::from_secs(10), started.notified())
        .await
        .unwrap();
    let key = f.key("/rotation/sibling");
    key.learn(
        U256::from(9000),
        crate::pricing::runtime::received().unwrap(),
    );
    assert!(
        f.call(key.clone(), Reply::Challenge(challenge(&[9000])))
            .await
            .is_err()
    );
    let replacement: String = db
        .query_row("SELECT id FROM wallets WHERE role='ACTIVE'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_ne!(replacement, original);
    assert_eq!(
        db.query_row("SELECT role FROM wallets WHERE id=?1", [&original], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "RETAINED"
    );
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    release.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(10), f.client.drain())
        .await
        .unwrap();
    assert_eq!(
        f.unsigned.load(Ordering::SeqCst),
        3,
        "no second-payer probe for the private call"
    );
    assert_eq!(f.signed.load(Ordering::SeqCst), 3);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM payment_attempts WHERE wallet_id=?1",
            [&original],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM payment_attempts WHERE wallet_id=?1",
            [&replacement],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM funding_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2,
        "accepted work cannot allocate or fund"
    );
    assert_eq!(f.rpc.load(Ordering::SeqCst), 0);
    drop(db);
    f.close().await;
}
