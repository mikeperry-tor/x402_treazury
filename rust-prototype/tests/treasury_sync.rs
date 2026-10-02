#![cfg(feature = "zcash")]
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, Uri},
    response::IntoResponse,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio_util::sync::CancellationToken;
use x402_mcp_prototype::{
    rotation::store::SyncPhase,
    treasury::{SyncSettings, Treasury},
};
const TIP: u64 = 2_000_000;
const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

// Minimal protobuf/gRPC fixture for an empty synthetic mainnet chain. No external
// endpoints, proving parameters, transactions or submission methods are involved.
fn varint(mut n: u64) -> Vec<u8> {
    let mut out = vec![];
    while n >= 128 {
        out.push(n as u8 | 128);
        n >>= 7;
    }
    out.push(n as u8);
    out
}
fn number(tag: u8, n: u64) -> Vec<u8> {
    [vec![tag << 3], varint(n)].concat()
}
fn bytes(tag: u8, b: &[u8]) -> Vec<u8> {
    [vec![(tag << 3) | 2], varint(b.len() as u64), b.to_vec()].concat()
}
fn take_varint(input: &mut &[u8]) -> u64 {
    let mut n = 0;
    let mut shift = 0;
    loop {
        let b = input[0];
        *input = &input[1..];
        n |= u64::from(b & 127) << shift;
        if b < 128 {
            return n;
        }
        shift += 7;
    }
}
fn fields(mut input: &[u8]) -> Vec<(u64, Vec<u8>)> {
    let mut out = vec![];
    while !input.is_empty() {
        let key = take_varint(&mut input);
        let value = if key & 7 == 2 {
            let len = take_varint(&mut input) as usize;
            let v = input[..len].to_vec();
            input = &input[len..];
            v
        } else {
            varint(take_varint(&mut input))
        };
        out.push((key >> 3, value));
    }
    out
}
fn height(input: &[u8]) -> u64 {
    fields(input)
        .into_iter()
        .find(|(tag, _)| *tag == 1)
        .map(|(_, v)| take_varint(&mut v.as_slice()))
        .unwrap_or(TIP)
}
fn hash(height: u64) -> Vec<u8> {
    [height.to_le_bytes().as_slice(), &[0; 24]].concat()
}
fn block(height: u64) -> Vec<u8> {
    [
        number(2, height),
        bytes(3, &hash(height)),
        bytes(4, &hash(height - 1)),
        bytes(8, &[]),
    ]
    .concat()
}
fn framed(message: Vec<u8>) -> Vec<u8> {
    [
        vec![0],
        (message.len() as u32).to_be_bytes().to_vec(),
        message,
    ]
    .concat()
}
#[derive(Clone)]
struct Mock {
    chain: &'static str,
    tip: u64,
    ironwood: &'static [u8],
    calls: Arc<Mutex<Vec<String>>>,
    stall_blocks: Arc<AtomicBool>,
}
async fn rpc(State(mock): State<Mock>, uri: Uri, body: Bytes) -> impl IntoResponse {
    let method = uri.path().rsplit('/').next().unwrap().to_owned();
    mock.calls.lock().unwrap().push(method.clone());
    let request = if body.len() >= 5 { &body[5..] } else { &[] };
    let messages = match method.as_str() {
        "GetLightdInfo" => vec![
            [
                bytes(4, mock.chain.as_bytes()),
                number(5, 419_200),
                number(7, mock.tip),
            ]
            .concat(),
        ],
        "GetLatestBlock" => vec![number(1, mock.tip)],
        "GetBlock" => vec![block(height(request))],
        "GetBlockRange" => {
            if mock.stall_blocks.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            let ids = fields(request);
            let start = height(&ids[0].1);
            let end = height(&ids[1].1);
            assert!(
                end >= start && end - start < 1000,
                "unexpected scan range {start}..{end}"
            );
            (start..=end).map(block).collect()
        }
        "GetTreeState" => {
            let h = height(request);
            let hash: String = hash(h).iter().rev().map(|b| format!("{b:02x}")).collect();
            vec![
                [
                    bytes(1, b"main"),
                    number(
                        2,
                        if mock.ironwood == b"wrong-height" {
                            h + 1
                        } else {
                            h
                        },
                    ),
                    bytes(3, hash.as_bytes()),
                    bytes(5, b"000000"),
                    bytes(6, b"000000"),
                    bytes(7, mock.ironwood),
                ]
                .concat(),
            ]
        }
        "GetAddressUtxos" | "GetTaddressBalance" => vec![vec![]],
        "GetSubtreeRoots" | "GetTaddressTransactions" => vec![],
        "GetTaddressTxids" => {
            // Extend discovery, without exceeding any per-RPC deadline, so the
            // stalled block fetch overlaps the 30-second checkpoint tick.
            if mock.stall_blocks.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            vec![]
        }
        "GetMempoolStream" => {
            std::future::pending::<()>().await;
            vec![]
        }
        _ => {
            let mut headers = HeaderMap::new();
            headers.insert("content-type", "application/grpc".parse().unwrap());
            headers.insert("grpc-status", "12".parse().unwrap());
            return (headers, vec![]);
        }
    };
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/grpc".parse().unwrap());
    headers.insert("grpc-status", "0".parse().unwrap());
    (
        headers,
        messages.into_iter().flat_map(framed).collect::<Vec<u8>>(),
    )
}
async fn mock(
    chain: &'static str,
) -> (
    String,
    Arc<Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
    Arc<AtomicBool>,
) {
    mock_with_tree(chain, TIP, b"000000").await
}
async fn mock_with_tree(
    chain: &'static str,
    tip: u64,
    ironwood: &'static [u8],
) -> (
    String,
    Arc<Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
    Arc<AtomicBool>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(vec![]));
    let stall_blocks = Arc::new(AtomicBool::new(false));
    let app = axum::Router::new().fallback(rpc).with_state(Mock {
        chain,
        tip,
        ironwood,
        calls: calls.clone(),
        stall_blocks: stall_blocks.clone(),
    });
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (endpoint, calls, task, stall_blocks)
}
async fn wallet(dir: &std::path::Path) -> Treasury {
    Treasury::create(
        dir.join("state"),
        dir.join("key"),
        TIP as u32,
        Some(zeroize::Zeroizing::new(SEED.into())),
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn wrong_network_and_cancellation_persist_failure_without_sending() {
    let dir = tempfile::tempdir().unwrap();
    let mut treasury = wallet(dir.path()).await;
    let (endpoint, calls, server, _) = mock("test").await;
    treasury.configure_sync(SyncSettings::new(endpoint, 7, 300).unwrap());
    let error = treasury
        .sync_once(&CancellationToken::new())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("mainnet"), "{error}");
    let status = treasury.status().await.unwrap();
    assert_eq!(status.sync.unwrap().phase, SyncPhase::Failed);
    assert_eq!(*calls.lock().unwrap(), vec!["GetLightdInfo"]);
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(treasury.sync_once(&cancel).await.is_err());
    assert_eq!(
        treasury.status().await.unwrap().sync.unwrap().phase,
        SyncPhase::Offline
    );
    treasury.close().await.unwrap();
    server.abort();
}
#[tokio::test]
async fn sync_empty_chain_persists_and_resumes_without_broadcast() {
    let dir = tempfile::tempdir().unwrap();
    let (endpoint, calls, server, _) = mock("main").await;
    let mut treasury = wallet(dir.path()).await;
    let settings = SyncSettings::new(endpoint.clone(), 7, 300).unwrap();
    treasury.configure_sync(settings.clone());
    let stop = CancellationToken::new();
    let timeout = stop.clone();
    let deadline = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        timeout.cancel();
    });
    let result = treasury.sync_once(&stop).await;
    deadline.abort();
    assert!(
        result.is_ok(),
        "{result:?}; calls: {:?}",
        calls.lock().unwrap()
    );
    let status = treasury.status().await.unwrap();
    let id = status.treasury_id;
    let observation = status.sync.unwrap();
    assert_eq!(observation.phase, SyncPhase::Ready);
    assert_eq!(observation.height, Some(TIP));
    assert_eq!(observation.confirmations, 7);
    assert_eq!(observation.spendable_shielded_zatoshis, 0);
    let pools = observation
        .confirmed_pool_balances_zatoshis
        .as_ref()
        .unwrap();
    assert_eq!(pools.ironwood, Some(0));
    assert_eq!(pools.orchard, Some(0));
    assert_eq!(pools.sapling, Some(0));
    let mut legacy = serde_json::to_value(&observation).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("confirmed_pool_balances_zatoshis");
    let legacy: x402_mcp_prototype::rotation::store::SyncObservation =
        serde_json::from_value(legacy).unwrap();
    assert!(legacy.confirmed_pool_balances_zatoshis.is_none());
    {
        use x402_mcp_prototype::rotation::transaction::{PrepareRequest, TransactionPreparer};
        let before = calls.lock().unwrap().len();
        let error = treasury
            .prepare(PrepareRequest {
                operation_id: uuid::Uuid::new_v4().to_string(),
                pool_id: None,
                daily_limit_zatoshis: 1_000_000,
                deadline: x402_mcp_prototype::rotation::base::now().unwrap() + 600,
                recipient: "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F".into(),
                amount_zatoshis: 50_000,
                max_fee_zatoshis: 20_000,
                max_input_zatoshis: 70_000,
            })
            .await
            .err()
            .expect("empty wallet prepared a deposit");
        assert!(
            error.to_string().contains("insufficient_spendable"),
            "{error}"
        );
        assert_eq!(calls.lock().unwrap().len(), before);
        assert_eq!(
            treasury.status().await.unwrap().snapshot_revision,
            status.snapshot_revision
        );
        assert!(
            treasury
                .status()
                .await
                .unwrap()
                .treasury_operations
                .is_empty()
        );
    }
    treasury.close().await.unwrap();
    let mut restored = Treasury::open(dir.path().join("state"), dir.path().join("key"), id.clone())
        .await
        .unwrap();
    assert_eq!(
        restored.status().await.unwrap().sync.unwrap().phase,
        SyncPhase::Offline
    );
    restored.configure_sync(settings);
    restored.sync_once(&CancellationToken::new()).await.unwrap();
    assert!(restored.status().await.unwrap().snapshot_revision > observation.snapshot_revision);
    assert!(!calls.lock().unwrap().iter().any(|m| m.contains("Send")));
    restored.close().await.unwrap();
    // The operator command uses treasury settings without fetching provider specs
    // or requiring the deliberately absent submission endpoint environment value.
    let config = dir.path().join("sync.toml");
    std::fs::write(
        &config,
        format!(
            r#"version=1
servers={{}}
[treasury]
id="{id}"
state_dir="state"
key_file="key"
indexer_url_env="INDEXER"
submission_url_env="MISSING_SUBMISSION"
daily_input_zec="0.1"
shield_max_fee_zec="0.001"
[sources.unloaded]
spec="nonexistent.json"
"#
        ),
    )
    .unwrap();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(env!("CARGO_BIN_EXE_x402-mcp-prototype"))
            .env_clear()
            .env("INDEXER", endpoint)
            .args(["wallet", "sync", "--meta-config"])
            .arg(config)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"]["sync"]["phase"], "ready");
    assert_eq!(output["state"]["sync_fresh"], true);
    assert!(!calls.lock().unwrap().iter().any(|m| m.contains("Send")));
    server.abort();
}

#[tokio::test]
async fn interrupted_sync_checkpoints_can_resume_and_stop_network_tasks() {
    use x402_mcp_prototype::rotation::store::status;
    let dir = tempfile::tempdir().unwrap();
    let (endpoint, calls, server, stalled) = mock("main").await;
    stalled.store(true, Ordering::SeqCst);
    let mut treasury = wallet(dir.path()).await;
    let id = treasury.status().await.unwrap().treasury_id;
    let settings = SyncSettings::new(endpoint, 3, 300).unwrap();
    treasury.configure_sync(settings.clone());
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let work = tokio::spawn(async move {
        let result = treasury.sync_once(&cancel).await;
        (treasury, result)
    });
    let observed = tokio::time::timeout(std::time::Duration::from_secs(40), async {
        loop {
            let observation = status(&dir.path().join("state")).unwrap().sync;
            if let Some(observation) = observation
                && observation.snapshot_revision >= 3
            {
                break observation;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await;
    stop.cancel();
    let (treasury, result) = tokio::time::timeout(std::time::Duration::from_secs(5), work)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    let observation = observed.expect("periodic encrypted checkpoint not observed");
    assert_eq!(observation.phase, SyncPhase::Syncing);
    assert_eq!(observation.target_height, Some(TIP));
    assert!(observation.checkpoint_at > 0);
    let count = calls.lock().unwrap().len();
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    assert_eq!(
        calls.lock().unwrap().len(),
        count,
        "sync tasks remained connected"
    );
    treasury.close().await.unwrap();
    stalled.store(false, Ordering::SeqCst);
    let mut restored = Treasury::open(dir.path().join("state"), dir.path().join("key"), id)
        .await
        .unwrap();
    restored.configure_sync(settings);
    restored.sync_once(&CancellationToken::new()).await.unwrap();
    assert_eq!(
        restored.status().await.unwrap().sync.unwrap().phase,
        SyncPhase::Ready
    );
    restored.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn treasury_command_owner_serializes_and_releases_exclusive_lock() {
    use x402_mcp_prototype::treasury::{actor, submission::GrpcSubmission};
    let dir = tempfile::tempdir().unwrap();
    let (endpoint, calls, server, _) = mock("main").await;
    let mut treasury = wallet(dir.path()).await;
    let id = treasury.status().await.unwrap().treasury_id;
    treasury.configure_sync(SyncSettings::new(endpoint.clone(), 3, 300).unwrap());
    let submission = GrpcSubmission::new(endpoint.clone(), endpoint).unwrap();
    let (handle, commands) = actor::channel();
    let stop = CancellationToken::new();
    let task = tokio::spawn(treasury.run_commands(commands, submission, stop.clone()));
    let (a, b) = tokio::join!(handle.sync(), handle.sync());
    a.unwrap();
    b.unwrap();
    stop.cancel();
    task.await.unwrap().unwrap();
    assert!(handle.sync().await.is_err());
    assert!(!calls.lock().unwrap().iter().any(|m| m.contains("Send")));
    Treasury::open(dir.path().join("state"), dir.path().join("key"), id)
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
    server.abort();
}

#[tokio::test]
async fn refund_address_and_derivation_range_survive_encrypted_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let treasury = wallet(dir.path()).await;
    treasury
        .ensure_pool("refunds".into(), "5".into())
        .await
        .unwrap();
    let id = treasury.status().await.unwrap().treasury_id;
    treasury.close().await.unwrap();
    // Materialize journal jobs without network or a funding worker.
    let mut store = x402_mcp_prototype::rotation::store::Store::open(
        &dir.path().join("state"),
        &dir.path().join("key"),
        &id,
    )
    .unwrap();
    let jobs = store.funding_jobs().unwrap();
    drop(store);
    let mut treasury = Treasury::open(dir.path().join("state"), dir.path().join("key"), id.clone())
        .await
        .unwrap();
    let first = treasury.refund_address(jobs[0].id.clone()).await.unwrap();
    assert_eq!(
        first,
        treasury.refund_address(jobs[0].id.clone()).await.unwrap()
    );
    treasury.close().await.unwrap();
    let mut treasury = Treasury::open(dir.path().join("state"), dir.path().join("key"), id)
        .await
        .unwrap();
    assert_eq!(
        first,
        treasury.refund_address(jobs[0].id.clone()).await.unwrap()
    );
    let second = treasury.refund_address(jobs[1].id.clone()).await.unwrap();
    assert_ne!(first, second);
    treasury.close().await.unwrap();
}

#[tokio::test]
async fn init_discovers_new_birthday_but_never_guesses_for_imports() {
    let (endpoint, calls, server, _) = mock("main").await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let run = |extra: Vec<&str>| {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402-mcp-prototype"));
        command
            .env_clear()
            .env("ZCASH_INDEXER_URL", &endpoint)
            .args(["wallet", "init", "--state-dir"])
            .arg(&state)
            .arg("--key-file")
            .arg(&key)
            .args(extra);
        command
    };
    let rejected = run(vec!["--mnemonic-file", "missing-seed.txt"])
        .output()
        .await
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("--birthday"));
    assert!(calls.lock().unwrap().is_empty());
    assert!(!state.exists() && !key.exists());
    let output = run(vec![]).output().await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["state"]["birthday"], TIP - 100);
    assert_eq!(calls.lock().unwrap().len(), 2);
    let rejected = run(vec![]).output().await.unwrap();
    assert!(!rejected.status.success());
    assert_eq!(calls.lock().unwrap().len(), 2);
    server.abort();
}

#[tokio::test]
async fn birthday_discovery_rejects_wrong_network_without_creating_state() {
    let (endpoint, _, server, _) = mock("test").await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402-mcp-prototype"))
        .env_clear()
        .env("TEST_INDEXER", endpoint)
        .args([
            "wallet",
            "init",
            "--indexer-url-env",
            "TEST_INDEXER",
            "--state-dir",
        ])
        .arg(&state)
        .arg("--key-file")
        .arg(&key)
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mainnet"));
    assert!(!state.exists() && !key.exists());
    server.abort();
}

#[tokio::test]
#[ignore = "read-only public mainnet endpoint qualification; requires network"]
async fn public_birthday_lookup() {
    let height = x402_mcp_prototype::treasury::birthday::discover(
        x402_mcp_prototype::treasury::birthday::DEFAULT_INDEXER,
    )
    .await
    .unwrap();
    println!("Discovered new-wallet birthday: {height}");
}

#[tokio::test]
async fn post_activation_indexer_must_supply_ironwood_before_sync_or_init() {
    const POST_ACTIVATION: u64 = 3_500_000;
    for tree in [b"".as_slice(), b"000000".as_slice()] {
        let (endpoint, calls, server, _) = mock_with_tree("main", POST_ACTIVATION, tree).await;
        let result = x402_mcp_prototype::treasury::birthday::discover(&endpoint).await;
        if tree.is_empty() {
            assert!(result.unwrap_err().to_string().contains("missing Ironwood"));
            let dir = tempfile::tempdir().unwrap();
            let mut treasury = wallet(dir.path()).await;
            treasury.configure_sync(SyncSettings::new(endpoint, 7, 300).unwrap());
            let error = treasury
                .sync_once(&CancellationToken::new())
                .await
                .unwrap_err();
            assert!(error.to_string().contains("missing Ironwood"), "{error}");
            let status = treasury.status().await.unwrap();
            assert_eq!(status.sync.unwrap().phase, SyncPhase::Failed);
            assert!(!status.sync_fresh);
            assert!(!calls.lock().unwrap().iter().any(|m| m == "GetBlockRange"));
            treasury.close().await.unwrap();
        } else {
            assert_eq!(result.unwrap(), POST_ACTIVATION as u32 - 100);
        }
        assert!(calls.lock().unwrap().iter().any(|m| m == "GetTreeState"));
        server.abort();
    }
}

#[tokio::test]
async fn ironwood_capability_rejects_tree_from_a_different_height() {
    let (endpoint, _, server, _) = mock_with_tree("main", 3_500_000, b"wrong-height").await;
    let error = x402_mcp_prototype::treasury::birthday::discover(&endpoint)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("wrong Ironwood tree height"));
    server.abort();
}
