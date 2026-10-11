use super::*;
use crate::rotation::store::funding::FundingPhase;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn paused_funding_observer_survives_startup_handoff_and_drains() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let owner = crate::treasury::Treasury::create(
        state.clone(), key.clone(), 2_000_000,
        Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into())),
    ).await.unwrap();
    owner
        .ensure_pool("research".into(), "5".into())
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
daily_funding_limit_usdc="10"
poll_seconds=1
base_rpc_url_env="BASE"
base_rpc_fallback_url_envs=[]
[wallets.research]
mode="zcash_rotation"
funding_amount_usdc="5"
max_funding_spend_zec="0.02"
max_conversion_overhead_percent=5
[sources.api]
spec="unused.json"
probe_pricing=false
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
    let wallet_resolution = config.validate().unwrap();
    let deployment = Deployment {
        unavailable_sources: Default::default(),
        config,
        wallet_resolution,
        initialized: None,
        sources: BTreeMap::new(),
        selected: BTreeMap::new(),
        config_path: dir.path().join("deployment.toml"),
    };
    let env = BTreeMap::from([
        ("BASE".into(), base),
        ("INDEXER".into(), "http://127.0.0.1:1".into()),
        ("SUBMIT".into(), "http://127.0.0.1:1".into()),
    ]);
    // An unfunded inventory must not make paused startup wait for bootstrap.
    let mut initialized = tokio::time::timeout(
        Duration::from_secs(10),
        deployment.bootstrap_for_serving(&env, Default::default()),
    )
    .await
    .unwrap()
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), observed)
        .await
        .unwrap()
        .unwrap();
    let supervisor = initialized.supervisor.take().unwrap();
    let store = supervisor.store();
    let stop = supervisor.stop.clone();
    let original_treasury = supervisor.treasury.as_ref().unwrap().id();
    let original_funding = supervisor.funding.as_ref().unwrap().id();
    let (treasury, funding) = supervisor.into_tasks();
    let treasury = treasury.unwrap();
    let funding = funding.unwrap();
    assert_eq!(treasury.id(), original_treasury);
    assert_eq!(funding.id(), original_funding);
    assert!(!stop.is_cancelled());
    assert!(
        !funding.is_finished(),
        "accepted observer must survive handoff"
    );
    stop.cancel();
    assert!(!funding.is_finished(), "accepted observation must drain");
    release.send(()).unwrap();
    rpc.await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), funding)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let after = store.call(|s| s.status()).await.unwrap();
    assert_eq!(after.funding_jobs.len(), before.funding_jobs.len());
    for (original, current) in before.funding_jobs.iter().zip(&after.funding_jobs) {
        assert_eq!(current.operation_id, original.operation_id);
        assert_eq!(current.phase, FundingPhase::Allocated);
    }
    assert!(after.treasury_operations.is_empty());
    assert!(
        store
            .call(|s| Ok(
                s.ensure_pool("forbidden", "5").unwrap_err().to_string() == "funding_disabled"
            ))
            .await
            .unwrap()
    );
    drop(store);
    drop(initialized);
    tokio::time::timeout(Duration::from_secs(10), treasury)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let reopened = crate::rotation::store::status(&state).unwrap();
    assert_eq!(reopened.funding_jobs.len(), before.funding_jobs.len());
}
