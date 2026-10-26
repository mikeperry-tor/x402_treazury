#[path = "support/capacity.rs"]
mod capacity_fixture;
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
use x402_treazury::{
    rotation::store::SyncPhase,
    treasury::{SyncSettings, Treasury},
};
#[path = "support/socks.rs"]
mod socks;
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
    [varint(u64::from(tag) << 3), varint(n)].concat()
}
fn bytes(tag: u8, b: &[u8]) -> Vec<u8> {
    [
        varint((u64::from(tag) << 3) | 2),
        varint(b.len() as u64),
        b.to_vec(),
    ]
    .concat()
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
    final_tip: Option<u64>,
    ironwood: &'static [u8],
    calls: Arc<Mutex<Vec<String>>>,
    stall_blocks: Arc<AtomicBool>,
    delay_first: Arc<AtomicBool>,
    reuse_probe: Option<Arc<ReuseProbe>>,
    checkpoint_lock: Option<Arc<CheckpointLock>>,
}
// A real Pepper pool-recovery request made while holding the wallet write lock.
#[derive(Default)]
struct CheckpointLock {
    mismatched_range_served: AtomicBool,
    faithful: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[derive(Default)]
struct ReuseProbe {
    tip: std::sync::atomic::AtomicU64,
    hash: Mutex<Vec<u8>>,
    fail: AtomicBool,
    stall: AtomicBool,
    delay_once: AtomicBool,
}
async fn rpc(State(mock): State<Mock>, uri: Uri, body: Bytes) -> impl IntoResponse {
    let method = uri.path().rsplit('/').next().unwrap().to_owned();
    mock.calls.lock().unwrap().push(method.clone());
    if let Some(probe) = &mock.reuse_probe
        && method == "GetLightdInfo"
    {
        if probe.delay_once.swap(false, Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_secs(6)).await;
        }
        if probe.stall.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        if probe.fail.load(Ordering::SeqCst) {
            let mut headers = HeaderMap::new();
            headers.insert("content-type", "application/grpc".parse().unwrap());
            headers.insert("grpc-status", "14".parse().unwrap());
            return (headers, vec![]);
        }
    }
    let request = if body.len() >= 5 { &body[5..] } else { &[] };
    let messages = match method.as_str() {
        "GetLightdInfo" => vec![
            [
                bytes(4, mock.chain.as_bytes()),
                // The v7 sync engine requires the transparent/Ironwood-capable
                // lightwallet protocol; keep this fixture faithful to that contract.
                bytes(18, b"0.5.0"),
                number(5, 419_200),
                number(
                    7,
                    if let Some(probe) = &mock.reuse_probe {
                        probe.tip.load(Ordering::SeqCst)
                    } else if mock
                        .calls
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|m| *m == "GetLightdInfo")
                        .count()
                        > 1
                    {
                        mock.final_tip.unwrap_or(mock.tip)
                    } else {
                        mock.tip
                    },
                ),
            ]
            .concat(),
        ],
        "GetLatestBlock" => {
            let (tip, hash) = mock.reuse_probe.as_ref().map_or_else(
                || (mock.tip, hash(mock.tip)),
                |p| (p.tip.load(Ordering::SeqCst), p.hash.lock().unwrap().clone()),
            );
            vec![[number(1, tip), bytes(2, &hash)].concat()]
        }
        "GetBlock" => {
            let h = height(request);
            if let Some(probe) = &mock.checkpoint_lock
                && !probe.faithful.load(Ordering::SeqCst)
                && probe.mismatched_range_served.load(Ordering::SeqCst)
                && h == TIP - 3
            {
                probe.entered.notify_one();
                probe.release.notified().await;
            }
            vec![block(h)]
        }
        "GetBlockRange" => {
            if mock.delay_first.swap(false, Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_secs(6)).await;
            }
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
            let mismatch = mock
                .checkpoint_lock
                .as_ref()
                .is_some_and(|probe| !probe.faithful.load(Ordering::SeqCst));
            if mismatch {
                mock.checkpoint_lock
                    .as_ref()
                    .unwrap()
                    .mismatched_range_served
                    .store(true, Ordering::SeqCst);
            }
            (start..=end)
                .map(|h| {
                    if mismatch && h == end {
                        // Keep the first block truthful: Pepper seeds its initial
                        // tree sizes from that block's metadata. Only the last
                        // empty block reports a spurious commitment, entering
                        // actual IncorrectTreeSize recovery under the wallet lock.
                        [block(h), bytes(8, &number(2, 1))].concat()
                    } else {
                        block(h)
                    }
                })
                .collect()
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
    mock_with_tip_change(chain, tip, ironwood, None).await
}
async fn mock_with_tip_change(
    chain: &'static str,
    tip: u64,
    ironwood: &'static [u8],
    final_tip: Option<u64>,
) -> (
    String,
    Arc<Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
    Arc<AtomicBool>,
) {
    mock_with_delay(chain, tip, ironwood, final_tip, false).await
}
async fn mock_with_delay(
    chain: &'static str,
    tip: u64,
    ironwood: &'static [u8],
    final_tip: Option<u64>,
    delay: bool,
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
        final_tip,
        ironwood,
        calls: calls.clone(),
        stall_blocks: stall_blocks.clone(),
        delay_first: Arc::new(AtomicBool::new(delay)),
        reuse_probe: None,
        checkpoint_lock: None,
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
    treasury.configure_sync(SyncSettings::new(endpoint, 7).unwrap());
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
    let settings = SyncSettings::new(endpoint.clone(), 7).unwrap();
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
    legacy
        .as_object_mut()
        .unwrap()
        .remove("observed_tip_height");
    let legacy: x402_treazury::rotation::store::SyncObservation =
        serde_json::from_value(legacy).unwrap();
    assert!(legacy.confirmed_pool_balances_zatoshis.is_none());
    assert!(legacy.observed_tip_height.is_none());
    {
        use x402_treazury::rotation::transaction::{PrepareRequest, TransactionPreparer};
        let before = calls.lock().unwrap().len();
        let error = treasury
            .prepare(PrepareRequest {
                managed_funding: None,

                operation_id: uuid::Uuid::new_v4().to_string(),
                pool_id: None,

                deadline: x402_treazury::rotation::base::now().unwrap() + 600,
                recipient: "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F".into(),
                amount_zatoshis: 50_000,
                max_fee_zatoshis: 20_000,
                max_input_zatoshis: 70_000,
            })
            .await
            .err()
            .expect("empty wallet prepared a deposit");
        assert!(
            error.is::<x402_treazury::rotation::transaction::PreparationDeferred>(),
            "{error:#}"
        );
        assert!(
            error
                .chain()
                .any(|cause| cause.to_string() == "treasury proposal failed"),
            "{error:#}"
        );
        {
            let observed = calls.lock().unwrap();
            assert_eq!(
                observed.len(),
                before,
                "an empty SDK proposal must not probe the indexer"
            );
        }
        let after = treasury.status().await.unwrap();
        assert_eq!(after.snapshot_revision, status.snapshot_revision);
        assert!(after.treasury_operations.is_empty() && !after.outgoing_pending);
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
    let proxy = socks::Socks::start(
        std::collections::BTreeMap::from([("loopback".into(), "127.0.0.1:1".parse().unwrap())]),
        socks::Fault::None,
    )
    .await;
    let proxy_address = proxy.address;
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
max_refund_shielding_fee_zec="0.001"
[network]
mode="tor"
socks_endpoint="{proxy_address}"
[sources.unloaded]
spec="nonexistent.json"
"#
        ),
    )
    .unwrap();
    let output = tokio::task::spawn_blocking(move || {
        let run = |json: bool| {
            let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"));
            command
                .env_clear()
                .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
                .env("INDEXER", &endpoint)
                .args(["wallet", "sync", "--config"])
                .arg(&config);
            if json {
                command.arg("--json");
            }
            command.output().unwrap()
        };
        (run(false), run(true))
    })
    .await
    .unwrap();
    let (human, output) = output;
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    let human = String::from_utf8(human.stdout).unwrap();
    assert!(human.contains(
        "Wallet synchronization: ZEC completed; managed USDC inapplicable (0 attempted, 0 completed, 0 failed)."
    ));
    assert!(human.contains("Scanned height: 2000000"));
    assert!(human.contains("Saved spendable shielded balance: 0.00000000 ZEC"));
    assert!(human.contains("Primary ZEC address:"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let logs = String::from_utf8_lossy(&output.stderr);
    for stage in [
        "loading configuration and opening encrypted wallet",
        "wallet opened; starting synchronization",
        "Synchronizing Zcash treasury wallet",
        "Zcash treasury wallet synchronized; progress saved",
    ] {
        assert!(logs.contains(stage), "missing {stage}: {logs}");
    }
    assert!(!logs.contains(SEED));
    let output: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"]["sync"]["phase"], "ready");
    assert_eq!(output["state"]["sync_complete"], true);
    assert_eq!(
        output["observations"]["zec"],
        serde_json::json!({"status":"completed", "attempted":1, "completed":1, "failed":0})
    );
    assert_eq!(
        output["observations"]["usdc"],
        serde_json::json!({"status":"inapplicable", "attempted":0, "completed":0, "failed":0})
    );
    let context =
        x402_treazury::network::NetworkContext::new(x402_treazury::network::NetworkPolicy {
            mode: x402_treazury::network::Mode::Tor,
            socks_endpoint: Some(proxy_address),
            ..Default::default()
        })
        .unwrap();
    let credentials = context.credentials(&x402_treazury::network::IsolationId::treasury(&id));
    assert!(!proxy.records.lock().unwrap().is_empty());
    assert!(
        proxy
            .records
            .lock()
            .unwrap()
            .iter()
            .all(|r| (r.user.clone(), r.password.clone()) == credentials)
    );
    assert!(!calls.lock().unwrap().iter().any(|m| m.contains("Send")));
    server.abort();
}

#[tokio::test]
async fn interrupted_sync_checkpoints_can_resume_and_stop_network_tasks() {
    use x402_treazury::rotation::store::status;
    let dir = tempfile::tempdir().unwrap();
    let (endpoint, calls, server, stalled) = mock("main").await;
    stalled.store(true, Ordering::SeqCst);
    let mut treasury = wallet(dir.path()).await;
    let id = treasury.status().await.unwrap().treasury_id;
    let settings = SyncSettings::new(endpoint, 3).unwrap();
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
async fn checkpoint_waiting_for_pepper_wallet_lock_does_not_stop_engine_progress() {
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let probe = Arc::new(CheckpointLock::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new().fallback(rpc).with_state(Mock {
        chain: "main",
        tip: TIP,
        final_tip: None,
        ironwood: b"000000",
        calls: calls.clone(),
        stall_blocks: Arc::new(AtomicBool::new(false)),
        delay_first: Arc::new(AtomicBool::new(false)),
        reuse_probe: None,
        checkpoint_lock: Some(probe.clone()),
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut treasury = Treasury::create(
        dir.path().join("state"),
        dir.path().join("key"),
        (TIP - 2) as u32,
        Some(zeroize::Zeroizing::new(SEED.into())),
    )
    .await
    .unwrap();
    let initial = treasury.status().await.unwrap();
    let settings = SyncSettings::new(endpoint.clone(), 3).unwrap();
    treasury.configure_sync(settings.clone());
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let mut work = tokio::spawn(async move {
        let result = treasury.sync_once(&cancel).await;
        (treasury, result)
    });
    if tokio::time::timeout(Duration::from_secs(10), probe.entered.notified())
        .await
        .is_err()
    {
        stop.cancel();
        probe.release.notify_one();
        let drained = tokio::time::timeout(Duration::from_secs(5), &mut work).await;
        server.abort();
        assert!(
            drained.is_ok(),
            "sync owner did not drain after fixture failure"
        );
        let outcome = drained.unwrap().map(|(_, result)| result);
        panic!(
            "Pepper did not reach pool-recovery GetBlock under the wallet lock; outcome={outcome:#?}; RPCs={:?}",
            calls.lock().unwrap()
        );
    }
    // The production checkpoint cadence is 30 seconds. Hold the response past
    // that tick, within ordinary 60-second inactivity, then let real recovery
    // release the wallet lock. An inline engine is no longer polled once the
    // checkpoint awaits this lock, so it cannot make progress after release.
    tokio::time::sleep(Duration::from_secs(31)).await;
    probe.release.notify_one();
    let finished = tokio::time::timeout(Duration::from_secs(10), &mut work).await;
    let joined = match finished {
        Ok(joined) => joined,
        Err(_) => {
            stop.cancel();
            probe.release.notify_one();
            let drained = tokio::time::timeout(Duration::from_secs(5), &mut work).await;
            server.abort();
            assert!(
                drained.is_ok(),
                "sync owner did not drain after checkpoint stall"
            );
            panic!("checkpoint blocked the Pepper engine holding its wallet lock");
        }
    };
    let (treasury, result) = joined.unwrap();
    let error = result.unwrap_err();
    let status = treasury.status().await.unwrap();
    let observation = status.sync.unwrap();
    assert_eq!(observation.phase, SyncPhase::Failed);
    // Initial pre-scan checkpoint, periodic checkpoint, then final checkpoint.
    assert_eq!(status.snapshot_revision, initial.snapshot_revision + 3);
    assert_eq!(observation.snapshot_revision, status.snapshot_revision);
    assert!(observation.checkpoint_at > 0);
    assert!(format!("{error:#}").contains("pool_history_reopened"));
    assert!(
        observation
            .last_error
            .as_deref()
            .is_some_and(|e| !e.contains(&endpoint))
    );
    assert!(!calls.lock().unwrap().iter().any(|m| m.contains("Send")));
    let count = calls.lock().unwrap().len();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        calls.lock().unwrap().len(),
        count,
        "sync I/O survived runtime teardown"
    );
    treasury.close().await.unwrap();
    probe.faithful.store(true, Ordering::SeqCst);
    let mut restored = Treasury::open(
        dir.path().join("state"),
        dir.path().join("key"),
        initial.treasury_id,
    )
    .await
    .unwrap();
    restored.configure_sync(settings);
    restored.sync_once(&CancellationToken::new()).await.unwrap();
    assert_eq!(
        restored.status().await.unwrap().sync.unwrap().phase,
        SyncPhase::Ready
    );
    assert!(!calls.lock().unwrap().iter().any(|m| m.contains("Send")));
    restored.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn treasury_command_owner_serializes_and_releases_exclusive_lock() {
    use x402_treazury::treasury::{actor, submission::GrpcSubmission};
    let dir = tempfile::tempdir().unwrap();
    let (endpoint, calls, server, _) = mock("main").await;
    let mut treasury = wallet(dir.path()).await;
    let id = treasury.status().await.unwrap().treasury_id;
    treasury.configure_sync(SyncSettings::new(endpoint.clone(), 3).unwrap());
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
    crate::capacity_fixture::treasury_pair(
        &treasury,
        "refunds",
        "5",
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0,
    )
    .await
    .unwrap();
    let id = treasury.status().await.unwrap().treasury_id;
    treasury.close().await.unwrap();
    // Read the atomically allocated journals without network or a funding worker.
    let store = x402_treazury::rotation::store::Store::open(
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
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"));
        command
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
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
    let value =
        serde_json::to_value(x402_treazury::rotation::store::status(&state).unwrap()).unwrap();
    assert_eq!(value["birthday"], TIP - 100);
    assert_eq!(calls.lock().unwrap().len(), 2);
    let rejected = run(vec![]).output().await.unwrap();
    assert!(!rejected.status.success());
    assert_eq!(calls.lock().unwrap().len(), 2);
    server.abort();
}

#[tokio::test]
async fn config_driven_initialization_uses_tor_and_no_endpoint_environment() {
    for external in [false, true] {
        let (endpoint, calls, server, _) = mock("main").await;
        let dir = tempfile::tempdir().unwrap();
        let proxy = socks::Socks::start(
            std::collections::BTreeMap::from([("loopback".into(), "127.0.0.1:1".parse().unwrap())]),
            socks::Fault::None,
        )
        .await;
        let config = dir.path().join("deployment.toml");
        std::fs::write(
            &config,
            format!(
                r#"version=1
servers={{}}
[treasury]
state_dir="wallet"
indexer_url="{endpoint}"
max_refund_shielding_fee_zec="0.001"
[network]
mode="tor"
socks_endpoint="{}"
[sources.unloaded]
spec="nonexistent.json"
"#,
                if external {
                    "127.0.0.1:1".parse().unwrap()
                } else {
                    proxy.address
                }
            ),
        )
        .unwrap();
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"));
        command
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
            .args(["wallet", "init", "--config"])
            .arg(&config);
        if external {
            let policy = dir.path().join("network.toml");
            std::fs::write(&policy, format!("[network]\nmode='tor'\nsocks_endpoint='{}'\nisolation_namespace='override-init'\n", proxy.address)).unwrap();
            command.arg("--network-config").arg(policy);
        }
        let output = command.output().await.unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value = serde_json::to_value(
            x402_treazury::rotation::store::status(&dir.path().join("wallet")).unwrap(),
        )
        .unwrap();
        assert_eq!(value["birthday"], TIP - 100);
        assert!(dir.path().join("wallet/wallet.key").is_file());
        assert_eq!(calls.lock().unwrap().len(), 2);
        let records = proxy.records.lock().unwrap();
        assert!(!records.is_empty());
        assert!(
            records.iter().all(|r| r.user == "<torS0X>0"
                && !r.password.is_empty()
                && r.upstream_peer.is_some())
        );
        if external {
            assert!(
                records
                    .iter()
                    .all(|record| record.password.starts_with("override-init:v1:"))
            );
        }
        server.abort();
    }
}

#[tokio::test]
async fn birthday_discovery_rejects_wrong_network_without_creating_state() {
    let (endpoint, _, server, _) = mock("test").await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
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
    let height = x402_treazury::treasury::birthday::discover(
        x402_treazury::treasury::birthday::DEFAULT_INDEXER,
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
        let result = x402_treazury::treasury::birthday::discover(&endpoint).await;
        if tree.is_empty() {
            assert!(result.unwrap_err().to_string().contains("missing Ironwood"));
            let dir = tempfile::tempdir().unwrap();
            let mut treasury = wallet(dir.path()).await;
            treasury.configure_sync(SyncSettings::new(endpoint, 7).unwrap());
            let error = treasury
                .sync_once(&CancellationToken::new())
                .await
                .unwrap_err();
            assert!(error.to_string().contains("missing Ironwood"), "{error}");
            let status = treasury.status().await.unwrap();
            assert_eq!(status.sync.unwrap().phase, SyncPhase::Failed);
            assert!(!status.sync_complete);
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
    let error = x402_treazury::treasury::birthday::discover(&endpoint)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("wrong Ironwood tree height"));
    server.abort();
}

#[tokio::test]
async fn actor_dispatches_prepare_submit_and_reconcile_without_funding_empty_wallet() {
    use x402_treazury::{
        rotation::transaction::PrepareRequest,
        treasury::{actor, submission::GrpcSubmission},
    };
    let dir = tempfile::tempdir().unwrap();
    let (endpoint, calls, server, _) = mock("main").await;
    let mut treasury = wallet(dir.path()).await;
    let id = treasury.status().await.unwrap().treasury_id;
    treasury.configure_sync(SyncSettings::new(endpoint.clone(), 3).unwrap());
    let (handle, commands) = actor::channel();
    let stop = CancellationToken::new();
    let owner = tokio::spawn(treasury.run_commands(
        commands,
        GrpcSubmission::new(endpoint.clone(), endpoint).unwrap(),
        stop.clone(),
    ));
    let error = handle
        .prepare(PrepareRequest {
            managed_funding: None,

            operation_id: uuid::Uuid::new_v4().to_string(),
            pool_id: None,

            deadline: x402_treazury::rotation::base::now().unwrap() + 1000,
            recipient: "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F".into(),
            amount_zatoshis: 50_000,
            max_fee_zatoshis: 20_000,
            max_input_zatoshis: 70_000,
        })
        .await
        .err()
        .unwrap();
    // This indexer mock cannot supply real proposal input/witness coverage.
    // Typed SDK insufficient-funds selection is covered by the proving fixture.
    assert!(
        error.is::<x402_treazury::rotation::transaction::PreparationDeferred>(),
        "{error:#}"
    );
    assert!(
        handle
            .submit(uuid::Uuid::new_v4().to_string(), false)
            .await
            .is_err()
    );
    assert!(
        handle
            .reconcile(uuid::Uuid::new_v4().to_string())
            .await
            .is_err()
    );
    handle.sync().await.unwrap(); // Failed commands did not kill the healthy owner.
    stop.cancel();
    owner.await.unwrap().unwrap();
    assert!(!calls.lock().unwrap().iter().any(|m| m.contains("Send")));
    let reopened = Treasury::open(dir.path().join("state"), dir.path().join("key"), id)
        .await
        .unwrap();
    assert!(
        reopened
            .status()
            .await
            .unwrap()
            .treasury_operations
            .is_empty()
    );
    reopened.close().await.unwrap();
    server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn supervised_sync_eof_checkpoints_and_releases_wallet() {
    use std::{process::Stdio, time::Duration};
    let dir = tempfile::tempdir().unwrap();
    let (endpoint, calls, server, stalled) = mock("main").await;
    stalled.store(true, Ordering::SeqCst);
    let treasury = wallet(dir.path()).await;
    let id = treasury.status().await.unwrap().treasury_id;
    treasury.close().await.unwrap();
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
max_refund_shielding_fee_zec="0.001"
[sources.unloaded]
spec="nonexistent.json"
"#
        ),
    )
    .unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .env("INDEXER", &endpoint)
        .args(["wallet", "sync", "--qualification-parent-stdin", "--config"])
        .arg(&config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    // Cold macOS executable validation can dominate startup; this is separate
    // from the strict ten-second cancellation deadline below.
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                use tokio::io::AsyncReadExt;
                let mut error = String::new();
                child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut error)
                    .await
                    .unwrap();
                panic!("sync exited before indexer: {status}: {error}");
            }
            if calls.lock().unwrap().iter().any(|m| m == "GetBlockRange") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("sync did not reach stalled indexer");
    drop(child.stdin.take());
    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .expect("supervised sync failed to drain")
        .unwrap();
    assert!(!output.status.success());
    assert!(
        output.status.code().is_some(),
        "sync must exit without a kill signal"
    );
    assert!(
        output.stdout.is_empty(),
        "cancelled sync must not publish readiness"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("deliberately cancelling and checkpointing"),
        "{stderr}"
    );
    assert!(
        stderr.contains("treasury sync cancelled by supervisor"),
        "{stderr}"
    );
    assert!(!calls.lock().unwrap().iter().any(|m| m.contains("Send")));
    let mut reopened = Treasury::open(dir.path().join("state"), dir.path().join("key"), id)
        .await
        .unwrap();
    assert_eq!(
        reopened.status().await.unwrap().sync.unwrap().phase,
        SyncPhase::Offline
    );
    stalled.store(false, Ordering::SeqCst);
    reopened.configure_sync(SyncSettings::new(endpoint, 3).unwrap());
    reopened.sync_once(&CancellationToken::new()).await.unwrap();
    assert_eq!(
        reopened.status().await.unwrap().sync.unwrap().phase,
        SyncPhase::Ready
    );
    reopened.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn moving_tip_accepts_progress_beyond_old_lag_and_rejects_regression() {
    for (delta, accepted) in [(1i64, true), (3, true), (4, true), (-1, false)] {
        let dir = tempfile::tempdir().unwrap();
        let tip = TIP.checked_add_signed(delta).unwrap();
        let (endpoint, calls, server, _) =
            mock_with_tip_change("main", TIP, b"000000", Some(tip)).await;
        let mut treasury = wallet(dir.path()).await;
        treasury.configure_sync(SyncSettings::new(endpoint, 7).unwrap());
        let before = x402_treazury::rotation::base::now().unwrap();
        let result = treasury.sync_once(&CancellationToken::new()).await;
        assert_eq!(result.is_ok(), accepted, "delta={delta}: {result:?}");
        let status = treasury.status().await.unwrap();
        assert_eq!(status.sync_complete, accepted);
        let observation = status.sync.unwrap();
        assert_eq!(observation.observed_tip_height, Some(tip));
        assert_eq!(observation.target_height, Some(TIP));
        if accepted {
            assert_eq!(observation.height, Some(TIP));
            assert_eq!(observation.confirmations, 7);
            assert!(observation.checked_at.unwrap() >= before);
            assert_eq!(observation.snapshot_revision, status.snapshot_revision);
        } else {
            assert_eq!(observation.phase, SyncPhase::Failed);
            assert!(observation.last_error.is_some());
        }
        assert!(!calls.lock().unwrap().iter().any(|m| m == "SendTransaction"));
        treasury.close().await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn birthday_uses_older_observation_when_tip_moves_between_reads() {
    for (info_height, tip_height) in [(TIP, TIP + 4), (TIP + 4, TIP)] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let app = axum::Router::new().fallback(move |uri: Uri| async move {
            let message = if uri.path().ends_with("GetLightdInfo") {
                [bytes(4, b"main"), number(7, info_height)].concat()
            } else {
                assert!(uri.path().ends_with("GetLatestBlock"));
                number(1, tip_height)
            };
            (
                [("content-type", "application/grpc"), ("grpc-status", "0")],
                framed(message),
            )
        });
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        assert_eq!(
            x402_treazury::treasury::birthday::discover(&endpoint)
                .await
                .unwrap(),
            TIP as u32 - x402_treazury::treasury::birthday::BIRTHDAY_REWIND
        );
        server.abort();
    }
}

#[tokio::test]
async fn slow_initial_scan_finishes_without_an_age_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (endpoint, calls, server, _) = mock_with_delay("main", TIP, b"000000", None, true).await;
    let mut treasury = wallet(dir.path()).await;
    treasury.configure_sync(SyncSettings::new(endpoint, 7).unwrap());
    let before = x402_treazury::rotation::base::now().unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        treasury.sync_once(&CancellationToken::new()),
    )
    .await
    .unwrap()
    .unwrap();
    let status = treasury.status().await.unwrap();
    assert!(status.sync_complete);
    let observation = status.sync.unwrap();
    assert_eq!(observation.height, Some(TIP));
    assert!(
        observation.checked_at.unwrap() <= before + 1,
        "slow work retains its original observation"
    );
    let requests = calls.lock().unwrap().clone();
    // Application starting/final observations plus the SDK protocol check.
    assert_eq!(requests.iter().filter(|m| *m == "GetLightdInfo").count(), 3);
    assert!(!requests.iter().any(|m| m == "SendTransaction"));
    treasury.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn sync_cli_reports_failed_stage_without_claiming_completion() {
    let (endpoint, calls, server, _) = mock("test").await;
    let dir = tempfile::tempdir().unwrap();
    wallet(dir.path()).await.close().await.unwrap();
    let config = dir.path().join("sync.toml");
    std::fs::write(&config, format!("version=1\nservers={{}}\n[treasury]\nstate_dir='state'\nkey_file='key'\nindexer_url='{endpoint}'\n")).unwrap();
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args(["wallet", "sync", "--json", "--config"])
        .arg(config)
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let reported: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(reported["state"].is_object());
    assert!(reported["receive_addresses"].is_array());
    assert_eq!(reported["observations"]["zec"]["status"], "failed");
    assert_eq!(
        reported["observations"]["zec"]["error"],
        "indexer_network_mismatch"
    );
    assert_eq!(reported["observations"]["usdc"]["status"], "inapplicable");
    let logs = String::from_utf8_lossy(&output.stderr);
    assert!(
        logs.contains("Synchronizing Zcash treasury wallet"),
        "{logs}"
    );
    assert!(logs.contains("indexer_network_mismatch"), "{logs}");
    assert!(!logs.contains("saving final wallet checkpoint"), "{logs}");
    assert!(
        !logs.contains("Zcash treasury wallet synchronized; progress saved"),
        "{logs}"
    );
    assert!(!logs.contains("scanning wallet history"), "{logs}");
    assert!(!logs.contains(SEED));
    assert!(!logs.contains(&endpoint));
    let saved = x402_treazury::rotation::store::status(&dir.path().join("state"))
        .unwrap()
        .sync
        .unwrap();
    assert_eq!(saved.phase, SyncPhase::Offline);
    assert_eq!(
        saved.last_error.as_deref(),
        Some("indexer is not on mainnet")
    );
    assert!(!calls.lock().unwrap().iter().any(|m| m.contains("Send")));
    server.abort();
}

#[tokio::test]
async fn command_sync_reuse_preserves_observation_and_rescans_after_mutation_restart_or_chain_change()
 {
    use x402_treazury::treasury::submission::GrpcSubmission;
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let probe = Arc::new(ReuseProbe::default());
    probe.tip.store(TIP, Ordering::SeqCst);
    *probe.hash.lock().unwrap() = hash(TIP);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new().fallback(rpc).with_state(Mock {
        chain: "main",
        tip: TIP,
        final_tip: None,
        ironwood: b"000000",
        calls: calls.clone(),
        stall_blocks: Arc::new(AtomicBool::new(false)),
        delay_first: Arc::new(AtomicBool::new(false)),
        reuse_probe: Some(probe.clone()),
        checkpoint_lock: None,
    });
    let peers = Arc::new(Mutex::new(std::collections::HashSet::new()));
    let observed_peers = peers.clone();
    let app = app.layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let peer = request
                .extensions()
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .unwrap()
                .0;
            observed_peers.lock().unwrap().insert(peer);
            next.run(request)
        },
    ));
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let mut treasury = wallet(dir.path()).await;
    let id = treasury.status().await.unwrap().treasury_id;
    let settings = SyncSettings::new(endpoint.clone(), 3).unwrap();
    treasury.configure_sync(settings.clone());
    let stop = CancellationToken::new();
    treasury.sync_once(&stop).await.unwrap();
    let mut sender = GrpcSubmission::new(endpoint.clone(), endpoint.clone()).unwrap();
    // A nonexistent operation stops after synchronization, before lookup/submission.
    let before = treasury.status().await.unwrap();
    let count = calls.lock().unwrap().len();
    peers.lock().unwrap().clear();
    for _ in 0..3 {
        assert!(
            treasury
                .reconcile_prepared("absent".into(), &mut sender, &stop)
                .await
                .is_err()
        );
    }
    assert_eq!(
        peers.lock().unwrap().len(),
        1,
        "repeated reuse probes must share the owner runtime's pooled connection"
    );
    let after = treasury.status().await.unwrap();
    assert_eq!(before.snapshot_revision, after.snapshot_revision);
    assert_eq!(
        before.sync.unwrap().checked_at,
        after.sync.unwrap().checked_at
    );
    assert_eq!(
        &calls.lock().unwrap()[count..],
        &["GetLightdInfo", "GetLatestBlock"].repeat(3)
    );
    for change in ["mutation", "restart", "tip", "hash", "missing_hash"] {
        match change {
            "mutation" => {
                treasury.derive_address().await.unwrap();
            }
            "restart" => {
                treasury.close().await.unwrap();
                treasury =
                    Treasury::open(dir.path().join("state"), dir.path().join("key"), id.clone())
                        .await
                        .unwrap();
                treasury.configure_sync(settings.clone());
            }
            "tip" => {
                probe.tip.store(TIP + 1, Ordering::SeqCst);
                *probe.hash.lock().unwrap() = hash(TIP + 1);
            }
            "hash" => {
                *probe.hash.lock().unwrap() = vec![7; 32];
            }
            "missing_hash" => {
                probe.hash.lock().unwrap().clear();
            }
            _ => unreachable!(),
        }
        let before = treasury.status().await.unwrap().snapshot_revision;
        assert!(
            treasury
                .submit_prepared("absent".into(), &mut sender, false, &stop)
                .await
                .is_err()
        );
        assert!(
            treasury.status().await.unwrap().snapshot_revision > before,
            "{change} must scan"
        );
        assert_eq!(
            treasury.status().await.unwrap().sync.unwrap().phase,
            SyncPhase::Ready,
            "{change}"
        );
    }
    // A slow, progressing tip read cannot renew the starting observation.
    *probe.hash.lock().unwrap() = hash(TIP + 1);
    treasury.configure_sync(SyncSettings::new(endpoint.clone(), 3).unwrap());
    treasury.sync_once(&stop).await.unwrap();
    let before = treasury.status().await.unwrap();
    probe.delay_once.store(true, Ordering::SeqCst);
    assert!(
        treasury
            .reconcile_prepared("absent".into(), &mut sender, &stop)
            .await
            .is_err()
    );
    let after = treasury.status().await.unwrap();
    assert_eq!(after.snapshot_revision, before.snapshot_revision);
    assert_eq!(
        after.sync.unwrap().checked_at,
        before.sync.unwrap().checked_at
    );
    treasury.configure_sync(settings);
    treasury.sync_once(&stop).await.unwrap();
    // A failed unsigned preflight must not publish a new observation or start a send.
    let before = treasury.status().await.unwrap().snapshot_revision;
    probe.fail.store(true, Ordering::SeqCst);
    let error = treasury
        .reconcile_prepared("absent".into(), &mut sender, &stop)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("grpc_code=14"));
    assert_eq!(treasury.status().await.unwrap().snapshot_revision, before);
    probe.fail.store(false, Ordering::SeqCst);
    probe.stall.store(true, Ordering::SeqCst);
    let count = calls.lock().unwrap().len();
    let cancel = stop.clone();
    let observed = calls.clone();
    tokio::spawn(async move {
        while observed.lock().unwrap().len() == count {
            tokio::task::yield_now().await;
        }
        cancel.cancel();
    });
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        treasury.reconcile_prepared("absent".into(), &mut sender, &stop),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    assert_eq!(treasury.status().await.unwrap().snapshot_revision, before);
    // Cancelling an unsigned probe must leave the owner/channel usable. A new
    // caller still verifies the chain without renewing the saved observation.
    probe.stall.store(false, Ordering::SeqCst);
    let count = calls.lock().unwrap().len();
    assert!(
        treasury
            .reconcile_prepared("absent".into(), &mut sender, &CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(treasury.status().await.unwrap().snapshot_revision, before);
    assert_eq!(
        &calls.lock().unwrap()[count..],
        &["GetLightdInfo", "GetLatestBlock"]
    );
    assert!(
        treasury
            .status()
            .await
            .unwrap()
            .treasury_operations
            .is_empty()
    );
    assert!(!calls.lock().unwrap().iter().any(|m| m.contains("Send")));
    treasury.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn command_scan_defers_the_immediate_background_tick_without_extra_network_work() {
    use x402_treazury::treasury::{actor, submission::GrpcSubmission};
    let dir = tempfile::tempdir().unwrap();
    let (endpoint, calls, server, _) = mock("main").await;
    let mut treasury = wallet(dir.path()).await;
    treasury.configure_sync(SyncSettings::new(endpoint.clone(), 3).unwrap());
    let submission = GrpcSubmission::new(endpoint.clone(), endpoint).unwrap();
    let (handle, commands) = actor::channel();
    let stop = CancellationToken::new();
    // Queue the command before starting the owner: it must take priority over
    // the initial immediate maintenance tick, and replace that tick on success.
    let requested = handle.clone();
    let first = tokio::spawn(async move { requested.sync().await });
    tokio::task::yield_now().await;
    let owner = tokio::spawn(treasury.run_commands(commands, submission, stop.clone()));
    first.await.unwrap().unwrap();
    let count = calls.lock().unwrap().len();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        calls.lock().unwrap().len(),
        count,
        "a just-completed scan must not be immediately repeated"
    );
    stop.cancel();
    owner.await.unwrap().unwrap();
    assert!(!calls.lock().unwrap().iter().any(|m| m.contains("Send")));
    server.abort();
}

#[tokio::test]
async fn actor_startup_is_finite_and_source_commands_share_its_success_or_failure() {
    use x402_treazury::{
        rotation::{store, transaction::PrepareRequest},
        treasury::{actor, submission::GrpcSubmission},
    };
    for chain in ["main", "test"] {
        let dir = tempfile::tempdir().unwrap();
        let (endpoint, calls, server, _) = mock(chain).await;
        let mut treasury = wallet(dir.path()).await;
        treasury.configure_sync(SyncSettings::new(endpoint.clone(), 3).unwrap());
        let (handle, commands) = actor::channel();
        let stop = CancellationToken::new();
        let owner = tokio::spawn(treasury.run_commands(
            commands,
            GrpcSubmission::new(endpoint.clone(), endpoint).unwrap(),
            stop.clone(),
        ));
        let expected = if chain == "main" {
            SyncPhase::Ready
        } else {
            SyncPhase::Failed
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if store::status(&dir.path().join("state"))
                    .unwrap()
                    .sync
                    .is_some_and(|sync| sync.phase == expected)
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let before = store::status(&dir.path().join("state")).unwrap();
        let count = calls.lock().unwrap().len();
        assert!(count > 0, "fresh startup must attempt discovery");
        for _ in 0..3 {
            assert!(handle.reconcile("absent".into()).await.is_err());
            let error = handle
                .prepare(PrepareRequest {
                    managed_funding: None,
                    operation_id: uuid::Uuid::new_v4().to_string(),
                    pool_id: None,
                    deadline: x402_treazury::rotation::base::now().unwrap() + 1000,
                    recipient: "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F".into(),
                    amount_zatoshis: 50_000,
                    max_fee_zatoshis: 20_000,
                    max_input_zatoshis: 70_000,
                })
                .await
                .err()
                .unwrap();
            assert!(
                error.is::<x402_treazury::rotation::transaction::PreparationDeferred>(),
                "{error:#}"
            );
        }
        assert_eq!(
            calls.lock().unwrap().len(),
            count,
            "commands must share startup observation or its failure cooldown"
        );
        let after = store::status(&dir.path().join("state")).unwrap();
        assert_eq!(before.snapshot_revision, after.snapshot_revision);
        assert_eq!(
            before.sync.unwrap().checked_at,
            after.sync.unwrap().checked_at
        );
        assert!(after.treasury_operations.is_empty());
        // No claims or accepted transactions need another scan. Neither a
        // successful nor failed one-pass startup installs an idle watcher.
        tokio::time::pause();
        tokio::time::advance(std::time::Duration::from_secs(600)).await;
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        assert!(
            handle
                .recover_funding("absent".into(), 3, false)
                .await
                .is_err()
        );
        assert_eq!(
            calls.lock().unwrap().len(),
            count,
            "idle startup must remain finite"
        );
        stop.cancel();
        owner.await.unwrap().unwrap();
        assert!(
            !calls
                .lock()
                .unwrap()
                .iter()
                .any(|method| method.contains("Send"))
        );
        server.abort();
        tokio::time::resume();
    }
}

#[tokio::test]
async fn actor_observes_jobless_original_bytes_with_disabled_funding_and_drains_lookup() {
    use x402_treazury::rotation::{
        store::Store,
        transaction::{
            BroadcastTransaction, PreparedTransaction, SubmissionOutcome, TransactionFacts,
            TransactionPresence, TransactionSubmission,
        },
    };
    use x402_treazury::treasury::actor;
    struct Observer {
        seen: tokio::sync::oneshot::Sender<()>,
        finish: tokio::sync::oneshot::Receiver<()>,
    }
    impl TransactionSubmission for Observer {
        async fn submit(&mut self, _: BroadcastTransaction) -> anyhow::Result<SubmissionOutcome> {
            panic!("observation must never submit");
        }
        async fn lookup(
            &mut self,
            prepared: &PreparedTransaction,
        ) -> anyhow::Result<TransactionPresence> {
            assert_eq!(prepared.operation_id(), "jobless");
            assert_eq!(prepared.bytes(), b"original jobless bytes");
            let (unused, _) = tokio::sync::oneshot::channel();
            let _ = std::mem::replace(&mut self.seen, unused).send(());
            let _ = (&mut self.finish).await;
            Ok(TransactionPresence::Unknown)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let (endpoint, calls, server, _) = mock("main").await;
    let treasury = wallet(dir.path()).await;
    crate::capacity_fixture::treasury_pair(
        &treasury,
        "disabled",
        "5",
        Some(alloy_primitives::U256::from(1)),
        0,
    )
    .await
    .unwrap();
    let id = treasury.status().await.unwrap().treasury_id;
    treasury.close().await.unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let mut store = Store::open(&state, &key, &id).unwrap();
    let (revision, snapshot) = store.snapshot().unwrap();
    store
        .accept_preparation("jobless", None, 100, None)
        .unwrap();
    store
        .prepare_with_facts(
            "jobless",
            revision,
            &snapshot,
            b"original jobless bytes",
            Some(TransactionFacts {
                txid: "original".into(),
                expiry_height: TIP as u32 + 100,
                amount_zatoshis: 80,
                fee_zatoshis: 20,
                deadline: Some(u64::MAX),
            }),
        )
        .unwrap();
    store.disable_pool("disabled").unwrap();
    drop(store);
    let mut treasury = Treasury::open(state.clone(), key.clone(), id.clone())
        .await
        .unwrap();
    treasury.configure_sync(SyncSettings::new(endpoint, 3).unwrap());
    let (seen, observed) = tokio::sync::oneshot::channel();
    let (finish, drained) = tokio::sync::oneshot::channel();
    let (handle, commands) = actor::channel();
    let stop = CancellationToken::new();
    let owner = tokio::spawn(treasury.run_commands(
        commands,
        Observer {
            seen,
            finish: drained,
        },
        stop.clone(),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(10), observed)
        .await
        .unwrap()
        .unwrap();
    // A disconnected waiter does not repeat or erase the original operation.
    drop(handle);
    finish.send(()).unwrap();
    stop.cancel();
    owner.await.unwrap().unwrap();
    assert!(!calls.lock().unwrap().iter().any(|m| m.contains("Send")));
    let store = Store::open(&state, &key, &id).unwrap();
    assert_eq!(
        store.prepared_bytes("jobless").unwrap().as_slice(),
        b"original jobless bytes"
    );
    assert!(store.operation_pending("jobless").unwrap());
    assert_eq!(store.operation("jobless").unwrap().attempts, 0);
    assert_eq!(store.status().unwrap().treasury_operations.len(), 1);
    server.abort();
}

#[tokio::test]
async fn sync_cli_reports_independent_assets_without_provider_or_near_credentials() {
    use std::sync::atomic::AtomicUsize;
    for (indexer_mode, base_available) in [
        ("missing", true),
        ("invalid", true),
        ("conflicting", true),
        ("valid", false),
        ("unrelated", true),
        ("snapshot", true),
    ] {
        let zec_available = matches!(indexer_mode, "valid" | "unrelated");
        let dir = tempfile::tempdir().unwrap();
        let treasury = wallet(dir.path()).await;
        capacity_fixture::treasury_pair(
            &treasury,
            "configured",
            "2",
            Some(alloy_primitives::U256::from(1_000_000)),
            0,
        )
        .await
        .unwrap();
        let treasury_id = treasury.status().await.unwrap().treasury_id;
        treasury.close().await.unwrap();
        if indexer_mode == "snapshot" {
            let mut store = x402_treazury::rotation::store::Store::open(
                &dir.path().join("state"),
                &dir.path().join("key"),
                &treasury_id,
            )
            .unwrap();
            let (revision, _) = store.snapshot().unwrap();
            store
                .save_snapshot(revision, b"authenticated but incompatible SDK snapshot")
                .unwrap();
        }
        let (indexer, indexer_calls, indexer_server, _) = mock("main").await;
        let base_calls = Arc::new(AtomicUsize::new(0));
        let seen = base_calls.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_endpoint = format!("http://{}/rpc", listener.local_addr().unwrap());
        let app = axum::Router::new().route("/rpc", axum::routing::post(move |axum::Json(request):axum::Json<serde_json::Value>| {
            seen.fetch_add(1,Ordering::SeqCst);
            async move {
                assert_eq!(request["method"],"eth_call");
                assert_eq!(request["params"][1],"latest");
                axum::Json(serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":format!("0x{:064x}",17)}))
            }
        }));
        let base_server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let indexer_setting = match indexer_mode {
            "missing" => "indexer_url_env='MISSING_ZEC_ENDPOINT'".into(),
            "invalid" => "indexer_url='http://off-machine.invalid'".into(),
            "conflicting" => format!("indexer_url='{indexer}'\nindexer_url_env='SYNC_INDEXER'"),
            "unrelated" => format!(
                "indexer_url='{indexer}'\nsubmission_url='http://off-machine.invalid'\nsubmission_url_env='MISSING_SUBMISSION'\nmax_funding_transaction_fee_zec='invalid'\nmax_refund_shielding_fee_zec='invalid'"
            ),
            _ => format!("indexer_url='{indexer}'"),
        };
        let config = dir.path().join("sync.toml");
        std::fs::write(&config, format!("version=1\nservers={{}}\n[treasury]\nstate_dir='state'\nkey_file='key'\n{indexer_setting}\n[funding]\nbase_rpc_url_env='SYNC_BASE_RPC'\nbase_rpc_fallback_url_envs=[]\nnear_api_key_env='MISSING_NEAR_KEY'\nnear_user_session_env='MISSING_NEAR_SESSION'\n[sources.unused]\nspec='http://127.0.0.1:1/never-fetch'\napi_key_env='MISSING_PROVIDER_KEY'\n[wallets.configured]\nmode='zcash_rotation'\nmax_api_payment_usdc='1'\nmax_conversion_overhead_percent=5\n[wallets.configured.reserves.small]\nfunding_amount_usdc='2'\n[wallets.configured.reserves.large]\nfunding_amount_usdc='2'\n")).unwrap();
        if indexer_mode == "unrelated" {
            let mut text =
                std::fs::read_to_string(&config)
                    .unwrap()
                    .replacen("servers={}\n", "", 1);
            text.push_str(
                "[wallet_templates.unused]\nmode='zcash_rotation'\nmax_api_payment_usdc='invalid'\nmax_funding_spend_zec='invalid'\nmax_conversion_overhead_percent=5\n[wallet_templates.unused.reserves.small]\nfunding_amount_usdc='invalid'\n[servers.unrelated]\nlisten='127.0.0.1:8080'\nsources=['unused','missing']\nwallet='missing-wallet'\n",
            );
            std::fs::write(&config, text).unwrap();
        }
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"));
        command
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
            .args(["wallet", "sync", "--json", "--config"])
            .arg(&config);
        if base_available {
            command.env("SYNC_BASE_RPC", &base_endpoint);
        }
        if indexer_mode == "conflicting" {
            command.env("SYNC_INDEXER", &indexer);
        }
        if indexer_mode == "snapshot" {
            let owner = x402_treazury::rotation::store::Store::open(
                &dir.path().join("state"),
                &dir.path().join("key"),
                &treasury_id,
            )
            .unwrap();
            let rejected = command.output().await.unwrap();
            assert!(!rejected.status.success());
            assert!(rejected.stdout.is_empty());
            assert_eq!(base_calls.load(Ordering::SeqCst), 0);
            assert!(indexer_calls.lock().unwrap().is_empty());
            drop(owner);
        }
        let output = command.output().await.unwrap();
        assert_eq!(
            output.status.success(),
            zec_available && base_available,
            "{indexer_mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let reported: serde_json::Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
        if indexer_mode == "unrelated" {
            let other_command = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
                .env_clear()
                .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
                .args(["wallet", "addresses", "--config"])
                .arg(&config)
                .output()
                .await
                .unwrap();
            assert!(
                !other_command.status.success(),
                "other commands retain full validation"
            );
            assert!(other_command.stdout.is_empty());
        }
        let zec = &reported["observations"]["zec"];
        let usdc = &reported["observations"]["usdc"];
        if zec_available {
            assert_eq!(zec["status"], "completed");
        } else {
            assert_eq!(zec["status"], "failed");
            assert_eq!(zec["attempted"], 0);
            assert!(indexer_calls.lock().unwrap().is_empty());
        }
        if indexer_mode == "snapshot" {
            assert_eq!(zec["error"], "zec_snapshot_unavailable");
            assert_eq!(reported["receive_addresses"], serde_json::json!([]));
            let store = x402_treazury::rotation::store::Store::open(
                &dir.path().join("state"),
                &dir.path().join("key"),
                &treasury_id,
            )
            .unwrap();
            assert_eq!(
                store.snapshot().unwrap().1.as_slice(),
                b"authenticated but incompatible SDK snapshot"
            );
            assert_eq!(store.funding_jobs().unwrap().len(), 2);
            let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
            for table in ["outgoing", "operation_network", "payment_attempts"] {
                let count: i64 = db
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(count, 0, "snapshot failure must not create {table}");
            }
        }
        if base_available {
            assert_eq!(usdc["status"], "completed");
            assert_eq!(usdc["completed"], 2);
            assert_eq!(base_calls.load(Ordering::SeqCst), 2);
            assert!(
                reported["state"]["pools"][0]["addresses"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|wallet| wallet["balance_estimate_atomic"] == "17")
            );
        } else {
            assert_eq!(usdc["status"], "failed");
            assert_eq!(usdc["attempted"], 0);
            assert_eq!(usdc["error"], "usdc_configuration_unavailable");
            assert_eq!(base_calls.load(Ordering::SeqCst), 0);
        }
        assert!(
            reported["state"]["treasury_operations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            !indexer_calls
                .lock()
                .unwrap()
                .iter()
                .any(|method| method == "SendTransaction")
        );
        base_server.abort();
        indexer_server.abort();
    }
}

#[tokio::test]
async fn sync_cli_keeps_exclusive_treasury_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let treasury = wallet(dir.path()).await;
    let config = dir.path().join("sync.toml");
    std::fs::write(&config,"version=1\nservers={}\n[treasury]\nstate_dir='state'\nkey_file='key'\nindexer_url_env='MISSING_INDEXER'\n").unwrap();
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args(["wallet", "sync", "--json", "--config"])
        .arg(config)
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!treasury.status().await.unwrap().outgoing_pending);
    treasury.close().await.unwrap();
}
