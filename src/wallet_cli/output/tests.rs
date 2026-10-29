use super::*;
use crate::rotation::{
    store::Store,
    transaction::{OperationStatus, TransactionFacts},
};

#[test]
fn saved_status_explains_zero_balances_and_unavailable_snapshot_amounts() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let mut saved = store.status().unwrap();
    // The persisted observation format remains unchanged.
    saved.sync = Some(
        serde_json::from_value(serde_json::json!({
            "phase":"ready", "last_error":null,"snapshot_revision":1,
            "checked_at":1000,"checkpoint_at":1000,"scanned_blocks":1,
            "target_height":100,"height":100,"confirmations":3,
            "confirmed_shielded_zatoshis":0,"spendable_shielded_zatoshis":0
        }))
        .unwrap(),
    );
    let text = status(&saved, None);
    assert!(
        text.contains(
            "Saved confirmed shielded balance (excluding reserved inputs): 0.00000000 ZEC"
        )
    );
    assert!(text.contains("Saved pending shielded incoming/change: unknown"));
    assert!(text.contains("Saved confirmed transparent balance: unknown"));
    assert!(text.contains("Saved pending transparent balance: unknown"));
    assert!(text.contains("from the last saved sync"));
    let balances = crate::treasury::SnapshotBalances {
        confirmed_shielded: Some(0),
        pending_shielded: Some(900_000),
        spendable_shielded: Some(0),
        confirmed_transparent: Some(100_000),
        pending_transparent: Some(0),
    };
    saved.outgoing_pending = true;
    saved
        .funding_jobs
        .push(crate::rotation::store::funding::FundingJob {
            id: "job".into(),
            pool_id: "pool".into(),
            pool_name: "example".into(),
            wallet_id: "wallet".into(),
            recipient: "recipient".into(),
            target: "5000000".into(),
            phase: crate::rotation::store::funding::FundingPhase::Swapping,
            operation_id: "original".into(),
            attempts: 1,
            started_at: Some(1000),
            error_streak: 0,
            timed_out: false,
            next_poll: 0,
            last_error: None,
        });
    saved.treasury_operations.push(OperationStatus {
        operation_id: "original".into(),
        submission: "BROADCAST".into(),
        attempts: 1,
        facts: TransactionFacts {
            txid: "original-tx".into(),
            expiry_height: 200,
            amount_zatoshis: 50_000,
            fee_zatoshis: 10_000,
            deadline: None,
        },
    });
    let text = status(&saved, Some(&balances));
    assert!(text.contains(
        "Last completed observation: 1970-01-01 00:16:40 UTC\nSaved scanned height: 100"
    ));
    assert!(text.contains("Saved pending shielded incoming/change: 0.00900000 ZEC"));
    assert!(text.contains("Saved confirmed transparent balance: 0.00100000 ZEC"));
    assert!(text.contains("Saved pending transparent balance: 0.00000000 ZEC"));
    assert!(text.contains("0.00050000 ZEC (fee: 0.00010000 ZEC)"));
    assert!(text.contains("Saved source status: broadcast; awaiting confirmation"));
    assert!(text.contains("ZEC source transaction original: 0.00050000 ZEC"));
    assert!(text.contains(
        "Saved swap progress for example (operation original): awaiting settlement; target:"
    ));
    assert!(text.contains(&usdc("5000000")));
    assert!(text.contains("do not add them to the balances above"));
    saved.sync = None;
    saved.treasury_operations[0].submission = "PREPARED".into();
    let text = status(&saved, Some(&balances));
    assert!(text.contains("Saved source status: prepared"));
    assert!(!text.contains("no broadcast"));
    assert!(status(&saved, None).contains("Shielded balances: unknown"));
}
