//! Consume READY through the real paid admission/journal path; no promotion API.
#![allow(dead_code)]
use std::{collections::BTreeMap, path::Path, sync::Arc};
use x402_treazury::{
    catalog::RoutedRequest,
    payment::{PaidClient, SpendPolicy, USDC},
    rotation::{
        base::{BaseRpc, now},
        manager::ManagedPool,
        store::{AllocationKind, Store, StoreHandle},
    },
};

pub fn consume_only(store: Store, state: &Path, key: &Path, pool: &str) -> Store {
    consume(store, state, key, pool, false)
}
pub fn consume_and_replace(store: Store, state: &Path, key: &Path, pool: &str) -> Store {
    consume(store, state, key, pool, true)
}
fn consume(store: Store, state: &Path, key: &Path, pool: &str, replace: bool) -> Store {
    let treasury = store.id().to_owned();
    // Fixture RPC changes balances. Reopen creates an empty evidence namespace,
    // so a previous valid active certificate cannot legitimately retain binding.
    drop(store);
    let store = Store::open(state, key, &treasury).unwrap();
    let pool = pool.to_owned();
    std::thread::scope(|scope| {
        scope.spawn(move || {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
            use axum::{Json,Router,http::{HeaderMap,StatusCode},response::IntoResponse,routing::{get,post}};
            use base64::{Engine,engine::general_purpose::STANDARD};
            use serde_json::{Value,json};
            let ready=store.status().unwrap().pools.into_iter().find(|p|p.id==pool).unwrap().addresses.into_iter().find(|w|w.role=="READY").unwrap();
            let address=ready.address.to_lowercase();
            let app=Router::new().route("/rpc",post(move |Json(v):Json<Value>| {
                let address=address.clone();
                async move {
                    let result=match v["method"].as_str().unwrap() {
                        "eth_chainId"=>json!("0x2105"),
                        "eth_getBlockByNumber"=>{
                            let tag=v["params"][0].as_str().unwrap();
                            let height=if tag=="latest"{200}else{u64::from_str_radix(&tag[2..],16).unwrap()};
                            json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",now().unwrap())})
                        },
                        "eth_call"=>{
                            let data=v["params"][0]["data"].as_str().unwrap();
                            let amount=if data.starts_with("0x70a08231") && data.ends_with(&address[2..]) {1_000_000u64}else{0};
                            json!(format!("0x{amount:064x}"))
                        },
                        method=>panic!("unexpected fixture RPC {method}"),
                    };
                    Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
                }
            })).route("/pay",get(|headers:HeaderMap|async move {
                if headers.contains_key("payment-signature") {return (StatusCode::OK,"paid").into_response();}
                let challenge=json!({"x402Version":2,"resource":{"url":"http://localhost/pay","description":"fixture","mimeType":"text/plain"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":"1","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
                (StatusCode::PAYMENT_REQUIRED,[("payment-required",STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))],"payment required").into_response()
            }));
            let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url=format!("http://{}",listener.local_addr().unwrap());
            let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
            let (handle,owner)=StoreHandle::spawn(store);
            let manager=ManagedPool::new(handle.clone(),pool.clone(),BaseRpc::new(&format!("{url}/rpc"), 12).unwrap(),SpendPolicy::dollars("1").unwrap()).unwrap();
            let client=PaidClient::managed(Arc::new(manager));
            assert_eq!(client.execute(RoutedRequest {method:"GET".into(),url:format!("{url}/pay"),query:BTreeMap::new(),body:None}).await.unwrap(),"paid");
            let selected_pool=pool.clone();
            let expected_ready=ready.id.clone();
            handle.call(move |store| {
                let group=store.status()?.pools.into_iter().find(|p|p.id==selected_pool).unwrap();
                assert_eq!(group.addresses.iter().find(|w|w.role=="ACTIVE").unwrap().id,expected_ready);
                Ok(())
            }).await.unwrap();
            if replace {handle.call(move |store| {store.allocate_capacity_kind(&pool,AllocationKind::Small,true)?;Ok(())}).await.unwrap();}
            drop(client);drop(handle);owner.await.unwrap();server.abort();
        });
    }).join().unwrap()
    });
    Store::open(state, key, &treasury).unwrap()
}
