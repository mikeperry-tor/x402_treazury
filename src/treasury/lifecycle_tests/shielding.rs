//! Serving maintenance uses actual local-chain bytes; restart never grants a retry.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct ShieldingSender {
    inner: submission::GrpcSubmission,
    submissions: Arc<AtomicUsize>,
    pause: Option<StoreHandle>,
}
impl TransactionSubmission for ShieldingSender {
    async fn preflight(&mut self, prepared: &PreparedTransaction, retry: bool) -> Result<()> {
        self.inner.preflight(prepared, retry).await?;
        if let Some(store) = self.pause.take() {
            store
                .call(|s| {
                    s.pause_new_funding();
                    Ok(())
                })
                .await?;
        }
        Ok(())
    }
    async fn submit(&mut self, prepared: BroadcastTransaction) -> Result<SubmissionOutcome> {
        self.submissions.fetch_add(1, Ordering::SeqCst);
        self.inner.submit(prepared).await
    }
    async fn lookup(&mut self, prepared: &PreparedTransaction) -> Result<TransactionPresence> {
        self.inner.lookup(prepared).await
    }
}
fn sender(
    endpoint: &str,
    submissions: &Arc<AtomicUsize>,
    pause: Option<StoreHandle>,
) -> ShieldingSender {
    ShieldingSender {
        inner: submission::GrpcSubmission::with_network(
            endpoint.into(),
            endpoint.into(),
            TreasuryNetwork::Synthetic,
        )
        .unwrap(),
        submissions: submissions.clone(),
        pause,
    }
}
async fn wait_submission(store: &StoreHandle, operation: &str, expected: &'static str) {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let id = operation.to_owned();
            if store
                .call(move |s| Ok(s.operation(&id)?.submission))
                .await
                .unwrap()
                == expected
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actor published shielding observation");
}
#[tokio::test]
async fn serving_shielding_rechecks_intent_and_observes_after_disabled_restart() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("x402_treazury=debug")
        .with_test_writer()
        .try_init();
    let (dir, net, mut owner, endpoint) = funded_fixture(1).await;
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let pool = owner
        .ensure_capacity_group(
            "automatic_shield".into(),
            crate::rotation::test_capacity::small_reserve("5"),
            None,
            0,
        )
        .await
        .unwrap();
    owner
        .allocate_capacity_kind(pool, AllocationKind::Small, true)
        .await
        .unwrap();
    let store = owner.store_handle();
    let job = store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    let address = owner.refund_address(job.id).await.unwrap();
    let returned = faucet_funding_transaction(vec![(&address, 100_000, None)]).await;
    net.chain.write().await.mine_block(vec![returned]);
    let stop = CancellationToken::new();
    owner.sync_once(&stop).await.unwrap();
    let profiles = std::collections::BTreeMap::from([("automatic_shield".into(), 50_000)]);
    let captured = profiles.clone();
    let authority = store
        .call(move |s| {
            s.capture_serving_shielding(&captured, true)?
                .context("serving authority missing")
        })
        .await
        .unwrap();
    let mut maintenance = refunds::maintenance::ShieldingMaintenance::default();
    let prepared = owner
        .prepare_automatic_refund(&mut maintenance, &authority, &stop)
        .await
        .unwrap()
        .unwrap();
    let operation = prepared.operation_id().to_owned();
    let raw = prepared.bytes().to_vec();
    let fee = prepared.facts().unwrap().fee_zatoshis;
    let submissions = Arc::new(AtomicUsize::new(0));
    // The policy changes after successful network preflight, before durable intent.
    assert!(
        owner
            .submit_automatic_shielding(
                operation.clone(),
                authority.clone(),
                &mut sender(&endpoint, &submissions, Some(store.clone())),
                &stop
            )
            .await
            .is_err()
    );
    assert_eq!(submissions.load(Ordering::SeqCst), 0);
    let id = operation.clone();
    store
        .call(move |s| {
            assert_eq!(s.operation(&id)?.attempts, 0);
            Ok(())
        })
        .await
        .unwrap();
    let treasury_id = owner.status().await.unwrap().treasury_id;
    drop(store);
    owner.close().await.unwrap();

    let mut owner = Treasury::open_with_network(
        state.clone(),
        key.clone(),
        treasury_id.clone(),
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner.configure_sync(SyncSettings::new(endpoint.clone(), 1, 300).unwrap());
    let store = owner.store_handle();
    let id = operation.clone();
    store
        .call(move |s| {
            assert!(!s.automatic_shielding_submission_candidate(&id, &authority)?);
            Ok(())
        })
        .await
        .unwrap();
    let captured = profiles.clone();
    let authority = store
        .call(move |s| {
            s.capture_serving_shielding(&captured, true)?
                .context("serving authority missing")
        })
        .await
        .unwrap();
    let (handle, mut commands) = actor::channel();
    let attempted_authority = authority.clone();
    commands.install_serving_shielding(Some(authority));
    let actor_stop = CancellationToken::new();
    let task = tokio::spawn(owner.run_commands(
        commands,
        sender(&endpoint, &submissions, None),
        actor_stop.clone(),
    ));
    handle.sync().await.unwrap();
    wait_submission(&store, &operation, "BROADCAST").await;
    assert_eq!(submissions.load(Ordering::SeqCst), 1);
    let id = operation.clone();
    let saved = store
        .call(move |s| {
            assert_eq!(s.operation(&id)?.attempts, 1);
            assert!(
                BroadcastTransaction::request_automatic_shielding(
                    s,
                    &id,
                    now()?,
                    24,
                    &attempted_authority
                )
                .is_err()
            );
            assert_eq!(s.operation(&id)?.attempts, 1);
            Ok(s.prepared_bytes(&id)?.to_vec())
        })
        .await
        .unwrap();
    assert_eq!(saved, raw);
    actor_stop.cancel();
    drop(store);
    drop(handle);
    task.await.unwrap().unwrap();

    // A disabled profile supplies no new authority, but its original bytes still
    // need confirmation/accounting. Mining does not authorize another send.
    net.chain.write().await.mine_mempool();
    let db = rusqlite::Connection::open(state.join("state.sqlite")).unwrap();
    db.execute("UPDATE pools SET enabled=0", []).unwrap();
    drop(db);
    let mut owner =
        Treasury::open_with_network(state.clone(), key, treasury_id, TreasuryNetwork::Synthetic)
            .await
            .unwrap();
    owner.configure_sync(SyncSettings::new(endpoint.clone(), 1, 300).unwrap());
    let store = owner.store_handle();
    assert!(
        store
            .call(move |s| s.capture_serving_shielding(&profiles, true))
            .await
            .unwrap()
            .is_none()
    );
    let (handle, commands) = actor::channel();
    let actor_stop = CancellationToken::new();
    let task = tokio::spawn(owner.run_commands(
        commands,
        sender(&endpoint, &submissions, None),
        actor_stop.clone(),
    ));
    handle.sync().await.unwrap();
    wait_submission(&store, &operation, "CONFIRMED").await;
    assert_eq!(submissions.load(Ordering::SeqCst), 1);
    let id = operation.clone();
    store
        .call(move |s| {
            assert_eq!(s.operation(&id)?.attempts, 1);
            Ok(())
        })
        .await
        .unwrap();
    let db = rusqlite::Connection::open(state.join("state.sqlite")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM shielding_operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT reserved,consumed FROM budget_entries WHERE id=?1",
            [&operation],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        )
        .unwrap(),
        (0, fee as i64)
    );
    actor_stop.cancel();
    drop(store);
    drop(handle);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn serving_shielding_prepares_new_return_and_confirms_once() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("x402_treazury=debug")
        .with_test_writer()
        .try_init();
    let (dir, net, mut owner, endpoint) = funded_fixture(1).await;
    let baseline = owner
        .status()
        .await
        .unwrap()
        .sync
        .unwrap()
        .spendable_shielded_zatoshis;
    let treasury_id = owner.status().await.unwrap().treasury_id;
    let pool = owner
        .ensure_capacity_group(
            "fresh_shield".into(),
            crate::rotation::test_capacity::small_reserve("5"),
            None,
            0,
        )
        .await
        .unwrap();
    owner
        .allocate_capacity_kind(pool, AllocationKind::Small, true)
        .await
        .unwrap();
    let store = owner.store_handle();
    let job = store
        .call(|s| Ok(s.funding_jobs()?.remove(0)))
        .await
        .unwrap();
    let address = owner.refund_address(job.id).await.unwrap();
    let returned = faucet_funding_transaction(vec![(&address, 100_000, None)]).await;
    net.chain.write().await.mine_block(vec![returned]);
    // No proposal, operation header or prepared bytes exist before actor startup.
    let authority = store
        .call(|s| {
            assert!(s.shielding_operation_page("")?.is_empty());
            s.capture_serving_shielding(
                &std::collections::BTreeMap::from([("fresh_shield".into(), 50_000)]),
                true,
            )?
            .context("serving authority missing")
        })
        .await
        .unwrap();
    let submissions = Arc::new(AtomicUsize::new(0));
    let (handle, mut commands) = actor::channel();
    commands.install_serving_shielding(Some(authority));
    let stop = CancellationToken::new();
    let task = tokio::spawn(owner.run_commands(
        commands,
        sender(&endpoint, &submissions, None),
        stop.clone(),
    ));
    handle.sync().await.unwrap();
    let operation = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        loop {
            let ids = store
                .call(|s| s.shielding_operation_page(""))
                .await
                .unwrap();
            if let Some(id) = ids.into_iter().next() {
                break id;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actor prepares fresh owned return");
    wait_submission(&store, &operation, "BROADCAST").await;
    assert_eq!(submissions.load(Ordering::SeqCst), 1);
    net.chain.write().await.mine_mempool();
    handle.sync().await.unwrap();
    wait_submission(&store, &operation, "CONFIRMED").await;
    handle.sync().await.unwrap();
    let id = operation.clone();
    let fee = store
        .call(move |s| {
            assert_eq!(s.shielding_operation_page("")?, vec![id.clone()]);
            assert_eq!(s.operation(&id)?.attempts, 1);
            Ok(PreparedTransaction::load_observation(s, &id)?
                .facts()
                .context("shielding facts missing")?
                .fee_zatoshis)
        })
        .await
        .unwrap();
    assert_eq!(submissions.load(Ordering::SeqCst), 1);
    stop.cancel();
    drop(store);
    drop(handle);
    task.await.unwrap().unwrap();
    let owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        treasury_id,
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    assert_eq!(
        owner
            .status()
            .await
            .unwrap()
            .sync
            .unwrap()
            .spendable_shielded_zatoshis,
        baseline + 100_000 - fee
    );
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT reserved,consumed FROM budget_entries WHERE id=?1",
            [&operation],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        )
        .unwrap(),
        (0, fee as i64)
    );
    drop(db);
    owner.close().await.unwrap();
}
