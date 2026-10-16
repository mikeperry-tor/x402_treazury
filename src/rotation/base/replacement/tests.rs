use super::*;
use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::post};
use std::sync::{Arc, Mutex};

struct State {
    requests: Vec<Value>,
    head: u64,
    fail: bool,
    fail_at: Option<usize>,
    reorg: bool,
    nonce: u64,
}
type Shared = Arc<Mutex<State>>;

async fn fixture() -> (BaseRpc, Shared, tokio::task::JoinHandle<()>, u64) {
    let stamp = now().unwrap();
    let state = Arc::new(Mutex::new(State {
        requests: vec![],
        head: 100,
        fail: false,
        fail_at: None,
        reorg: false,
        nonce: 1,
    }));
    let app = Router::new().route("/", post({
        let state = state.clone();
        move |Json(v): Json<Value>| {
            let state = state.clone();
            async move {
                let mut state = state.lock().unwrap();
                state.requests.push(v.clone());
                if state.fail || state.fail_at == Some(state.requests.len()) { return StatusCode::SERVICE_UNAVAILABLE.into_response(); }
                let result = match v["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_getBlockByNumber" => {
                        let tag = v["params"][0].as_str().unwrap();
                        let height = if tag == "latest" { state.head } else { u64::from_str_radix(&tag[2..],16).unwrap() };
                        let hash = if state.reorg && height == 88 { 999 } else { height };
                        let time = if height <= 100 { stamp - 500 } else { stamp };
                        json!({"number":format!("0x{height:x}"),"hash":format!("0x{hash:064x}"),"timestamp":format!("0x{time:x}")})
                    }
                    "eth_call" => {
                        assert_eq!(v["params"][1]["requireCanonical"],true);
                        let data = v["params"][0]["data"].as_str().unwrap();
                        let amount = if data.starts_with("0x70a08231") { 100 } else {
                            assert_eq!(data.len(), 138);
                            assert_eq!(v["params"][1]["blockHash"],format!("0x{:064x}",88));
                            state.nonce
                        };
                        json!(format!("0x{amount:064x}"))
                    }
                    method => panic!("unexpected method {method}"),
                };
                Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result})).into_response()
            }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc = BaseRpc::new(
        &format!("http://{}/", listener.local_addr().unwrap()),
        12,
        120,
    )
    .unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (rpc, state, server, stamp)
}
fn capture() -> EvidenceCapture {
    let old = Anchor {
        height: 1,
        hash: format!("0x{:064x}", 999),
    };
    EvidenceCapture {
        capacity_snapshot: None,
        namespace: uuid::Uuid::new_v4(),
        wallet: "wallet".into(),
        generation: 7,
        endpoint_generations: BTreeMap::new(),
        checkpoint: Some(old.clone()),
        previous: None,
        previous_confirmed_covered: false,
        durable_balance_anchors: vec![old],
    }
}
fn auth(n: u64, expiry: u64) -> PendingAuthorization {
    PendingAuthorization {
        id: n.to_string(),
        wallet: "wallet".into(),
        payer: format!("0x{:040x}", 1),
        nonce: format!("0x{n:064x}"),
        valid_before: expiry,
    }
}

#[tokio::test]
async fn pages_keep_current_anchor_and_final_balance_is_fresh_without_old_hashes() {
    let (rpc, state, server, stamp) = fixture().await;
    let original = capture();
    let pass = rpc
        .begin_replacement(original.clone(), auth(1, 0).payer, Some(150))
        .await
        .unwrap();
    assert!(pass.capture() == &original);
    assert_eq!(pass.anchor().height, 88);
    let page = rpc.replacement_page(&pass, vec![auth(1, 0)]).await.unwrap();
    assert_eq!(page.pass(), pass.id());
    assert_eq!(page.resolutions()[0].outcome(), &AuthorizationOutcome::Used);
    state.lock().unwrap().nonce = 0;
    let page = rpc
        .replacement_page(&pass, vec![auth(2, stamp - 501), auth(3, stamp - 500)])
        .await
        .unwrap();
    assert_eq!(
        page.resolutions()[0].outcome(),
        &AuthorizationOutcome::ExpiredUnused
    );
    assert_eq!(page.unresolved()[0].id, "3");
    assert!(
        rpc.finish_replacement(&pass).await.is_err(),
        "old current balances must fail freshness"
    );
    state.lock().unwrap().head = 200;
    let final_balance = rpc.finish_replacement(&pass).await.unwrap();
    assert_eq!(final_balance.pass(), pass.id());
    assert_eq!(
        final_balance.view().observations["wallet"]
            .confirmed
            .anchor
            .height,
        188
    );
    assert!(
        state
            .lock()
            .unwrap()
            .requests
            .iter()
            .all(|r| r["method"] != "eth_getBlockByNumber" || r["params"][0] != "0x1")
    );
    server.abort();
}

#[tokio::test]
async fn page_scope_reorg_malformed_nonce_and_numeric_floor_fail_closed() {
    let (rpc, state, server, stamp) = fixture().await;
    let pass = rpc
        .begin_replacement(capture(), auth(1, 0).payer, Some(189))
        .await
        .unwrap();
    let before = state.lock().unwrap().requests.len();
    for page in [
        vec![auth(1, 0); 65],
        vec![auth(1, 0), auth(1, 0)],
        vec![PendingAuthorization {
            wallet: "other".into(),
            ..auth(1, 0)
        }],
    ] {
        assert!(rpc.replacement_page(&pass, page).await.is_err());
    }
    assert_eq!(state.lock().unwrap().requests.len(), before);
    state.lock().unwrap().nonce = 2;
    assert!(
        rpc.replacement_page(&pass, vec![auth(1, stamp)])
            .await
            .is_err()
    );
    state.lock().unwrap().head = 200;
    let error = rpc.finish_replacement(&pass).await.err().unwrap();
    assert!(matches!(
        error.downcast_ref::<evidence::EvidenceFailure>(),
        Some(evidence::EvidenceFailure::BeforeDebit)
    ));
    state.lock().unwrap().reorg = true;
    assert!(rpc.replacement_page(&pass, vec![auth(1, 0)]).await.is_err());
    assert!(rpc.finish_replacement(&pass).await.is_err());
    server.abort();
}

#[tokio::test]
async fn fallback_is_selected_only_before_pass_escapes() {
    let (mut primary, pstate, pserver, _) = fixture().await;
    let (secondary, sstate, sserver, _) = fixture().await;
    primary.fallbacks.push(secondary);
    pstate.lock().unwrap().fail = true;
    let pass = primary
        .begin_replacement(capture(), auth(1, 0).payer, None)
        .await
        .unwrap();
    let primary_count = pstate.lock().unwrap().requests.len();
    primary
        .replacement_page(&pass, vec![auth(1, 0)])
        .await
        .unwrap();
    assert_eq!(pstate.lock().unwrap().requests.len(), primary_count);
    sstate.lock().unwrap().fail = true;
    pstate.lock().unwrap().fail = false;
    assert!(
        primary
            .replacement_page(&pass, vec![auth(2, 0)])
            .await
            .is_err()
    );
    assert_eq!(pstate.lock().unwrap().requests.len(), primary_count);
    pserver.abort();
    sserver.abort();
}

#[tokio::test]
async fn retryable_begin_recheck_failure_can_select_fallback() {
    let (mut primary, pstate, pserver, _) = fixture().await;
    let (secondary, sstate, sserver, _) = fixture().await;
    primary.fallbacks.push(secondary);
    // Three initial requests precede the extra chain identity recheck.
    pstate.lock().unwrap().fail_at = Some(4);
    let pass = primary
        .begin_replacement(capture(), auth(1, 0).payer, None)
        .await
        .unwrap();
    assert_eq!(pstate.lock().unwrap().requests.len(), 4);
    assert!(!sstate.lock().unwrap().requests.is_empty());
    primary
        .replacement_page(&pass, vec![auth(1, 0)])
        .await
        .unwrap();
    assert_eq!(pstate.lock().unwrap().requests.len(), 4);
    pserver.abort();
    sserver.abort();
}
