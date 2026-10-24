//! Kill the actual signing process at financial boundaries; only loopback peers
//! and temporary, unfunded wallets participate. No destructor simulates recovery.
use super::*;
use crate::{payment::PaidClient, rotation::store::Store};
use futures_util::FutureExt;
use std::{path::Path, process::Stdio, time::Duration};

const CHILD: &str = "rotation::manager::admission_tests::crash_tests::payment_crash_child";

#[tokio::test]
#[ignore = "private subprocess fixture invoked by crash-boundary test"]
async fn payment_crash_child() {
    let root = std::env::var_os("X402_CRASH_FIXTURE_ROOT").expect("explicit fixture only");
    let root = Path::new(&root);
    let base = std::env::var("X402_CRASH_FIXTURE_URL").unwrap();
    let stage = std::env::var("X402_CRASH_FIXTURE_STAGE").unwrap();
    let id = crate::rotation::store::status(&root.join("state"))
        .unwrap()
        .treasury_id;
    let state = Store::open(&root.join("state"), &root.join("key"), &id).unwrap();
    let pool = state
        .status()
        .unwrap()
        .pools
        .into_iter()
        .find(|p| p.name == "advisory")
        .unwrap()
        .id;
    let (store, _worker) = StoreHandle::spawn(state);
    let mut manager = ManagedPool::new(
        store,
        pool,
        BaseRpc::new("http://127.0.0.1:1", 12).unwrap(),
        SpendPolicy::dollars("5").unwrap(),
    )
    .unwrap();
    let marker = root.join("boundary");
    manager.test_admission_hook = Some(Arc::new(move |at| {
        let marker = marker.clone();
        let stage = stage.clone();
        async move {
            if matches!(
                (&*stage, at),
                ("signed", AdmissionTestStage::Signed { .. })
                    | ("journaled", AdmissionTestStage::Journaled)
            ) {
                std::fs::write(marker, b"ready").unwrap();
                std::future::pending::<()>().await;
            }
        }
        .boxed()
    }));
    PaidClient::managed(Arc::new(manager))
        .execute(route(&base))
        .await
        .unwrap();
    panic!("parent must kill this process before response completion");
}

fn route(base: &str) -> crate::catalog::RoutedRequest {
    crate::catalog::RoutedRequest {
        headers: Default::default(),
        method: "GET".into(),
        url: format!("{base}/pay"),
        query: Default::default(),
        body: None,
    }
}

fn attempts(path: &Path) -> Vec<(String, String, String, String, String)> {
    let db = rusqlite::Connection::open(path).unwrap();
    db.prepare("SELECT id,wallet_id,nonce,amount,state FROM payment_attempts ORDER BY id")
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[tokio::test]
async fn process_loss_preserves_authorizations_without_replay_and_peer_can_pay() {
    for stage in ["signed", "journaled", "dispatched"] {
        let f = fixture_with_held_response(stage == "dispatched").await;
        f.db()
            .execute(
                "UPDATE wallets SET balance='20' WHERE id=?1",
                [&f.payer.wallet],
            )
            .unwrap();
        let id = f.pool.store.call(|s| Ok(s.id().to_owned())).await.unwrap();
        let payer_pool = f.pool.pool.clone();
        let peer_pool = f
            .pool
            .store
            .call(|s| {
                let pool = crate::rotation::test_capacity::declare_and_allocate!(
                    s,
                    "peer",
                    crate::rotation::test_capacity::two_reserves("5"),
                    Some(U256::from(100)),
                    0
                )?;
                for job in s
                    .funding_jobs()?
                    .into_iter()
                    .filter(|job| job.pool_id == pool)
                {
                    s.record_credit(&job.wallet_id, "5000000", "fixture", 100)?;
                }
                Ok(pool)
            })
            .await
            .unwrap();
        let Fixture {
            dir,
            pool,
            url,
            signed,
            received,
            release,
            worker,
            server,
            ..
        } = f;
        drop(pool);
        worker.await.unwrap();
        let base = url.strip_suffix("/pay").unwrap();
        let hold = stage == "dispatched";
        let path = dir.path().join("state/state.sqlite");
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", CHILD, "--nocapture"])
            .env("X402_CRASH_FIXTURE_ROOT", dir.path())
            .env("X402_CRASH_FIXTURE_URL", base)
            .env("X402_CRASH_FIXTURE_STAGE", stage)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(15), async {
            if hold {
                received.notified().await;
            } else {
                while !dir.path().join("boundary").exists() {
                    assert!(
                        child.try_wait().unwrap().is_none(),
                        "child exited before {stage}"
                    );
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        })
        .await
        .unwrap();
        child.kill().await.unwrap();
        let status = child.wait().await.unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        let saved = attempts(&path);
        assert_eq!(saved.len(), usize::from(stage != "signed"));
        assert_eq!(signed.load(Ordering::SeqCst), usize::from(hold));
        let state = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
        assert_eq!(attempts(&path), saved);
        let (store, worker) = StoreHandle::spawn(state);
        let manager = |pool: String| {
            Arc::new(
                ManagedPool::new(
                    store.clone(),
                    pool,
                    BaseRpc::new("http://127.0.0.1:1", 12).unwrap(),
                    SpendPolicy::dollars("5").unwrap(),
                )
                .unwrap(),
            )
        };
        let payer = manager(payer_pool.clone());
        assert!(
            payer.reconcile().await.is_err(),
            "fixture Base RPC is unavailable"
        );
        assert_eq!(
            attempts(&path),
            saved,
            "an unavailable balance check cannot release an authorization"
        );
        assert_eq!(
            signed.load(Ordering::SeqCst),
            usize::from(hold),
            "reopen/reconciliation must not replay"
        );
        let db = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM payment_admission_evidence", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            saved.len() as i64
        );
        if let Some((_, wallet, _, amount, state)) = saved.first() {
            assert_eq!(state, "POSSIBLY_SUBMITTED");
            assert_eq!(amount, "7");
            assert_eq!(
                db.query_row("SELECT balance FROM wallets WHERE id=?1", [wallet], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
                "13"
            );
        }
        release.add_permits(2);
        let peer = PaidClient::managed(manager(peer_pool.clone()));
        assert_eq!(peer.execute(route(base)).await.unwrap(), "paid");
        assert_eq!(signed.load(Ordering::SeqCst), usize::from(hold) + 1);
        assert_eq!(attempts(&path).len(), saved.len() + 1);
        peer.drain().await;
        drop(peer);
        drop(payer);
        drop(store);
        worker.await.unwrap();
        server.abort();
    }
}
