//! One retained funding owner; observations never borrow the source backend.
use super::*;
use crate::rotation::store::inventory::read_result;
use crate::rotation::store::{
    funding::FundingFacts,
    scheduling::{FundingStatusRequest, FundingWork, MalformedFunding},
};
use futures_util::{FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};

const CREDIT_READS: usize = 4;
const STATUS_READS: usize = 4;
const QUOTE_READS: usize = 4;
// Recurring source, provider and destination observations each retain their own
// minute floor. Successful local preparation/submission handoffs remain immediate.
const OBSERVATION_POLL_SECONDS: u64 = 60;
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
enum RetrySchedule {
    Poll(u64, bool),
    At(u64),
}
struct Record {
    reported_phase: Option<FundingPhase>,
    due: u64,
    streak: u32,
    error: Option<String>,
    condition: Option<super::super::error::FundingCondition>,
}
#[derive(Debug, PartialEq, Eq)]
enum PhaseReport {
    Initial,
    Changed,
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

pub(super) enum Observation<Q> {
    None,
    Prepared,
    Credit(CreditObservation),
    Prerequisite(Q),
    Quote(Quote),
}
struct Completion<Q> {
    job: FundingJob,
    kind: Kind,
    result: Result<Observation<Q>>,
}
enum Completed<Q> {
    Work(Box<Completion<Q>>),
    Status {
        request: FundingStatusRequest,
        result: Result<SwapStatus>,
    },
}
impl<Q> Completed<Q> {
    fn identity(&self) -> (&str, &str, Kind) {
        match self {
            Self::Work(completion) => (
                &completion.job.id,
                &completion.job.operation_id,
                completion.kind,
            ),
            Self::Status { request, .. } => (&request.id, &request.operation_id, Kind::Status),
        }
    }
}
// Scheduling and health reporting evaluate the same current facts. The source
// step includes quote prerequisites; the Quote kind is the subsequent HTTP read.
fn applicable_work(job: &FundingJob, facts: &FundingFacts) -> [bool; 4] {
    [
        facts.credit_needed(),
        facts.status_needed(),
        !facts.provider_success
            && (facts.source_pending
                || (!facts.claim_fulfilled
                    && (facts.preparation_accepted
                        || crate::treasury::recovery::needs_recovery(job)
                        || matches!(job.phase, FundingPhase::Preparing | FundingPhase::Prepared)
                        || (matches!(job.phase, FundingPhase::Allocated | FundingPhase::Quoted)
                            && (facts.new_funding_allowed || facts.initial_credit_recorded))))),
        job.phase == FundingPhase::Allocated && facts.new_funding_allowed && !facts.accepted_quote,
    ]
}

fn report_decode_failure(fault: &MalformedFunding) {
    tracing::warn!(job_id=%fault.id,operation=%fault.operation,category="malformed_funding_inventory",
        "Saved wallet funding data could not be read; the affected operation is retained for inspection");
}

/// Report a new operator requirement once; polling details never change backoff.
fn report_observation_failure(
    job: &FundingJob,
    wallet_type: &str,
    kind: Kind,
    phase: &FundingPhase,
    error: &anyhow::Error,
    previous_error: Option<&str>,
) -> String {
    let category = safe_error(error, phase);
    // Presentation only: changing height details do not create a new disposition.
    let new_requirement = previous_error.and_then(|s| s.split("; treasury:").next())
        != category.split("; treasury:").next();
    if error.is::<RecoveryBlocked>() && new_requirement {
        tracing::warn!(wallet=%job.pool_name,size=wallet_type,observation=?kind,phase=?phase,
            operation=%job.operation_id,category,
            "Wallet funding requires operator review; existing work and reservations retained; observation checks continue");
    } else if new_requirement
        && (matches!(
            error.downcast_ref::<super::super::transaction::RecoveryEvidenceError>(),
            Some(
                super::super::transaction::RecoveryEvidenceError::ConfirmationMismatch
                    | super::super::transaction::RecoveryEvidenceError::ChainChanged
            )
        ) || error.is::<super::super::transaction::ConfirmationObservation>())
    {
        tracing::info!(wallet=%job.pool_name,size=wallet_type,job_id=%job.id,
            operation=%job.operation_id,category,
            "Wallet funding observations disagree with current blockchain state; the original work remains owned and canonical resolution will retry automatically");
    } else if new_requirement
        && funding_condition(error)
            == Some(super::super::error::FundingCondition::InsufficientFunds)
    {
        use super::super::error::TreasuryFunds;
        let address = error
            .downcast_ref::<super::super::error::TreasuryDepositAddress>()
            .map(|address| address.0.as_str());
        match TreasuryFunds::from_error(error) {
            TreasuryFunds::Waiting => {
                tracing::debug!(wallet=%job.pool_name,size=wallet_type,observation=?kind,phase=?phase,category,
                "Wallet funding is waiting for existing ZEC to become spendable; pending funds may still be insufficient; synchronization and funding retry automatically")
            }
            TreasuryFunds::Shortfall => {
                if let Some(address) = address {
                    tracing::info!(wallet=%job.pool_name,size=wallet_type,category,
                "Observed ZEC is insufficient for the next wallet funding deposit and fee; add ZEC to {address} and allow confirmations; synchronization and funding retry automatically");
                } else {
                    tracing::info!(wallet=%job.pool_name,size=wallet_type,category,
                "Observed ZEC is insufficient for the next wallet funding deposit and fee; run wallet status --config FILE with the serving configuration to find the deposit address; add ZEC and allow confirmations; synchronization and funding retry automatically");
                }
            }
            TreasuryFunds::Unavailable => {
                tracing::info!(wallet=%job.pool_name,size=wallet_type,category,
                "Treasury funds could not be assessed; inspect wallet status --config FILE for saved balances and synchronization progress; synchronization and funding retry automatically")
            }
        }
    } else if new_requirement
        && error.is::<super::super::transaction::PreparationDeferred>()
        && crate::treasury::diagnostics::sync_detail(error).is_some()
    {
        tracing::info!(wallet=%job.pool_name,size=wallet_type,category,
            "Wallet funding deferred until treasury synchronization succeeds");
    } else {
        tracing::debug!(wallet=%job.pool_name,size=wallet_type,observation=?kind,phase=?phase,
            operation=%job.operation_id,category,
            "Wallet funding observation unavailable; existing work retained and checks will retry");
    }
    tracing::debug!(job_id=%job.id,pool_id=%job.pool_id,kind=?kind,category,
        "Funding observation failure correlation");
    category
}

enum Pending {
    Work { job: FundingJob, lane: Kind },
    Status(FundingStatusRequest),
}
impl Pending {
    fn lane(&self) -> Kind {
        match self {
            Self::Work { lane, .. } => *lane,
            Self::Status(_) => Kind::Status,
        }
    }
    fn operation(&self) -> &str {
        match self {
            Self::Work { job, .. } => &job.operation_id,
            Self::Status(request) => &request.operation_id,
        }
    }
}

/// Each lane retains its own bounded inventory pass while sibling work runs.
struct LaneInventory {
    kind: Kind,
    through: i64,
    cursor: (String, String),
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
            cursor: (String::new(), String::new()),
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
    reads: FuturesUnordered<BoxFuture<'static, Completed<Q>>>,
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
            active: BTreeSet::new(),
            source_active: false,
            credit_active: 0,
            status_active: 0,
            quote_active: 0,
            quote_pending: None,
        }
    }

    fn release(&mut self, completion: &Completed<Q>) {
        let (_, operation, kind) = completion.identity();
        match kind {
            Kind::Credit => self.credit_active -= 1,
            Kind::Status => self.status_active -= 1,
            Kind::Source => self.source_active = false,
            Kind::Quote => self.quote_active -= 1,
        }
        self.active.remove(&Key {
            operation: operation.to_owned(),
            kind,
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
            started: tokio::time::Instant::now(),
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
                .call(move |s| {
                    let result = s.fill_funding_vacancies(&name);
                    s.report_blocked_reserves(&name)?;
                    result
                })
                .await
                .is_err()
            {
                tracing::debug!("funding vacancy allocation unavailable");
            }
        }
        let mut allocation_cursor = 0usize;
        let mut allocation_due = clock.now().saturating_add(self.poll_seconds.max(1));
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
        let pass_due = clock.now();
        let mut inventory = [Kind::Credit, Kind::Status, Kind::Source, Kind::Quote]
            .map(|kind| LaneInventory::new(kind, boundary, pass_due));
        let mut stopping = false;
        loop {
            stopping |= stop.is_cancelled();
            if !once && !stopping && !allocation_groups.is_empty() && clock.now() >= allocation_due
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
                    allocation_due = clock.now().saturating_add(self.poll_seconds.max(1));
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
                && flight.quote_pending.is_none()
            {
                return first_error.map_or(Ok(()), Err);
            }
            let needs_page = inventory.iter().any(|lane| {
                !lane.exhausted
                    && clock.now() >= lane.pass_due
                    && !pending.iter().any(|item| item.lane() == lane.kind)
            });
            let completion = if needs_page && !stopping {
                flight.reads.next().now_or_never().flatten()
            } else {
                tokio::select! {
                    completion = flight.reads.next(), if !flight.reads.is_empty() => completion,
                    _ = stop.cancelled(), if !stopping => {
                        stopping = true;
                        None
                    },
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)), if !stopping => None,
                }
            };
            if let Some(completion) = completion {
                flight.release(&completion);
                let (id, operation, _) = completion.identity();
                let (id, operation) = (id.to_owned(), operation.to_owned());
                let published = match completion {
                    Completed::Work(completion) => {
                        self.publish_observation(&lease, *completion, &clock).await
                    }
                    Completed::Status { request, result } => self
                        .publish_status(&lease, request, result, &clock)
                        .await
                        .map(|()| None),
                };
                match published {
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
            let waiting = pending.iter().any(|item| item.lane() == kind);
            if clock.now() < lane.pass_due {
                continue;
            }
            if !once && lane.exhausted && !waiting && clock.now() >= lane.pass_due {
                let Some(through) = read_result(
                    self.store.call(|s| s.funding_work_boundary()).await,
                    &mut lane.read_failure,
                )?
                else {
                    lane.pass_due = clock.now().saturating_add(self.poll_seconds.max(1));
                    continue;
                };
                lane.through = through;
                lane.cursor = (String::new(), String::new());
                lane.exhausted = false;
            }
            if lane.exhausted || waiting {
                continue;
            }
            if kind == Kind::Status {
                let after = lane.cursor.clone();
                let upper = lane.through;
                let page = self
                    .store
                    .call(move |s| s.funding_status_page(&after, upper))
                    .await;
                let Some(page) = read_result(page, &mut lane.read_failure)? else {
                    lane.pass_due = clock.now().saturating_add(self.poll_seconds.max(1));
                    lane.exhausted |= once;
                    continue;
                };
                if page.is_empty() {
                    lane.exhausted = true;
                    lane.pass_due = clock.now().saturating_add(self.poll_seconds.max(1));
                    continue;
                }
                #[cfg(test)]
                if let Some(hook) = &self.inventory_test_hook {
                    hook.captured(kind).await;
                }
                lane.cursor = page.last().unwrap().0.clone();
                for (_, entry) in page {
                    match entry {
                        Ok(Some(request)) => pending.push_back(Pending::Status(request)),
                        Ok(None) => {}
                        Err(fault) => {
                            report_decode_failure(&fault);
                            lane.decode_failure.get_or_insert(fault);
                        }
                    }
                }
                continue;
            }
            let after = lane.cursor.0.clone();
            let upper = lane.through;
            let page = self
                .store
                .call(move |s| s.funding_work_page(&after, upper))
                .await;
            let Some(page) = read_result(page, &mut lane.read_failure)? else {
                lane.pass_due = clock.now().saturating_add(self.poll_seconds.max(1));
                lane.exhausted |= once;
                continue;
            };
            if page.is_empty() {
                lane.exhausted = true;
                lane.pass_due = clock.now().saturating_add(self.poll_seconds.max(1));
                continue;
            }
            #[cfg(test)]
            if let Some(hook) = &self.inventory_test_hook {
                hook.captured(kind).await;
            }
            lane.cursor = (page.last().unwrap().0.clone(), String::new());
            for (_, entry) in page {
                let FundingWork { mut job, mut facts } = match entry {
                    Ok(work) => work,
                    Err(fault) => {
                        report_decode_failure(&fault);
                        lane.decode_failure.get_or_insert(fault);
                        continue;
                    }
                };
                if facts.provider_success && facts.source_pending {
                    let id = job.id.clone();
                    let operation = job.operation_id.clone();
                    let Some(updated) = self
                        .store
                        .call(move |s| s.settle_funding_source(&id, &operation, false))
                        .await?
                    else {
                        continue;
                    };
                    (job, facts) = updated;
                }
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
                let [credit, status, source, _] = applicable_work(&job, &facts);
                let wanted = match kind {
                    Kind::Credit => credit,
                    Kind::Status => status,
                    Kind::Source => job.phase != FundingPhase::Allocated && source,
                    Kind::Quote => job.phase == FundingPhase::Allocated && source,
                };
                if wanted {
                    pending.push_back(Pending::Work { job, lane: kind });
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
        let mut capacity_deferred = [false; 4];
        let mut metadata_deferred = false;
        for _ in 0..pending.len() {
            let item = pending.pop_front().unwrap();
            let kind = match item.lane() {
                Kind::Quote => Kind::Source,
                kind => kind,
            };
            if item.lane() == Kind::Quote
                && (flight.quote_active == QUOTE_READS
                    || flight.quote_pending.is_some()
                    || flight.active.contains(&Key {
                        operation: item.operation().to_owned(),
                        kind: Kind::Quote,
                    }))
            {
                capacity_deferred[Kind::Quote as usize] |=
                    flight.quote_active == QUOTE_READS || flight.quote_pending.is_some();
                // The prerequisite lane keeps its own bounded page;
                // occupied quote HTTP slots cannot hold the source
                // reconciliation cursor behind unrelated allocations.
                pending.push_back(item);
                continue;
            }
            let key = Key {
                operation: item.operation().to_owned(),
                kind,
            };
            if flight.active.contains(&key) {
                continue;
            }
            // Source is one rolling serialized lane. It never parks
            // a page of status/credit behind an occupied treasury.
            if kind == Kind::Source && flight.source_active {
                capacity_deferred[Kind::Source as usize] = true;
                // Retain only this source lane's bounded page. Other
                // lane cursors continue independently while owned work drains.
                pending.push_back(item);
                continue;
            }
            if (kind == Kind::Credit && flight.credit_active == CREDIT_READS)
                || (kind == Kind::Status && flight.status_active == STATUS_READS)
            {
                capacity_deferred[kind as usize] = true;
                pending.push_back(item);
                continue;
            }
            let ready = {
                let mut state = lease.state.lock().expect("funding schedule lock");
                state.admit(
                    &key,
                    match &item {
                        Pending::Work { job, .. } => Some(&job.phase),
                        Pending::Status(_) => None,
                    },
                    clock.now(),
                    &flight.active,
                )
            };
            match ready {
                Some(false) => continue,
                None => {
                    metadata_deferred = true;
                    pending.push_back(item);
                    continue;
                }
                Some(true) => {}
            }
            flight.active.insert(key);
            if let Pending::Status(request) = item {
                flight.status_active += 1;
                let reader = reader.clone();
                let store = self.store.clone();
                flight.reads.push(
                        async move {
                            let (id, operation) = (request.id.clone(), request.operation_id.clone());
                            let result = async {
                                let (quote, retained) = store
                                    .call(move |s| {
                                        let (bytes, retained) = s.funding_status_request(&id, &operation)?;
                                        Ok((serde_json::from_slice::<Quote>(&bytes)?, retained))
                                    })
                                    .await?;
                                if retained {
                                    tracing::debug!(wallet=%request.pool_name,wallet_id=%request.wallet_id,
                                        job_id=%request.id,operation=%request.operation_id,
                                        "Checking an earlier funding request that still awaits a final provider outcome; possible late credit keeps wallet retirement deferred");
                                }
                                reader.status(quote)
                                    .instrument(tracing::debug_span!("funding_status", wallet=%request.pool_name,
                                        job_id=%request.id, operation=%request.operation_id))
                                    .await
                            }
                            .await;
                            Completed::Status { request, result }
                        }
                        .boxed(),
                    );
                continue;
            }
            let Pending::Work { job, .. } = item else {
                unreachable!()
            };
            match kind {
                Kind::Credit => {
                    flight.credit_active += 1;
                    let reader = reader.clone();
                    flight.reads.push(
                        async move {
                            let result = Ok(Observation::Credit(
                                reader.credit_observation(job.clone()).await,
                            ));
                            Completed::Work(Box::new(Completion { job, kind, result }))
                        }
                        .boxed(),
                    );
                }
                Kind::Status => unreachable!(),
                Kind::Quote => unreachable!(),
                Kind::Source => {
                    flight.source_active = true;
                    let worker = self.clone();
                    let lease = lease.clone();
                    let epoch = clock.now();
                    // An accepted mutable step owns the lease even
                    // if an outer scheduler error drops its waiter.
                    let task = tokio::spawn(async move {
                        let _lease = lease;
                        let source_clock = TickClock {
                            epoch,
                            started: tokio::time::Instant::now(),
                        };
                        let result = worker.source_step(&job, &source_clock).await;
                        Completion { job, kind, result }
                    });
                    flight.reads.push(
                        async move {
                            Completed::Work(Box::new(
                                task.await.expect("funding source owner panicked"),
                            ))
                        }
                        .boxed(),
                    );
                }
            }
        }
        if capacity_deferred.into_iter().any(|deferred| deferred) || metadata_deferred {
            tracing::debug!(
                credit_capacity = CREDIT_READS,
                status_capacity = STATUS_READS,
                quote_capacity = QUOTE_READS,
                source_capacity = 1,
                quote_prerequisite_capacity = 1,
                metadata_capacity_per_kind = RECORDS,
                credit_deferred = capacity_deferred[Kind::Credit as usize],
                status_deferred = capacity_deferred[Kind::Status as usize],
                source_deferred = capacity_deferred[Kind::Source as usize],
                quote_deferred = capacity_deferred[Kind::Quote as usize],
                metadata_deferred,
                "Funding observation capacity reached; pending work retained for a later pass"
            );
        }
    }

    fn dispatch_quote(
        &self,
        flight: &mut InFlight<<B::Observations as FundingObservations>::QuoteRequest>,
        reader: &B::Observations,
        lease: &Lease,
        clock: &TickClock,
    ) {
        if flight.quote_active == QUOTE_READS && flight.quote_pending.is_some() {
            tracing::debug!(
                limit = QUOTE_READS,
                "Funding quote capacity reached; pending prerequisite retained"
            );
        }
        if flight.quote_active < QUOTE_READS
            && let Some((job, request)) = flight.quote_pending.take()
        {
            let key = Key {
                operation: job.operation_id.clone(),
                kind: Kind::Quote,
            };
            let admitted = lease.state.lock().expect("funding schedule lock").admit(
                &key,
                Some(&job.phase),
                clock.now(),
                &flight.active,
            );
            if admitted == Some(true) {
                flight.active.insert(key);
                flight.quote_active += 1;
                let reader = reader.clone();
                flight.reads.push(
                    async move {
                        let result = reader.quote(request).await.map(Observation::Quote);
                        Completed::Work(Box::new(Completion {
                            job,
                            kind: Kind::Quote,
                            result,
                        }))
                    }
                    .boxed(),
                );
            } else {
                if admitted.is_none() {
                    tracing::debug!(
                        limit_records = RECORDS,
                        "Funding scheduling metadata capacity reached; pending quote retained"
                    );
                }
                flight.quote_pending = Some((job, request));
            }
        }
    }

    async fn publish_status(
        &self,
        lease: &Lease,
        request: FundingStatusRequest,
        result: Result<SwapStatus>,
        clock: &TickClock,
    ) -> Result<()> {
        let instant = clock.now();
        let observed = result.as_ref().ok().copied();
        if let Some(status) = observed {
            tracing::debug!(wallet=%request.pool_name,job_id=%request.id,
                operation=%request.operation_id,near_status=?status,
                "NEAR swap status response validated");
        }
        let result = match result {
            Ok(status) => {
                let (id, operation) = (request.id.clone(), request.operation_id.clone());
                self.store
                    .call(move |s| s.record_provider_outcome(&id, &operation, &status, instant))
                    .await
            }
            Err(error) => Err(error),
        };
        // Every accepted request completes its cadence before any current-job
        // reporting. A rollover during publication cannot lose this request's floor.
        let condition = result.as_ref().err().and_then(funding_condition);
        let error = result.as_ref().err().map(|error| {
            let category = safe_error(error, &FundingPhase::Swapping);
            tracing::debug!(wallet=%request.pool_name,wallet_id=%request.wallet_id,
                job_id=%request.id,operation=%request.operation_id,category,
                "Wallet funding request observation unavailable; the accepted request is retained and checks will retry");
            category
        });
        let previously_unavailable = {
            let mut state = lease.state.lock().expect("funding schedule lock");
            let previous = state
                .records
                .get(&Key {
                    operation: request.operation_id.clone(),
                    kind: Kind::Status,
                })
                .is_some_and(|record| record.error.is_some());
            state.complete(
                (&request.id, &request.operation_id),
                Kind::Status,
                instant,
                RetrySchedule::Poll(self.poll_seconds, false),
                error.clone(),
                condition,
            );
            previous
        };
        report_status_result(&request, observed, error.as_deref(), previously_unavailable);
        let (id, operation) = (request.id.clone(), request.operation_id.clone());
        let current = self
            .store
            .call(move |s| {
                if s.funding_current_operation(&id)? != operation {
                    return Ok(None);
                }
                s.finish_funding_if_proven(&id, &operation)?;
                let job = s.funding_job(&id)?.context("funding job missing")?;
                let facts = s.funding_facts(&id, &operation)?;
                Ok(Some((job, facts)))
            })
            .await?;
        if let Some((job, facts)) = current {
            self.publish_health(
                lease,
                &job,
                Kind::Status,
                applicable_work(&job, &facts),
                instant,
            )
            .await?;
        }
        Ok(())
    }

    async fn publish_observation(
        &self,
        lease: &Lease,
        completion: Completion<<B::Observations as FundingObservations>::QuoteRequest>,
        clock: &TickClock,
    ) -> Result<
        Option<(
            FundingJob,
            <B::Observations as FundingObservations>::QuoteRequest,
        )>,
    > {
        let instant = clock.now();
        let Completion { job, kind, result } = completion;
        let id = job.id.clone();
        let current = self.store.call(move |s| s.funding_job(&id)).await?;
        if current
            .as_ref()
            .is_none_or(|current| current.operation_id != job.operation_id)
        {
            return Ok(None);
        }

        let mut prerequisite = None;
        let mut credit_due = None;
        let prepared = matches!(&result, Ok(Observation::Prepared));
        let result = match result {
            Ok(Observation::Credit(observation)) => {
                // Map from the existing monotonic root, rounding outward once.
                // Relative whole-second offsets can wake before a fractional due
                // time and count a cached failure again without a new acquisition.
                credit_due = observation.next_due.map(|due| clock.ceil_epoch(due));
                observation.result
            }
            Ok(Observation::None | Observation::Prepared) => Ok(()),
            Ok(Observation::Prerequisite(request)) => {
                prerequisite = Some((job.clone(), request));
                Ok(())
            }
            Ok(Observation::Quote(quote)) => {
                let (id, operation) = (job.id.clone(), job.operation_id.clone());
                self.store
                    .call(move |s| {
                        ensure!(quote.deposit.is_some(), "cannot fund from dry quote");
                        let target = quote.output_atomic()?.to_string();
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
        let successful = result.is_ok();
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
                // The initial-credit claim is a scalar fact. Storage failures
                // belong to this shared serialized owner.
                s.finish_funding_if_proven(&id, &operation)?;
                let current = s.funding_job(&id)?.context("funding job missing")?;
                let facts = s.funding_facts(&id, &operation)?;
                // Only these successful local publications advance
                // the next source step. Provider/credit progress and arbitrary
                // display-phase changes never shorten source observation waits.
                let handoff = successful
                    && facts.new_funding_allowed
                    && match kind {
                        Kind::Quote => {
                            current.phase == FundingPhase::Quoted && !facts.preparation_accepted
                        }
                        Kind::Source => {
                            prepared
                                && facts.source_pending
                                && facts.source_submission.as_deref() == Some("PREPARED")
                        }
                        _ => false,
                    };
                Ok(Some((applicable_work(&current, &facts), handoff)))
            })
            .await?;
        let Some((applicable, handoff)) = completion else {
            return Ok(None);
        };
        let failure = result.err();
        let wait = failure
            .as_ref()
            .and_then(|error| waiting_reason(error, &phase));
        if let Some(reason) = wait {
            tracing::debug!(job_id=%job.id,pool_id=%job.pool_id,phase=?phase,reason,"funding awaiting chain confirmation; polling automatically");
        }
        let condition = failure.as_ref().and_then(funding_condition);
        let error = match failure.filter(|_| wait.is_none()) {
            Some(error) => {
                let id = job.id.clone();
                let wallet_type = self.store.call(move |s| s.funding_wallet_type(&id)).await?;
                let previous_error = lease.state.lock().expect("funding schedule lock").records
                    [&Key {
                        operation: job.operation_id.clone(),
                        kind,
                    }]
                    .error
                    .clone();
                Some(report_observation_failure(
                    &job,
                    &wallet_type,
                    kind,
                    &phase,
                    &error,
                    previous_error.as_deref(),
                ))
            }
            None => None,
        };
        {
            let mut state = lease.state.lock().expect("funding schedule lock");
            state.complete(
                (&job.id, &job.operation_id),
                kind,
                instant,
                credit_due.map_or(
                    RetrySchedule::Poll(self.poll_seconds, job.timed_out),
                    RetrySchedule::At,
                ),
                error,
                condition,
            );
            if handoff {
                state.advance_source(&job.operation_id, instant);
            }
        }
        self.publish_health(lease, &job, kind, applicable, instant)
            .await?;
        Ok(prerequisite)
    }

    /// Health is reporting for a current operation. The acquisition has already
    /// completed its own cadence independently of this optional publication.
    async fn publish_health(
        &self,
        lease: &Lease,
        job: &FundingJob,
        kind: Kind,
        applicable: [bool; 4],
        instant: u64,
    ) -> Result<()> {
        let (next, streak, health, condition) = {
            let state = lease.state.lock().expect("funding schedule lock");
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
                if s.funding_current_operation(&id)? != operation {
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
            let report = lease
                .state
                .lock()
                .expect("funding schedule lock")
                .report_phase(&job.operation_id, &phase);
            if let Some(report) = report {
                // This is a current-state observation, not a database transition.
                // Sibling reads may have started with older job snapshots.
                match report {
                    PhaseReport::Initial => {
                        tracing::info!(wallet=%job.pool_name,size=%wallet_type,status=?phase,
                        "Current saved funding phase; NEAR status is checked separately")
                    }
                    PhaseReport::Changed => {
                        tracing::info!(wallet=%job.pool_name,size=%wallet_type,status=?phase,"Funding progress")
                    }
                }
                tracing::debug!(job_id=%job.id,pool_id=%job.pool_id,kind=?kind,observed_phase=?phase,"Funding progress correlation");
            }
        }
        Ok(())
    }
}

/// Report observation availability separately from the local workflow phase.
/// Only a successfully saved response can announce recovery or a final outcome.
fn report_status_result(
    request: &FundingStatusRequest,
    observed: Option<SwapStatus>,
    error: Option<&str>,
    previously_unavailable: bool,
) {
    if let Some(category) = error {
        if !previously_unavailable {
            let stage = if observed.is_some() {
                "publication"
            } else {
                "acquisition"
            };
            tracing::info!(wallet=%request.pool_name,operation=%request.operation_id,stage,category,
                "NEAR swap status check could not complete; the accepted request is retained and checks will retry");
        }
        return;
    }
    if previously_unavailable {
        tracing::info!(wallet=%request.pool_name,operation=%request.operation_id,
            "NEAR swap status checks resumed successfully");
    }
    let message = match observed {
        Some(SwapStatus::Success) => {
            "NEAR reported swap success; destination credit is verified separately"
        }
        Some(SwapStatus::Refunded) => "NEAR reported a refund; wallet scans reconcile returned ZEC",
        Some(SwapStatus::Failed) => {
            "NEAR reported swap failure; source and refund reconciliation continues"
        }
        _ => return,
    };
    tracing::info!(wallet=%request.pool_name,operation=%request.operation_id,"{message}");
}

impl State {
    /// A successful local handoff can advance an earlier successful source step,
    /// never erase a source observation failure or its independent backoff.
    fn advance_source(&mut self, operation: &str, now: u64) {
        if let Some(record) = self.records.get_mut(&Key {
            operation: operation.to_owned(),
            kind: Kind::Source,
        }) && record.error.is_none()
            && record.streak == 0
        {
            record.due = record.due.min(now);
        }
    }
    fn report_phase(&mut self, operation: &str, phase: &FundingPhase) -> Option<PhaseReport> {
        let previous = self
            .records
            .iter()
            .filter(|(key, _)| key.operation == operation)
            .find_map(|(_, record)| record.reported_phase.as_ref());
        let report = match previous {
            Some(previous) if previous == phase => None,
            Some(_) => Some(PhaseReport::Changed),
            None => Some(PhaseReport::Initial),
        };
        for (key, record) in &mut self.records {
            if key.operation == operation {
                record.reported_phase = Some(phase.clone());
            }
        }
        report
    }

    fn admit(
        &mut self,
        key: &Key,
        phase: Option<&FundingPhase>,
        now: u64,
        active: &BTreeSet<Key>,
    ) -> Option<bool> {
        if key.kind == Kind::Source
            && phase == Some(&FundingPhase::Allocated)
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
            tracing::debug!(limit_records = RECORDS, observation = ?key.kind,
                "Funding scheduling metadata capacity reached; due inactive record evicted");
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
        identity: (&str, &str),
        kind: Kind,
        now: u64,
        schedule: RetrySchedule,
        error: Option<String>,
        condition: Option<super::super::error::FundingCondition>,
    ) {
        let key = Key {
            operation: identity.1.to_owned(),
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
        record.due = match schedule {
            RetrySchedule::At(due) => due,
            RetrySchedule::Poll(poll, timed_out) => {
                let delay = poll
                    .saturating_mul(1u64 << record.streak.min(6))
                    .min(300)
                    .max(if kind != Kind::Quote {
                        OBSERVATION_POLL_SECONDS
                    } else if timed_out {
                        60
                    } else {
                        1
                    });
                let jitter =
                    identity.0.bytes().map(u64::from).sum::<u64>() % poll.saturating_add(1).max(1);
                now.saturating_add(delay).saturating_add(jitter)
            }
        };
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
