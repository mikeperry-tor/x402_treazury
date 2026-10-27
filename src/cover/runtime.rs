//! Lifecycle-owned cover tasks on the existing identity-bound HTTP client.
use super::{
    Config, Limits, Scope,
    budget::Budget,
    episode::Episode,
    range::{self, Range, Representation},
    registry::{Capability, Owner, Registry},
    sampling::Unit,
};
use anyhow::Result;
use rand::{Rng, SeedableRng, rngs::StdRng};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::Notify, task::JoinSet, time::Instant};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

mod diagnostics;

pub struct Engine {
    limits: Limits,
    metrics: super::metrics::Shared,
    summary_emitted: std::sync::atomic::AtomicBool,
    registry: Mutex<Registry>,
    budget: Budget,
    rng: Mutex<Option<StdRng>>,
    tasks: TaskTracker,
    stop: CancellationToken,
    // Observe receipt without retaining the range guard or its budget permit.
    #[cfg(test)]
    last_range_counter: Mutex<Option<Arc<std::sync::atomic::AtomicU64>>>,
}
#[derive(Default)]
struct RequestGate {
    paused: usize,
    handles: Vec<tokio::task::AbortHandle>,
}
pub struct Session {
    id: String,
    gate: Mutex<RequestGate>,
    quiet: Notify,
    episode: Arc<Mutex<Episode>>,
    config: Arc<Config>,
    scopes: Mutex<BTreeSet<Scope>>,
    notify: Notify,
    padding_disabled: Arc<Mutex<bool>>,
    http1_allowed: bool,
}
/// One lease per real attempt; a successful completion permits a bounded tail.
pub struct Call {
    engine: Arc<Engine>,
    pub session: Arc<Session>,
    completed: bool,
    request_pending: bool,
}
type RangeCompletion = (bool, range::Outcome);

impl Engine {
    pub fn new(limits: Limits) -> Arc<Self> {
        Arc::new(Self {
            limits: limits.clone(),
            metrics: Default::default(),
            summary_emitted: std::sync::atomic::AtomicBool::new(false),
            registry: Mutex::new(Registry::new(limits.clone())),
            budget: Budget::new(limits),
            rng: Mutex::new(None),
            tasks: TaskTracker::new(),
            stop: CancellationToken::new(),
            #[cfg(test)]
            last_range_counter: Mutex::new(None),
        })
    }
    fn random<T>(&self, f: impl FnOnce(&mut StdRng) -> Result<T>) -> Result<T> {
        let mut rng = self.rng.lock().unwrap();
        f(rng.get_or_insert_with(StdRng::from_os_rng))
    }
    pub fn metrics(&self) -> super::metrics::Metrics {
        self.metrics.lock().unwrap().clone()
    }
    #[cfg(test)]
    pub(crate) async fn wait_experiment_idle(&self) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while !self.tasks.is_empty() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("cover experiment exceeded its cleanup deadline");
    }
    #[cfg(test)]
    pub(crate) fn reset_experiment(&self, seed: u64) {
        assert!(
            self.tasks.is_empty(),
            "fixture must wait for cover task cleanup before resetting measurements"
        );
        *self.metrics.lock().unwrap() = Default::default();
        *self.rng.lock().unwrap() = Some(StdRng::seed_from_u64(seed));
    }
    fn record(&self, session: &Session, reason: &'static str) {
        let level = match reason {
            "cover_preempted_for_api"
            | "cover_padding_disabled"
            | "cover_shutdown"
            | "cover_range_size_adjusted"
            | "cover_padding_http1_compatibility"
            | "cover_episode_padding_limit" => tracing::Level::DEBUG,
            "cover_header_list_limit" | "cover_padding_limit" | "cover_history_limit" => {
                tracing::Level::WARN
            }
            _ => tracing::Level::INFO,
        };
        diagnostics::status(session, &self.limits, reason, level);
    }

    pub fn begin(
        self: &Arc<Self>,
        owner: Owner,
        config: Arc<Config>,
        scope: Scope,
        http: reqwest::Client,
    ) -> Result<Call> {
        anyhow::ensure!(!self.stop.is_cancelled(), "cover_shutdown");
        config.validate()?;
        config.validate_origin(&owner.origin)?;
        let mut registry = self.registry.lock().unwrap();
        anyhow::ensure!(!self.stop.is_cancelled(), "cover_shutdown");
        let (episode, fresh) =
            self.random(|rng| registry.attach(owner.clone(), config.clone(), Instant::now(), rng))?;
        let state = registry.owners.get_mut(&owner).unwrap();
        let session = if fresh {
            let s = Arc::new(Session {
                id: uuid::Uuid::new_v4().to_string(),
                gate: Mutex::new(RequestGate::default()),
                quiet: Notify::new(),
                episode,
                config,
                scopes: Mutex::new(BTreeSet::new()),
                notify: Notify::new(),
                padding_disabled: state.padding_disabled.clone(),
                http1_allowed: owner.transport.allow_http1,
            });
            state.session = Some(s.clone());
            s
        } else {
            state
                .session
                .as_ref()
                .expect("active cover session")
                .clone()
        };
        {
            let mut gate = session.gate.lock().unwrap();
            gate.paused += 1;
            for handle in &gate.handles {
                handle.abort();
            }
        }
        session.notify.notify_one();
        session.scopes.lock().unwrap().insert(scope.clone());
        tracing::debug!(listener = %scope.listener, source = %scope.source, code = "cover_episode_active", "optional cover episode active");
        if let Capability::Unavailable(code) = state.capability {
            diagnostics::cached(&session, code, state.resource_label());
        }
        if *state.padding_disabled.lock().unwrap() {
            self.record(&session, "cover_padding_disabled");
        }
        if fresh {
            self.metrics.lock().unwrap().episodes += 1;
        }
        if fresh && session.config.ranges_enabled {
            let engine = self.clone();
            let s = session.clone();
            self.tasks.spawn(async move {
                engine.run(owner, s, http).await;
            });
        }
        drop(registry);
        Ok(Call {
            engine: self.clone(),
            session,
            completed: false,
            request_pending: true,
        })
    }
    pub async fn shutdown(&self) {
        self.stop_ranges().await;
        self.emit_summary();
    }
    pub async fn stop_ranges(&self) {
        {
            let _registry = self.registry.lock().unwrap();
            self.stop.cancel();
            self.tasks.close();
        }
        tracing::info!(
            "Stopping optional cover tasks; financial requests retain their safety drain"
        );
        self.tasks.wait().await;
    }
    pub fn emit_summary(&self) {
        if self
            .summary_emitted
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }
        eprintln!(
            "{}{}",
            super::metrics::REPORT_PREFIX,
            serde_json::json!({"version":1,"connection_affinity":"pooled_best_effort_unobserved","metrics":*self.metrics.lock().unwrap()})
        );
    }
    fn choose_range(
        &self,
        owner: &Owner,
        s: &Arc<Session>,
        available: u64,
    ) -> Result<Option<(Range, bool)>, &'static str> {
        let capability = self.registry.lock().unwrap().owners[owner]
            .capability
            .clone();
        let choice = match capability {
            Capability::Unknown => (Range::qualification(&s.config), true),
            Capability::Available { length, validator } => {
                let requested = match self.random(|r| s.config.ranges.sample(Unit::Bytes, r)) {
                    Ok(n) => n,
                    Err(_) => {
                        return Err("cover_sampling_exhausted");
                    }
                };
                let length_request = requested.min(length).min(available);
                if length_request != requested {
                    tracing::debug!(
                        code = "cover_range_size_adjusted",
                        sampled_bytes = requested,
                        admitted_bytes = length_request,
                        "optional range reduced to resource/remaining episode capacity"
                    );
                    self.record(s, "cover_range_size_adjusted");
                }
                let start = self
                    .random(|r| Ok(r.random_range(0..=length - length_request)))
                    .unwrap();
                (
                    Range {
                        start,
                        length: length_request,
                        expected: Some(Representation { length, validator }),
                    },
                    false,
                )
            }
            Capability::Unavailable(code) => {
                return Err(code);
            }
            Capability::Checking => {
                return Ok(None);
            }
        };
        Ok(Some(choice))
    }
    fn dispatch_range(
        &self,
        owner: &Owner,
        s: &Arc<Session>,
        http: &reqwest::Client,
        pending: &mut JoinSet<RangeCompletion>,
        (range, qualifying): (Range, bool),
        (now, deadline): (Instant, Instant),
    ) -> Result<bool, &'static str> {
        let reserved = match s
            .episode
            .lock()
            .unwrap()
            .reserve(&s.config, now, range.length)
        {
            Ok(n) => n,
            Err(_) => {
                return Err("cover_episode_limit");
            }
        };
        let budget = match self.budget.reserve(now, reserved, 0, true) {
            Ok(b) => b,
            Err(e) => {
                s.episode.lock().unwrap().finish(reserved, 0, false);
                return Err(e.code());
            }
        };
        if qualifying {
            self.registry
                .lock()
                .unwrap()
                .owners
                .get_mut(owner)
                .unwrap()
                .begin_qualification();
        }
        let mut range = range;
        if reserved != range.length {
            tracing::debug!(
                code = "cover_range_size_adjusted",
                requested_bytes = range.length,
                admitted_bytes = reserved,
                "optional qualification reduced to remaining episode capacity"
            );
            self.record(s, "cover_range_size_adjusted");
        }
        range.length = reserved;
        let (resource_url, resource) = {
            let registry = self.registry.lock().unwrap();
            let state = &registry.owners[owner];
            (state.resource_url().to_owned(), state.resource_label())
        };
        let mut padding_request = reqwest::Request::new(
            reqwest::Method::GET,
            resource_url.parse().expect("validated cover URL"),
        );
        padding_request.headers_mut().insert(
            reqwest::header::RANGE,
            format!("bytes={}-{}", range.start, range.start + reserved - 1)
                .parse()
                .unwrap(),
        );
        padding_request.headers_mut().insert(
            reqwest::header::ACCEPT_ENCODING,
            reqwest::header::HeaderValue::from_static("identity"),
        );
        let padding_budget = self.pad(s, &mut padding_request, false);
        let padding = s.config.padding.as_ref().and_then(|p| {
            padding_request
                .headers()
                .get(p.header_name.as_str())
                .cloned()
                .map(|v| (p.header_name.parse().unwrap(), v))
        });
        let counter = budget.counter();
        #[cfg(test)]
        {
            *self.last_range_counter.lock().unwrap() = Some(counter.clone());
        }
        let mut guard = RangeGuard {
            metrics: self.metrics.clone(),
            outcome: None,
            dispatched: false,
            session: s.clone(),
            reserved,
            counter,
            resource,
            qualifying,
            range_start: range.start,
            started: Instant::now(),
        };
        let mut cfg = (*s.config).clone();
        cfg.url = resource_url;
        cfg.fallback_url = None;
        let client = http.clone();
        let mut gate = s.gate.lock().unwrap();
        if gate.paused > 0 {
            drop(gate);
            if qualifying {
                self.registry
                    .lock()
                    .unwrap()
                    .owners
                    .get_mut(owner)
                    .unwrap()
                    .capability = Capability::Unknown;
            }
            drop(guard);
            drop(budget);
            return Ok(false);
        }
        gate.handles.retain(|h| !h.is_finished());
        let handle = pending.spawn(async move {
            guard.dispatched = true;
            {
                let mut m = guard.metrics.lock().unwrap();
                m.in_flight_ranges += 1;
                m.peak_in_flight_ranges = m.peak_in_flight_ranges.max(m.in_flight_ranges);
            }
            let mut padding_budget = padding_budget;
            if let Some(p) = &mut padding_budget {
                p.dispatched();
            }
            let result =
                range::download_observed(&client, &cfg, range, deadline, budget, padding).await;
            diagnostics::attempt(
                &guard.session,
                guard.resource,
                qualifying,
                &result,
                None,
                false,
            );
            guard.outcome = Some(
                result
                    .result
                    .as_ref()
                    .map(|_| "qualified")
                    .unwrap_or_else(|e| e.0),
            );
            drop(guard);
            (qualifying, result)
        });
        gate.handles.push(handle);
        drop(gate);
        Ok(true)
    }
    fn complete_range(
        &self,
        owner: &Owner,
        s: &Arc<Session>,
        item: Option<Result<RangeCompletion, tokio::task::JoinError>>,
    ) -> Result<Option<Instant>, &'static str> {
        match item {
            Some(Ok((qualifying, outcome))) => {
                let result = &outcome.result;
                let resource = self.registry.lock().unwrap().owners[owner].resource_label();
                let mut next = None;
                // This is the local optional-work deadline, not a resource refusal.
                // A newly attached call may already have cleared its sampled tail.
                if matches!(result, Err(range::Failure("cover_deadline"))) {
                    let capability = {
                        let mut registry = self.registry.lock().unwrap();
                        let state = registry.owners.get_mut(owner).unwrap();
                        if matches!(state.capability, Capability::Checking) {
                            state.capability = Capability::Unknown;
                        }
                        state.capability.clone()
                    };
                    diagnostics::attempt(
                        s,
                        resource,
                        qualifying,
                        &outcome,
                        Some(&capability),
                        false,
                    );
                    return Ok(Some(Instant::now()));
                }
                if qualifying {
                    if let Err(error) = result {
                        let switched = self
                            .registry
                            .lock()
                            .unwrap()
                            .owners
                            .get_mut(owner)
                            .unwrap()
                            .try_fallback(*error);
                        if switched {
                            diagnostics::attempt(
                                s,
                                resource,
                                qualifying,
                                &outcome,
                                Some(&Capability::Unknown),
                                true,
                            );
                            diagnostics::status(
                                s,
                                &self.limits,
                                "cover_fallback_selected",
                                tracing::Level::INFO,
                            );
                            return Ok(Some(Instant::now()));
                        }
                    }
                    self.registry
                        .lock()
                        .unwrap()
                        .owners
                        .get_mut(owner)
                        .unwrap()
                        .qualified(outcome.result.clone());
                    next = Some(Instant::now());
                }
                if let Err(error) = result {
                    self.registry
                        .lock()
                        .unwrap()
                        .owners
                        .get_mut(owner)
                        .unwrap()
                        .capability = Capability::Unavailable(error.0);
                    diagnostics::attempt(
                        s,
                        resource,
                        qualifying,
                        &outcome,
                        Some(&Capability::Unavailable(error.0)),
                        false,
                    );
                    return Err(error.0);
                }
                let capability = self.registry.lock().unwrap().owners[owner]
                    .capability
                    .clone();
                diagnostics::attempt(s, resource, qualifying, &outcome, Some(&capability), false);
                Ok(next)
            }
            Some(Err(error)) if error.is_cancelled() => {
                // A joined cancellation proves the guard released its reservation
                // and accounted observed bytes. Only now may qualification resume.
                let mut registry = self.registry.lock().unwrap();
                let state = registry.owners.get_mut(owner).unwrap();
                if matches!(state.capability, Capability::Checking) {
                    state.capability = Capability::Unknown;
                }
                drop(registry);
                self.record(s, "cover_preempted_for_api");
                Ok(Some(Instant::now()))
            }
            _ => Err("cover_task_failed"),
        }
    }
    async fn drain_ranges(
        &self,
        owner: &Owner,
        s: &Arc<Session>,
        pending: &mut JoinSet<RangeCompletion>,
    ) -> (Capability, &'static str) {
        pending.abort_all();
        while let Some(item) = pending.join_next().await {
            // Preserve completed facts even when the scheduler's deadline branch
            // won the race. Cleanup never dispatches a fallback or another range.
            let _ = self.complete_range(owner, s, Some(item));
        }
        let mut registry = self.registry.lock().unwrap();
        let state = registry.owners.get_mut(owner).unwrap();
        if matches!(state.capability, Capability::Checking) {
            state.capability = Capability::Unknown;
        }
        (state.capability.clone(), state.resource_label())
    }
    async fn run(self: Arc<Self>, owner: Owner, s: Arc<Session>, http: reqwest::Client) {
        let mut pending = JoinSet::new();
        let mut next = s.episode.lock().unwrap().first_dispatch;
        let reason;
        loop {
            let now = Instant::now();
            let (deadline, streams, available, requests) = {
                let e = s.episode.lock().unwrap();
                (
                    e.tail_deadline.unwrap_or(e.deadline).min(e.deadline),
                    e.streams,
                    e.target
                        .saturating_sub(e.received)
                        .saturating_sub(e.reserved),
                    e.requests,
                )
            };
            if s.episode.lock().unwrap().stopped.is_some() {
                reason = "cover_episode_stopped";
                break;
            }
            if self.stop.is_cancelled() {
                reason = "cover_shutdown";
                break;
            }
            if now >= deadline {
                reason = "cover_episode_deadline";
                break;
            }
            if available == 0 && pending.is_empty() {
                reason = "cover_budget_completed";
                break;
            }
            if requests >= s.config.max_requests_per_episode && pending.is_empty() {
                reason = "cover_episode_request_limit";
                break;
            }
            let paused = s.gate.lock().unwrap().paused > 0;
            let can_schedule = !paused
                && now >= next
                && streams < s.config.concurrency
                && available > 0
                && requests < s.config.max_requests_per_episode;
            if can_schedule {
                let (range, qualifying) = match self.choose_range(&owner, &s, available) {
                    Ok(Some(choice)) => choice,
                    Ok(None) => {
                        next = deadline;
                        continue;
                    }
                    Err(code) => {
                        reason = code;
                        break;
                    }
                };
                match self.dispatch_range(
                    &owner,
                    &s,
                    &http,
                    &mut pending,
                    (range, qualifying),
                    (now, deadline),
                ) {
                    Ok(true) => (),
                    Ok(false) => continue,
                    Err(code) => {
                        reason = code;
                        break;
                    }
                }
                // Qualification is serialized; range replenishment rolls as slots free.
                let gap =
                    match self.random(|rng| s.config.request_gap.sample(Unit::Milliseconds, rng)) {
                        Ok(n) => n,
                        Err(_) => {
                            reason = "cover_sampling_exhausted";
                            break;
                        }
                    };
                next = now + Duration::from_millis(gap);
                continue;
            }
            let wake = if !paused
                && streams < s.config.concurrency
                && available > 0
                && requests < s.config.max_requests_per_episode
            {
                next.min(deadline)
            } else {
                deadline
            };
            tokio::select! {
                _=self.stop.cancelled()=>{reason="cover_shutdown";break;},
                _=s.notify.notified()=>{},
                _=tokio::time::sleep_until(wake)=>{},
                item=pending.join_next(),if !pending.is_empty()=>{
                    match self.complete_range(&owner, &s, item) {
                        Ok(Some(at)) => next = at,
                        Ok(None) => (),
                        Err(code) => { reason = code; break; }
                    }
                }
            }
        }
        let (capability, resource) = self.drain_ranges(&owner, &s, &mut pending).await;
        // Guards have now released reservations and accounted every observed chunk.
        let reason = if reason == "cover_episode_deadline" {
            s.episode.lock().unwrap().deadline_reason()
        } else {
            reason
        };
        diagnostics::episode(&s, &self.limits, reason, &capability, resource);
    }
}
struct RangeGuard {
    resource: &'static str,
    qualifying: bool,
    range_start: u64,
    started: Instant,
    metrics: super::metrics::Shared,
    outcome: Option<&'static str>,
    dispatched: bool,
    session: Arc<Session>,
    reserved: u64,
    counter: Arc<std::sync::atomic::AtomicU64>,
}
impl Drop for RangeGuard {
    fn drop(&mut self) {
        if self.dispatched {
            let mut m = self.metrics.lock().unwrap();
            m.in_flight_ranges -= 1;
            m.range_requests += 1;
            m.cover_body_bytes = m
                .cover_body_bytes
                .saturating_add(self.counter.load(std::sync::atomic::Ordering::Relaxed));
            let outcome = self.outcome.unwrap_or("cancelled");
            if outcome == "qualified" {
                m.qualified_ranges += 1;
            }
            *m.range_outcomes.entry(outcome.into()).or_default() += 1;
        }
        self.session.episode.lock().unwrap().finish(
            self.reserved,
            self.counter.load(std::sync::atomic::Ordering::Relaxed),
            self.dispatched,
        );
        if self.dispatched && self.outcome.is_none() {
            for scope in self.session.scopes.lock().unwrap().iter() {
                tracing::debug!(listener = %scope.listener, source = %scope.source,
                    episode_id = %self.session.id, resource = self.resource, qualifying = self.qualifying,
                    code = "cover_range_cancelled", range_start = self.range_start,
                    requested_bytes = self.reserved, received_bytes = self.counter.load(std::sync::atomic::Ordering::Relaxed),
                    elapsed_ms = self.started.elapsed().as_millis() as u64,
                    "optional range cancelled locally; capability reported at episode completion");
            }
        }
        self.session.quiet.notify_waiters();
    }
}
impl Call {
    /// Wait only for local cancellation cleanup, never a peer response or body drain.
    pub async fn prioritize(&self) {
        loop {
            let quiet = self.session.quiet.notified();
            tokio::pin!(quiet);
            quiet.as_mut().enable();
            if self.session.episode.lock().unwrap().streams == 0 {
                break;
            }
            quiet.await;
        }
    }
    pub fn response_headers(&mut self) {
        if self.request_pending {
            self.request_pending = false;
            self.session.gate.lock().unwrap().paused -= 1;
            self.session.notify.notify_one();
        }
    }
    pub fn protocol(&self, version: reqwest::Version) {
        if version != reqwest::Version::HTTP_2 {
            self.session
                .episode
                .lock()
                .unwrap()
                .stop("cover_http2_unavailable");
            self.engine.record(&self.session, "cover_http2_unavailable");
            self.session.notify.notify_one();
        }
    }
    pub fn complete(&mut self) {
        self.completed = true;
    }
    pub fn rejected_padding(&self, status: reqwest::StatusCode) {
        if !self
            .session
            .config
            .padding
            .as_ref()
            .is_some_and(|p| p.on_api_requests)
        {
            return;
        }
        if matches!(status.as_u16(), 400 | 403 | 431) {
            *self.session.padding_disabled.lock().unwrap() = true;
            self.engine.record(&self.session, "cover_padding_rejected");
        }
    }
    pub fn pad(
        &self,
        request: &mut reqwest::Request,
        api: bool,
    ) -> Option<super::metrics::PaddingReservation> {
        self.engine.pad(&self.session, request, api)
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        self.response_headers();
        let mut e = self.session.episode.lock().unwrap();
        e.detach(Instant::now());
        if !self.completed && e.calls == 0 {
            e.tail_deadline = Some(Instant::now());
        }
        drop(e);
        self.session.notify.notify_one();
    }
}

impl Engine {
    fn pad(
        &self,
        session: &Session,
        request: &mut reqwest::Request,
        api: bool,
    ) -> Option<super::metrics::PaddingReservation> {
        if self.stop.is_cancelled() {
            self.record(session, "cover_shutdown");
            return None;
        }
        let p = session.config.padding.as_ref()?;
        if session.http1_allowed {
            self.record(session, "cover_padding_http1_compatibility");
            return None;
        }
        if (api && !p.on_api_requests)
            || (!api && !p.on_cover_requests)
            || *session.padding_disabled.lock().unwrap()
        {
            return None;
        }
        let result = (|| -> Result<Option<super::metrics::PaddingReservation>> {
            let bytes = self.random(|r| p.size.sample(Unit::Bytes, r))?;
            if bytes == 0 {
                return Ok(None);
            }
            let name = reqwest::header::HeaderName::from_bytes(p.header_name.as_bytes())?;
            anyhow::ensure!(
                !request.headers().contains_key(&name),
                "cover_padding_header_collision"
            );
            let list = header_list_size(request);
            anyhow::ensure!(
                list.saturating_add(name.as_str().len() as u64 + bytes + 32)
                    <= p.max_total_header_list_bytes,
                "cover_header_list_limit"
            );
            let reservation = self
                .budget
                .reserve(Instant::now(), 0, bytes, false)
                .map_err(|e| anyhow::anyhow!(e.code()))?;
            session.episode.lock().unwrap().reserve_padding(
                &session.config,
                Instant::now(),
                bytes,
            )?;
            let contents = self.random(|r| {
                Ok((0..bytes)
                    .map(|_| char::from(b'!' + r.random_range(0..94u8)))
                    .collect::<String>())
            })?;
            let mut value = reqwest::header::HeaderValue::from_str(&contents)?;
            value.set_sensitive(true);
            request.headers_mut().insert(name, value);
            Ok(Some(super::metrics::PaddingReservation::new(
                reservation,
                self.metrics.clone(),
                bytes,
            )))
        })();
        match result {
            Ok(r) => r,
            Err(error) => {
                let text = error.to_string();
                let code = if text.starts_with("sampling_") {
                    "cover_sampling_exhausted"
                } else if text == "cover_header_list_limit" {
                    "cover_header_list_limit"
                } else if text == "cover_padding_header_collision" {
                    "cover_padding_header_collision"
                } else if text == "cover_episode_padding_limit" {
                    "cover_episode_padding_limit"
                } else if text == "cover_padding_limit" {
                    "cover_padding_limit"
                } else if text == "cover_history_limit" {
                    "cover_history_limit"
                } else {
                    "cover_padding_skipped"
                };
                self.record(session, code);
                None
            }
        }
    }
}

/// Uncompressed header-list accounting, including HTTP/2 pseudo-fields and the
/// defaults/body length added by this application's reqwest/hyper constructors.
fn header_list_size(request: &reqwest::Request) -> u64 {
    let field = |name: &str, len: usize| {
        (name.len() as u64)
            .saturating_add(len as u64)
            .saturating_add(32)
    };
    let mut total = request.headers().iter().fold(0u64, |sum, (name, value)| {
        sum.saturating_add(field(name.as_str(), value.len()))
    });
    let url = request.url();
    let authority = match url.port() {
        Some(p) => format!(
            "{}:{p}",
            url.host().map(|h| h.to_string()).unwrap_or_default()
        ),
        None => url.host().map(|h| h.to_string()).unwrap_or_default(),
    };
    let path = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_owned(),
    };
    for (name, len) in [
        (":method", request.method().as_str().len()),
        (":scheme", url.scheme().len()),
        (":authority", authority.len()),
        (":path", path.len().max(1)),
    ] {
        total = total.saturating_add(field(name, len));
    }
    if !request.headers().contains_key(reqwest::header::ACCEPT) {
        total = total.saturating_add(field("accept", 3));
    }
    if !request
        .headers()
        .contains_key(reqwest::header::CONTENT_LENGTH)
        && let Some(bytes) = request.body().and_then(|b| b.as_bytes())
    {
        total = total.saturating_add(field("content-length", bytes.len().to_string().len()));
    }
    total
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod retention_tests;

#[cfg(test)]
mod diagnostic_tests;
