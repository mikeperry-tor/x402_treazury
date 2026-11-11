//! Renewal scheduling and explicit execution modes, using unfunded local stores.
use super::*;
use crate::rotation::store::Store;
use std::time::Duration;

#[tokio::test(start_paused = true)]
async fn fractional_completion_preserves_poll_and_failure_floors() {
    let start = tokio::time::Instant::now();
    let clock = TickClock {
        epoch: 1000,
        started: start,
    };
    tokio::time::advance(Duration::from_millis(900)).await;
    let completed = tokio::time::Instant::now();
    assert_eq!(
        clock.now(),
        1000,
        "financial/report time still uses the original clock"
    );
    let mut state = State::default();
    let key = Key {
        operation: "op".into(),
        kind: Kind::Source,
    };
    assert_eq!(
        state.admit(
            &key,
            Some(&FundingPhase::Quoted),
            clock.now(),
            &BTreeSet::new()
        ),
        Admission::Ready
    );
    state.complete(
        ("", "op"),
        Kind::Source,
        clock.ceil_epoch(completed),
        RetrySchedule::Poll(5, false),
        Some("unavailable".into()),
        None,
    );
    let due = start + Duration::from_secs(state.records[&key].due - clock.epoch);
    assert!(due.duration_since(completed) >= Duration::from_secs(60));
    let inventory_due = start + Duration::from_secs(clock.after_poll(5) - clock.epoch);
    assert!(inventory_due.duration_since(completed) >= Duration::from_secs(5));
    // This is the same absolute wait used by the owner, now anchored outward.
    tokio::time::advance(Duration::from_secs(60)).await;
    assert!(tokio::time::Instant::now() < due);
    tokio::time::sleep_until(due).await;
    assert_eq!(
        state.admit(
            &key,
            Some(&FundingPhase::Quoted),
            clock.now(),
            &BTreeSet::new()
        ),
        Admission::Ready
    );
}

#[tokio::test(start_paused = true)]
async fn exhausted_once_lanes_wait_for_completion_without_spinning() {
    let start = tokio::time::Instant::now();
    let clock = TickClock {
        epoch: 1000,
        started: start,
    };
    let mut inventory = [Kind::Credit, Kind::Status, Kind::Source, Kind::Quote]
        .map(|kind| LaneInventory::new(kind, 0, 1005));
    for lane in &mut inventory {
        lane.exhausted = true;
    }
    tokio::time::advance(Duration::from_secs(60)).await;
    assert_eq!(
        next_wake(&inventory, &VecDeque::new(), None, &clock, None, true, None,),
        None
    );
    assert_eq!(
        next_wake(
            &inventory,
            &VecDeque::new(),
            Some(1065),
            &clock,
            None,
            true,
            None,
        ),
        Some(start + Duration::from_secs(65)),
        "retained quote metadata must wake even without a pending inventory page"
    );
    // Ordinary inventory retains its own discovery wake. A due but blocked
    // page is excluded, so unrelated completions do not create a busy loop.
    for lane in &mut inventory {
        lane.pass_due = 1065;
    }
    assert_eq!(
        next_wake(
            &inventory,
            &VecDeque::new(),
            None,
            &clock,
            None,
            false,
            None,
        ),
        Some(start + Duration::from_secs(65))
    );
    inventory[0].pass_due = u64::MAX;
    assert_eq!(
        next_wake(
            &inventory,
            &VecDeque::new(),
            None,
            &clock,
            None,
            false,
            None,
        ),
        Some(start + Duration::from_secs(65)),
        "unrepresentable epochs cannot panic or hide a nearer inventory pass"
    );
}

#[tokio::test(start_paused = true)]
async fn independent_intervals_ignore_activity_and_skip_missed_rounds() {
    let start = tokio::time::Instant::now();
    let groups = vec!["a".into(), "b".into(), "c".into()];
    let mut draws = [30, 120, 75, 90, 45, 60, 30, 120].into_iter();
    let mut draw = || Duration::from_secs(draws.next().unwrap());
    let mut schedule = Opportunities::new(groups, start, &mut draw);
    let original = schedule.0.clone();
    // Completion/payment activity causes no draw, advance or early refill.
    for elapsed in 0..30 {
        assert!(
            schedule
                .take_due(start + Duration::from_secs(elapsed), &mut draw)
                .is_empty()
        );
        assert_eq!(schedule.0, original);
    }
    assert_eq!(
        schedule.take_due(start + Duration::from_secs(30), &mut draw),
        ["a"]
    );
    assert_eq!(schedule.0["a"], start + Duration::from_secs(120));
    // All overdue groups rearm against one captured instant before any work.
    let delayed = start + Duration::from_secs(1000);
    assert_eq!(schedule.take_due(delayed, &mut draw), ["a", "b", "c"]);
    assert_eq!(schedule.0["a"], delayed + Duration::from_secs(45));
    assert_eq!(schedule.0["b"], delayed + Duration::from_secs(60));
    assert_eq!(schedule.0["c"], delayed + Duration::from_secs(30));
    assert!(schedule.take_due(delayed, &mut draw).is_empty());
    let restarted = Opportunities::new(vec!["a".into()], delayed, &mut draw);
    assert_eq!(restarted.0["a"], delayed + Duration::from_secs(120));
}

#[derive(Clone)]
struct NoReads;
impl FundingObservations for NoReads {
    type QuoteRequest = ();
    async fn quote(&self, _: ()) -> Result<Quote> {
        panic!("no quote without prerequisite")
    }
    async fn status(&self, _: Quote) -> Result<SwapStatus> {
        panic!("no accepted swap")
    }
    async fn credit(&self, _: FundingJob) -> Result<()> {
        panic!("no accepted source")
    }
}
struct EmptyBackend {
    groups: Vec<String>,
    requested: tokio::sync::mpsc::UnboundedSender<String>,
}
impl FundingBackend for EmptyBackend {
    type Observations = NoReads;
    fn observations(&self) -> NoReads {
        NoReads
    }
    fn allocation_groups(&self) -> Vec<String> {
        self.groups.clone()
    }
    async fn quote_prerequisite(&mut self, job: &FundingJob) -> Result<()> {
        self.requested.send(job.pool_name.clone()).unwrap();
        anyhow::bail!("unfunded fixture")
    }
    async fn prepare(&mut self, _: &FundingJob, _: &Quote) -> Result<()> {
        panic!("no quote")
    }
    async fn submit(&mut self, _: &FundingJob) -> Result<()> {
        panic!("no signed bytes")
    }
    async fn reconcile(&mut self, _: &FundingJob) -> Result<bool> {
        panic!("no accepted source")
    }
}

// Keep the runtime runnable while joining blocking Store work: paused time must
// advance only when this test explicitly requests it, never while SQLite runs.
async fn local<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::pin!(future);
    loop {
        tokio::select! {
            biased;
            value = &mut future => return value,
            _ = tokio::task::yield_now() => {},
        }
    }
}

#[tokio::test(start_paused = true)]
async fn serving_defers_new_allocations_but_resumes_jobs_and_bootstrap_is_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let reserves = crate::rotation::test_capacity::small_reserve("4");
    for name in ["existing", "cold"] {
        store
            .ensure_capacity_group(
                name,
                &reserves,
                Some(alloy_primitives::U256::from(1_000_000)),
                0,
            )
            .unwrap();
    }
    store.fill_funding_vacancies("existing").unwrap();
    let (store, actor) = super::super::StoreHandle::spawn(store);
    let (requested, mut requests) = tokio::sync::mpsc::unbounded_channel();
    let worker = FundingWorker::new(
        store.clone(),
        EmptyBackend {
            groups: vec!["existing".into(), "cold".into()],
            requested,
        },
        5,
    );
    let stop = CancellationToken::new();
    let task = tokio::spawn(worker.clone().run(stop.clone()));
    assert_eq!(
        local(requests.recv()).await.unwrap(),
        "existing",
        "accepted allocation resumes without waiting for refill"
    );
    tokio::time::advance(Duration::from_secs(29)).await;
    local(store.call(|s| {
        assert_eq!(s.funding_jobs()?.len(), 1);
        Ok(())
    }))
    .await
    .unwrap();
    // A capacity wake cannot grant an early refill or move its scheduled instant.
    local(store.call(|s| {
        s.capacity_changes("cold").notify_waiters();
        Ok(())
    }))
    .await
    .unwrap();
    local(store.call(|s| {
        assert_eq!(s.funding_jobs()?.len(), 1);
        Ok(())
    }))
    .await
    .unwrap();
    tokio::time::advance(Duration::from_secs(92)).await;
    local(async {
        loop {
            if local(store.call(|s| Ok(s.funding_jobs()?.len())))
                .await
                .unwrap()
                == 2
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    stop.cancel();
    local(task).await.unwrap().unwrap();
    // Explicit mode fills a new vacant group immediately, without a serving delay.
    local(store.call(move |s| {
        s.ensure_capacity_group(
            "explicit",
            &reserves,
            Some(alloy_primitives::U256::from(1_000_000)),
            0,
        )?;
        Ok(())
    }))
    .await
    .unwrap();
    let (requested, mut requests) = tokio::sync::mpsc::unbounded_channel();
    drop(worker);
    let worker = FundingWorker::new(
        store.clone(),
        EmptyBackend {
            groups: vec!["explicit".into()],
            requested,
        },
        5,
    );
    let stop = CancellationToken::new();
    let task = tokio::spawn(worker.run_bootstrap(stop.clone()));
    // Previously allocated jobs remain observable; their random durable IDs do
    // not establish an ordering relative to the newly captured explicit job.
    loop {
        let group = local(requests.recv()).await.unwrap();
        if group == "explicit" {
            break;
        }
        assert!(matches!(group.as_str(), "existing" | "cold"));
    }
    stop.cancel();
    local(task).await.unwrap().unwrap();
    local(store.call(|s| {
        assert_eq!(s.funding_jobs()?.len(), 3);
        Ok(())
    }))
    .await
    .unwrap();
    drop(store);
    local(actor).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn explicit_bootstrap_selection_fills_both_slots_and_keeps_previously_allocated_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    for name in ["selected", "unselected", "existing"] {
        let reserves = if name == "selected" {
            crate::rotation::test_capacity::two_reserves("4")
        } else {
            crate::rotation::test_capacity::small_reserve("4")
        };
        state
            .ensure_capacity_group(
                name,
                &reserves,
                Some(alloy_primitives::U256::from(1_000_000)),
                0,
            )
            .unwrap();
    }
    // This durable initial-funding intent predates command selection. It keeps
    // current-authority quote/preparation progress even before source acceptance.
    state.fill_funding_vacancies("existing").unwrap();
    let original = state.funding_jobs().unwrap().remove(0);
    let (store, actor) = StoreHandle::spawn(state);
    let (requested, mut requests) = tokio::sync::mpsc::unbounded_channel();
    let worker = FundingWorker::new(
        store.clone(),
        EmptyBackend {
            groups: vec!["selected".into(), "unselected".into(), "existing".into()],
            requested,
        },
        5,
    )
    .with_allocation_selection(["selected".into()].into());
    let stop = CancellationToken::new();
    let started = tokio::time::Instant::now();
    let task = tokio::spawn(worker.run_bootstrap(stop.clone()));
    let mut selected_requests = 0;
    let mut existing_requested = false;
    while selected_requests < 2 || !existing_requested {
        match local(requests.recv()).await.unwrap().as_str() {
            "selected" => selected_requests += 1,
            "existing" => existing_requested = true,
            other => panic!("unselected allocation reached funding: {other}"),
        }
    }
    assert_eq!(
        tokio::time::Instant::now(),
        started,
        "explicit allocations are prompt, without a serving opportunity"
    );
    local(store.call(move |s| {
        let jobs = s.funding_jobs()?;
        assert_eq!(jobs.len(), 3);
        let selected = jobs
            .iter()
            .filter(|job| job.pool_name == "selected")
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), 2);
        assert_ne!(
            s.funding_wallet_type(&selected[0].id)?,
            s.funding_wallet_type(&selected[1].id)?
        );
        assert!(
            jobs.iter()
                .any(|job| job.id == original.id && job.operation_id == original.operation_id)
        );
        for pool in s.status()?.pools {
            assert!(!s.has_bootstrap_demand(&pool.id)?);
            if pool.name == "unselected" {
                assert!(pool.addresses.is_empty());
            }
        }
        Ok(())
    }))
    .await
    .unwrap();
    stop.cancel();
    local(task).await.unwrap().unwrap();
    drop(store);
    local(actor).await.unwrap();
}

struct FailFirstAllocation {
    first: std::sync::atomic::AtomicBool,
    failed: tokio::sync::mpsc::UnboundedSender<()>,
}
impl crate::rotation::restriction::FundingPermits for FailFirstAllocation {
    fn validate_treasury(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn validate_state(&self, _: &serde_json::Value) -> Result<()> {
        Ok(())
    }
    fn reserve_allocations(&self, _: &str, _: &str, sequences: &[i64]) -> Result<Vec<String>> {
        if self.first.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.failed.send(()).unwrap();
            anyhow::bail!("temporary fixture allocation publication failure");
        }
        Ok(sequences
            .iter()
            .map(|_| uuid::Uuid::new_v4().to_string())
            .collect())
    }
    fn check_preparation(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    fn reserve_preparation(&self, _: &str, _: &str, _: &str, _: u64) -> Result<()> {
        Ok(())
    }
    fn retire_unprepared(&self, _: &str, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn bootstrap_and_qualification_retry_failed_allocation_without_random_delay() {
    for mode in [Mode::Bootstrap, Mode::Explicit] {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let (failed, mut failures) = tokio::sync::mpsc::unbounded_channel();
        store
            .install_funding_permits(Arc::new(FailFirstAllocation {
                first: std::sync::atomic::AtomicBool::new(true),
                failed,
            }))
            .unwrap();
        store
            .ensure_capacity_group(
                "pool",
                &crate::rotation::test_capacity::small_reserve("4"),
                Some(alloy_primitives::U256::from(1_000_000)),
                0,
            )
            .unwrap();
        let (store, actor) = super::super::StoreHandle::spawn(store);
        let (requested, _requests) = tokio::sync::mpsc::unbounded_channel();
        let worker = FundingWorker::new(
            store.clone(),
            EmptyBackend {
                groups: vec!["pool".into()],
                requested,
            },
            5,
        );
        let stop = CancellationToken::new();
        let task = tokio::spawn({
            let stop = stop.clone();
            async move { worker.schedule_mode(1000, stop, mode).await }
        });
        local(failures.recv()).await.unwrap();
        local(store.call(|s| {
            assert!(s.funding_jobs()?.is_empty());
            Ok(())
        }))
        .await
        .unwrap();
        // The retry instant is captured before allocation starts. Failure
        // notification therefore proves its clock is already armed.
        tokio::time::advance(Duration::from_secs(5)).await;
        local(async {
            loop {
                if local(store.call(|s| Ok(s.funding_jobs()?.len())))
                    .await
                    .unwrap()
                    == 1
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        // Allocation is the completion contract here; accepted-work dispatch is
        // covered independently and need not await a new inventory pass.
        stop.cancel();
        local(task).await.unwrap().unwrap();
        drop(store);
        local(actor).await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn first_use_opportunities_keep_due_times_through_wakes_and_credit() {
    let start = tokio::time::Instant::now();
    let mut schedule = Opportunities(BTreeMap::new());
    let mut draws = 0;
    let mut draw = || {
        draws += 1;
        Duration::from_secs(120)
    };
    schedule.admit(&BTreeSet::new(), start, &mut draw);
    assert!(schedule.0.is_empty(), "dormant groups have no opportunity");
    let eligible = BTreeSet::from(["participant".into()]);
    schedule.admit(&eligible, start, &mut draw);
    let first = schedule.0["participant"];
    for elapsed in [1, 30, 60, 119] {
        // Repeated demand/credit notices carry the same monotonic eligibility.
        schedule.admit(&eligible, start + Duration::from_secs(elapsed), &mut draw);
        assert_eq!(schedule.0["participant"], first);
    }
    // Discovery omits armed names; their opportunities remain owned regardless.
    schedule.admit(&BTreeSet::new(), start, &mut draw);
    assert_eq!(schedule.0["participant"], first);
    assert_eq!(draws, 1);
    assert!(
        schedule
            .take_due(first - Duration::from_secs(1), || panic!("not due"))
            .is_empty()
    );
    assert_eq!(
        schedule.take_due(first, || Duration::from_secs(30)),
        ["participant"]
    );
    assert_eq!(schedule.0["participant"], first + Duration::from_secs(30));
    for _ in 0..64 {
        assert!((30..=120).contains(&draw_interval().as_secs()));
    }
}

#[tokio::test(start_paused = true)]
async fn delayed_eligibility_read_preserves_first_use_and_retry_intervals() {
    let mut schedule = Opportunities(BTreeMap::new());
    let mut error = None;
    let poll = Duration::from_secs(5);
    let retry = schedule
        .discover(
            async {
                tokio::time::sleep(Duration::from_secs(241)).await;
                Ok(["participant".into()].into())
            },
            poll,
            &mut error,
        )
        .await
        .unwrap();
    let first = schedule.0["participant"];
    let elapsed = first - tokio::time::Instant::now();
    assert!((Duration::from_secs(30)..=Duration::from_secs(120)).contains(&elapsed));
    assert!(retry.is_none());

    let retry = schedule
        .discover(
            async {
                tokio::time::sleep(Duration::from_secs(12)).await;
                Err(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                    None,
                )
                .into())
            },
            poll,
            &mut error,
        )
        .await
        .unwrap();
    assert_eq!(retry.unwrap() - tokio::time::Instant::now(), poll);
    assert_eq!(schedule.0["participant"], first);
}

#[tokio::test(start_paused = true)]
async fn eligibility_failure_keeps_ordinary_and_armed_participant_opportunities_runnable() {
    let start = tokio::time::Instant::now();
    let clock = TickClock {
        epoch: 1000,
        started: start,
    };
    let mut inventory = [Kind::Credit, Kind::Status, Kind::Source, Kind::Quote]
        .map(|kind| LaneInventory::new(kind, 0, 2000));
    for lane in &mut inventory {
        lane.exhausted = true;
    }
    let mut schedule =
        Opportunities::new(vec!["ordinary".into()], start, || Duration::from_secs(30));
    schedule.admit(&["participant".into()].into(), start, || {
        Duration::from_secs(30)
    });
    let original = schedule.0.clone();
    tokio::time::advance(Duration::from_secs(30)).await;
    let mut error = None;
    let retry = schedule
        .discover(
            async {
                Err(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                    None,
                )
                .into())
            },
            Duration::from_secs(5),
            &mut error,
        )
        .await
        .unwrap();
    assert!(error.is_some());
    assert_eq!(schedule.0, original);
    assert_eq!(
        next_wake(
            &inventory,
            &VecDeque::new(),
            None,
            &clock,
            schedule.next_due().into_iter().chain(retry).min(),
            false,
            None
        ),
        Some(tokio::time::Instant::now()),
        "a discovery retry cannot mask either armed opportunity"
    );
    assert_eq!(
        schedule.take_due(tokio::time::Instant::now(), || Duration::from_secs(120)),
        ["ordinary", "participant"]
    );
    assert_eq!(
        schedule.0.len(),
        2,
        "failed discovery cannot admit dormant groups"
    );
    assert_eq!(
        schedule.next_due(),
        Some(tokio::time::Instant::now() + Duration::from_secs(120))
    );
}
