//! Financial evidence follows finalized chain state, not depth or local time alone.
use super::*;
use axum::{Json, Router, extract::State, routing::post};
use std::sync::{Arc, Mutex};

struct Chain {
    finalized: u64,
    mode: &'static str,
    requests: Vec<Value>,
}

async fn fixture(mode: &'static str) -> (BaseRpc, Arc<Mutex<Chain>>, tokio::task::JoinHandle<()>) {
    let chain = Arc::new(Mutex::new(Chain {
        finalized: 80,
        mode,
        requests: vec![],
    }));
    let app = Router::new().route("/", post(|State(chain): State<Arc<Mutex<Chain>>>, Json(request): Json<Value>| async move {
        let mut chain = chain.lock().unwrap();
        chain.requests.push(request.clone());
        let result = match request["method"].as_str().unwrap() {
            "eth_chainId" => json!("0x2105"),
            "eth_getBlockByNumber" => {
                let tag = request["params"][0].as_str().unwrap();
                if tag == "finalized" && chain.mode == "unsupported" {
                    return Json(json!({"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"private unsupported tag"}}));
                }
                let height = match tag {
                    "latest" => 100,
                    "finalized" if chain.mode == "future" => 101,
                    "finalized" => chain.finalized,
                    _ => u64::from_str_radix(&tag[2..],16).unwrap(),
                };
                let repeated = chain.requests.iter().filter(|r| r["method"] == "eth_getBlockByNumber" && r["params"][0] == "0x50").count() > 1;
                let hash = if chain.mode == "shared_hash" || (chain.mode == "reorg" && tag == "0x50" && repeated)
                    || (chain.mode == "tag_mismatch" && tag == "finalized") {999} else {height};
                json!({"number":format!("0x{height:x}"),"hash":format!("0x{hash:064x}"),"timestamp":format!("0x{height:x}")})
            }
            "eth_call" => {
                let balance = request["params"][0]["data"].as_str().unwrap().starts_with("0x70a08231");
                json!(format!("0x{:064x}", u64::from(balance)))
            }
            method => panic!("unexpected RPC {method}"),
        };
        Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
    })).with_state(chain.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc = BaseRpc::new(&format!("http://{}/", listener.local_addr().unwrap()), 12).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (rpc, chain, server)
}

fn query() -> ChainQuery {
    let wallet = "wallet".to_owned();
    let payer = format!("0x{:040x}", 1);
    ChainQuery {
        wallets: vec![(wallet.clone(), payer.clone())],
        pending: vec![PendingAuthorization {
            id: "attempt".into(),
            wallet: wallet.clone(),
            payer,
            nonce: format!("0x{:064x}", 1),
            valid_before: 85,
        }],
        anchor: None,
        captures: [(
            wallet.clone(),
            EvidenceCapture {
                namespace: uuid::Uuid::new_v4(),
                wallet,
                generation: 0,
                endpoint_generations: Default::default(),
                checkpoint: None,
                previous: None,
                previous_confirmed_covered: false,
            },
        )]
        .into(),
    }
}

#[tokio::test]
async fn finalized_progress_and_minimum_depth_both_bound_authorization_expiry() {
    let (rpc, chain, server) = fixture("valid").await;
    let first = rpc.view(query()).await.unwrap();
    assert_eq!(first.observations["wallet"].confirmed.anchor.height, 80);
    assert_eq!(first.observations["wallet"].latest.anchor.height, 100);
    assert!(
        first.released.is_empty(),
        "latest time cannot expire an authorization before finalized time"
    );
    first.observations["wallet"].validate().unwrap();
    chain.lock().unwrap().finalized = 95;
    let next = rpc.view(query()).await.unwrap();
    assert_eq!(next.observations["wallet"].confirmed.anchor.height, 88);
    assert_eq!(
        next.resolutions["attempt"].outcome,
        AuthorizationOutcome::ExpiredUnused
    );
    assert_eq!(next.resolutions["attempt"].block_time, 88);
    assert_eq!(next.resolution_certificates["attempt"].anchor().height, 88);
    server.abort();
}

#[tokio::test]
async fn unavailable_finality_uses_only_configured_complete_evidence_and_never_depth_alone() {
    let (unsupported, requests, first) = fixture("unsupported").await;
    let error = unsupported.view(query()).await.err().unwrap();
    assert!(error.downcast_ref::<RpcFailure>().is_some());
    assert!(
        !requests
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|r| r["method"] == "eth_call")
    );
    let (healthy, _, second) = fixture("valid").await;
    let rpc = BaseRpc::with_fallbacks(&[unsupported.url.to_string(), healthy.url.to_string()], 12)
        .unwrap();
    assert_eq!(
        rpc.view(query()).await.unwrap().observations["wallet"]
            .confirmed
            .anchor
            .height,
        80
    );
    assert_eq!(
        unsupported
            .current_balance(&format!("0x{:040x}", 1))
            .await
            .unwrap(),
        U256::from(1),
        "scalar latest reads need no finalized tag"
    );
    first.abort();
    second.abort();
}

#[tokio::test]
async fn incoherent_or_replaced_finalized_anchors_never_publish_proofs() {
    for mode in ["future", "tag_mismatch", "reorg", "shared_hash"] {
        let (rpc, _, server) = fixture(mode).await;
        assert!(rpc.view(query()).await.is_err(), "{mode}");
        server.abort();
    }
}

#[tokio::test]
async fn checkpoint_above_replaced_head_is_scoped_recovery_without_impossible_lookup() {
    let (rpc, chain, server) = fixture("valid").await;
    let mut query = query();
    query.captures.get_mut("wallet").unwrap().checkpoint = Some(Anchor {
        height: 101,
        hash: format!("0x{:064x}", 101),
    });
    let error = rpc.view(query).await.err().unwrap();
    let conflict = error.downcast_ref::<EvidenceConflict>().unwrap();
    assert!(conflict.financial_checkpoint);
    assert_eq!(conflict.anchor.height, 101);
    assert!(
        !chain
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|r| r["params"][0] == "0x65")
    );
    server.abort();
}

#[tokio::test]
async fn captured_chain_conflict_does_not_shop_for_another_provider() {
    let (primary, _, first) = fixture("valid").await;
    let (fallback, untouched, second) = fixture("valid").await;
    let rpc =
        BaseRpc::with_fallbacks(&[primary.url.to_string(), fallback.url.to_string()], 12).unwrap();
    let mut query = query();
    query.captures.get_mut("wallet").unwrap().checkpoint = Some(Anchor {
        height: 99,
        hash: format!("0x{:064x}", 999),
    });
    let error = rpc.view(query).await.err().unwrap();
    assert!(
        error
            .downcast_ref::<EvidenceConflict>()
            .unwrap()
            .financial_checkpoint
    );
    assert!(untouched.lock().unwrap().requests.is_empty());
    first.abort();
    second.abort();
}
