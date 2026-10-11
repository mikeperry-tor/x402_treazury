//! One resumable reconciliation owner per store/group. Runtime cursors enumerate
//! committed work; only separately verified SQL commits resolve liabilities.
use super::*;
use crate::rotation::{base::PendingAuthorization, store::history::PAGE_SIZE};
use futures_util::{FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use std::collections::{BTreeSet, VecDeque};
use std::time::Duration;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

// Bounds owned read futures and queued acquired rows, never financial capacity.
const BALANCE_READS: usize = 4;
const HISTORY_READS: usize = 4;
const CADENCE: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(crate) struct ReconciliationInProgress;
impl std::fmt::Display for ReconciliationInProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("reconciliation_already_in_progress")
    }
}
impl std::error::Error for ReconciliationInProgress {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Balance,
    History,
    Qualification,
}
struct Completion {
    kind: Kind,
    id: String,
    result: Result<()>,
}
struct Pass<T> {
    through: Option<i64>,
    cursor: i64,
    queued: VecDeque<(i64, T)>,
    due: Instant,
    completed: bool,
}
impl<T> Pass<T> {
    fn new(cursor: i64) -> Self {
        Self {
            through: None,
            cursor,
            queued: VecDeque::new(),
            due: Instant::now(),
            completed: false,
        }
    }
    fn finish(&mut self, delay: Duration) {
        self.through = None;
        self.due = Instant::now() + delay;
        self.completed = true;
    }
}
fn history_delay(failures: u32) -> Duration {
    Duration::from_secs(if failures == 0 {
        5
    } else {
        15u64.saturating_mul(1u64 << failures.min(5)).min(300)
    })
}

impl ManagedPool {
    /// A direct caller is a response waiter, not the owner of accepted read work.
    /// Dropping this waiter leaves its bounded pass running and publishing proofs.
    pub async fn reconcile(&self) -> Result<()> {
        let owner = self
            .reconciliation
            .clone()
            .try_lock_owned()
            .map_err(|_| ReconciliationInProgress)?;
        let pool = self.clone();
        tokio::spawn(async move {
            let _owner = owner;
            pool.maintenance(CancellationToken::new(), true).await
        })
        .await
        .context("reconciliation owner stopped")?
    }

    /// Deployment shutdown owns cancellation; unsigned acquired work is reaped
    /// while accepted store jobs retain their guards through commit acknowledgement.
    pub(crate) async fn run_reconciliation(&self, stop: CancellationToken) -> Result<()> {
        let _owner = tokio::select! {
            _=stop.cancelled()=>return Ok(()),
            owner=self.reconciliation.clone().lock_owned()=>owner,
        };
        self.maintenance(stop, false).await
    }

    async fn maintenance(&self, stop: CancellationToken, one_pass: bool) -> Result<()> {
        let mut balances = Pass::<String>::new(-1);
        let mut history = Pass::<PendingAuthorization>::new(0);
        let mut priority = VecDeque::<String>::new();
        let mut priority_ids = BTreeSet::new();
        let mut priority_due = Instant::now();
        let mut qualification_due = Instant::now();
        let mut qualification_active = false;
        let mut qualification_completed = false;
        let mut priority_initialized = false;
        let mut balance_active = BTreeSet::new();
        let mut history_active = BTreeSet::new();
        let mut failures = 0u32;
        let mut reads = FuturesUnordered::<BoxFuture<'_, Completion>>::new();
        let mut first_error = None;
        loop {
            if stop.is_cancelled() {
                break;
            }
            let instant = Instant::now();
            if instant >= priority_due && (!one_pass || !priority_initialized) {
                priority_initialized = true;
                let pool = self.pool.clone();
                let rows = self
                    .store
                    .call(move |s| {
                        let upper = s.wallet_boundary(&pool)?;
                        s.priority_wallets(&pool, upper)
                    })
                    .await?;
                priority_ids = rows.iter().map(|(_, id, _)| id.clone()).collect();
                priority = rows.into_iter().map(|(_, id, _)| id).collect();
                priority_due = instant + CADENCE;
            }
            // At most three preferred identities leave room for retained traversal.
            while balance_active.len() < BALANCE_READS {
                let next = if let Some(id) = priority.pop_front() {
                    Some(id)
                } else {
                    self.next_balance(&mut balances, one_pass, &priority_ids)
                        .await?
                };
                let Some(id) = next else { break };
                if balance_active.insert(id.clone()) {
                    reads.push(self.read_one(Kind::Balance, id, None).boxed());
                }
            }
            while history_active.len() < HISTORY_READS {
                let Some(auth) = self.next_history(&mut history, one_pass, failures).await? else {
                    break;
                };
                if history_active.insert(auth.id.clone()) {
                    reads.push(
                        self.read_one(Kind::History, auth.id.clone(), Some(auth))
                            .boxed(),
                    );
                }
            }
            if !qualification_active
                && instant >= qualification_due
                && (!one_pass || !qualification_completed)
            {
                qualification_active = true;
                reads.push(
                    async {
                        Completion {
                            kind: Kind::Qualification,
                            id: String::new(),
                            result: self.qualification_observation().await,
                        }
                    }
                    .boxed(),
                );
            }
            if one_pass && balances.completed && history.completed && reads.is_empty() {
                return first_error.map_or(Ok(()), Err);
            }
            let wake = priority_due
                .min(qualification_due)
                .min(balances.due)
                .min(history.due);
            // Active passes may have an old due time. They are driven by
            // completions; the cadence prevents an empty/all-owned busy loop.
            let wake = if wake <= Instant::now() {
                Instant::now() + CADENCE
            } else {
                wake
            };
            tokio::select! {
                _=stop.cancelled()=>break,
                Some(done)=reads.next(),if !reads.is_empty()=>{
                    match done.kind {
                        Kind::Balance=>{balance_active.remove(&done.id);},
                        Kind::History=>{
                            history_active.remove(&done.id);
                            failures=if done.result.is_err(){failures.saturating_add(1)}else{0};
                            if done.result.is_err() && history.through.is_none() {history.due=Instant::now()+history_delay(failures);}
                        },
                        Kind::Qualification=>{qualification_active=false;qualification_completed=true;qualification_due=Instant::now()+CADENCE;},
                    }
                    if let Err(error)=done.result {
                        let kind=match done.kind {Kind::Balance=>"balance",Kind::History=>"authorization_history",Kind::Qualification=>"qualification"};
                        tracing::warn!(kind,category=%super::super::base::safe_diagnostic(&error),"background observation unavailable; financial obligations remain owned");
                        if first_error.is_none(){first_error=Some(error);}
                    }
                },
                _=tokio::time::sleep_until(wake)=>{},
            }
        }
        // Dropping only unsigned query futures cannot release their liabilities.
        // Accepted SQL closures retain their group guard in StoreHandle.
        drop(reads);
        Ok(())
    }

    async fn next_balance(
        &self,
        pass: &mut Pass<String>,
        one_pass: bool,
        priority: &BTreeSet<String>,
    ) -> Result<Option<String>> {
        loop {
            if let Some((sequence, id)) = pass.queued.pop_front() {
                pass.cursor = sequence;
                if !priority.contains(&id) {
                    return Ok(Some(id));
                }
                continue;
            }
            if pass.through.is_none() {
                if (one_pass && pass.completed) || Instant::now() < pass.due {
                    return Ok(None);
                }
                let pool = self.pool.clone();
                pass.through = Some(self.store.call(move |s| s.wallet_boundary(&pool)).await?);
                pass.cursor = -1;
            }
            let pool = self.pool.clone();
            let (after, through) = (pass.cursor, pass.through.unwrap());
            let page = self
                .store
                .call(move |s| s.wallet_page(&pool, after, through, PAGE_SIZE))
                .await?;
            if page.is_empty() {
                pass.finish(CADENCE);
                return Ok(None);
            }
            pass.queued = page
                .into_iter()
                .map(|(sequence, id, _)| (sequence, id))
                .collect();
        }
    }
    async fn next_history(
        &self,
        pass: &mut Pass<PendingAuthorization>,
        one_pass: bool,
        failures: u32,
    ) -> Result<Option<PendingAuthorization>> {
        if let Some((sequence, auth)) = pass.queued.pop_front() {
            pass.cursor = sequence;
            return Ok(Some(auth));
        }
        if pass.through.is_none() {
            if (one_pass && pass.completed) || Instant::now() < pass.due {
                return Ok(None);
            }
            let pool = self.pool.clone();
            pass.through = Some(self.store.call(move |s| s.history_boundary(&pool)).await?);
            pass.cursor = 0;
        }
        let pool = self.pool.clone();
        let (after, through) = (pass.cursor, pass.through.unwrap());
        let page = self
            .store
            .call(move |s| s.history_page(&pool, after, through, PAGE_SIZE))
            .await?;
        if page.is_empty() {
            pass.finish(history_delay(failures));
            return Ok(None);
        }
        pass.queued = page.into();
        let (sequence, auth) = pass.queued.pop_front().expect("nonempty page");
        pass.cursor = sequence;
        Ok(Some(auth))
    }

    async fn read_one(
        &self,
        kind: Kind,
        id: String,
        auth: Option<PendingAuthorization>,
    ) -> Completion {
        let result = async {
            let guard = Arc::new(self.gate.clone().lock_owned().await);
            let pool = self.pool.clone();
            let wallets = if kind == Kind::Balance {
                vec![id.clone()]
            } else {
                vec![]
            };
            let query = self
                .store
                .call_with_guard(guard.clone(), move |s| {
                    s.scoped_chain_query(&pool, wallets, auth.into_iter().collect())
                })
                .await?;
            drop(guard);
            let mut view = self.acquire_view(query, false, None).await?;
            let guard = Arc::new(self.gate.clone().lock_owned().await);
            let pool = self.pool.clone();
            let completed = view.clone();
            let result = self
                .store
                .call_with_guard(guard, move |s| s.publish_scoped_evidence(&pool, completed))
                .await;
            let Err(error) = result else { return Ok(()) };
            use crate::rotation::base::evidence::EvidenceFailure;
            let recertified = match kind {
                Kind::History
                    if matches!(
                        error.downcast_ref::<EvidenceFailure>(),
                        Some(EvidenceFailure::CompatibilityRequired | EvidenceFailure::Invalidated)
                    ) =>
                {
                    let Some(certificate) = view.resolution_certificates.get(&id).cloned() else {
                        return Err(error);
                    };
                    let wallet = certificate.authorization().wallet.clone();
                    let capture = self
                        .store
                        .call(move |s| s.capture_evidence(&wallet))
                        .await?;
                    self.base
                        .recheck_resolution(certificate, capture)
                        .await
                        .map(|certificate| {
                            view.resolution_certificates.insert(id.clone(), certificate);
                        })
                }
                Kind::Balance
                    if error.downcast_ref::<EvidenceFailure>()
                        == Some(&EvidenceFailure::CompatibilityRequired) =>
                {
                    let Some(certificate) = view.balance_certificates.get(&id).cloned() else {
                        return Err(error);
                    };
                    let wallet = id.clone();
                    let capture = self
                        .store
                        .call(move |s| s.capture_evidence(&wallet))
                        .await?;
                    self.base
                        .recheck_balance(certificate, capture)
                        .await
                        .map(|certificate| {
                            view.balance_certificates.insert(id.clone(), certificate);
                        })
                }
                _ => return Err(error),
            };
            // One exact-anchor recertification, never a balance/nonce reread or
            // lifetime renewal. Another race or a USED floor defers to normal work.
            if let Err(error) = recertified {
                let guard = Arc::new(self.gate.clone().lock_owned().await);
                return self
                    .store
                    .call_with_guard(guard, move |s| {
                        s.record_base_failure(&error)?;
                        Err(error)
                    })
                    .await;
            }
            let guard = Arc::new(self.gate.clone().lock_owned().await);
            let pool = self.pool.clone();
            self.store
                .call_with_guard(guard, move |s| s.publish_scoped_evidence(&pool, view))
                .await
        }
        .await;
        Completion { kind, id, result }
    }

    async fn qualification_observation(&self) -> Result<()> {
        if let Some(observation) = crate::qualification::pool_observation(&self.pool).await {
            let pool = self.pool.clone();
            let through = self.store.call(move |s| s.wallet_boundary(&pool)).await?;
            let mut cursor = -1;
            let mut anchor = None;
            let mut confirmed = std::collections::BTreeMap::new();
            loop {
                let pool = self.pool.clone();
                let page = self
                    .store
                    .call(move |s| s.wallet_page(&pool, cursor, through, PAGE_SIZE))
                    .await?;
                if page.is_empty() {
                    break;
                }
                cursor = page.last().expect("nonempty page").0;
                let pool = self.pool.clone();
                let query = self
                    .store
                    .call(move |s| {
                        s.scoped_chain_query(
                            &pool,
                            page.into_iter().map(|(_, id, _)| id).collect(),
                            vec![],
                        )
                    })
                    .await?;
                let view = self.acquire_view(query, false, None).await?;
                ensure!(
                    anchor.as_ref().is_none_or(|saved| saved == &view.anchor),
                    "qualification_full_inventory_anchor_changed"
                );
                anchor = Some(view.anchor.clone());
                confirmed.extend(
                    view.balances
                        .iter()
                        .map(|(id, value)| (id.clone(), value.to_string())),
                );
                ensure!(
                    serde_json::to_vec(&confirmed)?.len() <= crate::qualification::LIMIT,
                    "qualification pool evidence exceeds byte limit"
                );
                let pool = self.pool.clone();
                let guard = Arc::new(self.gate.clone().lock_owned().await);
                self.store
                    .call_with_guard(guard, move |s| s.publish_scoped_evidence(&pool, view))
                    .await?;
            }
            let anchor = anchor.context("qualification pool has no observed wallets")?;
            let pool = self.pool.clone();
            let guard = Arc::new(self.gate.clone().lock_owned().await);
            self.store.call_with_guard(guard,move |s| {
                ensure!(s.wallet_boundary(&pool)?==through,"qualification inventory changed during observation");
                let evidence=serde_json::json!({"block_height":anchor.height,"block_hash":anchor.hash,"confirmed":confirmed,"unresolved":s.qualification_exposure(&pool)?});
                observation.record(s.qualification_lifecycle()?,evidence)
            }).await?;
        }
        crate::qualification::reconcile_debit(&self.base, &self.store, &self.pool).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Fixture {
        dir: tempfile::TempDir,
        pool: Arc<ManagedPool>,
        store: StoreHandle,
        worker: tokio::task::JoinHandle<()>,
        server: tokio::task::JoinHandle<()>,
        release: Arc<tokio::sync::Notify>,
        slow_started: Arc<tokio::sync::Notify>,
        balance_reads: Arc<AtomicUsize>,
        nonce_reads: Arc<AtomicUsize>,
        fail_nonces: Arc<AtomicBool>,
        treasury: String,
    }
    async fn fixture(count: usize) -> Fixture {
        use axum::{Json, Router, routing::post};
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let mut state = crate::rotation::store::Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let treasury = state.id().to_owned();
        let id = crate::rotation::test_capacity::declare_and_allocate!(
            state,
            "rolling",
            crate::rotation::test_capacity::small_reserve("5"),
            Some(crate::rotation::config::positive_usdc("5").unwrap()),
            0
        )
        .unwrap();
        for job in state.funding_jobs().unwrap() {
            state
                .record_credit(&job.wallet_id, "5000000", &format!("0x{:064x}", 88), 88)
                .unwrap();
        }
        let wallet = state.status().unwrap().pools.remove(0).addresses.remove(0);
        let mut db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        let tx = db.transaction().unwrap();
        for sequence in 1..=count {
            tx.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,payee,nonce,valid_after,valid_before,reconciliation_sequence) VALUES (?1,?2,?3,0,'1','fixture','POSSIBLY_SUBMITTED',?4,?4,?5,0,?6,?7)",rusqlite::params![format!("attempt-{sequence}"),id,wallet.id,wallet.address,format!("0x{sequence:064x}"),i64::try_from(now().unwrap()+1000).unwrap(),sequence as i64]).unwrap();
        }
        tx.execute(
            "UPDATE wallets SET unresolved_exposure=?2 WHERE id=?1",
            rusqlite::params![wallet.id, count.to_string()],
        )
        .unwrap();
        tx.commit().unwrap();
        drop(db);
        let release = Arc::new(tokio::sync::Notify::new());
        let slow_started = Arc::new(tokio::sync::Notify::new());
        let balance_reads = Arc::new(AtomicUsize::new(0));
        let nonce_reads = Arc::new(AtomicUsize::new(0));
        let fail_nonces = Arc::new(AtomicBool::new(false));
        let app=Router::new().route("/",post({
            let release=release.clone();let started=slow_started.clone();let balances=balance_reads.clone();let nonces=nonce_reads.clone();let fail=fail_nonces.clone();
            move |Json(request):Json<Value>| {
                let release=release.clone();let started=started.clone();let balances=balances.clone();let nonces=nonces.clone();let fail=fail.clone();
                async move {
                    let result=match request["method"].as_str().unwrap() {
                        "eth_chainId"=>json!("0x2105"),
                        "eth_getBlockByNumber"=>{
                            let tag=request["params"][0].as_str().unwrap();
                            let height=if tag=="latest"{100}else{u64::from_str_radix(&tag[2..],16).unwrap()};
                            json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{:x}",now().unwrap())})
                        },
                        "eth_call"=>{
                            let data=request["params"][0]["data"].as_str().unwrap();
                            let value=if data.starts_with("0x70a08231") {balances.fetch_add(1,Ordering::SeqCst);5_000_000}else{
                                nonces.fetch_add(1,Ordering::SeqCst);
                                let nonce=u64::from_str_radix(&data[data.len()-64..],16).unwrap();
                                if nonce==1 {started.notify_one();}
                                if fail.load(Ordering::SeqCst) {return Json(json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32602,"message":"fixture"}}));}
                                if nonce==1 {release.notified().await;}
                                1
                            };
                            json!(format!("0x{value:064x}"))
                        },
                        method=>panic!("unexpected mutation or unsupported fixture RPC: {method}"),
                    };
                    Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
                }
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = BaseRpc::new(
            &format!("http://{}/", listener.local_addr().unwrap()),
            12,
            120,
        )
        .unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (store, worker) = StoreHandle::spawn(state);
        let pool = Arc::new(
            ManagedPool::new(store.clone(), id, base, SpendPolicy::dollars("5").unwrap()).unwrap(),
        );
        Fixture {
            dir,
            pool,
            store,
            worker,
            server,
            release,
            slow_started,
            balance_reads,
            nonce_reads,
            fail_nonces,
            treasury,
        }
    }
    async fn unfinished(f: &Fixture) -> usize {
        let pool = f.pool.pool.clone();
        f.store
            .call(move |s| {
                let through = s.history_boundary(&pool)?;
                Ok(s.history_page(&pool, 0, through, PAGE_SIZE)?.len())
            })
            .await
            .unwrap()
    }
    async fn wait_for(f: &Fixture, remaining: usize) {
        tokio::time::timeout(Duration::from_secs(20), async {
            while unfinished(f).await != remaining {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn slow_history_keeps_rolling_across_pages_and_does_not_block_balance_cadence() {
        let f = fixture(PAGE_SIZE + 7).await;
        let stop = CancellationToken::new();
        let task = tokio::spawn({
            let pool = f.pool.clone();
            let stop = stop.clone();
            async move { pool.run_reconciliation(stop).await }
        });
        tokio::time::timeout(Duration::from_secs(5), f.slow_started.notified())
            .await
            .unwrap();
        wait_for(&f, 1).await;
        assert_eq!(f.nonce_reads.load(Ordering::SeqCst), PAGE_SIZE + 7);
        let before = f.balance_reads.load(Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(8), async {
            while f.balance_reads.load(Ordering::SeqCst) <= before {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(unfinished(&f).await, 1);
        assert_eq!(
            f.nonce_reads.load(Ordering::SeqCst),
            PAGE_SIZE + 7,
            "slow in-flight nonce must not redispatch across pass boundaries"
        );
        stop.cancel();
        task.await.unwrap().unwrap();
        let id = f.pool.pool.clone();
        drop(f.pool);
        drop(f.store);
        f.worker.await.unwrap();
        f.server.abort();
        let reopened = crate::rotation::store::Store::open(
            &f.dir.path().join("state"),
            &f.dir.path().join("key"),
            &f.treasury,
        )
        .unwrap();
        let remaining = reopened
            .history_page(&id, 0, reopened.history_boundary(&id).unwrap(), PAGE_SIZE)
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].1.id, "attempt-1");
    }

    #[tokio::test]
    async fn history_failure_backoff_does_not_suppress_balance_owner() {
        let f = fixture(1).await;
        f.fail_nonces.store(true, Ordering::SeqCst);
        let stop = CancellationToken::new();
        let task = tokio::spawn({
            let pool = f.pool.clone();
            let stop = stop.clone();
            async move { pool.run_reconciliation(stop).await }
        });
        tokio::time::timeout(Duration::from_secs(5), f.slow_started.notified())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let nonces = f.nonce_reads.load(Ordering::SeqCst);
        let balances = f.balance_reads.load(Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(8), async {
            while f.balance_reads.load(Ordering::SeqCst) <= balances {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(f.nonce_reads.load(Ordering::SeqCst), nonces);
        assert_eq!(unfinished(&f).await, 1);
        stop.cancel();
        task.await.unwrap().unwrap();
        drop(f.pool);
        drop(f.store);
        f.worker.await.unwrap();
        f.server.abort();
    }

    #[tokio::test]
    async fn cancelled_response_waiter_preserves_owned_history_and_duplicate_is_explicit() {
        let f = fixture(7).await;
        let waiter = tokio::spawn({
            let pool = f.pool.clone();
            async move { pool.reconcile().await }
        });
        tokio::time::timeout(Duration::from_secs(5), f.slow_started.notified())
            .await
            .unwrap();
        waiter.abort();
        wait_for(&f, 1).await;
        assert!(
            f.pool
                .reconcile()
                .await
                .unwrap_err()
                .downcast_ref::<ReconciliationInProgress>()
                .is_some()
        );
        f.release.notify_one();
        wait_for(&f, 0).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if f.pool.reconciliation.try_lock().is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(f.nonce_reads.load(Ordering::SeqCst), 7);
        drop(f.pool);
        drop(f.store);
        f.worker.await.unwrap();
        f.server.abort();
    }
    #[test]
    fn history_backoff_is_bounded_and_never_changes_balance_cadence() {
        assert_eq!(
            (0..7)
                .map(|n| history_delay(n).as_secs())
                .collect::<Vec<_>>(),
            vec![5, 30, 60, 120, 240, 300, 300]
        );
        assert_eq!(history_delay(u32::MAX).as_secs(), 300);
        assert_eq!(CADENCE.as_secs(), 5);
    }
}
