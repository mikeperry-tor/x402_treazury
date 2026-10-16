//! Startup failures must release managed relay clients before treasury shutdown.
#![cfg(feature = "zcash")]
use std::{collections::BTreeMap, time::Duration};
use x402_treazury::{deployment::Deployment, treasury::Treasury};

#[tokio::test]
async fn relay_startup_errors_finish_shutdown() {
    for (relay, missing_token) in [(false, false), (true, false), (true, true)] {
        let dir = tempfile::tempdir().unwrap();
        let owner = Treasury::create(
            dir.path().join("state"), dir.path().join("key"), 2_000_000,
            Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into())),
        ).await.unwrap();
        let id = owner.status().await.unwrap().treasury_id;
        owner.close().await.unwrap();
        std::fs::write(dir.path().join("spec.json"), r#"{"openapi":"3.0.0","info":{"title":"fixture","version":"1"},"paths":{"/curl":{"post":{"operationId":"curl","responses":{"200":{"description":"ok"}}}}}}"#).unwrap();
        std::fs::write(
            dir.path().join("relay.toml"),
            "spec=\"spec.json\"\nbase_url=\"https://relay.example\"\nprobe_pricing=false\n",
        )
        .unwrap();
        let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let policy = if relay {
            "[discovery_relay]\nprovider=\"relay.toml\"\nwallet=\"research\"\nserve=true\n"
        } else {
            ""
        };
        let config = format!(
            r#"
version=1
{policy}
[treasury]
state_dir="state"
key_file="key"
indexer_url_env="INDEXER"
submission_url_env="SUBMIT"
[funding]
auto_fund=false
poll_seconds=1
base_rpc_url_env="BASE"
base_rpc_fallback_url_envs=[]
[wallets.research]
mode="zcash_rotation"
reserves={{small={{funding_amount_usdc="5"}}}}
max_api_payment_usdc="1"
max_funding_spend_zec="0.02"
max_conversion_overhead_percent=5
[sources.api]
spec="spec.json"
base_url="https://api.example"
probe_pricing=false
[servers.one]
listen="{}"
bearer_token_env="TOKEN"
wallet="research"
sources=["api"]
"#,
            if missing_token {
                "127.0.0.1:0".parse().unwrap()
            } else {
                occupied.local_addr().unwrap()
            }
        );
        let path = dir.path().join("deployment.toml");
        std::fs::write(&path, config).unwrap();
        let env = BTreeMap::from([
            ("BASE".into(), "http://127.0.0.1:1".into()),
            ("INDEXER".into(), "http://127.0.0.1:1".into()),
            ("SUBMIT".into(), "http://127.0.0.1:1".into()),
            ("TOKEN".into(), "fixture-token".into()),
        ]);
        let mut bind_env = env.clone();
        if missing_token {
            bind_env.remove("TOKEN");
        }
        for _ in 0..2 {
            let result = tokio::time::timeout(Duration::from_secs(10), async {
                let deployment =
                    Deployment::load_for_serving_with_relay(&path, &env, Default::default())
                        .await?;
                deployment.bind(&bind_env).await
            })
            .await;
            let error = result
                .expect("startup error shutdown hung")
                .err()
                .expect("startup must fail");
            let text = format!("{error:#}");
            assert!(
                text.contains(if missing_token {
                    "TOKEN"
                } else {
                    "cannot bind"
                }),
                "unexpected error: {text}"
            );
        }
        let owner = Treasury::open(dir.path().join("state"), dir.path().join("key"), id)
            .await
            .unwrap();
        let status = owner.status().await.unwrap();
        assert!(!status.outgoing_pending);
        assert!(status.treasury_operations.is_empty());
        assert!(status.funding_jobs.is_empty());
        owner.close().await.unwrap();
    }
}
