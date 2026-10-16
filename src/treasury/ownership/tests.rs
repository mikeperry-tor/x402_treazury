use super::*;

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
async fn fixture() -> (tempfile::TempDir, Treasury, String) {
    let dir = tempfile::tempdir().unwrap();
    let treasury = Treasury::create(
        dir.path().join("state"),
        dir.path().join("key"),
        2_000_000,
        Some(Zeroizing::new(SEED.into())),
    )
    .await
    .unwrap();
    crate::rotation::test_capacity::treasury_pair(
        &treasury,
        "ownership",
        "5",
        Some(alloy_primitives::U256::from(1_000_000u64)),
        0,
    )
    .await
    .unwrap();
    let job = treasury.status().await.unwrap().funding_jobs[0].id.clone();
    (dir, treasury, job)
}

#[tokio::test]
async fn refund_mutation_failure_reconstructs_without_stopping_shared_store() {
    let (dir, mut treasury, job) = fixture().await;
    let base = treasury.store_handle();
    let before = treasury.addresses().await.unwrap();
    let revision = treasury.revision;
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_refund BEFORE INSERT ON funding_refunds BEGIN SELECT RAISE(ABORT,'fixture refund failure'); END;").unwrap();
    assert!(treasury.refund_address(job.clone()).await.is_err());
    assert!(!treasury.healthy);
    db.execute_batch("DROP TRIGGER reject_refund;").unwrap();
    treasury
        .reconstruct_live(&CancellationToken::new())
        .await
        .unwrap();
    assert!(treasury.healthy);
    assert_eq!(treasury.revision, revision);
    assert_eq!(treasury.addresses().await.unwrap(), before);
    assert_eq!(base.call(|s| Ok(s.snapshot()?.0)).await.unwrap(), revision);
    let address = treasury.refund_address(job.clone()).await.unwrap();
    treasury.healthy = false;
    treasury
        .reconstruct_live(&CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(treasury.refund_address(job).await.unwrap(), address);
    drop(base);
    treasury.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_drain_retains_sync_owner_until_accepted_store_work_settles() {
    let (_dir, mut treasury, _job) = fixture().await;
    let base = treasury.store_handle();
    let (release, wait) = std::sync::mpsc::channel();
    let (started, entered) = tokio::sync::oneshot::channel();
    let store = treasury.store.clone();
    let revision = treasury.revision;
    let bytes = snapshot(treasury.client.as_ref().unwrap()).await.unwrap();
    treasury.healthy = false;
    // Exercise the actual retained blocking owner and Store queue boundary. A
    // scan may finish its accepted checkpoint after its original waiter left.
    treasury.sync_owner = Some(SyncOwner {
        cancel: CancellationToken::new(),
        join: tokio::task::spawn_blocking(move || {
            started.send(()).unwrap();
            wait.recv().unwrap();
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(store.call(move |s| s.save_snapshot(revision, &bytes)))
                .unwrap();
            Err(anyhow::anyhow!("fixture scan ended"))
        }),
    });
    entered.await.unwrap();
    let stop = CancellationToken::new();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), treasury.reconstruct_live(&stop))
            .await
            .is_err()
    );
    assert!(treasury.sync_owner.is_some());
    assert_eq!(base.call(|s| Ok(s.snapshot()?.0)).await.unwrap(), revision);
    release.send(()).unwrap();
    treasury.reconstruct_live(&stop).await.unwrap();
    assert!(treasury.sync_owner.is_none());
    assert_eq!(treasury.revision, revision + 1);
    assert!(treasury.healthy);
    drop(base);
    treasury.close().await.unwrap();
}

#[tokio::test]
async fn shutdown_during_drain_cannot_publish_replacement_sdk() {
    let (_dir, mut treasury, _job) = fixture().await;
    let stop = CancellationToken::new();
    stop.cancel();
    assert!(treasury.reconstruct_live(&stop).await.is_err());
    assert!(!treasury.healthy);
    assert!(treasury.client.is_none());
    assert!(treasury.store.call(|s| s.status()).await.is_ok());
    treasury.close().await.unwrap();
}

#[tokio::test]
async fn committed_refund_survives_lost_acknowledgement_and_reconstruction() {
    let (_dir, mut treasury, job) = fixture().await;
    let client = treasury.client.as_mut().unwrap();
    let (_, address) = client
        .generate_transparent_address(zip32::AccountId::ZERO, false)
        .await
        .unwrap();
    let address =
        zcash_keys::address::Address::Transparent(address).encode(&treasury.network.chain());
    let bytes = snapshot(client).await.unwrap();
    let revision = treasury.revision;
    treasury.healthy = false;
    let (accepted, receipt) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let store = treasury.store.clone();
    let saved_job = job.clone();
    let saved_address = address.clone();
    let waiter = tokio::spawn(async move {
        store
            .call(move |s| {
                let result = s.save_refund_address(&saved_job, &saved_address, revision, &bytes);
                accepted.send(()).unwrap();
                wait.recv().unwrap();
                result
            })
            .await
    });
    receipt.await.unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    treasury
        .reconstruct_live(&CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(treasury.revision, revision + 1);
    assert_eq!(treasury.refund_address(job).await.unwrap(), address);
    treasury.close().await.unwrap();
}

#[tokio::test]
async fn drained_owner_closes_only_exact_uncommitted_preparation_contract() {
    let (dir, mut treasury, _job) = fixture().await;
    let revision = treasury.revision;
    let owner = treasury
        .store
        .call(move |s| {
            let owner =
                s.begin_owned_preparation(&uuid::Uuid::new_v4().to_string(), revision, None)?;
            s.require_owned_reservation(&owner)?;
            s.reserve(owner.operation(), None, 1, 80)?;
            Ok(owner)
        })
        .await
        .unwrap();
    treasury.healthy = false;
    treasury
        .reconstruct_live(&CancellationToken::new())
        .await
        .unwrap();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    assert!(
        db.query_row(
            "SELECT closed FROM treasury_preparations WHERE operation_id=?1",
            [owner.operation()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM budget_entries WHERE id=?1",
            [owner.operation()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(treasury.revision, revision);
    treasury.close().await.unwrap();
}

#[tokio::test]
async fn exclusive_reopen_closes_known_precommit_but_preserves_unknown_contract() {
    let (dir, treasury, _job) = fixture().await;
    let revision = treasury.revision;
    let id = treasury.status().await.unwrap().treasury_id;
    let owner = treasury
        .store
        .call(move |s| {
            let owner =
                s.begin_owned_preparation(&uuid::Uuid::new_v4().to_string(), revision, None)?;
            s.reserve(owner.operation(), None, 1, 80)?;
            Ok(owner)
        })
        .await
        .unwrap();
    let unknown = uuid::Uuid::new_v4().to_string();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    db.execute("INSERT INTO treasury_preparations(operation_id,base_revision,contract_version,token) VALUES(?1,?2,999,?3)",rusqlite::params![unknown,revision,uuid::Uuid::new_v4().to_string()]).unwrap();
    treasury.close().await.unwrap();
    let reopened = Treasury::open(dir.path().join("state"), dir.path().join("key"), id)
        .await
        .unwrap();
    assert!(
        db.query_row(
            "SELECT closed FROM treasury_preparations WHERE operation_id=?1",
            [owner.operation()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    assert!(
        !db.query_row(
            "SELECT closed FROM treasury_preparations WHERE operation_id=?1",
            [unknown],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    assert_eq!(reopened.revision, revision);
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_real_sync_waiter_is_drained_before_live_reconstruction() {
    let (_dir, mut treasury, _job) = fixture().await;
    let requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let notify = requested.clone();
    let app = axum::Router::new().fallback(move || {
        let notify = notify.clone();
        async move {
            notify.notify_one();
            std::future::pending::<axum::http::StatusCode>().await
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    treasury.configure_sync(SyncSettings::new(endpoint, 1, 300).unwrap());
    let base = treasury.store_handle();
    let stop = CancellationToken::new();
    let mut scan = Box::pin(treasury.sync_once(&stop));
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            result = &mut scan => panic!("scan ended before stalled fixture: {result:?}"),
            _ = requested.notified() => {}
        }
    })
    .await
    .unwrap();
    drop(scan);
    assert!(treasury.sync_owner.is_some());
    assert!(treasury.client.is_none());
    tokio::time::timeout(Duration::from_secs(5), treasury.reconstruct_live(&stop))
        .await
        .unwrap()
        .unwrap();
    assert!(treasury.sync_owner.is_none());
    assert!(treasury.healthy);
    assert!(base.call(|s| s.status()).await.is_ok());
    server.abort();
    drop(base);
    treasury.close().await.unwrap();
}

struct NoSubmission;
impl crate::rotation::transaction::TransactionSubmission for NoSubmission {
    async fn submit(
        &mut self,
        _: crate::rotation::transaction::BroadcastTransaction,
    ) -> Result<crate::rotation::transaction::SubmissionOutcome> {
        panic!("reconstruction must not submit")
    }
    async fn lookup(
        &mut self,
        _: &crate::rotation::transaction::PreparedTransaction,
    ) -> Result<crate::rotation::transaction::TransactionPresence> {
        panic!("refund address recovery must not look up source bytes")
    }
}

#[tokio::test]
async fn command_owner_recovers_without_replaying_failed_command_or_stopping_base() {
    let (dir, mut treasury, job) = fixture().await;
    treasury.configure_sync(SyncSettings::new("http://127.0.0.1:1".into(), 1, 300).unwrap());
    let base = treasury.store_handle();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_refund BEFORE INSERT ON funding_refunds BEGIN SELECT RAISE(ABORT,'fixture refund failure'); END;").unwrap();
    let (handle, commands) = super::super::actor::channel();
    let stop = CancellationToken::new();
    let first_handle = handle.clone();
    let first_job = job.clone();
    let first = tokio::spawn(async move { first_handle.refund_address(first_job).await });
    tokio::task::yield_now().await;
    let actor = tokio::spawn(treasury.run_commands(commands, NoSubmission, stop.clone()));
    assert!(
        tokio::time::timeout(Duration::from_secs(5), first)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    // Failure belongs to its caller. There is no implicit repeat after recovery.
    assert!(
        base.call({
            let job = job.clone();
            move |s| s.refund_address(&job)
        })
        .await
        .unwrap()
        .is_none()
    );
    db.execute_batch("DROP TRIGGER reject_refund;").unwrap();
    let address = tokio::time::timeout(Duration::from_secs(5), handle.refund_address(job.clone()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        base.call(move |s| s.refund_address(&job)).await.unwrap(),
        Some(address)
    );
    assert!(!stop.is_cancelled());
    stop.cancel();
    drop(base);
    drop(handle);
    actor.await.unwrap().unwrap();
}

#[tokio::test]
async fn retired_handoff_pages_physical_unknown_prefix_before_eligible_owner() {
    let (dir, mut treasury, _job) = fixture().await;
    let revision = treasury.revision;
    let owner = treasury
        .store
        .call(move |s| s.begin_owned_preparation("z-eligible", revision, None))
        .await
        .unwrap();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    for i in 0..130 {
        db.execute("INSERT INTO treasury_preparations(operation_id,base_revision,contract_version,token) VALUES(?1,?2,999,?3)",rusqlite::params![format!("a-{i:03}"),revision,uuid::Uuid::new_v4().to_string()]).unwrap();
    }
    let retired = treasury.retire_sdk().await.unwrap();
    let first = treasury
        .store
        .call(move |s| s.close_retired_preparations_page(&retired, String::new()))
        .await
        .unwrap();
    assert_eq!(first.as_deref(), Some("a-063"));
    assert!(
        !db.query_row(
            "SELECT closed FROM treasury_preparations WHERE operation_id=?1",
            [owner.operation()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    treasury
        .reconstruct_live(&CancellationToken::new())
        .await
        .unwrap();
    assert!(
        db.query_row(
            "SELECT closed FROM treasury_preparations WHERE operation_id=?1",
            [owner.operation()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    treasury.close().await.unwrap();
}
