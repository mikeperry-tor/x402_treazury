//! One ordered relay traversal, with adapter translation separate from consumer validation.
use super::*;
use crate::network::acquisition::Failure;
use adapters::{Decoded, Format, PolicyViolation, TargetDeliveryFailure, TargetRequest};
use reqwest::{Method, StatusCode};
use std::time::Duration;

#[derive(Clone)]
pub(super) struct Entry {
    pub key: String,
    pub provider_key: String,
    pub endpoint: String,
    pub cfg: Config,
    pub format: Format,
    pub position: usize,
    #[cfg(test)]
    fixture: bool,
}
impl Entry {
    fn new(
        method: &str,
        position: usize,
        path: &Path,
        cfg: Config,
        format: Format,
    ) -> Result<Self> {
        let endpoint = format!(
            "{}{}",
            cfg.base_url
                .as_deref()
                .context("discovery relay requires base_url")?
                .trim_end_matches('/'),
            operation(format)
        );
        crate::network::public_url(&endpoint)?;
        let provider_key = serde_json::to_string(&(
            path,
            label(format),
            &endpoint,
            cfg.allow_http1,
            cfg.allow_tls12,
            cfg.read_timeout_seconds.map(f64::to_bits),
            cfg.max_response_bytes,
        ))?;
        let key = serde_json::to_string(&(method, position, &provider_key))?;
        Ok(Self {
            key,
            provider_key,
            endpoint,
            cfg,
            format,
            position,
            #[cfg(test)]
            fixture: false,
        })
    }
    fn client(
        &self,
        client: PaidClient,
        limit: Option<usize>,
        help: usize,
        timeout: Option<Duration>,
    ) -> PaidClient {
        let client = client
            .for_relay()
            .with_source_identity("builtin:discovery-relay")
            .with_transport(self.cfg.transport())
            .with_timeout(
                timeout.or_else(|| self.cfg.read_timeout_seconds.map(Duration::from_secs_f64)),
            )
            .with_download_limits(limit.unwrap_or(self.cfg.max_response_bytes), help);
        #[cfg(test)]
        if self.fixture {
            return client;
        }
        client.public_destinations()
    }
}
fn operation(format: Format) -> &'static str {
    match format {
        Format::Curl => "/curl",
        Format::Agproxy => "/x402/fetch",
        Format::Horizonpulse => "/api/http",
    }
}
pub(super) fn label(format: Format) -> &'static str {
    match format {
        Format::Curl => "curl",
        Format::Agproxy => "agproxy",
        Format::Horizonpulse => "horizonpulse",
    }
}

/// Response semantics are selected by the reader, never by vendor or envelope shape.
#[derive(Clone, Copy)]
pub(crate) enum Contract {
    Catalog,
    Document,
    Api,
    Search(fn(&[u8]) -> Result<()>),
}
impl Contract {
    pub(super) fn decision(self, response: &Decoded) -> Decision {
        let status = response.status;
        if status == 402 {
            return Decision::Terminal("target_payment_required");
        }
        if matches!(self, Self::Catalog) && status == 304 {
            return Decision::Terminal("unsolicited_not_modified");
        }
        if !matches!(self, Self::Api)
            && (status == 206
                || response
                    .headers
                    .contains_key(reqwest::header::CONTENT_RANGE))
        {
            return Decision::Terminal("partial_response");
        }
        // Terminal runtime statuses remain terminal even when the relay also
        // reports failed delivery. Such metadata grants no second paid read.
        if matches!(self, Self::Document | Self::Api)
            && !(200..300).contains(&status)
            && !crate::network::acquisition::blocking_status(status)
        {
            return Decision::Terminal("http_status");
        }
        if response.delivery_failed || response.truncated {
            return Decision::Next("incomplete_delivery");
        }
        match self {
            Self::Catalog if !(200..300).contains(&status) => Decision::Next("http_status"),
            Self::Catalog
                if serde_json::from_slice::<serde_json::Value>(&response.body).is_err() =>
            {
                Decision::Next("catalog_parse")
            }
            Self::Catalog => Decision::Accept,
            Self::Document | Self::Api if (200..300).contains(&status) => Decision::Accept,
            Self::Document | Self::Api => Decision::Next("http_status"),
            Self::Search(_) if status != 200 => Decision::Next("http_status"),
            Self::Search(validate) if validate(&response.body).is_err() => {
                Decision::Next("response_rejected")
            }
            Self::Search(_) => Decision::Accept,
        }
    }
}
pub(super) enum Decision {
    Accept,
    Next(&'static str),
    Terminal(&'static str),
}

/// Immutable bootstrap; only static discovery supplies its existing mutable cache owner.
#[derive(Clone)]
pub(crate) struct SearchRelay {
    pub(super) entries: BTreeMap<String, Vec<Entry>>,
    pub(super) policy: Policy,
}
impl SearchRelay {
    pub(crate) fn enabled(&self, purpose: Purpose) -> bool {
        match purpose {
            Purpose::SourceDiscovery => self.policy.source_discovery,
            Purpose::ApiRequests => self.policy.api_requests,
            Purpose::ProviderMetadata => self.policy.provider_metadata,
        }
    }
    pub(crate) async fn load(policy: &Policy, path: &Path) -> Result<Self> {
        let mut entries = BTreeMap::new();
        // Every configured position is validated locally before any request or key load.
        for (method, providers) in policy.relays() {
            let mut configured = Vec::new();
            for (position, provider) in providers.iter().enumerate() {
                let provider = if provider.is_absolute() {
                    provider.clone()
                } else {
                    path.parent().unwrap_or(Path::new(".")).join(provider)
                };
                let cfg = crate::config::load(&provider).await?.settings;
                let format = cfg
                    .relay_format
                    .as_deref()
                    .context("relay fallback provider requires relay_format")?
                    .parse::<Format>()?;
                ensure!(
                    !cfg.spec.starts_with("https://") && !cfg.spec.starts_with("http://"),
                    "discovery relay requires a local bootstrap catalog"
                );
                let document: serde_json::Value =
                    serde_json::from_slice(&tokio::fs::read(&cfg.spec).await?)?;
                local_references(&document)?;
                let tools = crate::catalog::build_tools(&cfg, &document, "relay")?;
                ensure!(
                    tools
                        .iter()
                        .any(|tool| tool.method == "POST" && tool.path == operation(format)),
                    "discovery relay provider must expose POST {}",
                    operation(format)
                );
                configured.push(Entry::new(method, position, &provider, cfg, format)?);
            }
            entries.insert(method.clone(), configured);
        }
        Ok(Self {
            entries,
            policy: policy.clone(),
        })
    }
    #[cfg(test)]
    pub(crate) fn fixture(endpoint: String, cfg: Config) -> Self {
        Self {
            policy: Policy::default(),
            entries: BTreeMap::from([(
                "GET".into(),
                vec![Entry {
                    key: "fixture-position".into(),
                    provider_key: "fixture-provider".into(),
                    endpoint,
                    cfg,
                    format: Format::Curl,
                    position: 0,
                    fixture: true,
                }],
            )]),
        }
    }
    #[cfg(test)]
    pub(crate) fn fixture_methods(
        methods: BTreeMap<String, Vec<(String, String, Config)>>,
    ) -> Result<Self> {
        let mut entries = BTreeMap::new();
        for (method, positions) in methods {
            let mut selected = Vec::new();
            for (position, (format, endpoint, cfg)) in positions.into_iter().enumerate() {
                selected.push(Entry {
                    key: format!("{method}:{position}"),
                    provider_key: format!("{format}:{endpoint}"),
                    endpoint,
                    cfg,
                    format: format.parse()?,
                    position,
                    fixture: true,
                });
            }
            entries.insert(method, selected);
        }
        Ok(Self {
            entries,
            policy: Policy::default(),
        })
    }
    #[cfg(test)]
    pub(crate) fn with_policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }
    pub(crate) fn cache_keys(&self, wallet: &str) -> Vec<String> {
        self.entries
            .get("GET")
            .into_iter()
            .flatten()
            .map(|entry| cache_key(wallet, entry))
            .collect()
    }
    pub(crate) async fn fetch(
        &self,
        client: PaidClient,
        url: &str,
        validate: fn(&[u8]) -> Result<()>,
        retry_after: Option<crate::network::RetryAfter>,
    ) -> Result<Arc<Response>> {
        let target = TargetRequest {
            method: Method::GET,
            url: url.parse()?,
            headers: Default::default(),
            body: None,
            allowed_origins: Vec::new(),
        };
        self.acquire(
            client,
            &target,
            Contract::Search(validate),
            crate::limits::RESPONSE_BYTES,
            "max_response_bytes",
            Some(crate::limits::RESPONSE_BYTES),
            crate::limits::HELP_BYTES,
            None,
            &[],
            None,
            retry_after,
        )
        .await
    }

    pub(super) fn prepare(&self, target: &TargetRequest, limit: usize) -> Result<Prepared> {
        let mut entries = Vec::new();
        for entry in self
            .entries
            .get(target.method.as_str())
            .into_iter()
            .flatten()
        {
            if let Some(body) = entry.format.encode(target, limit)? {
                entries.push((entry.clone(), body));
            }
        }
        Ok(Prepared { entries })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn acquire(
        &self,
        client: PaidClient,
        target: &TargetRequest,
        contract: Contract,
        limit: usize,
        setting: &'static str,
        outer_limit: Option<usize>,
        help_limit: usize,
        timeout: Option<Duration>,
        capacity: &[Arc<tokio::sync::Semaphore>],
        cache: Option<(Arc<tokio::sync::Mutex<State>>, &str)>,
        initial_delay: Option<crate::network::RetryAfter>,
    ) -> Result<Arc<Response>> {
        self.prepare(target, limit)?
            .acquire(
                client,
                target,
                contract,
                limit,
                setting,
                outer_limit,
                help_limit,
                timeout,
                capacity,
                cache,
                initial_delay,
            )
            .await
    }
}

/// All locally representable positions, encoded before readiness or server delays.
/// Prepared requests contain no financial authorization or mutable cache ownership.
pub(super) struct Prepared {
    entries: Vec<(Entry, serde_json::Value)>,
}
impl Prepared {
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub(super) async fn cached(
        &self,
        state: Arc<tokio::sync::Mutex<State>>,
        scope: &str,
        target: &TargetRequest,
        limit: usize,
        setting: &'static str,
    ) -> Option<Result<Arc<Response>>> {
        let cache = StaticRequest {
            state,
            scope,
            url: target.url.to_string(),
        };
        let mut any_payment = false;
        let mut last = TargetFailure::new("no_representable_relay", None, false);
        for (entry, _) in &self.entries {
            match cache.admit(entry).await {
                Admission::Cached(Ok(response)) => {
                    return Some(checked_cached(response, entry, limit, setting, any_payment));
                }
                Admission::Cached(Err(failure)) => {
                    any_payment |= failure.possible;
                    if !failure.advance {
                        return Some(Err(failure.error(any_payment)));
                    }
                    last = failure;
                }
                Admission::Unavailable => {
                    last = TargetFailure::new("provider_unavailable", None, true).at_relay(
                        entry,
                        "relay unavailable after an earlier uncertain payment",
                    );
                }
                Admission::Ready(_) => return None,
            }
        }
        Some(Err(last.error(any_payment)))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn acquire(
        &self,
        client: PaidClient,
        target: &TargetRequest,
        contract: Contract,
        limit: usize,
        setting: &'static str,
        outer_limit: Option<usize>,
        help_limit: usize,
        timeout: Option<Duration>,
        capacity: &[Arc<tokio::sync::Semaphore>],
        cache: Option<(Arc<tokio::sync::Mutex<State>>, &str)>,
        mut initial_delay: Option<crate::network::RetryAfter>,
    ) -> Result<Arc<Response>> {
        let cache = cache.map(|(state, scope)| StaticRequest {
            state,
            scope,
            url: target.url.to_string(),
        });
        let mut any_payment = false;
        let mut pending_delay = None;
        let mut last = TargetFailure::new("no_representable_relay", None, false);
        for (index, (entry, body)) in self.entries.iter().enumerate() {
            let admission = match &cache {
                Some(cache) => cache.admit(entry).await,
                None => Admission::Ready(None),
            };
            let admission = match admission {
                Admission::Ready(guard) if initial_delay.is_some() || pending_delay.is_some() => {
                    // A server delay reserves no provider. Recheck publication
                    // after the wait before dispatching a new acquisition.
                    drop(guard);
                    crate::network::sleep_retry_after(initial_delay.take()).await;
                    crate::network::sleep_retry_after(pending_delay.take()).await;
                    match &cache {
                        Some(cache) => cache.admit(entry).await,
                        None => Admission::Ready(None),
                    }
                }
                other => other,
            };
            let outcome = match admission {
                Admission::Cached(outcome) => outcome,
                Admission::Unavailable => {
                    last = TargetFailure::new("provider_unavailable", None, true).at_relay(
                        entry,
                        "relay unavailable after an earlier uncertain payment",
                    );
                    continue;
                }
                Admission::Ready(guard) => {
                    // This is a current readiness query, never a funding wait.
                    // DependencyWait releases the gate before the caller waits on
                    // its existing readiness notification.
                    if cache.is_some()
                        && !client
                            .discovery_ready()
                            .await
                            .map_err(|e| RelayFailure::from_error(&e))?
                    {
                        return Err(RetryDisposition::DependencyWait.into());
                    }
                    if let Some(cache) = &cache {
                        cache.reserve(entry).await;
                    }
                    let exchange = entry
                        .exchange(
                            client.clone().with_attempt_capacity(capacity),
                            body.clone(),
                            target,
                            contract,
                            limit,
                            setting,
                            outer_limit,
                            help_limit,
                            timeout,
                        )
                        .await;
                    let definitive = match &exchange {
                        Ok(_) => true,
                        Err(failure) => failure.definitive,
                    };
                    let outcome = exchange
                        .map(|(response, elapsed, possible)| {
                            Arc::new(Response {
                                status: response.status,
                                headers: response.headers,
                                body: response.body,
                                final_url: response.final_url,
                                delay: elapsed,
                                received: Instant::now(),
                                cache_key: cache
                                    .as_ref()
                                    .map(|cache| cache_key(cache.scope, entry))
                                    .unwrap_or_else(|| entry.key.clone()),
                                via: label(entry.format).into(),
                                payment_may_have_occurred: any_payment || possible,
                            })
                        })
                        .map_err(|outcome| outcome.failure);
                    if let Some(cache) = &cache {
                        cache.publish(entry, outcome.clone(), definitive).await;
                    }
                    drop(guard);
                    outcome
                }
            };
            match outcome {
                Ok(response) => {
                    return checked_cached(response, entry, limit, setting, any_payment);
                }
                Err(failure) => {
                    any_payment |= failure.possible;
                    if !failure.advance {
                        return Err(failure.error(any_payment));
                    }
                    if cache.is_some() && index + 1 < self.entries.len() {
                        tracing::info!(
                            relay_format = label(entry.format),
                            relay_position = entry.position + 1,
                            failure_location = failure.location,
                            reason = failure.reason,
                            cause = RelayFailure {
                                reason: failure.reason,
                                status: failure.status
                            }
                            .explanation(),
                            http_status = failure.status,
                            possible_submission = any_payment,
                            "Configured paid relay could not provide a usable catalog; continuing through the remaining configured relays"
                        );
                    }
                    pending_delay = later_delay(pending_delay, failure.retry_after);
                    last = failure;
                }
            }
        }
        Err(last.error(any_payment))
    }
}

fn later_delay(
    left: Option<crate::network::RetryAfter>,
    right: Option<crate::network::RetryAfter>,
) -> Option<crate::network::RetryAfter> {
    use crate::network::RetryAfter;
    match (left, right) {
        (Some(RetryAfter::BeyondClock), _) | (_, Some(RetryAfter::BeyondClock)) => {
            Some(RetryAfter::BeyondClock)
        }
        (Some(RetryAfter::At(left)), Some(RetryAfter::At(right))) => {
            Some(RetryAfter::At(left.max(right)))
        }
        (Some(delay), None) | (None, Some(delay)) => Some(delay),
        (None, None) => None,
    }
}

fn checked_cached(
    mut response: Arc<Response>,
    entry: &Entry,
    limit: usize,
    setting: &'static str,
    possible: bool,
) -> Result<Arc<Response>> {
    if response.body.len() > limit {
        let mut failure = TargetFailure::new("response_rejected", None, false)
            .at_relay(entry, "local response policy");
        failure.limit = Some((setting, limit));
        return Err(failure.error(possible || response.payment_may_have_occurred));
    }
    if possible && !response.payment_may_have_occurred {
        Arc::make_mut(&mut response).payment_may_have_occurred = true;
    }
    Ok(response)
}
/// A single provider exchange owns payment evidence and envelope validation.
/// The ordered traversal alone owns static publication and provider availability.
struct ExchangeFailure {
    failure: TargetFailure,
    definitive: bool,
}
impl Entry {
    #[allow(clippy::too_many_arguments)]
    async fn exchange(
        &self,
        client: PaidClient,
        body: serde_json::Value,
        target: &TargetRequest,
        contract: Contract,
        limit: usize,
        setting: &'static str,
        outer_limit: Option<usize>,
        help_limit: usize,
        timeout: Option<Duration>,
    ) -> std::result::Result<(Decoded, Duration, bool), ExchangeFailure> {
        let scoped = self.client(client, outer_limit, help_limit, timeout);
        let progress = crate::payment::SubmissionProgress::default();
        let started = Instant::now();
        tracing::debug!(
            relay_format = label(self.format),
            relay_position = self.position + 1,
            "Requesting the target response from a configured paid relay"
        );
        let result = scoped
            .execute_response_scoped(
                RoutedRequest {
                    headers: Default::default(),
                    method: "POST".into(),
                    url: self.endpoint.clone(),
                    query: BTreeMap::new(),
                    body: Some(body),
                },
                &progress,
                scoped.operation_key(None, "POST", operation(self.format), &self.endpoint),
            )
            .await;
        let possible = progress.possible();
        let output = result.map_err(|error| {
            let typed = error.downcast_ref::<Failure>();
            let cause = typed.map(|f| &f.error).unwrap_or(&error);
            let http = cause.downcast_ref::<crate::payment::HttpFailure>();
            let safe = RelayFailure::from_error(cause);
            let mut failure = TargetFailure::new(safe.reason, safe.status, false)
                .at_relay(self, "relay service request");
            if cause.is::<crate::limits::DownloadLimit>() {
                failure.location = "local response policy";
                failure.limit = Some((
                    "max_response_bytes",
                    outer_limit.unwrap_or(self.cfg.max_response_bytes),
                ));
            }
            failure.possible = possible;
            failure.advance = !possible
                && (typed.is_some_and(|f| f.fallback)
                    || cause
                        .downcast_ref::<reqwest::Error>()
                        .is_some_and(crate::network::eligible_retry_transport)
                    || http.is_some_and(|f| f.status != StatusCode::PAYMENT_REQUIRED));
            failure.retry_after = typed.and_then(|f| f.retry_after);
            tracing::debug!(
                relay_format = label(self.format),
                relay_position = self.position + 1,
                failure_location = failure.location,
                reason = safe.reason,
                http_status = safe.status,
                possible_submission = possible,
                advance_permitted = failure.advance,
                elapsed_seconds = started.elapsed().as_secs_f64(),
                max_response_bytes = outer_limit.unwrap_or(self.cfg.max_response_bytes),
                read_timeout_seconds = crate::network::global()
                    .read_timeout(scoped.timeout())
                    .as_secs_f64(),
                "Configured relay request failed"
            );
            ExchangeFailure {
                failure,
                definitive: !possible,
            }
        })?;
        let response = self
            .format
            .decode(&output.bytes, target, limit, setting)
            .map_err(|error| {
                let attributable = error.is::<TargetDeliveryFailure>();
                let local =
                    error.is::<PolicyViolation>() || error.is::<crate::limits::DownloadLimit>();
                let (reason, location) = if attributable {
                    ("origin_request", "target delivery reported by relay")
                } else if local {
                    ("response_rejected", "local response policy")
                } else {
                    ("invalid_relay_envelope", "relay response validation")
                };
                let mut failure =
                    TargetFailure::new(reason, None, attributable || (!possible && !local))
                        .at_relay(self, location);
                failure.possible = possible;
                if error.is::<crate::limits::DownloadLimit>() {
                    failure.limit = Some((setting, limit));
                }
                tracing::debug!(
                    relay_format = label(self.format),
                    relay_position = self.position + 1,
                    failure_location = failure.location,
                    reason = failure.reason,
                    possible_submission = possible,
                    advance_permitted = failure.advance,
                    "Configured paid relay response could not be used"
                );
                ExchangeFailure {
                    failure,
                    definitive: attributable || local || !possible,
                }
            })?;
        // Refine only accepted complete responses: incomplete delivery must
        // retain the consumer's advancement decision.
        let decision = match contract.decision(&response) {
            Decision::Accept
                if matches!(contract, Contract::Api)
                    && self.format == Format::Curl
                    && target.method != Method::HEAD
                    && response
                        .headers
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|v| {
                            !crate::output::text_mime(
                                &v.split(';').next().unwrap().trim().to_ascii_lowercase(),
                            )
                        }) =>
            {
                Decision::Terminal("unsupported_response")
            }
            decision => decision,
        };
        match decision {
            Decision::Accept => {
                tracing::debug!(
                    relay_format = label(self.format),
                    relay_position = self.position + 1,
                    target_http_status = response.status,
                    possible_submission = possible,
                    "Configured paid relay supplied a usable target response"
                );
                Ok((response, started.elapsed(), possible))
            }
            decision => {
                let (reason, advance) = match decision {
                    Decision::Next(reason) => (reason, true),
                    Decision::Terminal(reason) => (reason, false),
                    Decision::Accept => unreachable!(),
                };
                let mut failure = TargetFailure::new(reason, Some(response.status), advance)
                    .at_relay(self, "target response reported by relay");
                failure.possible = possible;
                failure.retry_after = crate::network::parse_retry_after(
                    StatusCode::from_u16(response.status).expect("adapter validated HTTP status"),
                    &response.headers,
                )
                .map(crate::network::RetryAfter::after);
                tracing::debug!(
                    relay_format = label(self.format),
                    relay_position = self.position + 1,
                    failure_location = failure.location,
                    reason,
                    target_http_status = response.status,
                    possible_submission = possible,
                    advance_permitted = advance,
                    "Configured paid relay reported an unusable target response"
                );
                Err(ExchangeFailure {
                    failure,
                    definitive: true,
                })
            }
        }
    }
}

/// Static discovery owns one scoped cache outcome, including failed accepted work.
struct StaticRequest<'a> {
    state: Arc<tokio::sync::Mutex<State>>,
    scope: &'a str,
    url: String,
}
enum Admission {
    Cached(std::result::Result<Arc<Response>, TargetFailure>),
    Unavailable,
    Ready(Option<tokio::sync::OwnedMutexGuard<()>>),
}
impl StaticRequest<'_> {
    fn key(&self, entry: &Entry) -> (String, String) {
        (cache_key(self.scope, entry), self.url.clone())
    }
    async fn admit(&self, entry: &Entry) -> Admission {
        let gate = {
            let mut state = self.state.lock().await;
            if let Some(outcome) = state.outcomes.get(&self.key(entry)) {
                return Admission::Cached(outcome.clone());
            }
            state
                .providers
                .entry(entry.provider_key.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        // An active provider's temporary stopped marker is not proof that it
        // failed. Wait outside State, then observe its complete publication.
        let guard = gate.lock_owned().await;
        let state = self.state.lock().await;
        if let Some(outcome) = state.outcomes.get(&self.key(entry)) {
            Admission::Cached(outcome.clone())
        } else if state.stopped.contains(&entry.provider_key) {
            tracing::debug!(
                relay_format = label(entry.format),
                relay_position = entry.position + 1,
                "Configured paid relay is unavailable after earlier uncertain work; no request was sent"
            );
            Admission::Unavailable
        } else {
            Admission::Ready(Some(guard))
        }
    }
    async fn reserve(&self, entry: &Entry) {
        self.state
            .lock()
            .await
            .stopped
            .insert(entry.provider_key.clone());
    }
    async fn publish(
        &self,
        entry: &Entry,
        outcome: std::result::Result<Arc<Response>, TargetFailure>,
        definitive: bool,
    ) {
        let mut state = self.state.lock().await;
        if definitive {
            state.stopped.remove(&entry.provider_key);
        }
        state.outcomes.insert(self.key(entry), outcome);
        state.changes.notify_waiters();
    }
}

fn cache_key(wallet: &str, entry: &Entry) -> String {
    serde_json::to_string(&(wallet, &entry.key)).expect("serializable relay cache identity")
}
fn local_references(value: &serde_json::Value) -> Result<()> {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if key == "$ref" {
                    ensure!(
                        value.as_str().is_some_and(|v| v.starts_with("#/")),
                        "relay bootstrap requires internal schema references"
                    );
                }
                local_references(value)?;
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                local_references(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}
