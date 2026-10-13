//! RPC-level coverage of balance acquisition and independent historical anchors.
use super::*;
use axum::{Json, Router, routing::post};
use std::sync::{Arc, Mutex};

type Requests = Arc<Mutex<Vec<Value>>>;

async fn fixture(reorg: bool) -> (BaseRpc, Requests, tokio::task::JoinHandle<()>, u64) {
    fixture_nonce(reorg, 1).await
}

async fn fixture_nonce(
    reorg: bool,
    nonce: u64,
) -> (BaseRpc, Requests, tokio::task::JoinHandle<()>, u64) {
    let stamp = now().unwrap();
    let requests = Requests::default();
    let head = Arc::new(std::sync::atomic::AtomicU64::new(100));
    let app = Router::new().route("/", post({
        let requests = requests.clone();
        move |Json(v): Json<Value>| {
            let (requests, head) = (requests.clone(), head.clone());
            async move {
                let mut requests = requests.lock().unwrap();
                requests.push(v.clone());
                let result = match v["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_getBlockByNumber" => {
                        let tag = v["params"][0].as_str().unwrap();
                        let height = if tag == "latest" {
                            head.load(std::sync::atomic::Ordering::SeqCst)
                        } else {
                            u64::from_str_radix(&tag[2..], 16).unwrap()
                        };
                        let repeated = requests.iter().filter(|r| r["method"] == "eth_getBlockByNumber" && r["params"][0] == "0x58").count() > 1;
                        let hash = if reorg && height == 88 && repeated { 999 } else { height };
                        let timestamp = if height <= 100 { stamp - 60 } else { stamp };
                        json!({"number":format!("0x{height:x}"), "hash":format!("0x{hash:064x}"), "timestamp":format!("0x{timestamp:x}")})
                    },
                    "eth_call" => {
                        assert_eq!(v["params"][1]["requireCanonical"], true);
                        if v["params"][0]["data"].as_str().unwrap().starts_with("0x70a08231") {
                            json!(format!("0x{:064x}", 10))
                        } else {
                            // Advance while scanning historical authorization state.
                            assert_eq!(v["params"][1]["blockHash"], format!("0x{:064x}", 88));
                            head.store(200, std::sync::atomic::Ordering::SeqCst);
                            json!(format!("0x{nonce:064x}"))
                        }
                    },
                    method => panic!("unexpected RPC {method}"),
                };
                Json(json!({"jsonrpc":"2.0", "id":v["id"], "result":result}))
            }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc = BaseRpc::new(
        &format!("http://{}/", listener.local_addr().unwrap()),
        12,
        120,
    )
    .unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (rpc, requests, server, stamp)
}

fn query(history: bool) -> ChainQuery {
    let payer = format!("0x{:040x}", 1);
    ChainQuery {
        wallets: vec![("wallet".into(), payer.clone())],
        pending: if history {
            vec![PendingAuthorization {
                id: "attempt".into(),
                wallet: "wallet".into(),
                payer,
                nonce: format!("0x{:064x}", 1),
                valid_before: u64::MAX,
            }]
        } else {
            vec![]
        },
        anchor: None,
        captures: BTreeMap::new(),
    }
}

fn capture(wallet: &str, checkpoint: Option<Anchor>) -> EvidenceCapture {
    EvidenceCapture {
        namespace: uuid::Uuid::new_v4(),
        wallet: wallet.into(),
        generation: 0,
        endpoint_generations: BTreeMap::new(),
        checkpoint,
        previous: None,
        previous_confirmed_covered: false,
        durable_balance_anchors: vec![],
    }
}

fn anchor(height: u64) -> Anchor {
    Anchor {
        height,
        hash: format!("0x{height:064x}"),
    }
}

fn header_reads(requests: &Requests, height: u64) -> usize {
    requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| {
            request["method"] == "eth_getBlockByNumber"
                && request["params"][0] == format!("0x{height:x}")
        })
        .count()
}

#[tokio::test]
async fn captured_and_acquired_anchors_share_post_balance_checks_across_wallets() {
    let (rpc, requests, server, stamp) = fixture(false).await;
    let mut query = query(false);
    query
        .wallets
        .push(("second".into(), format!("0x{:040x}", 2)));
    for wallet in ["wallet", "second"] {
        let mut captured = capture(wallet, Some(anchor(88)));
        captured.durable_balance_anchors = vec![anchor(88), anchor(100)];
        query.captures.insert(wallet.into(), captured);
    }
    let view = rpc.view(query).await.unwrap();
    assert_eq!(view.balance_certificates.len(), 2);
    assert_eq!(view.admission_valid_until, stamp - 60 + 120);
    assert_eq!(
        header_reads(&requests, 88),
        2,
        "initial header and one post-state recheck"
    );
    assert_eq!(header_reads(&requests, 100), 1);
    assert_eq!(requests.lock().unwrap().len(), 9);
    server.abort();
}

#[tokio::test]
async fn shared_header_checks_preserve_captured_conflict_scope() {
    // Both the financial checkpoint and immutable readiness-only support must
    // retain their typed attribution when an acquired anchor changes.
    for financial in [false, true] {
        let (rpc, _, server, _) = fixture(true).await;
        let mut query = query(false);
        let mut captured = capture("wallet", financial.then(|| anchor(88)));
        captured.durable_balance_anchors.push(anchor(88));
        query.captures.insert("wallet".into(), captured);
        let error = rpc.view(query).await.err().unwrap();
        let conflict = error.downcast_ref::<EvidenceConflict>().unwrap();
        assert_eq!(conflict.wallet, "wallet");
        assert_eq!(conflict.anchor, anchor(88));
        assert_eq!(conflict.financial_checkpoint, financial);
        assert!(!conflict.latest_only);
        server.abort();
    }
}

#[tokio::test]
async fn shared_height_still_compares_each_captured_hash() {
    let (rpc, requests, server, _) = fixture(false).await;
    let mut query = query(false);
    let conflicting = Anchor {
        height: 88,
        hash: anchor(999).hash,
    };
    query.captures.insert(
        "wallet".into(),
        capture("wallet", Some(conflicting.clone())),
    );
    let error = rpc.view(query).await.err().unwrap();
    let conflict = error.downcast_ref::<EvidenceConflict>().unwrap();
    assert_eq!(conflict.anchor, conflicting);
    assert!(conflict.financial_checkpoint);
    assert_eq!(header_reads(&requests, 88), 2);
    server.abort();
}

#[tokio::test]
async fn historical_resolution_and_compatibility_share_identical_anchors() {
    let (rpc, requests, server, _) = fixture(false).await;
    let mut query = query(true);
    query.wallets.clear();
    let captured = capture("wallet", Some(anchor(88)));
    query.captures.insert("wallet".into(), captured.clone());
    let view = rpc.view(query).await.unwrap();
    assert_eq!(header_reads(&requests, 88), 2);
    let original = view.resolution_certificates["attempt"].clone();
    assert!(view.balances.is_empty());
    assert!(view.observations.is_empty());
    assert!(view.balance_certificates.is_empty());
    assert_eq!(view.anchor, anchor(88));
    {
        let requests = requests.lock().unwrap();
        let methods: Vec<_> = requests
            .iter()
            .map(|r| r["method"].as_str().unwrap())
            .collect();
        assert_eq!(
            methods,
            [
                "eth_chainId",
                "eth_getBlockByNumber",
                "eth_getBlockByNumber",
                "eth_call",
                "eth_getBlockByNumber"
            ]
        );
        assert_eq!(requests[1]["params"][0], "latest");
        assert_eq!(requests[2]["params"][0], "0x58");
        assert_eq!(requests[4]["params"][0], "0x58");
    }
    requests.lock().unwrap().clear();
    let verified = rpc
        .recheck_resolution(original.clone(), captured)
        .await
        .unwrap();
    assert_eq!(header_reads(&requests, 88), 1);
    assert_eq!(verified.anchor(), original.anchor());
    assert_eq!(verified.block_time(), original.block_time());
    assert_eq!(verified.outcome(), original.outcome());
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "chain ID and exact header only"
    );
    server.abort();
}

#[tokio::test]
async fn history_replacement_retains_readiness_only_conflict_scope() {
    let (rpc, _, server, _) = fixture(true).await;
    let mut query = query(true);
    query.wallets.clear();
    let mut captured = capture("wallet", None);
    captured.durable_balance_anchors.push(anchor(88));
    query.captures.insert("wallet".into(), captured);
    let error = rpc.view(query).await.err().unwrap();
    let conflict = error
        .downcast_ref::<EvidenceConflict>()
        .expect("historical readiness conflict must remain typed");
    assert_eq!(conflict.wallet, "wallet");
    assert_eq!(conflict.anchor, anchor(88));
    assert!(!conflict.financial_checkpoint);
    assert!(!conflict.latest_only);
    server.abort();
}

#[tokio::test]
async fn historical_balance_reverification_shares_only_within_each_state_read_phase() {
    for reorg in [false, true] {
        let (rpc, requests, server, _) = fixture(reorg).await;
        let observation = evidence::BalanceObservation {
            wallet: "wallet".into(),
            address: format!("0x{:040x}", 1),
            network: "eip155:8453".into(),
            asset: USDC.into(),
            endpoint: rpc.endpoint_id(),
            confirmations: 12,
            confirmed: evidence::BalancePoint {
                anchor: anchor(88),
                amount: U256::from(10),
            },
            latest: evidence::BalancePoint {
                anchor: anchor(100),
                amount: U256::from(10),
            },
            // Historical reverification must neither require fresh amounts nor
            // turn these deliberately expired observations into payment authority.
            valid_until: 1,
        };
        let request = historical::HistoricalBalanceRequest {
            capture: capture("wallet", Some(anchor(88))),
            observation: observation.clone(),
            checkpoint: Some(anchor(88)),
        };
        let result = rpc.verify_historical_balance(request).await;
        if reorg {
            let error = result.err().unwrap();
            let conflict = error.downcast_ref::<EvidenceConflict>().unwrap();
            assert!(conflict.financial_checkpoint);
        } else {
            assert_eq!(result.unwrap().request().observation, observation);
            assert_eq!(header_reads(&requests, 100), 2);
        }
        assert_eq!(
            header_reads(&requests, 88),
            2,
            "one pre-state and one post-state check"
        );
        let reads = requests.lock().unwrap();
        let balances: Vec<_> = reads
            .iter()
            .enumerate()
            .filter(|(_, r)| r["method"] == "eth_call")
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            balances.len(),
            2,
            "reorg must be detected after acquiring historical amounts"
        );
        let confirmations: Vec<_> = reads
            .iter()
            .enumerate()
            .filter(|(_, r)| r["method"] == "eth_getBlockByNumber" && r["params"][0] == "0x58")
            .map(|(n, _)| n)
            .collect();
        assert!(confirmations[0] < balances[0] && confirmations[1] > balances[1]);
        server.abort();
    }
}

#[tokio::test]
async fn balance_only_reads_one_header_pair_and_rechecks_both_anchors() {
    let (rpc, requests, server, stamp) = fixture(false).await;
    let view = rpc.view(query(false)).await.unwrap();
    let observation = &view.observations["wallet"];
    assert_eq!(observation.confirmed.anchor.height, 88);
    assert_eq!(observation.latest.anchor.height, 100);
    assert_eq!(view.admission_valid_until, stamp - 60 + 120);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 7, "no redundant admission header fetches");
    let headers: Vec<_> = requests
        .iter()
        .filter(|r| r["method"] == "eth_getBlockByNumber")
        .map(|r| r["params"][0].as_str().unwrap())
        .collect();
    assert_eq!(headers, ["latest", "0x58", "0x58", "0x64"]);
    let balances: Vec<_> = requests
        .iter()
        .filter(|r| r["method"] == "eth_call")
        .map(|r| r["params"][1]["blockHash"].as_str().unwrap())
        .collect();
    assert_eq!(
        balances,
        [
            observation.confirmed.anchor.hash.as_str(),
            observation.latest.anchor.hash.as_str()
        ]
    );
    server.abort();
}

#[tokio::test]
async fn combined_history_acquires_new_admission_headers_without_relabeling_resolution() {
    let (rpc, requests, server, stamp) = fixture(false).await;
    let view = rpc.view(query(true)).await.unwrap();
    let observation = &view.observations["wallet"];
    assert_eq!(
        view.resolutions["attempt"].anchor.as_ref().unwrap().height,
        88
    );
    assert_eq!(view.resolutions["attempt"].block_time, stamp - 60);
    assert_eq!(observation.confirmed.anchor.height, 188);
    assert_eq!(observation.latest.anchor.height, 200);
    assert_eq!(view.admission_valid_until, stamp + 120);
    let requests = requests.lock().unwrap();
    let headers: Vec<_> = requests
        .iter()
        .filter(|r| r["method"] == "eth_getBlockByNumber")
        .map(|r| r["params"][0].as_str().unwrap())
        .collect();
    assert_eq!(
        headers,
        ["latest", "0x58", "latest", "0xbc", "0x58", "0xbc", "0xc8"]
    );
    let balances: Vec<_> = requests
        .iter()
        .filter(|r| {
            r["method"] == "eth_call"
                && r["params"][0]["data"]
                    .as_str()
                    .unwrap()
                    .starts_with("0x70a08231")
        })
        .map(|r| r["params"][1]["blockHash"].as_str().unwrap())
        .collect();
    assert_eq!(
        balances,
        [
            observation.confirmed.anchor.hash.as_str(),
            observation.latest.anchor.hash.as_str()
        ]
    );
    server.abort();
}

#[tokio::test]
async fn balance_only_rejects_anchor_replacement_after_balance_reads() {
    let (rpc, _, server, _) = fixture(true).await;
    let error = rpc.view(query(false)).await.err().unwrap();
    assert!(
        error
            .to_string()
            .contains("Base view changed during reconciliation")
    );
    server.abort();
}

#[tokio::test]
async fn unused_authorization_requires_strictly_later_confirmed_time_for_release() {
    for offset in [-1i64, 0, 1] {
        let (rpc, requests, server, stamp) = fixture_nonce(false, 0).await;
        let confirmed_time = stamp - 60;
        let mut query = query(true);
        query.wallets.clear();
        query.pending[0].valid_before = confirmed_time.checked_add_signed(offset).unwrap();
        query
            .captures
            .insert("wallet".into(), capture("wallet", Some(anchor(88))));
        let view = rpc.view(query).await.unwrap();
        assert!(
            view.balances.is_empty()
                && view.observations.is_empty()
                && view.balance_certificates.is_empty()
        );
        if offset < 0 {
            assert_eq!(view.released, ["attempt"]);
            let resolution = &view.resolutions["attempt"];
            assert_eq!(resolution.outcome, AuthorizationOutcome::ExpiredUnused);
            assert_eq!(resolution.anchor.as_ref(), Some(&anchor(88)));
            assert_eq!(resolution.block_time, confirmed_time);
            let certificate = &view.resolution_certificates["attempt"];
            assert_eq!(certificate.outcome(), &AuthorizationOutcome::ExpiredUnused);
            assert_eq!(certificate.anchor(), &anchor(88));
            assert_eq!(certificate.block_time(), confirmed_time);
        } else {
            assert!(
                view.released.is_empty()
                    && view.resolutions.is_empty()
                    && view.resolution_certificates.is_empty(),
                "equal/future expiry must preserve authorization liability"
            );
        }
        let reads = requests.lock().unwrap();
        assert_eq!(
            reads.len(),
            5,
            "historical proof requires one exact anchor recheck and no balance acquisition"
        );
        assert_eq!(reads[3]["params"][1]["blockHash"], anchor(88).hash);
        assert_eq!(reads[4]["params"][0], "0x58");
        drop(reads);
        server.abort();
    }
}

#[tokio::test]
async fn invalid_authorization_boolean_fails_before_admission_balance_reads() {
    let (rpc, requests, server, _) = fixture_nonce(false, 2).await;
    let error = rpc
        .view(query(true))
        .await
        .err()
        .expect("non-boolean nonce result must fail");
    assert!(
        error.downcast_ref::<RpcFailure>().is_some(),
        "malformed RPC evidence must retain its typed failure"
    );
    let reads = requests.lock().unwrap();
    assert_eq!(
        reads.len(),
        4,
        "invalid historical evidence must stop before fresh admission headers or balances"
    );
    assert_eq!(reads[0]["method"], "eth_chainId");
    assert_eq!(reads[1]["params"][0], "latest");
    assert_eq!(reads[2]["params"][0], "0x58");
    assert_eq!(reads[3]["method"], "eth_call");
    assert!(
        !reads[3]["params"][0]["data"]
            .as_str()
            .unwrap()
            .starts_with("0x70a08231")
    );
    drop(reads);
    server.abort();
}
