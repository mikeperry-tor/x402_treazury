//! Durable endpoint distrust must survive cache eviction and process restart.
use super::*;

#[test]
fn typed_endpoint_conflict_restores_only_owned_release_liabilities_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let mut store = Store::create(&state, &key, 1, b"fixture").unwrap();
    let treasury = store.id().to_owned();
    for name in ["first", "second"] {
        crate::rotation::store::test_support::declare_two_reserves(&mut store, name, "1", None, 0)
            .unwrap();
    }
    let wallets: Vec<_> = store
        .status()
        .unwrap()
        .pools
        .into_iter()
        .flat_map(|pool| pool.addresses)
        .collect();
    let endpoint = "a".repeat(64);
    let other = "b".repeat(64);
    let insert = |id: &str, index: usize, support: String| {
        let wallet = &wallets[index].id;
        store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) SELECT ?1,pool_id,id,0,'1','fixture','RESOLVED' FROM wallets WHERE id=?2", params![id,wallet]).unwrap();
        store
            .db
            .execute(
                "INSERT INTO payment_resolutions VALUES(?1,'USED',80,'original',1,?2)",
                params![id, support],
            )
            .unwrap();
    };
    insert(
        "affected",
        0,
        serde_json::json!({"endpoint":endpoint}).to_string(),
    );
    insert(
        "same-wallet-other-proof",
        0,
        serde_json::json!({"endpoint":other}).to_string(),
    );
    insert(
        "unaffected",
        1,
        serde_json::json!({"endpoint":other}).to_string(),
    );
    insert("unknown", 2, "{broken".into());
    insert(
        "existing-amount-dispute",
        3,
        serde_json::json!({"endpoint":endpoint}).to_string(),
    );
    store.db.execute("UPDATE wallets SET financial_height=80,financial_hash='original',debit_height=80,debit_hash='original'", []).unwrap();
    store
        .db
        .execute(
            "UPDATE wallets SET evidence_repair='evidence_repair_required' WHERE id=?1",
            [&wallets[3].id],
        )
        .unwrap();
    let captures: Vec<_> = wallets
        .iter()
        .map(|wallet| store.capture_evidence(&wallet.id).unwrap())
        .collect();
    // A read outage carries no distrust authority and cannot resurrect liabilities.
    store
        .record_base_failure(&anyhow::anyhow!("RPC temporarily unavailable"))
        .unwrap();
    assert_eq!(exposure(&store.db, &wallets[0].id).unwrap(), U256::ZERO);
    assert_eq!(generation(&store.db, "endpoint", &endpoint).unwrap(), 0);
    for support in [
        "{broken",
        "{}",
        "{\"endpoint\":3}",
        "{\"endpoint\":null}",
        "{\"endpoint\":[]}",
        "{\"endpoint\":\"bad\"}",
    ] {
        store
            .db
            .execute(
                "UPDATE payment_resolutions SET support=?1 WHERE attempt_id='unknown'",
                [support],
            )
            .unwrap();
        assert!(payment_endpoint_dependents(&store.db, &endpoint).unwrap()[&wallets[2].id]);
    }
    // An error late in the transaction cannot leave only liabilities or trust
    // generations changed. The real funding dispute update follows payments.
    store
        .db
        .execute(
            "UPDATE funding_jobs SET readiness_endpoint=?1 WHERE wallet_id=?2",
            params![endpoint, wallets[0].id],
        )
        .unwrap();
    store.db.execute_batch("CREATE TEMP TRIGGER reject_endpoint_dispute BEFORE UPDATE OF readiness_disputed ON funding_jobs BEGIN SELECT RAISE(ABORT,'fixture rollback'); END;").unwrap();
    assert!(store.invalidate_endpoint_evidence(&endpoint).is_err());
    assert_eq!(exposure(&store.db, &wallets[0].id).unwrap(), U256::ZERO);
    assert_eq!(generation(&store.db, "wallet", &wallets[0].id).unwrap(), 0);
    assert_eq!(generation(&store.db, "endpoint", &endpoint).unwrap(), 0);
    store
        .db
        .execute_batch("DROP TRIGGER reject_endpoint_dispute;")
        .unwrap();
    store
        .record_base_failure(
            &crate::rotation::base::EndpointMismatch {
                endpoint: endpoint.clone(),
            }
            .into(),
        )
        .unwrap();
    for (index, capture) in captures.iter().enumerate() {
        let result = check_capture(&store.db, store.evidence_namespace, capture, &other, 0);
        assert_eq!(
            result.is_ok(),
            index == 1,
            "other-endpoint cache on affected wallet must be stale"
        );
    }
    assert_eq!(exposure(&store.db, &wallets[0].id).unwrap(), U256::from(2));
    assert_eq!(exposure(&store.db, &wallets[1].id).unwrap(), U256::ZERO);
    assert_eq!(exposure(&store.db, &wallets[2].id).unwrap(), U256::from(1));
    store.invalidate_endpoint_evidence(&endpoint).unwrap();
    assert_eq!(exposure(&store.db, &wallets[0].id).unwrap(), U256::from(2));
    drop(store);
    let store = Store::open(&state, &key, &treasury).unwrap();
    for (index, expected) in [
        Some("chain_repair_required"),
        None,
        Some("evidence_repair_required"),
        Some("evidence_repair_required"),
    ]
    .into_iter()
    .enumerate()
    {
        let repair: Option<String> = store
            .db
            .query_row(
                "SELECT evidence_repair FROM wallets WHERE id=?1",
                [&wallets[index].id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(repair.as_deref(), expected);
        assert_eq!(
            checkpoint(&store.db, &wallets[index].id).unwrap(),
            Some(Anchor {
                height: 80,
                hash: "original".into()
            })
        );
    }
    assert_eq!(exposure(&store.db, &wallets[0].id).unwrap(), U256::from(2));
    assert!(store.capture_payment_evidence(&wallets[0].id).is_ok());
    assert!(
        store
            .capture_payment_evidence(&wallets[2].id)
            .unwrap()
            .capacity_snapshot
            .is_none()
    );
    let original: String = store
        .db
        .query_row(
            "SELECT support FROM payment_resolutions WHERE attempt_id='affected'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        original,
        serde_json::json!({"endpoint":endpoint}).to_string()
    );
}
