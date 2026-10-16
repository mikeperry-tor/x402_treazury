//! One retained funding owner; observations never borrow the source backend.
use super::*;
use crate::rotation::store::inventory::read_result;
use crate::rotation::store::{
    funding::FundingFacts,
    scheduling::{FundingWork, MalformedFunding},
};
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
    reported_phase: Option<FundingPhase>,
    due: u64,
    streak: u32,
    error: Option<String>,
    condition: Option<super::super::error::FundingCondition>,
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
    #[cfg(test)]
    completion_fault: Arc<Mutex<Option<(Kind, i32, usize)>>>,
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
// Scheduling and health reporting evaluate the same current facts. The source
// step includes quote prerequisites; the Quote kind is the subsequent HTTP read.
fn applicable_work(job: &FundingJob, facts: &FundingFacts) -> [bool; 4] {
    [
        !facts.readiness_recorded || facts.readiness_disputed,
        facts.status_needed(),
        facts.source_reconciliation_needed()
            || crate::treasury::recovery::needs_recovery(job)
            || matches!(job.phase, FundingPhase::Preparing | FundingPhase::Prepared)
            || (matches!(job.phase, FundingPhase::Allocated | FundingPhase::Quoted)
                && (facts.enabled || (facts.readiness_recorded && !facts.readiness_disputed))),
        job.phase == FundingPhase::Allocated
            && !facts.hidden_prepared
            && facts.enabled
            && !facts.readiness_recorded
            && !facts.accepted_quote,
    ]
}

fn report_decode_failure(fault: &MalformedFunding) {
    tracing::warn!(
        category = "malformed_funding_inventory",
        "funding payload unavailable; original operation retained"
    );
    tracing::debug!(job=%fault.id, operation=%fault.operation, "malformed funding ownership");
}

struct Pending {
    job: FundingJob,
    // Quote inventory contains source-owned prerequisites, not quote HTTP reads.
    lane: Kind,
}

/// Each lane retains its own bounded inventory pass while sibling work runs.
struct LaneInventory {
    kind: Kind,
    through: i64,
    cursor: String,
    exhausted: bool,
    pass_due: u64,
    decode_failure: Option<MalformedFunding>,
    read_failure: Option<anyhow::Error>,
}
impl LaneInventory {
    fn new(kind: Kind, through: i64, pass_due: u64) -> Self {
        Self {
            kind,
            through,
            cursor: String::new(),
            exhausted: false,
            pass_due,
            decode_failure: None,
            read_failure: None,
        }
    }
}

/// Accepted reads and their slots, including the source-to-quote handoff.
/// The source task itself retains the lease independently of these read waiters.
struct InFlight<Q> {
    reads: FuturesUnordered<BoxFuture<'static, Completion<Q>>>,
    // Completed I/O retains its original lane permit while the admission read
    // is unavailable. Only that read retries; publication remains one-shot.
    completions: VecDeque<(u64, Completion<Q>)>,
    active: BTreeSet<Key>,
    source_active: bool,
    credit_active: usize,
    status_active: usize,
    quote_active: usize,
    quote_pending: Option<(FundingJob, Q)>,
}
impl<Q> InFlight<Q> {
    fn new() -> Self {
        Self {
            reads: FuturesUnordered::new(),
            completions: VecDeque::new(),
            active: BTreeSet::new(),
            source_active: false,
            credit_active: 0,
            status_active: 0,
            quote_active: 0,
            quote_pending: None,
        }
    }

    fn release(&mut self, completion: &Completion<Q>) {
        match completion.kind {
            Kind::Credit => self.credit_active -= 1,
            Kind::Status => self.status_active -= 1,
            Kind::Source => self.source_active = false,
            Kind::Quote => self.quote_active -= 1,
        }
        self.active.remove(&Key {
            operation: completion.job.operation_id.clone(),
            kind: completion.kind,
        });
    }
}

#[cfg(test)]
pub(super) struct InventoryTestHook {
    lane: Kind,
    barrier: Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
        )>,
    >,
}
#[cfg(test)]
impl InventoryTestHook {
    async fn captured(&self, lane: Kind) {
        if self.lane != lane {
            return;
        }
        let barrier = self.barrier.lock().unwrap().take();
        if let Some((captured, resume)) = barrier {
            captured.send(()).unwrap();
            resume.await.unwrap();
        }
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
        let (reader, health_after, allocation_groups) = {
            let backend = self.backend.lock().await;
            (
                backend.observations(),
                backend.swap_timeout_seconds(),
                backend.allocation_groups(),
            )
        };
        // Configuration is immutable for this owner. Current store restrictions,
        // group enablement, cap and exact claim are rechecked on every commit.
        for name in &allocation_groups {
            let name = name.clone();
            if self
                .store
                .call(move |s| s.fill_funding_vacancies(&name))
                .await
                .is_err()
            {
                tracing::debug!("funding vacancy allocation unavailable");
            }
        }
        let mut allocation_cursor = 0usize;
        let mut allocation_due = clock
            .schedule_now()
            .saturating_add(self.poll_seconds.max(1));
        let mut flight = InFlight::new();
        let mut pending = VecDeque::<Pending>::new();
        // Quote here names the prerequisite inventory lane; its owned step
        // shares Source serialization, then yields an independent Quote read.
        let mut first_error = None;
        let boundary = loop {
            if let Some(boundary) = read_result(
                self.store.call(|s| s.funding_work_boundary()).await,
                &mut first_error,
            )? {
                break boundary;
            }
            if once {
                return Err(first_error.take().expect("failed inventory read"));
            }
            tokio::select! {
                _=stop.cancelled()=>return Ok(()),
                _=tokio::time::sleep(std::time::Duration::from_secs(self.poll_seconds.max(1)))=>{},
            }
        };
        let pass_due = clock.schedule_now();
        let mut inventory = [Kind::Credit, Kind::Status, Kind::Source, Kind::Quote]
            .map(|kind| LaneInventory::new(kind, boundary, pass_due));
        let mut stopping = false;
        loop {
            stopping |= stop.is_cancelled();
            if !once
                && !stopping
                && !allocation_groups.is_empty()
                && clock.schedule_now() >= allocation_due
            {
                let name = allocation_groups[allocation_cursor].clone();
                if self
                    .store
                    .call(move |s| s.fill_funding_vacancies(&name))
                    .await
                    .is_err()
                {
                    tracing::debug!("funding vacancy allocation unavailable");
                }
                allocation_cursor += 1;
                if allocation_cursor == allocation_groups.len() {
                    allocation_cursor = 0;
                    allocation_due = clock
                        .schedule_now()
                        .saturating_add(self.poll_seconds.max(1));
                }
            }
            if !stopping {
                self.collect_inventory(&mut inventory, &mut pending, &clock, health_after, once)
                    .await?;
                for lane in &mut inventory {
                    if let Some(error) = lane.decode_failure.take() {
                        first_error.get_or_insert(error.into());
                    }
                    if let Some(error) = lane.read_failure.take() {
                        first_error.get_or_insert(error);
                    }
                }
                self.dispatch_pending(&mut pending, &mut flight, &reader, &lease, &clock);
                self.dispatch_quote(&mut flight, &reader, &lease, &clock);
            }
            if stopping {
                pending.clear();
                // Abort caller-owned unsigned observations, but retain the
                // separately spawned source task through its accepted work.
                if !flight.source_active {
                    return Ok(());
                }
            }
            if inventory.iter().all(|lane| lane.exhausted)
                && pending.is_empty()
                && once
                && flight.reads.is_empty()
                && flight.completions.is_empty()
                && flight.quote_pending.is_none()
            {
                return first_error.map_or(Ok(()), Err);
            }
            let needs_page = inventory.iter().any(|lane| {
                !lane.exhausted
                    && clock.schedule_now() >= lane.pass_due
                    && !pending.iter().any(|item| item.lane == lane.kind)
            });
            let ready = flight
                .completions
                .iter()
                .position(|(due, _)| clock.schedule_now() >= *due);
            let completion = if let Some(index) = ready {
                Some(flight.completions.remove(index).unwrap().1)
            } else if needs_page && !stopping {
                flight.reads.next().now_or_never().flatten()
            } else {
                tokio::select! {
                    completion = flight.reads.next(), if !flight.reads.is_empty() => completion,
                    _ = stop.cancelled(), if !stopping => {
                        stopping = true;
                        None
                    },
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)), if !stopping || !flight.completions.is_empty() => None,
                }
            };
            if let Some(completion) = completion {
                let current = self.completion_current(&lease, &completion).await;
                if let Err(error) = &current
                    && crate::rotation::store::inventory::transient_read(error)
                {
                    tracing::warn!(
                        category = "funding_completion_read_busy",
                        "funding observation retained while its admission read is unavailable"
                    );
                    // A one-pass caller also retains accepted completion work
                    // until this read succeeds; no observation is reacquired
                    // and no whole-pass deadline is introduced.
                    flight.completions.push_back((
                        clock
                            .schedule_now()
                            .saturating_add(self.poll_seconds.max(1)),
                        completion,
                    ));
                    continue;
                }
                flight.release(&completion);
                let id = completion.job.id.clone();
                let operation = completion.job.operation_id.clone();
                let publication = match current {
                    Ok(true) => {
                        self.publish_current_observation(&lease, completion, clock.schedule_now())
                            .await
                    }
                    Ok(false) => Ok(None),
                    Err(error) => Err(error),
                };
                match publication {
                    Ok(Some(ready)) => flight.quote_pending = Some(ready),
                    Ok(None) => {}
                    Err(error) => {
                        if let Some(fault) = self
                            .store
                            .call(move |s| {
                                s.scope_funding_completion_failure(&id, &operation, error)
                            })
                            .await?
                        {
                            report_decode_failure(&fault);
                            first_error.get_or_insert(fault.into());
                        }
                    }
                }
            }
        }
    }

    async fn collect_inventory(
        &self,
        inventory: &mut [LaneInventory; 4],
        pending: &mut VecDeque<Pending>,
        clock: &TickClock,
        health_after: u64,
        once: bool,
    ) -> Result<()> {
        for lane in inventory {
            let kind = lane.kind;
            let waiting = pending.iter().any(|item| item.lane == kind);
            if clock.schedule_now() < lane.pass_due {
                continue;
            }
            if !once && lane.exhausted && !waiting && clock.schedule_now() >= lane.pass_due {
                let Some(through) = read_result(
                    self.store.call(|s| s.funding_work_boundary()).await,
                    &mut lane.read_failure,
                )?
                else {
                    lane.pass_due = clock
                        .schedule_now()
                        .saturating_add(self.poll_seconds.max(1));
                    continue;
                };
                lane.through = through;
                lane.cursor.clear();
                lane.exhausted = false;
            }
            if lane.exhausted || waiting {
                continue;
            }
            let after = lane.cursor.clone();
            let upper = lane.through;
            let page = self
                .store
                .call(move |s| s.funding_work_page(&after, upper))
                .await;
            let Some(page) = read_result(page, &mut lane.read_failure)? else {
                lane.pass_due = clock
                    .schedule_now()
                    .saturating_add(self.poll_seconds.max(1));
                lane.exhausted |= once;
                continue;
            };
            if page.is_empty() {
                lane.exhausted = true;
                lane.pass_due = clock
                    .schedule_now()
                    .saturating_add(self.poll_seconds.max(1));
                continue;
            }
            #[cfg(test)]
            if let Some(hook) = &self.inventory_test_hook {
                hook.captured(kind).await;
            }
            lane.cursor = page.last().unwrap().0.clone();
            for (_, entry) in page {
                let FundingWork { mut job, facts } = match entry {
                    Ok(work) => work,
                    Err(fault) => {
                        report_decode_failure(&fault);
                        lane.decode_failure.get_or_insert(fault);
                        continue;
                    }
                };
                if job.phase == FundingPhase::Complete
                    && !applicable_work(&job, &facts)
                        .into_iter()
                        .any(|needed| needed)
                {
                    continue;
                }
                if job.started_at.is_some_and(|started| {
                    clock.schedule_now().saturating_sub(started) >= health_after
                }) && !job.timed_out
                {
                    let id = job.id.clone();
                    let operation = job.operation_id.clone();
                    self.store
                        .call(move |s| s.mark_funding_operation_timeout(&id, &operation))
                        .await?;
                    job.timed_out = true;
                }
                let [credit, status, source, _] = applicable_work(&job, &facts);
                let prerequisite = job.phase == FundingPhase::Allocated && !facts.hidden_prepared;
                let wanted = match kind {
                    Kind::Credit => credit,
                    Kind::Status => status,
                    Kind::Source => !prerequisite && source,
                    Kind::Quote => prerequisite && source,
                };
                if wanted {
                    pending.push_back(Pending { job, lane: kind });
                }
            }
        }
        Ok(())
    }

    fn dispatch_pending(
        &self,
        pending: &mut VecDeque<Pending>,
        flight: &mut InFlight<<B::Observations as FundingObservations>::QuoteRequest>,
        reader: &B::Observations,
        lease: &Lease,
        clock: &TickClock,
    ) {
        for _ in 0..pending.len() {
            let item = pending.pop_front().unwrap();
            let kind = match item.lane {
                Kind::Quote => Kind::Source,
                kind => kind,
            };
            if item.lane == Kind::Quote
                && (flight.quote_active == QUOTE_READS
                    || flight.quote_pending.is_some()
                    || flight.active.contains(&Key {
                        operation: item.job.operation_id.clone(),
                        kind: Kind::Quote,
                    }))
            {
                // The prerequisite lane keeps its own bounded page;
                // occupied quote HTTP slots cannot hold the source
                // reconciliation cursor behind unrelated allocations.
                pending.push_back(item);
                continue;
            }
            let key = Key {
                operation: item.job.operation_id.clone(),
                kind,
            };
            if flight.active.contains(&key) {
                continue;
            }
            // Source is one rolling serialized lane. It never parks
            // a page of status/credit behind an occupied treasury.
            if kind == Kind::Source && flight.source_active {
                // Retain only this source lane's bounded page. Other
                // lane cursors continue independently while owned work drains.
                pending.push_back(item);
                continue;
            }
            if (kind == Kind::Credit && flight.credit_active == CREDIT_READS)
                || (kind == Kind::Status && flight.status_active == STATUS_READS)
            {
                pending.push_back(item);
                continue;
            }
            let ready = {
                let mut state = lease.state.lock().expect("funding schedule lock");
                state.admit(&key, &item.job, clock.schedule_now(), &flight.active)
            };
            match ready {
                Some(false) => continue,
                None => {
                    pending.push_back(item);
                    continue;
                }
                Some(true) => {}
            }
            flight.active.insert(key);
            let job = item.job;
            match kind {
                Kind::Credit => {
                    flight.credit_active += 1;
                    let reader = reader.clone();
                    flight.reads.push(
                        async move {
                            let result =
                                reader.credit(job.clone()).await.map(|()| Observation::None);
                            Completion { job, kind, result }
                        }
                        .boxed(),
                    );
                }
                Kind::Status => {
                    flight.status_active += 1;
                    let reader = reader.clone();
                    let store = self.store.clone();
                    flight.reads.push(
                        async move {
                            let (id, operation) = (job.id.clone(), job.operation_id.clone());
                            let result = async {
                                let quote = store
                                    .call(move |s| {
                                        Ok(serde_json::from_slice::<Quote>(
                                            &s.funding_quote_for_operation(&id, &operation)?,
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
                    flight.source_active = true;
                    let worker = self.clone();
                    let lease = lease.clone();
                    // An accepted mutable step owns the lease even
                    // if an outer scheduler error drops its waiter.
                    let task = tokio::spawn(async move {
                        let _lease = lease;
                        let result = worker.source_step(&job).await.map(|request| {
                            request.map_or(Observation::None, Observation::Prerequisite)
                        });
                        Completion { job, kind, result }
                    });
                    flight.reads.push(
                        async move { task.await.expect("funding source owner panicked") }.boxed(),
                    );
                }
            }
        }
    }

    fn dispatch_quote(
        &self,
        flight: &mut InFlight<<B::Observations as FundingObservations>::QuoteRequest>,
        reader: &B::Observations,
        lease: &Lease,
        clock: &TickClock,
    ) {
        if flight.quote_active < QUOTE_READS
            && let Some((job, request)) = flight.quote_pending.take()
        {
            let key = Key {
                operation: job.operation_id.clone(),
                kind: Kind::Quote,
            };
            let admitted = lease.state.lock().expect("funding schedule lock").admit(
                &key,
                &job,
                clock.schedule_now(),
                &flight.active,
            );
            if admitted == Some(true) {
                flight.active.insert(key);
                flight.quote_active += 1;
                let reader = reader.clone();
                flight.reads.push(
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
                flight.quote_pending = Some((job, request));
            }
        }
    }

    async fn completion_current(
        &self,
        _lease: &Lease,
        completion: &Completion<<B::Observations as FundingObservations>::QuoteRequest>,
    ) -> Result<bool> {
        let id = completion.job.id.clone();
        #[cfg(test)]
        let fault = _lease.completion_fault.clone();
        #[cfg(test)]
        let kind = completion.kind;
        let current = self
            .store
            .call(move |s| {
                #[cfg(test)]
                {
                    let mut fault = fault.lock().unwrap();
                    if let Some((selected, code, remaining)) = fault.as_mut()
                        && *selected == kind
                        && *remaining > 0
                    {
                        *remaining -= 1;
                        return Err(rusqlite::Error::SqliteFailure(
                            rusqlite::ffi::Error::new(*code),
                            None,
                        )
                        .into());
                    }
                }
                s.funding_job(&id)
            })
            .await?;
        Ok(current.is_some_and(|current| current.operation_id == completion.job.operation_id))
    }

    #[cfg(test)]
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
        if !self.completion_current(lease, &completion).await? {
            return Ok(None);
        }
        self.publish_current_observation(lease, completion, instant)
            .await
    }

    async fn publish_current_observation(
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
        let (id, operation) = (job.id.clone(), job.operation_id.clone());
        let completion = self
            .store
            .call(move |s| {
                let Some(current) = s.funding_job(&id)? else {
                    return Ok(None);
                };
                if current.operation_id != operation {
                    return Ok(None);
                }
                // Completion consumes wallet-local readiness evidence. Its typed
                // rejection must not terminate observation of independent jobs.
                // Untyped/storage failures still stop the shared owner.
                let failure = match s.finish_funding_if_proven(&id, &operation) {
                    Ok(_) => None,
                    Err(error)
                        if error
                            .downcast_ref::<super::super::base::evidence::EvidenceFailure>()
                            == Some(
                                &super::super::base::evidence::EvidenceFailure::MalformedSupport,
                            ) =>
                    {
                        Some(error)
                    }
                    Err(error) => return Err(error),
                };
                let current = s.funding_job(&id)?.context("funding job missing")?;
                let facts = s.funding_facts(&id, &operation)?;
                let mut applicable = applicable_work(&current, &facts);
                // Even a terminal provider outcome cannot hide a rejected local
                // completion proof when it makes its observation lane unnecessary.
                applicable[kind as usize] |= failure.is_some();
                Ok(Some((applicable, failure)))
            })
            .await?;
        let Some((applicable, completion_failure)) = completion else {
            return Ok(None);
        };
        if completion_failure.is_some() {
            prerequisite = None;
        }
        let failure = completion_failure.or_else(|| result.err());
        let wait = failure
            .as_ref()
            .and_then(|error| waiting_reason(error, &phase));
        if let Some(reason) = wait {
            tracing::debug!(job_id=%job.id,pool_id=%job.pool_id,phase=?phase,reason,"funding awaiting chain confirmation; polling automatically");
        }
        let condition = failure.as_ref().and_then(funding_condition);
        let error = failure
            .filter(|_| wait.is_none())
            .map(|error| safe_error(&error, &phase));
        if let Some(category) = &error {
            let id = job.id.clone();
            let wallet_type = self.store.call(move |s| s.funding_wallet_type(&id)).await?;
            tracing::warn!(wallet=%job.pool_name,size=%wallet_type,phase=?phase,category,"Funding observation failed; existing work retained and observation will retry");
            tracing::debug!(job_id=%job.id,pool_id=%job.pool_id,kind=?kind,category,"Funding observation failure correlation");
        }
        let (next, streak, health, condition) = {
            let mut state = lease.state.lock().expect("funding schedule lock");
            state.complete(&job, kind, instant, self.poll_seconds, error, condition);
            let (next, streak, health) = state.health(&job.operation_id, instant, applicable);
            (
                next,
                streak,
                health,
                state.condition(&job.operation_id, applicable),
            )
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
                s.record_funding_observation_health(
                    &id,
                    &operation,
                    next,
                    streak,
                    health.as_deref(),
                )?;
                s.observe_funding_condition(&id, &operation, condition)?;
                let current = s.funding_job(&id)?.context("funding job missing")?;
                Ok(Some((current.phase, s.funding_wallet_type(&id)?)))
            })
            .await?;
        if let Some((phase, wallet_type)) = phase {
            let changed = lease
                .state
                .lock()
                .expect("funding schedule lock")
                .report_phase(&job.operation_id, &phase);
            if changed {
                // This is a current-state observation, not a database transition.
                // Sibling reads may have started with older job snapshots.
                tracing::info!(wallet=%job.pool_name,size=%wallet_type,status=?phase,"Funding progress");
                tracing::debug!(job_id=%job.id,pool_id=%job.pool_id,kind=?kind,observed_phase=?phase,"Funding progress correlation");
            }
        }
        Ok(prerequisite)
    }
}

impl State {
    fn report_phase(&mut self, operation: &str, phase: &FundingPhase) -> bool {
        let changed = !self.records.iter().any(|(key, record)| {
            key.operation == operation && record.reported_phase.as_ref() == Some(phase)
        });
        for (key, record) in &mut self.records {
            if key.operation == operation {
                record.reported_phase = Some(phase.clone());
            }
        }
        changed
    }

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
                reported_phase: None,
                due,
                streak: 0,
                error: None,
                condition: None,
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
        condition: Option<super::super::error::FundingCondition>,
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
        record.condition = condition;
    }
    fn condition(
        &self,
        operation: &str,
        applicable: [bool; 4],
    ) -> Option<super::super::error::FundingCondition> {
        [Kind::Source, Kind::Quote, Kind::Status, Kind::Credit]
            .into_iter()
            .filter(|kind| applicable[*kind as usize])
            .find_map(|kind| {
                self.records
                    .get(&Key {
                        operation: operation.to_owned(),
                        kind,
                    })
                    .and_then(|record| record.condition)
            })
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
        // Diagnostics remain presentation-only. Typed current observations are
        // independently retained in the same bounded scheduling records.
        let integrity = entries.iter().find_map(|(_, record)| {
            record
                .error
                .as_ref()
                .filter(|_| {
                    record.condition == Some(super::super::error::FundingCondition::EvidenceInvalid)
                })
                .cloned()
        });
        let error = integrity.or_else(|| {
            [Kind::Source, Kind::Quote, Kind::Status, Kind::Credit]
                .into_iter()
                .find_map(|kind| {
                    entries
                        .iter()
                        .find(|(key, record)| key.kind == kind && record.error.is_some())
                        .and_then(|(_, record)| record.error.clone())
                })
        });
        (due, streak, error)
    }
}

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod tests;
