//! Lifecycle-owned cover tasks on the existing identity-bound HTTP client.
use super::{
    Config, Limits,
    budget::{Budget, Reservation},
    episode::Episode,
    range::{self, Range, Representation},
    registry::{Capability, Owner, Registry},
    sampling::Unit,
    status::{Scope, Status},
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

pub struct Engine {
    limits: Limits,
    registry: Mutex<Registry>,
    budget: Budget,
    rng: Mutex<Option<StdRng>>,
    status: Mutex<Status>,
    tasks: TaskTracker,
    stop: CancellationToken,
}
#[derive(Default)]
struct RequestGate {
    paused: usize,
    handles: Vec<tokio::task::AbortHandle>,
}
pub struct Session {
    gate: Mutex<RequestGate>,
    quiet: Notify,
    episode: Arc<Mutex<Episode>>,
    config: Arc<Config>,
    scopes: Mutex<BTreeSet<Scope>>,
    reason: Mutex<BTreeSet<&'static str>>,
    notify: Notify,
    padding_disabled: Arc<Mutex<bool>>,
    http1_allowed: bool,
}
/// One lease per real attempt; a successful completion permits a bounded tail.
pub struct Call {
    engine: Arc<Engine>,
    pub session: Arc<Session>,
    scope: Scope,
    completed: bool,
    request_pending: bool,
}
impl Engine {
    pub fn new(limits: Limits) -> Arc<Self> {
        Arc::new(Self {
            limits: limits.clone(),
            registry: Mutex::new(Registry::new(limits.clone())),
            budget: Budget::new(limits),
            rng: Mutex::new(None),
            status: Mutex::new(Status::default()),
            tasks: TaskTracker::new(),
            stop: CancellationToken::new(),
        })
    }
    fn random<T>(&self, f: impl FnOnce(&mut StdRng) -> Result<T>) -> Result<T> {
        let mut rng = self.rng.lock().unwrap();
        f(rng.get_or_insert_with(StdRng::from_os_rng))
    }
    pub fn register(&self, scope: Scope) {
        self.status.lock().unwrap().register(scope);
    }
    pub fn status(&self, scopes: &[Scope]) -> serde_json::Value {
        serde_json::to_value(self.status.lock().unwrap().report(scopes))
            .expect("cover status serializable")
    }
    fn record(&self, session: &Session, reason: &'static str) {
        session.reason.lock().unwrap().insert(reason);
        tracing::warn!(
            code = reason,
            episode_request_limit = session.config.max_requests_per_episode,
            episode_body_byte_limit = session.config.max_cover_body_bytes_per_episode,
            process_request_limit = self.limits.max_requests_per_window,
            process_body_byte_limit = self.limits.max_cover_body_bytes_per_window,
            process_padding_byte_limit = self.limits.max_padding_value_bytes_per_window,
            "optional cover status; pooled best-effort connection reuse"
        );
        for scope in session.scopes.lock().unwrap().iter() {
            self.status.lock().unwrap().record(scope, reason);
        }
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
                gate: Mutex::new(RequestGate::default()),
                quiet: Notify::new(),
                episode,
                config,
                scopes: Mutex::new(BTreeSet::new()),
                reason: Mutex::new(BTreeSet::new()),
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
        self.status
            .lock()
            .unwrap()
            .record(&scope, "cover_episode_active");
        if let Capability::Unavailable(code) = state.capability {
            self.record(&session, code);
        }
        if *state.padding_disabled.lock().unwrap() {
            self.record(&session, "cover_padding_disabled");
        }
        if fresh {
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
            scope,
            completed: false,
            request_pending: true,
        })
    }
    pub async fn shutdown(&self) {
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
                reason = if available == 0 {
                    "cover_budget_completed"
                } else {
                    "cover_budget_unspent"
                };
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
                let capability = self.registry.lock().unwrap().owners[&owner]
                    .capability
                    .clone();
                let (range, qualifying) = match capability {
                    Capability::Unknown => (Range::qualification(&s.config), true),
                    Capability::Available { length, validator } => {
                        let requested =
                            match self.random(|r| s.config.ranges.sample(Unit::Bytes, r)) {
                                Ok(n) => n,
                                Err(_) => {
                                    reason = "cover_sampling_exhausted";
                                    break;
                                }
                            };
                        let length_request = requested.min(length).min(available);
                        if length_request != requested {
                            self.record(&s, "cover_range_size_adjusted");
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
                        reason = code;
                        break;
                    }
                    Capability::Checking => {
                        next = deadline;
                        continue;
                    }
                };
                let reserved = match s
                    .episode
                    .lock()
                    .unwrap()
                    .reserve(&s.config, now, range.length)
                {
                    Ok(n) => n,
                    Err(_) => {
                        reason = "cover_episode_limit";
                        break;
                    }
                };
                let budget = match self.budget.reserve(now, reserved, 0, true) {
                    Ok(b) => b,
                    Err(e) => {
                        s.episode.lock().unwrap().finish(reserved, 0, false);
                        reason = e.code();
                        break;
                    }
                };
                if qualifying {
                    self.registry
                        .lock()
                        .unwrap()
                        .owners
                        .get_mut(&owner)
                        .unwrap()
                        .begin_qualification();
                }
                let mut range = range;
                range.length = reserved;
                let mut padding_request = reqwest::Request::new(
                    reqwest::Method::GET,
                    s.config.url.parse().expect("validated cover URL"),
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
                let padding_budget = self.pad(&s, &mut padding_request, false);
                let padding = s.config.padding.as_ref().and_then(|p| {
                    padding_request
                        .headers()
                        .get(p.header_name.as_str())
                        .cloned()
                        .map(|v| (p.header_name.parse().unwrap(), v))
                });
                let counter = budget.counter();
                let mut guard = RangeGuard {
                    dispatched: false,
                    session: s.clone(),
                    reserved,
                    counter,
                };
                let cfg = s.config.clone();
                let client = http.clone();
                let mut gate = s.gate.lock().unwrap();
                if gate.paused > 0 {
                    drop(gate);
                    if qualifying {
                        self.registry
                            .lock()
                            .unwrap()
                            .owners
                            .get_mut(&owner)
                            .unwrap()
                            .capability = Capability::Unknown;
                    }
                    drop(guard);
                    drop(budget);
                    continue;
                }
                gate.handles.retain(|h| !h.is_finished());
                let handle = pending.spawn(async move {
                    guard.dispatched = true;
                    let mut padding_budget = padding_budget;
                    if let Some(p) = &mut padding_budget {
                        p.dispatched();
                    }
                    let result =
                        range::download(&client, &cfg, range, deadline, budget, padding).await;
                    drop(guard);
                    (qualifying, result)
                });
                gate.handles.push(handle);
                drop(gate);
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
                    match item {
                        Some(Ok((qualifying,result)))=>{
                            if qualifying {self.registry.lock().unwrap().owners.get_mut(&owner).unwrap().qualified(result.clone());next=Instant::now();}
                            if let Err(error)=result {self.registry.lock().unwrap().owners.get_mut(&owner).unwrap().capability=Capability::Unavailable(error.0);reason=error.0;break;}
                        },
                        Some(Err(error)) if error.is_cancelled()=>{
                            if matches!(self.registry.lock().unwrap().owners[&owner].capability,Capability::Checking) {reason="cover_qualification_preempted";break;}
                            self.record(&s,"cover_preempted_for_api");
                        },
                        _=>{reason="cover_task_failed";break;},
                    }
                }
            }
        }
        pending.abort_all();
        while pending.join_next().await.is_some() {}
        let mut registry = self.registry.lock().unwrap();
        if matches!(registry.owners[&owner].capability, Capability::Checking) {
            registry.owners.get_mut(&owner).unwrap().capability =
                Capability::Unavailable("cover_qualification_cancelled");
        }
        drop(registry);
        self.record(&s, reason);
        let e = s.episode.lock().unwrap();
        tracing::info!(
            code = reason,
            mode = "extra_body",
            sampled_body_bytes = e.target,
            cover_body_bytes = e.received,
            padding_value_bytes = e.padding,
            range_requests = e.requests,
            elapsed_ms = e.started.elapsed().as_millis() as u64,
            "cover episode range work finished; pooled best-effort, not a privacy guarantee"
        );
    }
}
struct RangeGuard {
    dispatched: bool,
    session: Arc<Session>,
    reserved: u64,
    counter: Arc<std::sync::atomic::AtomicU64>,
}
impl Drop for RangeGuard {
    fn drop(&mut self) {
        self.session.episode.lock().unwrap().finish(
            self.reserved,
            self.counter.load(std::sync::atomic::Ordering::Relaxed),
            self.dispatched,
        );
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
    pub fn advisory(&self) -> Option<String> {
        let reasons = self
            .session
            .reason
            .lock()
            .unwrap()
            .iter()
            .filter(|c| **c != "cover_budget_completed")
            .copied()
            .collect::<Vec<_>>();
        (!reasons.is_empty()).then(||format!("Optional cover traffic: {}. Connection reuse is pooled/best-effort; consult treazury_cover_status.",reasons.join(", ")))
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
    pub fn pad(&self, request: &mut reqwest::Request, api: bool) -> Option<Reservation> {
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
        for reason in self.session.reason.lock().unwrap().iter() {
            self.engine
                .status
                .lock()
                .unwrap()
                .record(&self.scope, reason);
        }
    }
}

impl Engine {
    fn pad(
        &self,
        session: &Session,
        request: &mut reqwest::Request,
        api: bool,
    ) -> Option<Reservation> {
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
        let result = (|| -> Result<Option<Reservation>> {
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
            Ok(Some(reservation))
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
