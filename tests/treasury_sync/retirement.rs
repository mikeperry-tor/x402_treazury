//! Real finite-sync output for pending retirement and independent failed observations.
use alloy_primitives::U256;
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use x402_treazury::rotation::{
    base::{BaseRpc, now},
    store::{AllocationKind, Store},
};

#[tokio::test]
async fn sync_cli_explains_pending_retirement_and_preserves_failed_wallet_results() {
    for failure in ["none", "canonical", "scalar", "policy"] {
        let dir = tempfile::tempdir().unwrap();
        // An authenticated incompatible snapshot selects the existing Store-only
        // runtime; this fixture needs no indexer, SDK scan or funding owner.
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"unfunded-store-only-fixture",
        )
        .unwrap();
        let id = store.id().to_owned();
        let head = Arc::new(AtomicU64::new(100));
        let failed_payer = Arc::new(Mutex::new(String::new()));
        let rpc_head = head.clone();
        let rpc_payer = failed_payer.clone();
        let app = Router::new().route("/", post(move |Json(request): Json<Value>| {
            let head = rpc_head.load(Ordering::SeqCst);
            let failed_payer = rpc_payer.lock().unwrap().clone();
            async move {
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_getBlockByNumber" => {
                        let tag = request["params"][0].as_str().unwrap();
                        let height = if tag == "latest" { head } else { u64::from_str_radix(&tag[2..], 16).unwrap() };
                        json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}", now().unwrap())})
                    }
                    "eth_call" => {
                        let data = request["params"][0]["data"].as_str().unwrap();
                        let balance = data.starts_with("0x70a08231");
                        let unavailable = head > 100 && match failure {
                            "canonical" => !balance && data.ends_with(&format!("{:064x}", 1)),
                            "scalar" => balance && request["params"][1] == "latest" && data.ends_with(&failed_payer),
                            _ => false,
                        };
                        if unavailable {
                            return Json(json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32000,"message":"private-fixture-failure"}}));
                        }
                        json!(format!("0x{:064x}", if balance && head == 100 {2_000_000u64} else {0}))
                    }
                    method => panic!("unexpected mutation {method}"),
                };
                Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let base = BaseRpc::new(&endpoint, 12).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let reserves = super::capacity_fixture::two_reserves("2");
        let pool = store
            .ensure_capacity_group("configured", &reserves, Some(U256::from(1_000_000)), 1_000)
            .unwrap();
        let mut wallets = Vec::new();
        for _ in 0..2 {
            store
                .configure_capacity_group(&pool, Some(U256::from(1_000_000)), 1_000, &reserves)
                .unwrap();
            let job = store
                .allocate_capacity_kind(&pool, AllocationKind::Large, true)
                .unwrap()
                .unwrap();
            let wallet = store.funding_job(&job).unwrap().unwrap().wallet_id;
            let view = base.view(store.chain_query(&pool).unwrap()).await.unwrap();
            store.record_verified_credit(&wallet, view).unwrap();
            store
                .configure_capacity_group(
                    &pool,
                    Some(U256::from(1_000_000)),
                    1_000,
                    &super::capacity_fixture::small_reserve("2"),
                )
                .unwrap();
            wallets.push(wallet);
        }
        wallets.sort();
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        // Minimal original outstanding authorizations; the command must neither
        // replay them nor confuse later pending work with the first failed wallet.
        for (index, wallet) in wallets.iter().enumerate() {
            let payer: String = db
                .query_row("SELECT address FROM wallets WHERE id=?1", [wallet], |r| {
                    r.get(0)
                })
                .unwrap();
            if index == 0 {
                *failed_payer.lock().unwrap() = payer[2..].to_ascii_lowercase();
            }
            db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,nonce,valid_before,reconciliation_sequence) VALUES(?1,?2,?3,0,'1','fixture','POSSIBLY_SUBMITTED',?4,?5,?6,?7)", rusqlite::params![format!("attempt-{index}"),pool,wallet,payer,format!("0x{:064x}",index+1),i64::try_from(now().unwrap()+1_000).unwrap(),i64::try_from(index+1).unwrap()]).unwrap();
        }
        drop(store);
        head.store(200, Ordering::SeqCst);
        let config = dir.path().join("sync.toml");
        let retirement_setting = if failure == "policy" {
            "max_retirement_remainder_usdc='invalid'\n"
        } else {
            ""
        };
        std::fs::write(&config, format!("version=1\nservers={{}}\n[treasury]\nid='{id}'\nstate_dir='state'\nkey_file='key'\n[funding]\nbase_rpc_url_env='SYNC_BASE_RPC'\nbase_rpc_fallback_url_envs=[]\n[wallets.configured]\nmode='zcash_rotation'\nmax_api_payment_usdc='1'\n{retirement_setting}max_conversion_overhead_percent=5\n[wallets.configured.reserves.small]\nfunding_amount_usdc='2'\n")).unwrap();
        let expected = match failure {
            "canonical" => "usdc_retirement_unavailable",
            "scalar" => "usdc_balance_unavailable",
            "policy" => "usdc_inventory_unavailable",
            _ => "usdc_retirement_pending",
        };
        for json_output in [false, true] {
            let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"));
            command
                .env_clear()
                .env("RUST_LOG", "off")
                .env("SYNC_BASE_RPC", &endpoint)
                .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
                .args(["wallet", "sync", "--config"])
                .arg(&config);
            if json_output {
                command.arg("--json");
            }
            let output = command.output().await.unwrap();
            assert!(!output.status.success());
            let stdout = String::from_utf8(output.stdout).unwrap();
            let stderr = String::from_utf8(output.stderr).unwrap();
            if json_output {
                let report: Value =
                    serde_json::from_str(&stdout).unwrap_or_else(|_| panic!("{stdout}\n{stderr}"));
                assert_eq!(report["observations"]["usdc"]["error"], expected);
                assert_eq!(
                    report["observations"]["usdc"]["attempted"],
                    if failure == "policy" { 3 } else { 2 }
                );
                assert_eq!(
                    report["observations"]["usdc"]["failed"],
                    usize::from(matches!(failure, "scalar" | "policy"))
                );
            } else {
                let reason = match failure {
                    "canonical" => "Retirement could not be verified",
                    "scalar" => "A managed USDC balance could not be observed",
                    "policy" => {
                        "Some managed USDC wallets or retirement settings could not be inspected"
                    }
                    _ => "Retirement remains pending",
                };
                assert!(
                    stdout.contains(&format!("Managed USDC detail: {reason}")),
                    "{failure}: {stdout}\n{stderr}"
                );
            }
            assert!(!stdout.contains("private-fixture-failure"));
            assert_eq!(
                db.query_row(
                    "SELECT COUNT(*) FROM payment_attempts WHERE state='POSSIBLY_SUBMITTED'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                2
            );
            for table in [
                "wallet_retirements",
                "payment_resolutions",
                "outgoing",
                "treasury_operations",
                "operation_network",
            ] {
                assert_eq!(
                    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))
                        .unwrap(),
                    0,
                    "forbidden side effect in {table}"
                );
            }
        }
        server.abort();
    }
}
