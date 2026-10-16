//! Durable observation ownership does not manufacture a current payment profile.
use super::initialization_tests::{env, managed_config, write_spec};
use super::*;
use crate::rotation::store::{AllocationKind, Store};
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn static_config() -> String {
    let config = managed_config();
    let start = config.find("[wallets.selected]").unwrap();
    let end = config.find("[sources.api]").unwrap();
    format!(
        "{}[wallets.static]\nmode='static'\nprivate_key_env='STATIC'\n{}",
        &config[..start],
        &config[end..]
    )
    .replace("wallet='selected'", "wallet='static'")
}
fn durable_obligation(dir: &Path, retired: bool) -> (String, String) {
    let state = dir.join("state");
    let mut store = Store::create(&state, &dir.join("key"), 1, b"invalid SDK fixture").unwrap();
    let id = store.id().to_owned();
    let pool = store
        .ensure_capacity_group(
            "removed",
            &crate::rotation::store::test_support::reserves("5"),
            Some(alloy_primitives::U256::from(1_000_000)),
            0,
        )
        .unwrap();
    store
        .allocate_capacity_kind(&pool, AllocationKind::Small, true)
        .unwrap();
    let wallet = store.status().unwrap().pools.remove(0).addresses.remove(0);
    drop(store);
    let db = rusqlite::Connection::open(state.join("state.sqlite")).unwrap();
    db.execute("UPDATE pools SET enabled=0", []).unwrap();
    if retired {
        db.execute("UPDATE reserve_claims SET wallet_id=NULL", [])
            .unwrap();
        db.execute("UPDATE wallets SET role='RETIRED'", []).unwrap();
    }
    db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,payee,nonce,valid_after,valid_before,reconciliation_sequence) VALUES('retained-attempt',?1,?2,0,'1','fixture','POSSIBLY_SUBMITTED',?3,?3,?4,0,1,1)",rusqlite::params![pool,wallet.id,wallet.address,format!("0x{:064x}",7)]).unwrap();
    (id, pool)
}

#[tokio::test]
async fn removed_disabled_pool_keeps_only_observation_ownership_after_reopen() {
    for retired in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (treasury, pool) = durable_obligation(dir.path(), retired);
        write_spec(dir.path());
        let path = dir.path().join("deployment.toml");
        std::fs::write(&path, static_config()).unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        let seen = reads.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let rpc = tokio::spawn(async move {
            let app = Router::new().route("/", post(move |Json(v): Json<Value>| {
                let seen = seen.clone();
                async move {
                    let result = match v["method"].as_str().unwrap() {
                        "eth_chainId" => json!("0x2105"),
                        "eth_getBlockByNumber" => {
                            let tag = v["params"][0].as_str().unwrap();
                            let height = if tag == "latest" { 100 } else { u64::from_str_radix(&tag[2..],16).unwrap() };
                            json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",crate::rotation::base::now().unwrap())})
                        },
                        "eth_call" => {
                            if !v["params"][0]["data"].as_str().unwrap().starts_with("0x70a08231") {
                                seen.fetch_add(1, Ordering::SeqCst);
                            }
                            json!(format!("0x{:064x}",0))
                        },
                        other => panic!("unexpected RPC {other}"),
                    };
                    Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result}))
                }
            }));
            axum::serve(listener, app).await.unwrap();
        });
        let mut env = env();
        env.insert("STATIC".into(), format!("{:064x}", 1));
        env.insert("BASE".into(), url);
        let running = Deployment::load(&path)
            .await
            .unwrap()
            .bind(&env)
            .await
            .unwrap();
        assert!(
            running.managed_pools.is_empty(),
            "removed profile cannot become a payer"
        );
        assert_eq!(running.reconciliation_owners.len(), 1);
        assert_eq!(running.reconciliation_owners[0].pool_id(), pool);
        assert!(running.funding_runtime.is_none());
        let store = running.store_only.as_ref().unwrap().store.clone();
        let address = running.addresses()[0].1;
        let stop = CancellationToken::new();
        let serving = tokio::spawn(running.serve(stop.clone()));
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let group = pool.clone();
                if !store
                    .call(move |s| {
                        Ok(s.chain_query(&group)?
                            .pending
                            .iter()
                            .any(|a| a.id == "retained-attempt"))
                    })
                    .await
                    .unwrap()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(reads.load(Ordering::SeqCst) > 0);
        let response = reqwest::Client::new()
            .post(format!("http://{address}/mcp"))
            .bearer_auth("fixture-token")
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        drop(store);
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), serving)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let reopened = Store::open(
            &dir.path().join("state"),
            &dir.path().join("key"),
            &treasury,
        )
        .unwrap();
        let status = reopened.status().unwrap();
        assert_eq!(status.funding_jobs.len(), 1);
        assert_eq!(status.pools.len(), 1);
        assert!(!status.pools[0].enabled);
        assert_eq!(
            status.pools[0].payment_limit_atomic.as_deref(),
            Some("1000000")
        );
        if retired {
            assert_eq!(status.pools[0].addresses[0].role, "RETIRED");
        }
        assert_eq!(status.pools[0].addresses.len(), 1);
        assert!(status.treasury_operations.is_empty());
        assert_eq!(
            reopened.snapshot().unwrap().1.as_slice(),
            b"invalid SDK fixture"
        );
        rpc.abort();
    }
}

#[tokio::test]
async fn durable_observers_require_serving_scope_existing_state_and_configured_base() {
    for mode in [
        "subset",
        "strict",
        "missing_base",
        "missing_state",
        "missing_funding",
    ] {
        let dir = tempfile::tempdir().unwrap();
        if mode != "missing_state" {
            durable_obligation(dir.path(), false);
        }
        write_spec(dir.path());
        let path = dir.path().join("deployment.toml");
        let mut config = static_config();
        if mode == "missing_funding" {
            let start = config.find("[funding]").unwrap();
            let end = config.find("[wallets.static]").unwrap();
            config.replace_range(start..end, "");
        }
        std::fs::write(&path, config).unwrap();
        let mut env = env();
        env.insert("STATIC".into(), format!("{:064x}", 1));
        if mode == "missing_base" {
            env.remove("BASE");
        }
        let deployment = Deployment::load(&path).await.unwrap();
        let selected = BTreeSet::from(["static".to_owned()]);
        let initialized = deployment
            .initialize_wallets_mode(
                &env,
                crate::rotation::restriction::FundingRestriction::DenyNewFunding,
                (mode == "subset").then_some(&selected),
                &BTreeSet::new(),
                mode != "strict",
            )
            .await
            .unwrap();
        assert!(initialized.reconciliation_owners.is_empty(), "{mode}");
        assert!(initialized.managed_pools.is_empty());
        assert_eq!(
            initialized
                .wallets
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["static"]
        );
        assert!(initialized.funding_runtime.is_none());
        initialized.shutdown().await.unwrap();
        if mode == "missing_state" {
            assert!(!dir.path().join("state").exists());
            assert!(!dir.path().join("key").exists());
        }
    }
}
