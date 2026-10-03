//! Pinned-block evidence and authorization expiry boundaries, with no signer.
use alloy_primitives::U256;
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use x402_treazury::rotation::base::{Anchor, BaseRpc, ChainQuery, PendingAuthorization, now};

#[tokio::test]
async fn balances_use_the_minimum_and_expiry_requires_a_later_confirmed_block() {
    for (stable, current) in [(2u64, 9u64), (9, 2)] {
        for (nonce, offset, released) in [
            (0u64, 0i64, false),
            (0, -1, true),
            (0, 1, false),
            (1, 1, true),
        ] {
            let stamp = now().unwrap() - 10;
            let requests = Arc::new(Mutex::new(vec![]));
            let logs = requests.clone();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(listener,Router::new().route("/",post(move |Json(v):Json<Value>|{
                    let logs=logs.clone();async move {
                        logs.lock().unwrap().push(v.clone());
                        let result=match v["method"].as_str().unwrap() {
                            "eth_chainId"=>json!("0x2105"),
                            "eth_getBlockByNumber"=>{
                                let height=if v["params"][0]=="latest" {100} else {u64::from_str_radix(v["params"][0].as_str().unwrap().trim_start_matches("0x"),16).unwrap()};
                                json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{stamp:x}")})
                            }
                            "eth_call"=>{
                                assert_eq!(v["params"][1]["requireCanonical"],true);
                                let n=if v["params"][0]["data"].as_str().unwrap().starts_with("0x70a08231") {
                                    if v["params"][1]["blockHash"]==format!("0x{:064x}",88) {stable} else {current}
                                } else {nonce};
                                json!(format!("0x{n:064x}"))
                            }
                            _=>panic!("unexpected RPC")
                        };
                        Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
                    }
                }))).await.unwrap();
            });
            let rpc = BaseRpc::new(&url, 12, 120).unwrap();
            let query = |anchor| ChainQuery {
                wallets: vec![("wallet".into(), format!("0x{:040x}", 1))],
                pending: vec![PendingAuthorization {
                    id: "attempt".into(),
                    wallet: "wallet".into(),
                    payer: format!("0x{:040x}", 1),
                    nonce: format!("0x{:064x}", 1),
                    valid_before: stamp.checked_add_signed(offset).unwrap(),
                }],
                anchor,
            };
            let view = rpc.view(query(None)).await.unwrap();
            assert_eq!(view.balances["wallet"], U256::from(2));
            assert_eq!(
                view.released,
                if released {
                    vec!["attempt".to_owned()]
                } else {
                    vec![]
                }
            );
            let calls = requests.lock().unwrap().clone();
            let calls = calls
                .iter()
                .filter(|v| v["method"] == "eth_call")
                .collect::<Vec<_>>();
            assert_eq!(calls.len(), 3);
            for (call, height) in calls.into_iter().zip([88, 100, 88]) {
                assert_eq!(
                    call["params"][1],
                    json!({"blockHash":format!("0x{height:064x}"),"requireCanonical":true})
                );
            }
            assert!(
                rpc.view(query(Some(Anchor {
                    height: 89,
                    hash: format!("0x{:064x}", 89)
                })))
                .await
                .is_err(),
                "regressing confirmed height"
            );
            server.abort();
        }
    }
}
