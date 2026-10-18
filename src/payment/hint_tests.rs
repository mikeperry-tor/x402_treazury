//! Learn only from normal unsigned HTTP challenges, after full SDK validation.
use super::*;
use crate::{
    pricing::runtime::{OperationKey, process_hints},
    rotation::{
        base::BaseRpc,
        manager::ManagedPool,
        store::{Store, StoreHandle},
    },
};
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
    store: StoreHandle,
    worker: tokio::task::JoinHandle<()>,
    server: tokio::task::JoinHandle<()>,
    base: String,
    replies: Arc<Mutex<VecDeque<Reply>>>,
    unsigned: Arc<AtomicUsize>,
    signed: Arc<AtomicUsize>,
    rpc: Arc<AtomicUsize>,
}
fn challenge(amounts: &[u64]) -> Value {
    json!({"x402Version":2,"resource":{"url":"https://fixture.example/items","description":"fixture","mimeType":"text/plain"},"accepts":amounts.iter().map(|amount|json!({"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":amount.to_string(),"payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}})).collect::<Vec<_>>()})
}
async fn fixture() -> Fixture {
    let replies = Arc::new(Mutex::new(VecDeque::<Reply>::new()));
    let unsigned = Arc::new(AtomicUsize::new(0));
    let signed = Arc::new(AtomicUsize::new(0));
    let rpc = Arc::new(AtomicUsize::new(0));
    let balance = Arc::new(AtomicUsize::new(10_000));
    let app=Router::new().route("/items",get({
        let (replies,unsigned,signed)=(replies.clone(),unsigned.clone(),signed.clone());
        move |headers:HeaderMap| {
            let (replies,unsigned,signed)=(replies.clone(),unsigned.clone(),signed.clone());
            async move {
                if headers.contains_key("payment-signature") {
                    signed.fetch_add(1,Ordering::SeqCst);
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
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
                    let height=if tag=="latest"{if balance.load(Ordering::SeqCst)>0 {100} else {124}}else{u64::from_str_radix(&tag[2..],16).unwrap()};
                    json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",crate::rotation::base::now().unwrap())})
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
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
    manager.reconcile().await.unwrap();
    balance.store(0, Ordering::SeqCst);
    manager.reconcile().await.unwrap();
    rpc.store(0, Ordering::SeqCst);
    let client = PaidClient::managed(Arc::new(manager)).with_source_identity("hint-fixture");
    Fixture {
        _dir: dir,
        client,
        store,
        worker,
        server,
        base,
        replies,
        unsigned,
        signed,
        rpc,
    }
}
impl Fixture {
    fn key(&self, template: &str) -> OperationKey {
        self.client
            .operation_key(None, "GET", template, &self.base)
            .unwrap()
    }
    fn route(&self) -> RoutedRequest {
        RoutedRequest {
            method: "GET".into(),
            url: format!("{}/items", self.base),
            query: Default::default(),
            body: None,
        }
    }
    async fn call(&self, key: OperationKey, reply: Reply) -> Result<crate::output::HttpOutput> {
        self.replies.lock().unwrap().push_back(reply);
        self.client
            .execute_response_scoped(self.route(), &SubmissionProgress::default(), Some(key))
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
        .call(key, Reply::Challenge(challenge(&[9000, 7000])))
        .await
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<HttpFailure>()
            .is_some_and(|e| e.status == StatusCode::INTERNAL_SERVER_ERROR),
        "{error:#}"
    );
    assert_eq!(process_hints().get(key), Some(U256::from(7000)));
    assert_eq!(
        f.rpc.load(Ordering::SeqCst),
        0,
        "advisory insufficiency cannot require a balance RPC"
    );
    assert_eq!(f.signed.load(Ordering::SeqCst), 1);
    let original = process_hints().saved_receipt(key).unwrap();
    assert!(
        f.call(key, Reply::Challenge(challenge(&[3000])))
            .await
            .is_err()
    );
    assert_eq!(process_hints().get(key), Some(U256::from(3000)));
    assert!(process_hints().saved_receipt(key).unwrap() != original);
    assert_eq!(f.unsigned.load(Ordering::SeqCst), 2);
    f.close().await;
}

#[tokio::test]
async fn invalid_unsupported_overcap_incomplete_and_free_responses_never_teach_or_renew() {
    let f = fixture().await;
    let learned = f.key("/learned");
    let absent = f.key("/absent");
    assert!(
        f.call(learned, Reply::Challenge(challenge(&[7000])))
            .await
            .is_err()
    );
    let receipt = process_hints().saved_receipt(learned).unwrap();
    let rpc = f.rpc.load(Ordering::SeqCst);
    for mode in 0..5 {
        for key in [learned, absent] {
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
            let result = f.call(key, reply).await;
            assert_eq!(result.is_ok(), mode == 4);
            assert_eq!(process_hints().get(learned), Some(U256::from(7000)));
            assert!(process_hints().saved_receipt(learned) == Some(receipt));
            assert!(process_hints().get(absent).is_none());
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
    let first = tokio::spawn(async move {
        client
            .execute_response_scoped(route, &SubmissionProgress::default(), Some(key))
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), started.notified())
        .await
        .unwrap();
    assert!(
        process_hints().get(key).is_none(),
        "unvalidated envelope cannot teach from headers alone"
    );
    assert!(
        f.call(key, Reply::Challenge(challenge(&[3000])))
            .await
            .is_err()
    );
    let newer = process_hints().saved_receipt(key).unwrap();
    release.add_permits(1);
    assert!(first.await.unwrap().is_err());
    assert_eq!(process_hints().get(key), Some(U256::from(3000)));
    assert!(process_hints().saved_receipt(key) == Some(newer));
    assert_eq!(f.unsigned.load(Ordering::SeqCst), 2);
    f.close().await;
}
