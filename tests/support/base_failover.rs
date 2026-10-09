use axum::{Json, Router, extract::Path, http::StatusCode, response::IntoResponse, routing::post};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use x402_treazury::rotation::base::{ChainQuery, now};

pub type Requests = Arc<Mutex<Vec<(String, Value)>>>;
pub async fn fixture() -> (std::net::SocketAddr, Requests, tokio::task::JoinHandle<()>) {
    let requests = Requests::default();
    let logs = requests.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let block_time = now().unwrap();
    let app = Router::new().route("/{mode}", post(move |Path(mode): Path<String>, Json(v): Json<Value>| {
        let logs = logs.clone();
        async move {
            logs.lock().unwrap().push((mode.clone(), v.clone()));
            let method = v["method"].as_str().unwrap();
            if mode == "slow" { tokio::time::sleep(std::time::Duration::from_secs(2)).await; }
            if mode == "stall" { tokio::time::sleep(std::time::Duration::from_secs(60)).await; }
            if mode == "malformed" { return "private invalid JSON".into_response(); }
            // Fail only after some useful-looking evidence has been read.
            if let Ok(code) = mode.parse::<u16>()
                && method == "eth_call" && v["params"][1]["blockHash"] == format!("0x{:064x}", 100) {
                return (StatusCode::from_u16(code).unwrap(), "private error").into_response();
            }
            if mode == "limited" {
                return Json(json!({"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"private error"}})).into_response();
            }
            let result = match method {
                "eth_chainId" => json!(if mode == "wrong-chain" {"0x1"} else {"0x2105"}),
                "eth_getBlockByNumber" => {
                    let height = if v["params"][0] == "latest" {100} else {u64::from_str_radix(v["params"][0].as_str().unwrap().trim_start_matches("0x"), 16).unwrap()};
                    let hash = if mode == "conflict" {999} else {height};
                    json!({"number":format!("0x{height:x}"),"hash":format!("0x{hash:064x}"),"timestamp":format!("0x{block_time:x}")})
                }
                "eth_call" => {
                    assert_eq!(v["params"][1]["requireCanonical"], true);
                    json!(format!("0x{:064x}", if mode == "ok" {7} else {999}))
                }
                _ => panic!("unexpected non-read RPC"),
            };
            Json(json!({"jsonrpc":"2.0","id":1,"result":result})).into_response()
        }
    }));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (address, requests, server)
}
pub fn query() -> ChainQuery {
    ChainQuery {
        wallets: vec![
            ("one".into(), format!("0x{:040x}", 1)),
            ("two".into(), format!("0x{:040x}", 2)),
        ],
        pending: vec![],
        anchor: None,
    }
}
