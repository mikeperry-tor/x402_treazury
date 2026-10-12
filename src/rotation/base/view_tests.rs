//! RPC-level coverage of balance acquisition and independent historical anchors.
use super::*;
use axum::{Json, Router, routing::post};
use std::sync::{Arc, Mutex};

type Requests = Arc<Mutex<Vec<Value>>>;

async fn fixture(reorg: bool) -> (BaseRpc, Requests, tokio::task::JoinHandle<()>, u64) {
    let stamp = now().unwrap();
    let requests = Requests::default();
    let head = Arc::new(std::sync::atomic::AtomicU64::new(100));
    let app = Router::new().route("/", post({
        let requests = requests.clone();
        move |Json(v): Json<Value>| {
            let (requests, head) = (requests.clone(), head.clone());
            async move {
                let mut requests = requests.lock().unwrap();
                requests.push(v.clone());
                let result = match v["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_getBlockByNumber" => {
                        let tag = v["params"][0].as_str().unwrap();
                        let height = if tag == "latest" {
                            head.load(std::sync::atomic::Ordering::SeqCst)
                        } else {
                            u64::from_str_radix(&tag[2..], 16).unwrap()
                        };
                        let repeated = requests.iter().filter(|r| r["method"] == "eth_getBlockByNumber" && r["params"][0] == "0x58").count() > 1;
                        let hash = if reorg && height == 88 && repeated { 999 } else { height };
                        let timestamp = if height <= 100 { stamp - 60 } else { stamp };
                        json!({"number":format!("0x{height:x}"), "hash":format!("0x{hash:064x}"), "timestamp":format!("0x{timestamp:x}")})
                    },
                    "eth_call" => {
                        assert_eq!(v["params"][1]["requireCanonical"], true);
                        if v["params"][0]["data"].as_str().unwrap().starts_with("0x70a08231") {
                            json!(format!("0x{:064x}", 10))
                        } else {
                            // Advance while scanning historical authorization state.
                            assert_eq!(v["params"][1]["blockHash"], format!("0x{:064x}", 88));
                            head.store(200, std::sync::atomic::Ordering::SeqCst);
                            json!(format!("0x{:064x}", 1))
                        }
                    },
                    method => panic!("unexpected RPC {method}"),
                };
                Json(json!({"jsonrpc":"2.0", "id":v["id"], "result":result}))
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
    (rpc, requests, server, stamp)
}

fn query(history: bool) -> ChainQuery {
    let payer = format!("0x{:040x}", 1);
    ChainQuery {
        wallets: vec![("wallet".into(), payer.clone())],
        pending: if history {
            vec![PendingAuthorization {
                id: "attempt".into(),
                wallet: "wallet".into(),
                payer,
                nonce: format!("0x{:064x}", 1),
                valid_before: u64::MAX,
            }]
        } else {
            vec![]
        },
        anchor: None,
        captures: BTreeMap::new(),
    }
}

#[tokio::test]
async fn balance_only_reads_one_header_pair_and_rechecks_both_anchors() {
    let (rpc, requests, server, stamp) = fixture(false).await;
    let view = rpc.view(query(false)).await.unwrap();
    let observation = &view.observations["wallet"];
    assert_eq!(observation.confirmed.anchor.height, 88);
    assert_eq!(observation.latest.anchor.height, 100);
    assert_eq!(view.admission_valid_until, stamp - 60 + 120);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 7, "no redundant admission header fetches");
    let headers: Vec<_> = requests
        .iter()
        .filter(|r| r["method"] == "eth_getBlockByNumber")
        .map(|r| r["params"][0].as_str().unwrap())
        .collect();
    assert_eq!(headers, ["latest", "0x58", "0x58", "0x64"]);
    let balances: Vec<_> = requests
        .iter()
        .filter(|r| r["method"] == "eth_call")
        .map(|r| r["params"][1]["blockHash"].as_str().unwrap())
        .collect();
    assert_eq!(
        balances,
        [
            observation.confirmed.anchor.hash.as_str(),
            observation.latest.anchor.hash.as_str()
        ]
    );
    server.abort();
}

#[tokio::test]
async fn combined_history_acquires_new_admission_headers_without_relabeling_resolution() {
    let (rpc, requests, server, stamp) = fixture(false).await;
    let view = rpc.view(query(true)).await.unwrap();
    let observation = &view.observations["wallet"];
    assert_eq!(
        view.resolutions["attempt"].anchor.as_ref().unwrap().height,
        88
    );
    assert_eq!(view.resolutions["attempt"].block_time, stamp - 60);
    assert_eq!(observation.confirmed.anchor.height, 188);
    assert_eq!(observation.latest.anchor.height, 200);
    assert_eq!(view.admission_valid_until, stamp + 120);
    let requests = requests.lock().unwrap();
    let headers: Vec<_> = requests
        .iter()
        .filter(|r| r["method"] == "eth_getBlockByNumber")
        .map(|r| r["params"][0].as_str().unwrap())
        .collect();
    assert_eq!(
        headers,
        ["latest", "0x58", "latest", "0xbc", "0x58", "0xbc", "0xc8"]
    );
    let balances: Vec<_> = requests
        .iter()
        .filter(|r| {
            r["method"] == "eth_call"
                && r["params"][0]["data"]
                    .as_str()
                    .unwrap()
                    .starts_with("0x70a08231")
        })
        .map(|r| r["params"][1]["blockHash"].as_str().unwrap())
        .collect();
    assert_eq!(
        balances,
        [
            observation.confirmed.anchor.hash.as_str(),
            observation.latest.anchor.hash.as_str()
        ]
    );
    server.abort();
}

#[tokio::test]
async fn balance_only_rejects_anchor_replacement_after_balance_reads() {
    let (rpc, _, server, _) = fixture(true).await;
    let error = rpc.view(query(false)).await.err().unwrap();
    assert!(
        error
            .to_string()
            .contains("Base view changed during reconciliation")
    );
    server.abort();
}
