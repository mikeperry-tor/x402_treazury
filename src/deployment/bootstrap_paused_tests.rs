use super::*;
use crate::rotation::store::funding::FundingPhase;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn paused_funding_observer_starts_with_serving_and_drains() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let owner = crate::treasury::Treasury::create(
        state.clone(), key.clone(), 2_000_000,
        Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into())),
    ).await.unwrap();
    crate::rotation::test_capacity::treasury_pair(
        &owner,
        "research",
        "5",
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0,
    )
    .await
    .unwrap();
    let before = owner.status().await.unwrap();
    owner.close().await.unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (seen, observed) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let rpc = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        assert!(socket.read(&mut bytes).await.unwrap() > 0);
        seen.send(()).unwrap();
        released.await.unwrap();
        socket.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
    });
    let config: MetaConfig = toml::from_str(&format!(
        r#"
version=1
[treasury]
id="{}"
state_dir="{}"
key_file="{}"
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
spec="unused.json"
[servers.one]
listen="127.0.0.1:0"
bearer_token_env="TOKEN"
wallet="research"
sources=["api"]
"#,
        before.treasury_id,
        state.display(),
        key.display()
    ))
    .unwrap();
    let path = dir.path().join("deployment.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    std::fs::write(dir.path().join("unused.json"), r#"{"servers":[{"url":"https://api.example"}],"operations":[{"method":"GET","path":"/read"}]}"#).unwrap();
    let env = BTreeMap::from([
        ("TOKEN".into(), "fixture".into()),
        ("BASE".into(), base),
        ("INDEXER".into(), "http://127.0.0.1:1".into()),
        ("SUBMIT".into(), "http://127.0.0.1:1".into()),
    ]);
    let deployment = tokio::time::timeout(
        Duration::from_secs(10),
        Deployment::load_for_serving_with_relay(&path, &env, Default::default()),
    )
    .await
    .unwrap()
    .unwrap();
    let running = deployment.bind(&env).await.unwrap();
    let store = running.runtime.store().unwrap();
    assert!(
        store
            .call(|s| s.funding_jobs())
            .await
            .unwrap()
            .iter()
            .all(|job| job.phase == FundingPhase::Allocated)
    );
    let stop = CancellationToken::new();
    let serving = tokio::spawn(running.serve(stop.clone()));
    tokio::time::timeout(Duration::from_secs(10), observed)
        .await
        .unwrap()
        .unwrap();
    stop.cancel();
    assert!(!serving.is_finished(), "accepted observation must drain");
    release.send(()).unwrap();
    rpc.await.unwrap();
    assert!(
        store
            .call(
                |s| Ok(crate::rotation::test_capacity::declare_and_allocate!(
                    s,
                    "forbidden",
                    crate::rotation::test_capacity::two_reserves("5"),
                    Some(alloy_primitives::U256::from(1_000_000u64)),
                    0
                )
                .unwrap_err()
                .to_string()
                    == "funding_disabled")
            )
            .await
            .unwrap()
    );
    drop(store);
    tokio::time::timeout(Duration::from_secs(10), serving)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let reopened = crate::rotation::store::status(&state).unwrap();
    assert_eq!(reopened.funding_jobs.len(), before.funding_jobs.len());
    for (original, current) in before.funding_jobs.iter().zip(&reopened.funding_jobs) {
        assert_eq!(current.operation_id, original.operation_id);
        assert_eq!(current.phase, FundingPhase::Allocated);
    }
    assert!(reopened.treasury_operations.is_empty());
}
