//! One retained funding owner; observations never borrow the source backend.
use super::*;
use futures_util::{FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};

const CREDIT_READS: usize = 4;
const STATUS_READS: usize = 4;
const QUOTE_READS: usize = 4;
// Bounds only cached scheduling metadata. Expired, inactive records can be
// forgotten; in-flight ownership and an unelapsed backoff cannot be evicted.
const RECORDS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Kind {
    Credit,
    Status,
    Source,
    Quote,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    operation: String,
    kind: Kind,
}
struct Record {
    due: u64,
    streak: u32,
    error: Option<String>,
}
#[derive(Default)]
struct State {
    records: BTreeMap<Key, Record>,
}
/// StoreHandle owns this gate. Independent worker handles cannot overlap even
/// after their response waiter disappears or a source command outlives a pass.
#[derive(Default)]
pub(crate) struct Owner {
    state: Mutex<State>,
}
type Lease = Arc<tokio::sync::OwnedMutexGuard<Owner>>;

#[derive(Debug)]
pub struct AlreadyOwned;
impl std::fmt::Display for AlreadyOwned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("funding_owner_already_running")
    }
}
impl std::error::Error for AlreadyOwned {}

enum Observation<Q> {
    None,
    Status(SwapStatus),
    Prerequisite(Q),
    Quote(Quote),
}
struct Completion<Q> {
    job: FundingJob,
    kind: Kind,
    result: Result<Observation<Q>>,
}
struct Pending {
    job: FundingJob,
    credit: bool,
    status: bool,
    source: bool,
}
impl Pending {
    fn done(&self) -> bool {
        !self.credit && !self.status && !self.source
    }
}

impl<B: FundingBackend + Send + 'static> FundingWorker<B> {
    pub(super) async fn schedule(
        &self,
        instant: u64,
        stop: CancellationToken,
        once: bool,
    ) -> Result<()> {
        let lease = Arc::new(
            self.store
                .funding_owner()
                .try_lock_owned()
                .map_err(|_| AlreadyOwned)?,
        );
        let worker = self.clone();
        // The spawn, not its response waiter, owns accepted work and the lease.
        tokio::spawn(async move { worker.schedule_owned(lease, instant, stop, once).await })
            .await
            .context("funding owner stopped")?
    }

    async fn schedule_owned(
        &self,
        lease: Lease,
        instant: u64,
        stop: CancellationToken,
        once: bool,
    ) -> Result<()> {
        let clock = TickClock {
            epoch: instant,
            started: std::time::Instant::now(),
        };
        let (reader, health_after) = {
            let backend = self.backend.lock().await;
            (backend.observations(), backend.swap_timeout_seconds())
        };
        let mut reads = FuturesUnordered::<
            BoxFuture<'static, Completion<<B::Observations as FundingObservations>::QuoteRequest>>,
        >::new();
        let mut active = BTreeSet::<Key>::new();
        let mut pending = VecDeque::<Pending>::new();
        // Quote here names the prerequisite inventory lane; its owned step
        // shares Source serialization, then yields an independent Quote read.
        let boundary = self.store.call(|s| s.funding_work_boundary()).await?;
        let mut through = [boundary; 4];
        let mut cursors = std::array::from_fn::<_, 4, _>(|_| String::new());
        let mut exhausted = [false; 4];
        let mut pass_due = [clock.now(); 4];
        let mut source_active = false;
        let mut credit_active = 0;
        let mut status_active = 0;
        let mut quote_active = 0;
        let mut quote_pending: Option<(
            FundingJob,
            <B::Observations as FundingObservations>::QuoteRequest,
        )> = None;
        let mut stopping = false;
        loop {
            stopping |= stop.is_cancelled();
            for (lane, kind) in [Kind::Credit, Kind::Status, Kind::Source, Kind::Quote]
                .into_iter()
                .enumerate()
            {
                let waiting = pending.iter().any(|item| match kind {
                    Kind::Credit => item.credit,
                    Kind::Status => item.status,
                    Kind::Source => item.source && item.job.phase != FundingPhase::Allocated,
                    Kind::Quote => item.source && item.job.phase == FundingPhase::Allocated,
                });
                if !once
                    && exhausted[lane]
                    && !waiting
                    && clock.now() >= pass_due[lane]
                    && !stopping
                {
                    through[lane] = self.store.call(|s| s.funding_work_boundary()).await?;
                    cursors[lane].clear();
                    exhausted[lane] = false;
                }
                if stopping || exhausted[lane] || waiting {
                    continue;
                }
                let after = cursors[lane].clone();
                let upper = through[lane];
                let page = self
                    .store
                    .call(move |s| s.funding_work_page(&after, upper))
                    .await?;
                if page.is_empty() {
                    exhausted[lane] = true;
                    pass_due[lane] = clock.now().saturating_add(self.poll_seconds.max(1));
                    continue;
                }
                cursors[lane] = page.last().unwrap().id.clone();
                for mut job in page {
                    if job
                        .started_at
                        .is_some_and(|started| clock.now().saturating_sub(started) >= health_after)
                        && !job.timed_out
                    {
                        let id = job.id.clone();
                        let operation = job.operation_id.clone();
                        self.store
                            .call(move |s| s.mark_funding_operation_timeout(&id, &operation))
                            .await?;
                        job.timed_out = true;
                    }
                    let (id, operation) = (job.id.clone(), job.operation_id.clone());
                    let facts = self
                        .store
                        .call(move |s| s.funding_facts(&id, &operation))
                        .await?;
                    let item = Pending {
                        credit: kind == Kind::Credit
                            && (!facts.readiness_recorded || facts.readiness_disputed),
                        status: kind == Kind::Status && facts.status_needed(),
                        source: ((kind == Kind::Source && job.phase != FundingPhase::Allocated)
                            || (kind == Kind::Quote && job.phase == FundingPhase::Allocated))
                            && (facts.source_reconciliation_needed()
                                || crate::treasury::recovery::needs_recovery(&job)
                                || matches!(
                                    job.phase,
                                    FundingPhase::Preparing | FundingPhase::Prepared
                                )
                                || (matches!(
                                    job.phase,
                                    FundingPhase::Allocated | FundingPhase::Quoted
                                ) && (facts.enabled
                                    || (facts.readiness_recorded && !facts.readiness_disputed)))),
                        job,
                    };
                    if !item.done() {
                        pending.push_back(item);
                    }
                }
            }
            if !stopping {
                for _ in 0..pending.len() {
                    let mut item = pending.pop_front().unwrap();
                    for kind in [Kind::Credit, Kind::Status, Kind::Source] {
                        let wanted = match kind {
                            Kind::Credit => &mut item.credit,
                            Kind::Status => &mut item.status,
                            Kind::Source => &mut item.source,
                            Kind::Quote => unreachable!(),
                        };
                        if !*wanted {
                            continue;
                        }
                        if kind == Kind::Source
                            && item.job.phase == FundingPhase::Allocated
                            && (quote_active == QUOTE_READS
                                || quote_pending.is_some()
                                || active.contains(&Key {
                                    operation: item.job.operation_id.clone(),
                                    kind: Kind::Quote,
                                }))
                        {
                            // The prerequisite lane keeps its own bounded page;
                            // occupied quote HTTP slots cannot hold the source
                            // reconciliation cursor behind unrelated allocations.
                            continue;
                        }
                        let key = Key {
                            operation: item.job.operation_id.clone(),
                            kind,
                        };
                        if active.contains(&key) {
                            *wanted = false;
                            continue;
                        }
                        // Source is one rolling serialized lane. It never parks
                        // a page of status/credit behind an occupied treasury.
                        if kind == Kind::Source && source_active {
                            // Retain only this source lane's bounded page. Other
                            // lane cursors continue independently while owned work drains.
                            continue;
                        }
                        if (kind == Kind::Credit && credit_active == CREDIT_READS)
                            || (kind == Kind::Status && status_active == STATUS_READS)
                        {
                            continue;
                        }
                        let ready = {
                            let mut state = lease.state.lock().expect("funding schedule lock");
                            state.admit(&key, &item.job, clock.now(), &active)
                        };
                        match ready {
                            Some(false) => {
                                *wanted = false;
                                continue;
                            }
                            None => continue,
                            Some(true) => {}
                        }
                        *wanted = false;
                        active.insert(key);
                        let job = item.job.clone();
                        match kind {
                            Kind::Credit => {
                                credit_active += 1;
                                let reader = reader.clone();
                                reads.push(
                                    async move {
                                        let result = reader
                                            .credit(job.clone())
                                            .await
                                            .map(|()| Observation::None);
                                        Completion { job, kind, result }
                                    }
                                    .boxed(),
                                );
                            }
                            Kind::Status => {
                                status_active += 1;
                                let reader = reader.clone();
                                let store = self.store.clone();
                                reads.push(
                                    async move {
                                        let (id, operation) =
                                            (job.id.clone(), job.operation_id.clone());
                                        let result = async {
                                            let quote = store
                                                .call(move |s| {
                                                    Ok(serde_json::from_slice::<Quote>(
                                                        &s.funding_quote_for_operation(
                                                            &id, &operation,
                                                        )?,
                                                    )?)
                                                })
                                                .await?;
                                            reader.status(quote).await.map(Observation::Status)
                                        }
                                        .await;
                                        Completion { job, kind, result }
                                    }
                                    .boxed(),
                                );
                            }
                            Kind::Quote => unreachable!(),
                            Kind::Source => {
                                source_active = true;
                                let worker = self.clone();
                                let lease = lease.clone();
                                let epoch = clock.now();
                                // An accepted mutable step owns the lease even
                                // if an outer scheduler error drops its waiter.
                                let task = tokio::spawn(async move {
                                    let _lease = lease;
                                    let source_clock = TickClock {
                                        epoch,
                                        started: std::time::Instant::now(),
                                    };
                                    let result = worker.source_step(&job, &source_clock).await.map(
                                        |request| {
                                            request.map_or(
                                                Observation::None,
                                                Observation::Prerequisite,
                                            )
                                        },
                                    );
                                    Completion { job, kind, result }
                                });
                                reads.push(async move {task.await.expect("funding source owner panicked")}.boxed());
                            }
                        }
                    }
                    if !item.done() {
                        pending.push_back(item);
                    }
                }
            }
            if !stopping
                && quote_active < QUOTE_READS
                && let Some((job, request)) = quote_pending.take()
            {
                let key = Key {
                    operation: job.operation_id.clone(),
                    kind: Kind::Quote,
                };
                let admitted = lease.state.lock().expect("funding schedule lock").admit(
                    &key,
                    &job,
                    clock.now(),
                    &active,
                );
                if admitted == Some(true) {
                    active.insert(key);
                    quote_active += 1;
                    let reader = reader.clone();
                    reads.push(
                        async move {
                            let result = reader.quote(request).await.map(Observation::Quote);
                            Completion {
                                job,
                                kind: Kind::Quote,
                                result,
                            }
                        }
                        .boxed(),
                    );
                } else {
                    quote_pending = Some((job, request));
                }
            }
            if stopping {
                pending.clear();
                // Abort caller-owned unsigned observations, but retain the
                // separately spawned source task through its accepted work.
                if !source_active {
                    return Ok(());
                }
            }
            if exhausted.iter().all(|done| *done)
                && pending.is_empty()
                && once
                && reads.is_empty()
                && quote_pending.is_none()
            {
                return Ok(());
            }
            let needs_page = [Kind::Credit, Kind::Status, Kind::Source, Kind::Quote]
                .into_iter()
                .enumerate()
                .any(|(lane, kind)| {
                    !exhausted[lane]
                        && !pending.iter().any(|item| match kind {
                            Kind::Credit => item.credit,
                            Kind::Status => item.status,
                            Kind::Source => {
                                item.source && item.job.phase != FundingPhase::Allocated
                            }
                            Kind::Quote => item.source && item.job.phase == FundingPhase::Allocated,
                        })
                });
            if needs_page && !stopping {
                if let Some(Some(completion)) = reads.next().now_or_never() {
                    match completion.kind {
                        Kind::Credit => credit_active -= 1,
                        Kind::Status => status_active -= 1,
                        Kind::Source => source_active = false,
                        Kind::Quote => quote_active -= 1,
                    }
                    active.remove(&Key {
                        operation: completion.job.operation_id.clone(),
                        kind: completion.kind,
                    });
                    if let Some(ready) = self
                        .publish_observation(&lease, completion, clock.now())
                        .await?
                    {
                        quote_pending = Some(ready);
                    }
                }
                continue;
            }
            tokio::select! {
                completion=reads.next(),if !reads.is_empty()=>{
                    let completion=completion.unwrap();
                    match completion.kind {Kind::Credit=>credit_active-=1,Kind::Status=>status_active-=1,Kind::Source=>source_active=false,Kind::Quote=>quote_active-=1}
                    active.remove(&Key{operation:completion.job.operation_id.clone(),kind:completion.kind});
                    if let Some(ready) = self.publish_observation(&lease,completion,clock.now()).await? { quote_pending = Some(ready); }
                },
                _=stop.cancelled(),if !stopping=>stopping=true,
                _=tokio::time::sleep(std::time::Duration::from_secs(1)),if !stopping=>{},
            }
        }
    }

    async fn publish_observation(
        &self,
        lease: &Lease,
        completion: Completion<<B::Observations as FundingObservations>::QuoteRequest>,
        instant: u64,
    ) -> Result<
        Option<(
            FundingJob,
            <B::Observations as FundingObservations>::QuoteRequest,
        )>,
    > {
        let Completion { job, kind, result } = completion;
        let id = job.id.clone();
        let current = self.store.call(move |s| s.funding_job(&id)).await?;
        if current.is_none_or(|current| current.operation_id != job.operation_id) {
            return Ok(None);
        }

        let mut prerequisite = None;
        let result = match result {
            Ok(Observation::Status(status)) => {
                let (id, operation) = (job.id.clone(), job.operation_id.clone());
                self.store
                    .call(move |s| s.record_provider_outcome(&id, &operation, &status, instant))
                    .await
            }
            Ok(Observation::None) => Ok(()),
            Ok(Observation::Prerequisite(request)) => {
                prerequisite = Some((job.clone(), request));
                Ok(())
            }
            Ok(Observation::Quote(quote)) => {
                let (id, operation) = (job.id.clone(), job.operation_id.clone());
                self.store
                    .call(move |s| {
                        ensure!(quote.deposit.is_some(), "cannot fund from dry quote");
                        let target = quote.request["amount"]
                            .as_str()
                            .context("funding quote missing output target")?
                            .to_owned();
                        s.save_funding_quote_for_operation(
                            &id,
                            &operation,
                            &serde_json::to_vec(&quote)?,
                            &target,
                        )
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        let phase = match kind {
            Kind::Credit => FundingPhase::VerifyingCredit,
            Kind::Status => FundingPhase::Swapping,
            Kind::Source => job.phase.clone(),
            Kind::Quote => FundingPhase::Allocated,
        };
        let failure = result.err();
        let wait = failure
            .as_ref()
            .and_then(|error| waiting_reason(error, &phase));
        if let Some(reason) = wait {
            tracing::debug!(job_id=%job.id,pool_id=%job.pool_id,phase=?phase,reason,"funding awaiting chain confirmation; polling automatically");
        }
        let error = failure
            .filter(|_| wait.is_none())
            .map(|error| safe_error(&error, &phase));
        if let Some(category) = &error {
            tracing::warn!(job_id=%job.id,pool_id=%job.pool_id,phase=?phase,category,"funding observation failed; retaining owned work and backing off this observation");
        }
        let (id, operation) = (job.id.clone(), job.operation_id.clone());
        let applicable = self
            .store
            .call(move |s| {
                let current = s.funding_job(&id)?.context("funding job missing")?;
                if current.operation_id != operation {
                    return Ok([false; 4]);
                }
                let facts = s.funding_facts(&id, &operation)?;
                Ok([
                    !facts.readiness_recorded || facts.readiness_disputed,
                    facts.status_needed(),
                    facts.source_reconciliation_needed()
                        || crate::treasury::recovery::needs_recovery(&current)
                        || matches!(
                            current.phase,
                            FundingPhase::Preparing | FundingPhase::Prepared
                        )
                        || (matches!(
                            current.phase,
                            FundingPhase::Allocated | FundingPhase::Quoted
                        ) && (facts.enabled
                            || (facts.readiness_recorded && !facts.readiness_disputed))),
                    current.phase == FundingPhase::Allocated
                        && facts.enabled
                        && !facts.readiness_recorded
                        && !facts.accepted_quote,
                ])
            })
            .await?;
        let (next, streak, health) = {
            let mut state = lease.state.lock().expect("funding schedule lock");
            state.complete(&job, kind, instant, self.poll_seconds, error);
            state.health(&job.operation_id, instant, applicable)
        };
        let (id, operation) = (job.id.clone(), job.operation_id.clone());
        let phase = self
            .store
            .call(move |s| {
                if !s
                    .funding_job(&id)?
                    .is_some_and(|job| job.operation_id == operation)
                {
                    return Ok(None);
                }
                s.finish_funding_if_proven(&id, &operation)?;
                s.record_funding_observation_health(
                    &id,
                    &operation,
                    next,
                    streak,
                    health.as_deref(),
                )?;
                Ok(s.funding_job(&id)?.map(|job| job.phase))
            })
            .await?;
        if let Some(phase) = phase
            && phase != job.phase
        {
            tracing::info!(job_id=%job.id,pool_id=%job.pool_id,from=?job.phase,to=?phase,"funding phase changed");
        }
        Ok(prerequisite)
    }
}

impl State {
    fn admit(
        &mut self,
        key: &Key,
        job: &FundingJob,
        now: u64,
        active: &BTreeSet<Key>,
    ) -> Option<bool> {
        if key.kind == Kind::Source
            && job.phase == FundingPhase::Allocated
            && self
                .records
                .get(&Key {
                    operation: key.operation.clone(),
                    kind: Kind::Quote,
                })
                .is_some_and(|record| record.due > now)
        {
            return Some(false);
        }
        if let Some(record) = self.records.get(key) {
            return Some(record.due <= now);
        }
        if self
            .records
            .keys()
            .filter(|other| other.kind == key.kind)
            .count()
            == RECORDS
        {
            let old = self
                .records
                .iter()
                .filter(|(other, record)| {
                    other.kind == key.kind && record.due <= now && !active.contains(*other)
                })
                .min_by_key(|(_, record)| record.due)
                .map(|(key, _)| key.clone())?;
            self.records.remove(&old);
        }
        // Persisted next_poll is aggregate reporting from older coordinators;
        // it cannot assign one kind's outage to another kind after reopen.
        let due = 0;
        self.records.insert(
            key.clone(),
            Record {
                due,
                streak: 0,
                error: None,
            },
        );
        Some(due <= now)
    }
    fn complete(
        &mut self,
        job: &FundingJob,
        kind: Kind,
        now: u64,
        poll: u64,
        error: Option<String>,
    ) {
        let key = Key {
            operation: job.operation_id.clone(),
            kind,
        };
        let record = self
            .records
            .get_mut(&key)
            .expect("owned observation record");
        record.streak = if error.is_some() {
            record.streak.saturating_add(1)
        } else {
            0
        };
        let delay = poll
            .saturating_mul(1u64 << record.streak.min(6))
            .min(300)
            .max(if job.timed_out { 60 } else { 1 });
        let jitter = job.id.bytes().map(u64::from).sum::<u64>() % poll.saturating_add(1).max(1);
        record.due = now.saturating_add(delay).saturating_add(jitter);
        record.error = error;
    }
    fn health(
        &self,
        operation: &str,
        now: u64,
        applicable: [bool; 4],
    ) -> (u64, u32, Option<String>) {
        let entries: Vec<_> = self
            .records
            .iter()
            .filter(|(key, _)| key.operation == operation && applicable[key.kind as usize])
            .collect();
        let due = entries
            .iter()
            .map(|(_, record)| record.due)
            .min()
            .unwrap_or(now)
            .max(now);
        let streak = entries
            .iter()
            .map(|(_, record)| record.streak)
            .max()
            .unwrap_or(0);
        // Keep the fixed leading category consumed by startup and recovery.
        // Per-kind errors remain independently retained in the bounded owner;
        // source/quote policy obstructions take precedence over read outages.
        let error = [Kind::Source, Kind::Quote, Kind::Status, Kind::Credit]
            .into_iter()
            .find_map(|kind| {
                entries
                    .iter()
                    .find(|(key, record)| key.kind == kind && record.error.is_some())
                    .and_then(|(_, record)| record.error.clone())
            });
        (due, streak, error)
    }
}
