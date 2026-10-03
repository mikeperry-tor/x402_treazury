#![cfg(feature = "zcash")]
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, Uri},
    response::IntoResponse,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, AtomicUsize, Ordering},
};
use x402_treazure::{
    rotation::{
        base::now,
        store::Store,
        transaction::{
            BroadcastTransaction, PreparedTransaction, SubmissionOutcome, TransactionFacts,
            TransactionPresence, TransactionSubmission,
        },
    },
    treasury::submission::GrpcSubmission,
};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::BranchId;
fn varint(mut n: u64) -> Vec<u8> {
    let mut v = vec![];
    while n >= 128 {
        v.push(n as u8 | 128);
        n >>= 7;
    }
    v.push(n as u8);
    v
}
fn bytes(tag: u8, v: &[u8]) -> Vec<u8> {
    [vec![tag << 3 | 2], varint(v.len() as u64), v.to_vec()].concat()
}
fn number(tag: u8, n: u64) -> Vec<u8> {
    [vec![tag << 3], varint(n)].concat()
}
fn frame(v: Vec<u8>) -> Vec<u8> {
    [vec![0], (v.len() as u32).to_be_bytes().to_vec(), v].concat()
}
// Syntactically valid empty V5 encoding, not a funded/consensus-valid payment.
// The mock accepts it solely to exercise raw transport and identity checks.
fn raw() -> Vec<u8> {
    let mut v = vec![];
    for n in [0x80000005u32, 0x26a7270a, 0xc2d6d0b4, 0, 500] {
        v.extend(n.to_le_bytes());
    }
    v.extend([0, 0, 0, 0, 0]);
    v
}
#[derive(Clone)]
struct Mock {
    mode: Arc<AtomicU8>,
    blocks: Arc<AtomicUsize>,
    sends: Arc<Mutex<Vec<Vec<u8>>>>,
    raw: Vec<u8>,
    txid: String,
    hash: Vec<u8>,
}
async fn rpc(State(s): State<Mock>, uri: Uri, body: Bytes) -> impl IntoResponse {
    let mode = s.mode.load(Ordering::SeqCst);
    let mut status = "0";
    let response = match uri.path().rsplit('/').next().unwrap() {
        "GetLightdInfo" => bytes(4, if mode == 4 { b"test" } else { b"main" }),
        "GetLatestBlock" => number(1, if mode == 5 { 500 } else { 499 }),
        "SendTransaction" => {
            s.sends.lock().unwrap().push(body.to_vec());
            if mode == 3 {
                std::future::pending::<()>().await;
            }
            if mode == 2 {
                status = "14";
                vec![]
            } else {
                bytes(
                    2,
                    if mode == 1 {
                        b"DUMMY_SUBMISSION_SECRET"
                    } else {
                        s.txid.as_bytes()
                    },
                )
            }
        }
        "GetTransaction" => {
            if mode == 6 {
                status = "5";
                vec![]
            } else {
                [
                    bytes(1, if mode == 7 { b"conflicting" } else { &s.raw }),
                    number(
                        2,
                        if mode == 14 {
                            u64::MAX
                        } else if mode >= 8 {
                            490
                        } else {
                            0
                        },
                    ),
                ]
                .concat()
            }
        }
        "GetBlock" => {
            let call = s.blocks.fetch_add(1, Ordering::SeqCst);
            // Fork during lookup, absent tx, wrong height, malformed hash,
            // and a failed canonical recheck must all remain unresolved.
            if mode == 13 && call > 0 {
                status = "14";
            }
            [
                number(2, if mode == 11 { 489 } else { 490 }),
                bytes(
                    3,
                    if mode == 12 {
                        &[1; 31]
                    } else if mode == 9 && call > 0 {
                        &[2; 32]
                    } else {
                        &[1; 32]
                    },
                ),
                bytes(7, &bytes(2, if mode == 10 { &[0; 32] } else { &s.hash })),
            ]
            .concat()
        }
        _ => {
            status = "12";
            vec![]
        }
    };
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/grpc".parse().unwrap());
    headers.insert("grpc-status", status.parse().unwrap());
    if status != "0" {
        headers.insert("grpc-message", "DUMMY_SUBMISSION_SECRET".parse().unwrap());
    }
    (
        headers,
        if status == "0" {
            frame(response)
        } else {
            vec![]
        },
    )
}
#[tokio::test]
async fn explicit_sender_checks_identity_network_expiry_and_lookup_without_retries() {
    let raw = raw();
    let tx = Transaction::read(raw.as_slice(), BranchId::Nu5).unwrap();
    let state = Mock {
        mode: Arc::new(AtomicU8::new(0)),
        blocks: Arc::new(AtomicUsize::new(0)),
        sends: Arc::new(Mutex::new(vec![])),
        raw: raw.clone(),
        txid: tx.txid().to_string(),
        hash: tx.txid().as_ref().to_vec(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new().fallback(rpc).with_state(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"test-wallet",
    )
    .unwrap();
    store
        .bind_operation_recipient("op", "0x0000000000000000000000000000000000000001")
        .unwrap();
    store.reserve("op", None, 0, 100, 100).unwrap();
    let facts = TransactionFacts {
        txid: tx.txid().to_string(),
        expiry_height: 500,
        amount_zatoshis: 50,
        fee_zatoshis: 10,
        deadline: now().unwrap() + 600,
    };
    store
        .prepare_with_facts("op", 1, b"calculated", &raw, Some(facts))
        .unwrap();
    let prepared = PreparedTransaction::load(&store, "op").unwrap();
    // Different endpoints: lookup must work even if the submission endpoint fails.
    let mut sender = GrpcSubmission::new(endpoint.clone(), endpoint.clone()).unwrap();
    assert_eq!(
        sender
            .submit(
                BroadcastTransaction::request(&mut store, "op", now().unwrap(), 499, true).unwrap()
            )
            .await
            .unwrap(),
        SubmissionOutcome::Accepted
    );
    assert_eq!(state.sends.lock().unwrap()[0], frame(bytes(1, &raw)));
    for mode in [1, 2] {
        state.mode.store(mode, Ordering::SeqCst);
        assert_eq!(
            sender
                .submit(
                    BroadcastTransaction::request(&mut store, "op", now().unwrap(), 499, true)
                        .unwrap()
                )
                .await
                .unwrap(),
            SubmissionOutcome::Unknown
        );
    }
    assert_eq!(state.sends.lock().unwrap().len(), 3); // never implicit retries
    for mode in [4, 5] {
        state.mode.store(mode, Ordering::SeqCst);
        assert_eq!(
            sender
                .submit(
                    BroadcastTransaction::request(&mut store, "op", now().unwrap(), 499, true)
                        .unwrap()
                )
                .await
                .unwrap(),
            SubmissionOutcome::Unknown
        );
    }
    assert_eq!(state.sends.lock().unwrap().len(), 3);
    for (mode, want) in [
        (0, TransactionPresence::Mempool),
        (6, TransactionPresence::Absent),
        (7, TransactionPresence::Unknown),
        (8, TransactionPresence::Confirmed { height: 490 }),
        (9, TransactionPresence::Unknown),
        (10, TransactionPresence::Unknown),
        (11, TransactionPresence::Unknown),
        (12, TransactionPresence::Unknown),
        (13, TransactionPresence::Unknown),
        (14, TransactionPresence::Unknown),
    ] {
        state.mode.store(mode, Ordering::SeqCst);
        state.blocks.store(0, Ordering::SeqCst);
        assert_eq!(sender.lookup(&prepared).await.unwrap(), want);
        assert!(
            store.operation_pending("op").unwrap(),
            "lookup alone never settles accounting"
        );
    }
    state.mode.store(3, Ordering::SeqCst);
    let intent =
        BroadcastTransaction::request(&mut store, "op", now().unwrap(), 499, true).unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), sender.submit(intent))
            .await
            .is_err()
    );
    assert_eq!(state.sends.lock().unwrap().len(), 4);
    assert_eq!(
        store.operation("op").unwrap().submission,
        "BROADCAST_REQUESTED"
    );
    assert!(store.operation_pending("op").unwrap());
    let mut split = GrpcSubmission::new("http://127.0.0.1:1".into(), endpoint).unwrap();
    state.mode.store(0, Ordering::SeqCst);
    assert_eq!(
        split.lookup(&prepared).await.unwrap(),
        TransactionPresence::Mempool
    );
    server.abort();
}
