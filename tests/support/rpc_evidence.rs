//! Obtain real opaque Base verification results from a local unfunded chain fixture.
#![allow(dead_code)]
use std::collections::BTreeMap;
use x402_treazury::rotation::base::{BaseRpc, ChainQuery, ChainView, now};

pub async fn acquire(query: ChainQuery, amounts: BTreeMap<String, u64>, head: u64) -> ChainView {
    use axum::{Json, Router, routing::post};
    use serde_json::{Value, json};
    let balances: BTreeMap<String, u64> = query
        .wallets
        .iter()
        .map(|(id, address)| {
            (
                address
                    .parse::<alloy_primitives::Address>()
                    .unwrap()
                    .to_string(),
                amounts[id],
            )
        })
        .collect();
    let app = Router::new().route("/",post(move |Json(request): Json<Value>| {
        let balances = balances.clone();
        async move {
            let result = match request["method"].as_str().unwrap() {
                "eth_chainId" => json!("0x2105"),
                "eth_getBlockByNumber" => {
                    let tag = request["params"][0].as_str().unwrap();
                    let height = if tag == "latest" { head } else {
                        u64::from_str_radix(tag.strip_prefix("0x").unwrap(),16).unwrap()
                    };
                    json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",now().unwrap())})
                },
                "eth_call" => {
                    assert_eq!(request["params"][1]["requireCanonical"],true);
                    let data = request["params"][0]["data"].as_str().unwrap();
                    let amount = if data.starts_with("0x70a08231") {
                        let address = format!("0x{}", &data[data.len()-40..])
                            .parse::<alloy_primitives::Address>().unwrap().to_string();
                        balances[&address]
                    } else { 0 };
                    json!(format!("0x{amount:064x}"))
                },
                other => panic!("unexpected mutation or unsupported fixture RPC: {other}"),
            };
            Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let result = BaseRpc::new(&url, 12).unwrap().view(query).await;
    server.abort();
    result.unwrap()
}

/// Capture only the requested wallet; fixture helpers never start a historical
/// nonce scan or manufacture opaque certificates.
pub fn credit_query(
    store: &x402_treazury::rotation::store::Store,
    wallet: &str,
) -> anyhow::Result<ChainQuery> {
    let pool = store
        .status()?
        .pools
        .into_iter()
        .find(|pool| pool.addresses.iter().any(|address| address.id == wallet))
        .ok_or_else(|| anyhow::anyhow!("fixture wallet not found"))?
        .id;
    let mut query = store.chain_query(&pool)?;
    query.wallets.retain(|(id, _)| id == wallet);
    query.captures.retain(|id, _| id == wallet);
    query.pending.clear();
    query.anchor = None;
    Ok(query)
}

/// Synchronous setup may be called inside a Tokio fixture. A separate scoped
/// runtime owns only this local RPC server, avoiding nested runtime/block_on.
pub fn credit(
    store: &mut x402_treazury::rotation::store::Store,
    wallet: &str,
    amount: &str,
    head: u64,
) -> anyhow::Result<()> {
    let query = credit_query(store, wallet)?;
    let amounts = [(wallet.to_owned(), amount.parse::<u64>()?)].into();
    let view = std::thread::scope(|scope| {
        scope
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(acquire(query, amounts, head))
            })
            .join()
            .unwrap()
    });
    store.record_verified_credit(wallet, view).map(|_| ())
}

pub async fn credit_owned(
    store: &x402_treazury::rotation::store::StoreHandle,
    wallet: String,
    amount: &str,
    head: u64,
) -> anyhow::Result<()> {
    let id = wallet.clone();
    let query = store.call(move |s| credit_query(s, &id)).await?;
    let view = acquire(
        query,
        [(wallet.clone(), amount.parse::<u64>()?)].into(),
        head,
    )
    .await;
    store
        .call(move |s| s.record_verified_credit(&wallet, view).map(|_| ()))
        .await
}
