use super::*;

// Capture exact logical rows, including encrypted accepted bytes, rather than
// comparing SQLite files whose coordination metadata can legitimately change.
fn contents(db: &Connection) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    let tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let mut query = db
                .prepare(&format!("SELECT * FROM \"{table}\" ORDER BY rowid"))
                .unwrap();
            let columns = query.column_count();
            let rows = query
                .query_map([], |r| (0..columns).map(|i| r.get(i)).collect())
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            (table, rows)
        })
        .collect()
}

#[test]
fn rename_preserves_all_owned_work_and_current_name_funding_lookup() {
    for enabled in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("wallet");
        let key = temp.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"not an SDK wallet").unwrap();
        let treasury = store.id().to_owned();
        let reserves = test_support::reserves("2");
        let pool =
            test_support::declare_two_reserves(&mut store, "old", "2", Some(U256::from(123)), 7)
                .unwrap();
        let jobs = store.funding_jobs().unwrap();
        let job = &jobs[0];
        let secret = store.wallet_secret(&pool, &job.wallet_id).unwrap();
        let quote = serde_json::to_vec(&serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient},"fixture":"accepted original"})).unwrap();
        store.save_funding_quote(&job.id, &quote).unwrap();
        store
            .accept_preparation(&job.operation_id, Some(&pool), 10, None)
            .unwrap();
        store
            .prepare(
                &job.operation_id,
                store.snapshot().unwrap().0,
                b"accepted snapshot",
                b"exact signed bytes",
            )
            .unwrap();
        store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES ('liability',?1,?2,0,'1','original','POSSIBLY_SUBMITTED')", params![pool,job.wallet_id]).unwrap();
        // Positive observation consumes the other wallet's initial-funding claim.
        store
            .record_credit(&jobs[1].wallet_id, "1", "saved", 2)
            .unwrap();
        if !enabled {
            store.disable_pool("old").unwrap();
        }
        let before = contents(&store.db);
        assert_eq!(
            store
                .rename_capacity_group("old", "auto_v1_server_new")
                .unwrap(),
            pool
        );
        let mut expected = before.clone();
        for (table, rows) in &mut expected {
            if table == "pools" {
                rows[0][1] = "auto_v1_server_new".to_owned().into();
            }
        }
        assert_eq!(contents(&store.db), expected);
        drop(store);
        let mut store = Store::open(&state, &key, &treasury).unwrap();
        assert_eq!(contents(&store.db), expected);
        assert_eq!(
            *store.wallet_secret(&pool, &job.wallet_id).unwrap(),
            *secret
        );
        assert_eq!(
            &*store.prepared_bytes(&job.operation_id).unwrap(),
            b"exact signed bytes"
        );
        assert_eq!(
            store.funding_job(&job.id).unwrap().unwrap().pool_name,
            "auto_v1_server_new"
        );
        store
            .configure_profiles(&["auto_v1_server_new".into()].into(), &Default::default())
            .unwrap();
        let same = store
            .ensure_capacity_group("auto_v1_server_new", &reserves, Some(U256::from(123)), 7)
            .unwrap();
        assert_eq!(same, pool);
        assert_eq!(
            store
                .startup_groups(&["auto_v1_server_new".into()])
                .unwrap()
                .len(),
            1
        );
        assert_eq!(store.funding_jobs().unwrap().len(), 2);
        assert_eq!(
            store
                .rename_capacity_group("auto_v1_server_new", "explicit")
                .unwrap(),
            pool
        );
    }
}

#[test]
fn rename_rejections_and_transaction_failure_are_atomic() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("wallet");
    let key = temp.path().join("key");
    let mut store = Store::create(&state, &key, 1, b"fixture").unwrap();
    let pool = store
        .ensure_capacity_group("old", &test_support::reserves("2"), None, 0)
        .unwrap();
    store
        .ensure_capacity_group("occupied", &test_support::reserves("2"), None, 0)
        .unwrap();
    let before = contents(&store.db);
    for (old, new) in [
        ("missing", "new"),
        ("missing", "missing"),
        ("old", "occupied"),
        ("old", ""),
        ("old", "Bad"),
        ("old", "a-b"),
        ("Bad", "new"),
    ] {
        assert!(
            store.rename_capacity_group(old, new).is_err(),
            "{old} -> {new}"
        );
        assert_eq!(contents(&store.db), before);
    }
    assert_eq!(store.rename_capacity_group("old", "old").unwrap(), pool);
    assert_eq!(contents(&store.db), before);
    store.db.execute_batch("CREATE TRIGGER reject_rename BEFORE UPDATE OF name ON pools BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(store.rename_capacity_group("old", "new").is_err());
    assert_eq!(contents(&store.db), before);
    assert!(Store::open(&state, &key, store.id()).is_err());
}
