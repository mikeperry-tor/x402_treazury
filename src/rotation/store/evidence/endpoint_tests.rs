//! Durable endpoint distrust must survive cache eviction and process restart.
use super::*;

#[test]
fn adopted_balance_support_detects_same_hash_amount_conflict_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let mut store = Store::create(&state, &key, 1, b"fixture").unwrap();
    let treasury = store.id().to_owned();
    crate::rotation::store::test_support::declare_two_reserves(&mut store, "support", "1", None, 0)
        .unwrap();
    let wallet = store.status().unwrap().pools.remove(0).addresses.remove(0);
    let observation = BalanceObservation {
        wallet: wallet.id.clone(),
        address: wallet.address,
        network: "eip155:8453".into(),
        asset: crate::payment::USDC.into(),
        endpoint: "a".repeat(64),
        confirmations: 12,
        confirmed: crate::rotation::base::evidence::BalancePoint {
            anchor: Anchor {
                height: 100,
                hash: format!("0x{:064x}", 100),
            },
            amount: U256::from(20),
        },
        latest: crate::rotation::base::evidence::BalancePoint {
            anchor: Anchor {
                height: 112,
                hash: format!("0x{:064x}", 112),
            },
            amount: U256::from(20),
        },
        valid_until: 1,
    };
    let support = serde_json::to_string(&BalanceSupport {
        observation: observation.clone(),
        checkpoint: Some(observation.confirmed.anchor.clone()),
    })
    .unwrap();
    store.db.execute("INSERT INTO payment_resolution_repairs(id,wallet_id,verified_at,evidence_digest,resolution_count,confirmed_height,confirmed_hash,latest_height,latest_hash,endpoint,balance_support) VALUES('balance',?1,1,'fixture',1,100,?2,112,?3,?4,?5)",params![wallet.id,observation.confirmed.anchor.hash,observation.latest.anchor.hash,observation.endpoint,support]).unwrap();
    drop(store);
    let mut store = Store::open(&state, &key, &treasury).unwrap();
    // Age never removes an exact same-hash integrity dependency.
    store
        .check_journal_support_or_invalidate(&wallet.id, &observation)
        .unwrap();
    for latest in [false, true] {
        let mut changed = observation.clone();
        if latest {
            changed.latest.amount += U256::from(1);
        } else {
            changed.confirmed.amount += U256::from(1);
        }
        assert_eq!(
            store
                .check_journal_support_or_invalidate(&wallet.id, &changed)
                .unwrap_err()
                .downcast_ref::<EvidenceFailure>(),
            Some(&EvidenceFailure::AmountContradiction)
        );
        let flag: Option<String> = store
            .db
            .query_row(
                "SELECT evidence_repair FROM wallets WHERE id=?1",
                [&wallet.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            flag.is_none(),
            "an archival amount conflict cannot invent a financial checkpoint dependency"
        );
    }
    let saved: String = store
        .db
        .query_row(
            "SELECT balance_support FROM payment_resolution_repairs WHERE id='balance'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved, support);
}

#[test]
fn replacement_proofs_and_checkpoint_supply_effective_endpoint_dependencies() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let mut store = Store::create(&state, &key, 1, b"fixture").unwrap();
    let treasury = store.id().to_owned();
    let pool = crate::rotation::store::test_support::declare_two_reserves(
        &mut store,
        "replacement",
        "1",
        None,
        0,
    )
    .unwrap();
    let wallet = store.funding_jobs().unwrap()[0].wallet_id.clone();
    let original_endpoint = "a".repeat(64);
    let proof_endpoint = "b".repeat(64);
    let balance_endpoint = "c".repeat(64);
    let original = serde_json::json!({"endpoint":original_endpoint}).to_string();
    store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES('replaced',?1,?2,0,'7','fixture','RESOLVED')", params![pool,wallet]).unwrap();
    store
        .db
        .execute(
            "INSERT INTO payment_resolutions VALUES('replaced','USED',80,'original',1,?1)",
            [&original],
        )
        .unwrap();
    store.db.execute("INSERT INTO payment_resolution_repairs(id,wallet_id,verified_at,original_financial_height,original_financial_hash,original_debit_height,original_debit_hash,evidence_digest,resolution_count,confirmed_height,confirmed_hash,latest_height,latest_hash,endpoint,balance_support) VALUES('repair',?1,1,80,'original',80,'original','fixture',1,100,'current',112,'latest',?2,'fixture')",params![wallet,balance_endpoint]).unwrap();
    store.db.execute("INSERT INTO payment_resolution_replacements(repair_id,attempt_id,outcome,height,hash,block_time,support,endpoint) VALUES('repair','replaced','EXPIRED_UNUSED',90,'canonical',2,?1,?2)",params![serde_json::json!({"endpoint":proof_endpoint}).to_string(),proof_endpoint]).unwrap();
    assert_eq!(
        store.qualification_state().unwrap()["payment_resolutions"][0]["outcome"],
        "EXPIRED_UNUSED"
    );
    assert!(
        !payment_endpoint_dependents(&store.db, &original_endpoint)
            .unwrap()
            .contains_key(&wallet)
    );
    for endpoint in [&proof_endpoint, &balance_endpoint] {
        assert_eq!(
            payment_endpoint_dependents(&store.db, endpoint)
                .unwrap()
                .get(&wallet),
            Some(&false)
        );
    }
    store
        .invalidate_endpoint_evidence(&original_endpoint)
        .unwrap();
    assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::ZERO);
    store
        .invalidate_endpoint_evidence(&balance_endpoint)
        .unwrap();
    assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::from(7));
    assert_eq!(
        store.qualification_state().unwrap()["payment_resolutions"],
        serde_json::json!([])
    );
    drop(store);
    let store = Store::open(&state, &key, &treasury).unwrap();
    assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::from(7));
    let saved: (String, String) = store
        .db
        .query_row(
            "SELECT outcome,support FROM payment_resolutions WHERE attempt_id='replaced'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(saved, ("USED".into(), original));
    assert_eq!(
        store.qualification_state().unwrap()["payment_resolutions"],
        serde_json::json!([])
    );
}

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
        Some("chain_repair_required"),
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
            .is_some()
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
