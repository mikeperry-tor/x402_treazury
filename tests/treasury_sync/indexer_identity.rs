//! Full-header SDK identity must outrank stale/missing compact hash projections.
use super::*;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, AtomicUsize},
};

#[derive(Clone)]
struct Header {
    bytes: Vec<u8>,
    computed_hash: Vec<u8>,
    previous_hash: Vec<u8>,
}

pub(super) struct Blocks {
    pub tip: AtomicU64,
    mode: AtomicUsize,
    headers: BTreeMap<u64, Header>,
}
impl Blocks {
    fn new() -> Self {
        let mut previous = hash(TIP - 7);
        let mut headers = BTreeMap::new();
        for height in TIP - 6..=TIP {
            // A serialized v4 header with an empty solution. This fixture tests
            // SDK header parsing/identity, never PoW or consensus validity.
            let mut header = vec![0; 141];
            header[..4].copy_from_slice(&4i32.to_le_bytes());
            header[4..36].copy_from_slice(&previous);
            header[100..104].copy_from_slice(&(height as u32).to_le_bytes());
            let parsed = zcash_primitives::block::BlockHeader::read(header.as_slice()).unwrap();
            let computed = parsed.hash().0.to_vec();
            headers.insert(
                height,
                Header {
                    bytes: header,
                    computed_hash: computed.clone(),
                    previous_hash: previous,
                },
            );
            previous = computed;
        }
        Self {
            tip: AtomicU64::new(TIP),
            mode: AtomicUsize::new(0),
            headers,
        }
    }
    pub fn hash(&self, height: u64) -> Vec<u8> {
        self.headers
            .get(&height)
            .map_or_else(|| hash(height), |header| header.computed_hash.clone())
    }
    pub fn block(&self, height: u64, unary: bool) -> Vec<u8> {
        let Header {
            bytes: mut header,
            computed_hash: mut hash,
            previous_hash: previous,
        } = self.headers[&height].clone();
        let mut returned_height = height;
        if unary {
            match self.mode.load(Ordering::SeqCst) {
                1 => hash = vec![99; 32],    // Header remains the established identity.
                2 => hash.clear(),           // A valid full header needs no raw projection.
                3 => header = vec![1, 2, 3], // Existing SDK fallback uses validated raw hash.
                4 => {
                    header.clear();
                    hash = vec![99; 31];
                }
                5 => returned_height += 1,
                _ => (),
            }
        }
        [
            number(2, returned_height),
            bytes(3, &hash),
            bytes(4, &previous),
            bytes(6, &header),
            bytes(8, &[]),
        ]
        .concat()
    }
}

async fn retained_facts(dir: &std::path::Path, identity: &str) -> (Vec<u8>, Vec<(u32, Vec<u8>)>) {
    let store =
        x402_treazury::rotation::store::Store::open(&dir.join("state"), &dir.join("key"), identity)
            .unwrap();
    let (_, bytes) = store.snapshot().unwrap();
    drop(store);
    let config = zingolib::config::ClientConfig::builder()
        .set_wallet_dir(dir.join("inspection"))
        .set_wallet_config(zingolib::config::WalletConfig::MnemonicPhrase {
            mnemonic_phrase: SEED.into(),
            no_of_accounts: std::num::NonZeroU32::new(1).unwrap(),
            birthday: (TIP - 5) as u32,
            wallet_settings: zingolib::wallet::WalletSettings::default(),
        })
        .build()
        .unwrap();
    let client = zingolib::lightclient::LightClient::from_bytes(bytes.to_vec(), config)
        .await
        .unwrap();
    let wallet = client.wallet().read().await;
    let mut scan = Vec::new();
    wallet.sync_state.clone().write(&mut scan).unwrap();
    let blocks = wallet
        .wallet_blocks
        .iter()
        .map(|(height, block)| (u32::from(*height), block.block_hash().0.to_vec()))
        .collect();
    (scan, blocks)
}

#[tokio::test]
async fn regressed_block_identity_preserves_sdk_headers_and_rejects_unbound_responses() {
    let dir = tempfile::tempdir().unwrap();
    let blocks = Arc::new(Blocks::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
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
        checkpoint_lock: None,
        block_identity: Some(blocks.clone()),
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut owner = Treasury::create(
        dir.path().join("state"),
        dir.path().join("key"),
        (TIP - 5) as u32,
        Some(zeroize::Zeroizing::new(SEED.into())),
    )
    .await
    .unwrap();
    owner.configure_sync(SyncSettings::new(endpoint.clone(), 1).unwrap());
    let stop = CancellationToken::new();
    owner.sync_once(&stop).await.unwrap();
    let identity = owner.status().await.unwrap().treasury_id;
    owner.close().await.unwrap();
    let expected = retained_facts(dir.path(), &identity).await;
    blocks.tip.store(TIP - 2, Ordering::SeqCst);
    for mode in 1..=5 {
        blocks.mode.store(mode, Ordering::SeqCst);
        let mut owner = Treasury::open(
            dir.path().join("state"),
            dir.path().join("key"),
            identity.clone(),
        )
        .await
        .unwrap();
        owner.configure_sync(SyncSettings::new(endpoint.clone(), 1).unwrap());
        assert!(owner.sync_once(&stop).await.is_err());
        let observation = owner.status().await.unwrap().sync.unwrap();
        assert_eq!(observation.phase, SyncPhase::Failed);
        assert_eq!(observation.height, Some(TIP));
        let reason = if mode <= 3 {
            "indexer_tip_regressed_without_divergence"
        } else {
            "indexer_block_identity_invalid"
        };
        assert!(
            observation.last_error.unwrap().contains(reason),
            "mode {mode}"
        );
        owner.close().await.unwrap();
        assert_eq!(
            retained_facts(dir.path(), &identity).await,
            expected,
            "mode {mode}"
        );
    }
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|method| method == "SendTransaction")
    );
    server.abort();
}
