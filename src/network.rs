//! All runtime egress is constructed here. Policy is immutable after startup.
pub(crate) mod acquisition;
mod grpc;
mod indexer;
pub(crate) use grpc::ConnectionDiagnostic;
pub use indexer::GrpcIndexer;
#[cfg(test)]
#[path = "network/http_queue_tests.rs"]
mod http_queue_tests;
#[cfg(test)]
#[path = "network/http_timeout_tests.rs"]
mod http_timeout_tests;
#[cfg(test)]
mod retry_policy_tests;
mod scope_observation;
#[cfg(test)]
mod setup_retry_tests;
mod startup_socks;
#[cfg(test)]
mod tls_retry_tests;
use anyhow::{Context, Result, ensure};
pub use scope_observation::{ScopeObservation, install_http_scope_observer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Direct,
    Tor,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SocksAuth {
    #[default]
    TorExtended,
    Legacy,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NetworkPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_traffic_enabled: Option<bool>,
    #[serde(default)]
    pub cover_limits: crate::cover::Limits,
    #[serde(default)]
    pub mode: Mode,
    pub socks_endpoint: Option<SocketAddr>,
    pub isolation_namespace: Option<String>,
    pub socks_auth: Option<SocksAuth>,
    pub connect_timeout_seconds: Option<u64>,
    /// Fresh owner-bound Tor scopes after the initial unsigned attempt.
    pub tor_isolation_retries: Option<usize>,
    /// Maximum HTTP read inactivity, identically in direct and Tor mode.
    #[serde(alias = "request_timeout_seconds")]
    pub read_timeout_seconds: Option<u64>,
}
impl NetworkPolicy {
    pub fn cover_enabled(&self) -> bool {
        self.cover_traffic_enabled.unwrap_or(self.mode == Mode::Tor)
    }

    pub fn validate(&self) -> Result<()> {
        self.cover_limits.validate()?;
        self.tor_attempts()?;
        if self.mode == Mode::Direct {
            ensure!(
                self.socks_endpoint.is_none()
                    && self.isolation_namespace.is_none()
                    && self.socks_auth.is_none(),
                "Tor settings require network.mode = tor"
            );
        } else {
            let endpoint = self
                .socks_endpoint
                .context("network.socks_endpoint is required in Tor mode")?;
            ensure!(
                endpoint.ip().is_loopback() && endpoint.port() != 0,
                "SOCKS endpoint must be a literal loopback address and nonzero port"
            );
            let namespace = self.namespace();
            ensure!(
                !namespace.is_empty()
                    && namespace.len() <= 128
                    && namespace
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
                "invalid isolation namespace (1..128 ASCII letters, digits, underscores or hyphens)"
            );
        }
        ensure!(
            (1..=300).contains(&self.connect_timeout().as_secs()),
            "network connect timeout must be 1..300 seconds"
        );
        ensure!(
            (1..=86400).contains(&self.read_timeout_seconds.unwrap_or(60)),
            "network read timeout must be 1..86400 seconds"
        );
        Ok(())
    }
    fn tor_attempts(&self) -> Result<usize> {
        self.tor_isolation_retries
            .unwrap_or(2)
            .checked_add(1)
            .context("network.tor_isolation_retries overflows total attempt count")
    }
    fn namespace(&self) -> &str {
        self.isolation_namespace
            .as_deref()
            .unwrap_or("x402_treazury")
    }
    pub(crate) fn connect_timeout(&self) -> Duration {
        Duration::from_secs(self.connect_timeout_seconds.unwrap_or(match self.mode {
            Mode::Direct => 15,
            Mode::Tor => 120,
        }))
    }
    pub fn inspection(&self) -> serde_json::Value {
        serde_json::json!({"cover_traffic_enabled":self.cover_enabled(),"cover_limits":self.cover_limits,"mode":self.mode,"socks_endpoint":self.socks_endpoint,"isolation_namespace":self.namespace(),"socks_auth":self.socks_auth.as_ref().unwrap_or(&SocksAuth::TorExtended),"connect_timeout_seconds":self.connect_timeout().as_secs(),"read_timeout_seconds":self.read_timeout_seconds.unwrap_or(60),"tor_isolation_retries":self.tor_isolation_retries.unwrap_or(2),"identity_scopes":["evm_address","treasury_uuid","discovery_origin","bootstrap_invocation"]})
    }
    pub fn load(path: &Path) -> Result<Self> {
        Ok(NetworkConfig::load(path)?.network)
    }
}
/// Invocation-local transport and optional deployment relay-fallback overrides.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    pub network: NetworkPolicy,
    pub relay_fallback: Option<crate::discovery_relay::Policy>,
}
impl NetworkConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let mut file: Self =
            toml::from_str(&std::fs::read_to_string(path).context("reading network config")?)
                .context("invalid network config")?;
        file.network.validate()?;
        if let Some(relay) = &mut file.relay_fallback {
            relay.resolve_paths(path)?;
        }
        Ok(file)
    }
}
/// Canonical, domain-separated identity. Never contains signing keys or API credentials.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IsolationId(Vec<u8>);
impl IsolationId {
    fn fields(kind: &str, values: &[&[u8]]) -> Self {
        let mut bytes = Vec::new();
        for field in std::iter::once(kind.as_bytes()).chain(values.iter().copied()) {
            bytes.extend_from_slice(&(field.len() as u32).to_be_bytes());
            bytes.extend_from_slice(field);
        }
        Self(bytes)
    }
    pub fn evm(address: &str) -> Result<Self> {
        let address: alloy_primitives::Address = address
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid network EVM identity"))?;
        Ok(Self::fields("evm", &[b"eip155:8453", address.as_slice()]))
    }
    pub fn treasury(id: &str) -> Self {
        Self::fields("treasury", &[id.as_bytes()])
    }
    /// Owned transparent shielding has no EVM recipient. Keep each address in
    /// its own stable network/treasury scope, including after journal reload.
    pub(crate) fn transparent_treasury(
        network: crate::rotation::store::TreasuryNetwork,
        treasury: &str,
        address: &str,
    ) -> Result<Self> {
        let chain = network.chain();
        let Some(zcash_keys::address::Address::Transparent(receiver)) =
            zcash_keys::address::Address::decode(&chain, address)
        else {
            anyhow::bail!("invalid transparent network identity")
        };
        let canonical = zcash_keys::address::Address::Transparent(receiver).encode(&chain);
        Ok(Self::fields(
            "zcash-transparent",
            &[
                network.name().as_bytes(),
                treasury.as_bytes(),
                canonical.as_bytes(),
            ],
        ))
    }
    pub fn bootstrap() -> Self {
        Self::fields("bootstrap", &[uuid::Uuid::new_v4().as_bytes()])
    }
    pub fn discovery(url: &str) -> Result<Self> {
        let url = reqwest::Url::parse(url).context("invalid discovery URL")?;
        ensure!(
            matches!(url.scheme(), "http" | "https"),
            "unsupported network URL scheme"
        );
        let host = url.host_str().context("missing discovery host")?;
        let port = url
            .port_or_known_default()
            .context("missing discovery port")?
            .to_be_bytes();
        Ok(Self::fields(
            "discovery",
            &[url.scheme().as_bytes(), host.as_bytes(), &port],
        ))
    }
    pub fn token(&self, namespace: &str) -> String {
        let mut hash = Sha256::new();
        hash.update(b"x402_treazury-tor-isolation-v1\0");
        hash.update((namespace.len() as u32).to_be_bytes());
        hash.update(namespace.as_bytes());
        hash.update(&self.0);
        alloy_primitives::hex::encode(hash.finalize())
    }
    pub fn fresh_scope(&self) -> Self {
        Self::fields("tor-scope", &[&self.0, uuid::Uuid::new_v4().as_bytes()])
    }
}
/// Provider HTTPS defaults; infrastructure clients retain their existing policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HttpPolicy {
    pub allow_http1: bool,
    pub allow_tls12: bool,
}
impl HttpPolicy {
    pub(crate) const COMPATIBLE: Self = Self {
        allow_http1: true,
        allow_tls12: true,
    };
}

/// Only protocol metadata: never log URLs, credentials or payment headers.
pub fn log_http(response: &reqwest::Response, stage: &str) {
    tracing::debug!(target: "x402_treazury::network", stage,
        http_version = ?response.version(), "HTTP response protocol");
}

/// Separates legacy deterministic HTTP pooling from participating Tor selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelectionPolicy {
    #[default]
    Legacy,
    Participating,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct HttpKey {
    selection: SelectionPolicy,
    transport: HttpPolicy,
    public_only: bool,
    runtime: Option<tokio::runtime::Id>,
    identity: IsolationId,
    origin: String,
    read_timeout: Duration,
}

/// Immutable HTTP transport record pairing a pooled client with its canonical
/// owner identity and effective isolation scope.
#[derive(Debug)]
pub struct HttpTransport {
    pub client: reqwest::Client,
    pub identity: IsolationId,
    pub scope: IsolationId,
    key: HttpKey,
}
impl HttpTransport {
    pub fn selection(&self) -> SelectionPolicy {
        self.key.selection
    }
    pub fn origin(&self) -> &str {
        &self.key.origin
    }
    pub fn transport_policy(&self) -> HttpPolicy {
        self.key.transport
    }
    pub fn public_only(&self) -> bool {
        self.key.public_only
    }
    pub fn read_timeout(&self) -> Duration {
        self.key.read_timeout
    }
}

struct CachedClient<T> {
    client: T,
    last_used: std::time::Instant,
}
impl<T> CachedClient<T> {
    fn new(client: T) -> Self {
        Self {
            client,
            last_used: std::time::Instant::now(),
        }
    }
    fn touch(&mut self) -> &T {
        self.last_used = std::time::Instant::now();
        &self.client
    }
}
fn evict_oldest<K: Clone + Eq + std::hash::Hash, T>(
    cache: &mut HashMap<K, CachedClient<T>>,
    resource: &'static str,
) {
    if cache.len() >= 256
        && let Some(key) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| key.clone())
    {
        cache.remove(&key);
        tracing::debug!(
            resource,
            limit_entries = 256,
            action = "evicted",
            "Transport cache capacity reached; oldest cache ownership evicted; active handles remain usable"
        );
    }
}
pub struct NetworkContext {
    pub cover: Option<Arc<crate::cover::runtime::Engine>>,
    pub policy: NetworkPolicy,
    http: Mutex<HashMap<HttpKey, CachedClient<Arc<HttpTransport>>>>,
    startup_socks: tokio::sync::OnceCell<()>,
    #[cfg(test)]
    test_root: Option<&'static [u8]>,
    #[cfg(test)]
    test_dns: Option<Arc<dyn reqwest::dns::Resolve>>,
    grpc: tokio::sync::Mutex<
        HashMap<(tokio::runtime::Id, IsolationId, String), CachedClient<GrpcIndexer>>,
    >,
}
impl NetworkContext {
    /// Local startup prerequisite only; no destination or SOCKS credentials cross it.
    async fn connect_startup_socks(&self, endpoint: SocketAddr) -> std::io::Result<()> {
        tokio::time::timeout(
            self.policy.connect_timeout(),
            tokio::net::TcpStream::connect(endpoint),
        )
        .await
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::TimedOut))?
        .map(drop)
    }
    #[cfg(test)]
    pub(crate) fn http_cache_len(&self) -> usize {
        self.http.lock().expect("network cache poisoned").len()
    }
    pub fn new(policy: NetworkPolicy) -> Result<Self> {
        policy.validate()?;
        Ok(Self {
            cover: policy
                .cover_enabled()
                .then(|| crate::cover::runtime::Engine::new(policy.cover_limits.clone())),
            policy,
            http: Mutex::new(HashMap::new()),
            startup_socks: tokio::sync::OnceCell::new(),
            #[cfg(test)]
            test_root: None,
            #[cfg(test)]
            test_dns: None,
            grpc: tokio::sync::Mutex::new(HashMap::new()),
        })
    }
    /// HTTP inactivity has one policy in both transports. Explicit caller
    /// overrides replace the network default; there is no Tor floor.
    pub fn read_timeout(&self, requested: impl Into<Option<Duration>>) -> Duration {
        requested
            .into()
            .unwrap_or_else(|| Duration::from_secs(self.policy.read_timeout_seconds.unwrap_or(60)))
    }
    pub fn credentials(&self, id: &IsolationId) -> (String, String) {
        let namespace = self.policy.namespace();
        let token = id.token(namespace);
        match self
            .policy
            .socks_auth
            .as_ref()
            .unwrap_or(&SocksAuth::TorExtended)
        {
            SocksAuth::TorExtended => ("<torS0X>0".into(), format!("{namespace}:v1:{token}")),
            SocksAuth::Legacy => (namespace.into(), format!("v1:{token}")),
        }
    }
    /// A fixture CA changes trust only in unit-test binaries, never URL policy,
    /// SNI, certificate verification, identities, redirects or proxy selection.
    #[cfg(test)]
    pub(crate) fn with_test_dns(mut self, resolver: Arc<dyn reqwest::dns::Resolve>) -> Self {
        self.test_dns = Some(resolver);
        self
    }
    #[cfg(test)]
    pub(crate) fn with_test_root(mut self, root: &'static [u8]) -> Self {
        self.test_root = Some(root);
        self
    }
    pub fn http(
        &self,
        id: &IsolationId,
        url: &str,
        timeout: impl Into<Option<Duration>>,
    ) -> Result<reqwest::Client> {
        self.http_policy(id, url, timeout, false, HttpPolicy::COMPATIBLE)
    }
    pub fn http_public(
        &self,
        id: &IsolationId,
        url: &str,
        timeout: impl Into<Option<Duration>>,
    ) -> Result<reqwest::Client> {
        public_url(url)?;
        self.http_policy(id, url, timeout, true, HttpPolicy::COMPATIBLE)
    }
    pub fn http_policy(
        &self,
        id: &IsolationId,
        url: &str,
        timeout: impl Into<Option<Duration>>,
        public_only: bool,
        transport: HttpPolicy,
    ) -> Result<reqwest::Client> {
        Ok(self
            .http_transport(
                id,
                url,
                timeout,
                public_only,
                transport,
                SelectionPolicy::Legacy,
            )?
            .client
            .clone())
    }
    pub fn http_participating(
        &self,
        id: &IsolationId,
        url: &str,
        timeout: impl Into<Option<Duration>>,
        public_only: bool,
        transport: HttpPolicy,
    ) -> Result<Arc<HttpTransport>> {
        self.http_transport(
            id,
            url,
            timeout,
            public_only,
            transport,
            SelectionPolicy::Participating,
        )
    }
    pub fn http_transport(
        &self,
        id: &IsolationId,
        url: &str,
        timeout: impl Into<Option<Duration>>,
        public_only: bool,
        transport: HttpPolicy,
        selection: SelectionPolicy,
    ) -> Result<Arc<HttpTransport>> {
        if public_only {
            public_url(url)?;
        }
        let timeout = self.read_timeout(timeout);
        let parsed = reqwest::Url::parse(url).context("invalid network URL")?;
        ensure!(
            matches!(parsed.scheme(), "http" | "https"),
            "unsupported network URL scheme"
        );
        let key = HttpKey {
            selection,
            transport,
            public_only,
            runtime: tokio::runtime::Handle::try_current().ok().map(|h| h.id()),
            identity: id.clone(),
            origin: parsed.origin().ascii_serialization(),
            read_timeout: timeout,
        };
        let mut clients = self.http.lock().expect("network cache poisoned");
        if let Some(entry) = clients.get_mut(&key) {
            return Ok(entry.touch().clone());
        }
        let record = self.build_http_transport(key.clone())?;
        // Eviction drops only cache ownership. Existing requests keep their connections.
        evict_oldest(&mut clients, "http_transport");
        clients.insert(key, CachedClient::new(record.clone()));
        Ok(record)
    }
    pub(crate) fn max_http_attempts(&self, direct_override: bool) -> usize {
        if self.policy.mode == Mode::Tor && !direct_override {
            self.policy
                .tor_attempts()
                .expect("validated network policy")
        } else {
            1
        }
    }
    pub(crate) fn http_attempt(
        &self,
        selected: &Arc<HttpTransport>,
        attempt: usize,
    ) -> Result<Arc<HttpTransport>> {
        if attempt == 1 {
            Ok(selected.clone())
        } else {
            self.http_candidate(selected)
        }
    }
    pub fn http_candidate(&self, selected: &Arc<HttpTransport>) -> Result<Arc<HttpTransport>> {
        ensure!(
            selected.selection() == SelectionPolicy::Participating,
            "candidate transport requires a participating selected record"
        );
        self.build_http_transport(selected.key.clone())
    }
    pub fn promote_http(
        &self,
        expected: &Arc<HttpTransport>,
        candidate: &Arc<HttpTransport>,
    ) -> bool {
        if expected.selection() != SelectionPolicy::Participating
            || expected.key != candidate.key
            || Arc::ptr_eq(expected, candidate)
        {
            return false;
        }
        let mut clients = self.http.lock().expect("network cache poisoned");
        let Some(entry) = clients.get_mut(&expected.key) else {
            return false;
        };
        if !Arc::ptr_eq(&entry.client, expected) {
            return false;
        }
        *entry = CachedClient::new(candidate.clone());
        true
    }
    fn build_http_transport(&self, key: HttpKey) -> Result<Arc<HttpTransport>> {
        let scope =
            if key.selection == SelectionPolicy::Participating && self.policy.mode == Mode::Tor {
                key.identity.fresh_scope()
            } else {
                key.identity.clone()
            };
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .retry(reqwest::retry::never())
            .read_timeout(key.read_timeout)
            .connect_timeout(self.policy.connect_timeout())
            .redirect(reqwest::redirect::Policy::none());
        if key.origin.starts_with("https://") {
            if !key.transport.allow_http1 {
                builder = builder.http2_prior_knowledge();
            }
            if !key.transport.allow_tls12 {
                builder = builder.min_tls_version(reqwest::tls::Version::TLS_1_3);
            }
        }
        #[cfg(test)]
        if self.policy.mode == Mode::Direct
            && let Some(resolver) = &self.test_dns
        {
            builder = builder.dns_resolver(resolver.clone());
        }
        if key.public_only && self.policy.mode == Mode::Direct {
            let resolver = PublicResolver::default();
            #[cfg(test)]
            let resolver = PublicResolver {
                lookup: self.test_dns.clone().unwrap_or(resolver.lookup),
            };
            builder = builder.dns_resolver(Arc::new(resolver));
        }
        #[cfg(test)]
        if let Some(root) = self.test_root {
            builder = builder.tls_certs_only([reqwest::Certificate::from_pem(root)?]);
        }
        if self.policy.mode == Mode::Tor {
            let (user, pass) = self.credentials(&scope);
            let proxy =
                reqwest::Proxy::all(format!("socks5h://{}", self.policy.socks_endpoint.unwrap()))?
                    .basic_auth(&user, &pass);
            builder = builder.proxy(proxy);
        }
        let client = builder
            .build()
            .context("network client construction failed")?;
        if key.selection == SelectionPolicy::Participating && self.policy.mode == Mode::Tor {
            scope_observation::observe(|| ScopeObservation {
                canonical: self.credentials(&key.identity),
                scope: self.credentials(&scope),
                origin: key.origin.clone(),
            })?;
        }
        Ok(Arc::new(HttpTransport {
            client,
            identity: key.identity.clone(),
            scope,
            key,
        }))
    }
    pub fn discovery(
        &self,
        url: &str,
        timeout: impl Into<Option<Duration>>,
    ) -> Result<reqwest::Client> {
        self.http(&IsolationId::discovery(url)?, url, timeout)
    }
}
static NETWORK: OnceLock<Arc<NetworkContext>> = OnceLock::new();
pub fn global() -> &'static Arc<NetworkContext> {
    NETWORK.get_or_init(|| {
        Arc::new(NetworkContext::new(NetworkPolicy::default()).expect("valid direct default"))
    })
}
pub fn install(policy: NetworkPolicy) -> Result<()> {
    let ctx = Arc::new(NetworkContext::new(policy.clone())?);
    if NETWORK.set(ctx).is_err() {
        ensure!(
            global().policy == policy,
            "network policy cannot change after initialization; restart required"
        );
    }
    Ok(())
}
#[cfg(test)]
pub(crate) fn install_test_context(context: NetworkContext) {
    assert!(
        NETWORK.set(Arc::new(context)).is_ok(),
        "test policy already installed"
    );
}
pub fn provider_discovery(
    url: &str,
    timeout: impl Into<Option<Duration>>,
    policy: HttpPolicy,
) -> Result<reqwest::Client> {
    global().http_policy(&IsolationId::discovery(url)?, url, timeout, false, policy)
}
pub fn discovery(url: &str, timeout: impl Into<Option<Duration>>) -> Result<reqwest::Client> {
    global().discovery(url, timeout)
}

pub(crate) fn eligible_retry_status(status: reqwest::StatusCode) -> bool {
    matches!(
        status.as_u16(),
        400 | 403 | 408 | 429 | 500 | 502 | 503 | 504
    )
}

pub(crate) fn tls_failure(error: &reqwest::Error) -> bool {
    let mut source = std::error::Error::source(error);
    while let Some(error) = source {
        if error.is::<rustls::Error>() {
            return true;
        }
        // tokio-rustls emits this fixed local marker without a rustls::Error
        // when the peer closes during the handshake. It is not upstream prose.
        if let Some(io) = error.downcast_ref::<std::io::Error>()
            && io.kind() == std::io::ErrorKind::UnexpectedEof
            && (io.to_string() == "tls handshake eof"
                || io
                    .to_string()
                    .starts_with("peer closed connection without sending TLS close_notify"))
        {
            return true;
        }
        source = error
            .downcast_ref::<std::io::Error>()
            .and_then(|io| io.get_ref())
            .map(|inner| inner as &(dyn std::error::Error + 'static))
            .or_else(|| error.source());
    }
    false
}

pub(crate) fn eligible_retry_transport(error: &reqwest::Error) -> bool {
    if error.is_builder() || error.is_redirect() || error.is_status() {
        return false;
    }
    // A Tor exit can break TLS. A new scope retries with unchanged trust rules.
    if tls_failure(error) {
        return true;
    }
    eligible_retry_cause(error)
}

// These exact labels come from the pinned local connector implementation, never
// a remote response body. Unlocated refusals retain ordinary transport eligibility.
fn eligible_retry_cause(mut error: &(dyn std::error::Error + 'static)) -> bool {
    let mut local_proxy = false;
    loop {
        if error.is::<PublicDestinationRejected>() {
            return false;
        }
        if let Some(hyper) = error.downcast_ref::<hyper::Error>()
            && (hyper.is_parse() || hyper.is_user() || hyper.is_canceled())
        {
            return false;
        }
        let text = error.to_string();
        local_proxy |= text == "SOCKS local proxy connection failed";
        if local_proxy
            && error.downcast_ref::<std::io::Error>().is_some_and(|io| {
                matches!(
                    io.kind(),
                    std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::InvalidInput
                )
            })
        {
            return false;
        }
        if matches!(
            text.as_str(),
            "socks proxy destination has no host"
                | "SOCKS error: server does not support user/pass authentication"
                | "SOCKS error: server implements authentication incorrectly"
                | "SOCKS error: credentials not accepted"
                | "SOCKS error: failed parsing server response"
                | "SOCKS error: failed serialize request"
                | "SOCKS error: missing destination host"
                | "SOCKS error: missing destination port"
                | "SOCKS error: host address is more than 255 characters"
                | "SOCKS error: IPV6 is not supported"
        ) {
            return false;
        }
        let next = error
            .downcast_ref::<std::io::Error>()
            .and_then(|io| io.get_ref())
            .map(|inner| inner as &(dyn std::error::Error + 'static))
            .or_else(|| error.source());
        match next {
            Some(next) => error = next,
            None => return true,
        }
    }
}

pub(crate) fn parse_retry_after(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
) -> Option<Duration> {
    if !matches!(status.as_u16(), 429 | 503) {
        return None;
    }
    let mut values = headers.get_all(reqwest::header::RETRY_AFTER).iter();
    let first = values.next()?.to_str().ok()?.trim();
    if first.is_empty() {
        return None;
    }
    for value in values {
        if value.to_str().ok()?.trim() != first {
            return None;
        }
    }
    if first.bytes().all(|b| b.is_ascii_digit()) {
        let secs = first.parse::<u64>().ok()?;
        return (secs > 0).then(|| Duration::from_secs(secs));
    }
    let target = httpdate::parse_http_date(first).ok()?;
    target
        .duration_since(std::time::SystemTime::now())
        .ok()
        .filter(|delay| !delay.is_zero())
}

/// Retain a valid server delay even when the platform cannot represent its
/// deadline. Such a delay must never collapse into permission to dispatch now.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RetryAfter {
    At(tokio::time::Instant),
    BeyondClock,
}
impl RetryAfter {
    pub(crate) fn after(delay: Duration) -> Self {
        match tokio::time::Instant::now().checked_add(delay) {
            Some(deadline) => Self::At(deadline),
            None => Self::BeyondClock,
        }
    }
}
pub(crate) async fn sleep_retry_after(deadline: Option<RetryAfter>) {
    match deadline {
        Some(RetryAfter::At(deadline)) => tokio::time::sleep_until(deadline).await,
        Some(RetryAfter::BeyondClock) => std::future::pending::<()>().await,
        None => {}
    }
}

impl NetworkContext {
    pub async fn release_grpc(&self, id: &IsolationId) {
        let runtime = tokio::runtime::Handle::current().id();
        self.grpc
            .lock()
            .await
            .retain(|(owner, identity, _), _| *owner != runtime || identity != id);
    }
    pub async fn grpc(&self, id: &IsolationId, url: &str) -> Result<GrpcIndexer> {
        use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
        let key = (
            tokio::runtime::Handle::current().id(),
            id.clone(),
            url.to_owned(),
        );
        if let Some(client) = self.grpc.lock().await.get_mut(&key) {
            return Ok(client.touch().clone());
        }
        zingo_netutils::ensure_default_crypto_provider();
        let uri: tonic::codegen::http::Uri = url
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid indexer URI"))?;
        ensure!(
            matches!(uri.scheme_str(), Some("http" | "https")),
            "unsupported indexer URI scheme"
        );
        let mut endpoint = Endpoint::from_shared(url.to_owned())
            .map_err(|_| anyhow::anyhow!("invalid indexer endpoint"))?
            .connect_timeout(self.policy.connect_timeout())
            .tcp_nodelay(true);
        if uri.scheme_str() == Some("https") {
            let tls = ClientTlsConfig::new().with_webpki_roots();
            #[cfg(test)]
            let tls = if let Some(root) = self.test_root {
                tls.ca_certificate(tonic::transport::Certificate::from_pem(root))
            } else {
                tls
            };
            endpoint = endpoint.tls_config(tls)?;
        }
        let readiness = grpc::Readiness::new(self.policy.connect_timeout());
        let channel: Channel = if self.policy.mode == Mode::Tor {
            let proxy = self
                .policy
                .socks_endpoint
                .context("SOCKS endpoint missing")?;
            let (user, pass) = self.credentials(id);
            let mut tcp = hyper_util::client::legacy::connect::HttpConnector::new();
            tcp.enforce_http(false);
            let proxy_uri = format!("http://{proxy}")
                .parse()
                .expect("literal SOCKS endpoint");
            let connector =
                hyper_util::client::legacy::connect::proxy::SocksV5::new(proxy_uri, tcp)
                    .with_auth(user, pass)
                    .local_dns(false);
            endpoint
                .connect_with_connector(readiness.connector(connector))
                .await
                .map_err(|error| grpc::connection_failure(&error, "tor_grpc_connection_failed"))?
        } else {
            let mut tcp = hyper_util::client::legacy::connect::HttpConnector::new();
            tcp.enforce_http(false);
            tcp.set_nodelay(true);
            endpoint
                .connect_with_connector(readiness.connector(tcp))
                .await
                .map_err(|error| grpc::connection_failure(&error, "grpc_connection_failed"))?
        };
        readiness.established();
        let client = GrpcIndexer {
            clear_net_client: zingo_netutils::lightwallet_protocol::CompactTxStreamerClient::new(
                indexer::IndexerTransport::new(grpc::ProgressChannel::new(
                    channel,
                    self.read_timeout(None),
                    readiness,
                )),
            ),
        };
        let mut cache = self.grpc.lock().await;
        // Another caller may have populated this key while connection setup ran.
        if let Some(existing) = cache.get_mut(&key) {
            return Ok(existing.touch().clone());
        }
        evict_oldest(&mut cache, "grpc_transport");
        cache.insert(key, CachedClient::new(client.clone()));
        Ok(client)
    }
}

/// Restrict untrusted, agent-selected endpoints without changing trusted TOML sources.
pub fn public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_broadcast()
                || v.is_documentation()
                || v.is_unspecified()
                || v.is_multicast()
                || o[0] == 0
                || o[0] >= 240
                || (o[0] == 100 && (64..=127).contains(&o[1]))
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 192 && o[1] == 88 && o[2] == 99))
        }
        std::net::IpAddr::V6(v) => v
            .to_ipv4_mapped()
            .map(|v| public_ip(v.into()))
            .unwrap_or_else(|| {
                let s = v.segments();
                (s[0] & 0xe000) == 0x2000
                    && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
                    && s[0] != 0x2002
                    && !(s[0] == 0x3fff && s[1] < 0x1000)
            }),
    }
}
pub fn public_url(url: &str) -> Result<reqwest::Url> {
    let u = reqwest::Url::parse(url).map_err(|_| anyhow::anyhow!("invalid source URL"))?;
    ensure!(
        u.scheme() == "https"
            && u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none(),
        "source URLs require HTTPS without userinfo or fragments"
    );
    let host = u
        .host_str()
        .context("source URL requires host")?
        .trim_end_matches('.');
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        ensure!(public_ip(ip), "non-public destination rejected");
    } else {
        ensure!(
            host.contains('.')
                && ![
                    "localhost",
                    "local",
                    "internal",
                    "home",
                    "lan",
                    "test",
                    "invalid",
                    "example",
                    "onion"
                ]
                .iter()
                .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}"))),
            "non-public destination name rejected"
        );
    }
    Ok(u)
}
struct SystemResolver;
impl reqwest::dns::Resolve for SystemResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}
struct PublicResolver {
    lookup: Arc<dyn reqwest::dns::Resolve>,
}
impl Default for PublicResolver {
    fn default() -> Self {
        Self {
            lookup: Arc::new(SystemResolver),
        }
    }
}
impl reqwest::dns::Resolve for PublicResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let lookup = self.lookup.resolve(name);
        Box::pin(async move {
            let addresses: Vec<_> = lookup.await?.collect();
            validate_public_addresses(&addresses)?;
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

#[cfg(test)]
#[path = "../tests/support/network_boundary.rs"]
mod boundary_tests;

#[derive(Debug)]
struct PublicDestinationRejected;
impl std::fmt::Display for PublicDestinationRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("non-public DNS destination rejected")
    }
}
impl std::error::Error for PublicDestinationRejected {}

fn validate_public_addresses(addresses: &[std::net::SocketAddr]) -> std::io::Result<()> {
    if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
        return Err(std::io::Error::other(PublicDestinationRejected));
    }
    Ok(())
}
#[cfg(test)]
mod public_destination_tests {
    use super::*;
    #[test]
    fn every_dns_answer_is_checked_at_each_resolution() {
        let good = "1.1.1.1:443".parse().unwrap();
        let bad = "127.0.0.1:443".parse().unwrap();
        assert!(validate_public_addresses(&[good]).is_ok());
        assert!(validate_public_addresses(&[good, bad]).is_err());
        assert!(validate_public_addresses(&[bad]).is_err());
        assert!(validate_public_addresses(&[]).is_err());
        for ip in [
            "10.0.0.1",
            "100.64.0.1",
            "192.88.99.1",
            "198.19.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "2002:7f00:1::",
            "3fff::1",
            "::ffff:127.0.0.1",
            "0.0.0.0",
            "255.255.255.255",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
    }
}

#[cfg(test)]
#[path = "network_http_tests.rs"]
mod http_tests;

/// Dedicated supervisor boundary: literal loopback Tor control, never provider egress.
/// The application does not use this to bypass its installed network policy.
pub async fn local_control(
    address: std::net::SocketAddr,
    timeout: Duration,
) -> Result<tokio::net::TcpStream> {
    ensure!(
        address.ip().is_loopback() && address.port() != 0,
        "control transport requires a literal loopback endpoint"
    );
    ensure!(
        timeout >= Duration::from_millis(1) && timeout <= Duration::from_secs(10),
        "control connection deadline must be 1..10000 milliseconds"
    );
    Ok(
        tokio::time::timeout(timeout, tokio::net::TcpStream::connect(address))
            .await
            .map_err(|error| {
                tracing::warn!(
                    stage = "local_control_connection",
                    limit_ms = timeout.as_millis(),
                    "Local control connection deadline exceeded"
                );
                anyhow::Error::new(error).context("local control connection deadline exceeded")
            })??,
    )
}

/// Loopback-only probes for an explicitly invoked qualification subprocess.
/// This is separate from provider egress and is never selected by API configuration.
pub async fn qualification_probe(
    address: std::net::SocketAddr,
    udp: bool,
) -> Result<serde_json::Value> {
    ensure!(
        address.ip().is_loopback() && address.port() != 0,
        "qualification probe requires literal loopback"
    );
    let operation = async {
        if udp {
            let local = if address.is_ipv4() {
                "127.0.0.1:0"
            } else {
                "[::1]:0"
            };
            let socket = tokio::net::UdpSocket::bind(local).await?;
            socket.send_to(b"qualification", address).await?;
            let mut bytes = [0; 65536];
            let (size, peer) = socket.recv_from(&mut bytes).await?;
            if peer != address || &bytes[..size] != b"qualification" {
                return Err(std::io::Error::other("probe reply differs"));
            }
        } else {
            let _ = tokio::net::TcpStream::connect(address).await?;
        }
        Ok::<_, std::io::Error>(())
    };
    Ok(
        match tokio::time::timeout(Duration::from_secs(2), operation).await {
            Ok(Ok(())) => serde_json::json!({"reached":true,"denied":false}),
            Ok(Err(error)) => {
                serde_json::json!({"reached":false,"denied":matches!(error.raw_os_error(),Some(libc::EPERM|libc::EACCES)),"error_kind":format!("{:?}",error.kind())})
            }
            Err(_) => {
                tracing::warn!(
                    stage = "qualification_probe",
                    limit_seconds = 2,
                    "Explicit qualification connectivity probe timed out"
                );
                serde_json::json!({"reached":false,"denied":false,"error_kind":"timeout"})
            }
        },
    )
}
/// Owner-held positive-control listeners, used only by the explicit test supervisor.
pub struct QualificationProbes {
    pub addresses: std::collections::BTreeMap<String, std::net::SocketAddr>,
    counters: std::sync::Arc<[std::sync::atomic::AtomicU64; 4]>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl QualificationProbes {
    pub async fn start() -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        let counters = std::sync::Arc::new(std::array::from_fn(|_| AtomicU64::new(0)));
        let mut owner = Self {
            addresses: Default::default(),
            counters,
            tasks: Vec::new(),
        };
        for (index, (name, local, udp)) in [
            ("tcp4", "127.0.0.1:0", false),
            ("tcp6", "[::1]:0", false),
            ("udp4", "127.0.0.1:0", true),
            ("udp6", "[::1]:0", true),
        ]
        .into_iter()
        .enumerate()
        {
            let counters = owner.counters.clone();
            if udp {
                let socket = tokio::net::UdpSocket::bind(local).await?;
                owner.addresses.insert(name.into(), socket.local_addr()?);
                owner.tasks.push(tokio::spawn(async move {
                    let mut bytes=vec![0;65536];
                    while let Ok((size,peer))=socket.recv_from(&mut bytes).await {
                        counters[index].fetch_add(1,Ordering::SeqCst);
                        if &bytes[..size]==b"qualification" {let _=socket.send_to(b"qualification",peer).await;}
                        else {eprintln!("qualification UDP probe received unexpected data; positive-control counts will reject the observation");}
                    }
                }));
            } else {
                let listener = tokio::net::TcpListener::bind(local).await?;
                owner.addresses.insert(name.into(), listener.local_addr()?);
                owner.tasks.push(tokio::spawn(async move {
                    while let Ok((stream, _)) = listener.accept().await {
                        counters[index].fetch_add(1, Ordering::SeqCst);
                        drop(stream);
                    }
                }));
            }
        }
        Ok(owner)
    }
    pub fn counts(&self) -> Vec<u64> {
        self.counters
            .iter()
            .map(|c| c.load(std::sync::atomic::Ordering::SeqCst))
            .collect()
    }
}
impl Drop for QualificationProbes {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[cfg(test)]
mod retention_tests {
    use super::*;

    #[tokio::test]
    async fn fractional_millisecond_timeouts_separate_transports_and_cover_owners() {
        let context = NetworkContext::new(NetworkPolicy::default()).unwrap();
        let origin = "https://api.example.com";
        let owner = IsolationId::discovery(origin).unwrap();
        for selection in [SelectionPolicy::Legacy, SelectionPolicy::Participating] {
            let acquire = |timeout| {
                context
                    .http_transport(
                        &owner,
                        origin,
                        timeout,
                        false,
                        HttpPolicy::default(),
                        selection,
                    )
                    .unwrap()
            };
            let first = acquire(Duration::from_micros(500));
            let second = acquire(Duration::from_micros(501));
            assert!(Arc::ptr_eq(&first, &acquire(Duration::from_micros(500))));
            assert!(!Arc::ptr_eq(&first, &second));
            assert_eq!(first.read_timeout(), Duration::from_micros(500));
            assert_eq!(second.read_timeout(), Duration::from_micros(501));
            // The pinned reqwest client's Debug exposes its actual configured
            // read timeout, avoiding a scheduler-dependent submillisecond I/O test.
            assert!(format!("{:?}", first.client).contains("read_timeout: 500µs"));
            assert!(format!("{:?}", second.client).contains("read_timeout: 501µs"));
            assert_ne!(
                crate::cover::registry::Owner::from_transport(&first),
                crate::cover::registry::Owner::from_transport(&second)
            );
            let candidate = if selection == SelectionPolicy::Participating {
                context.http_candidate(&first).unwrap()
            } else {
                continue;
            };
            assert_eq!(candidate.read_timeout(), first.read_timeout());
            assert!(format!("{:?}", candidate.client).contains("read_timeout: 500µs"));
        }
    }

    #[test]
    fn transport_cache_eviction_preserves_handles_without_warning_or_keys() {
        let logs = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(logs.reopen().unwrap())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            for resource in ["http_transport", "grpc_transport"] {
                let mut cache = HashMap::new();
                for n in 0..256 {
                    cache.insert(format!("PRIVATE_KEY_{n}"), CachedClient::new(Arc::new(n)));
                }
                let old = cache.get_mut("PRIVATE_KEY_0").unwrap();
                old.last_used -= Duration::from_secs(1);
                let retained = old.client.clone();
                evict_oldest(&mut cache, resource);
                assert_eq!(cache.len(), 255);
                assert!(!cache.contains_key("PRIVATE_KEY_0"));
                assert_eq!(*retained, 0);
                assert_eq!(Arc::strong_count(&retained), 1);
            }
        });
        let text = std::fs::read_to_string(logs.path()).unwrap();
        assert_eq!(
            text.lines().filter(|line| line.contains("DEBUG")).count(),
            2,
            "{text}"
        );
        for expected in [
            "http_transport",
            "grpc_transport",
            "limit_entries=256",
            "action=\"evicted\"",
        ] {
            assert!(text.contains(expected), "{text}");
        }
        assert!(!text.contains("PRIVATE_"), "{text}");
        assert!(!text.contains("WARN"), "{text}");
    }

    #[tokio::test]
    async fn interleaved_wallet_clients_preserve_frequent_provider_entry() {
        let context = NetworkContext::new(NetworkPolicy::default()).unwrap();
        let origin = "https://provider.example";
        let provider = IsolationId::discovery(origin).unwrap();
        let wallet = IsolationId::treasury("wallet-0");
        context
            .http(
                &wallet,
                "https://base.example/PRIVATE_PATH?key=PRIVATE_TOKEN",
                None,
            )
            .unwrap();
        let retained = context
            .http
            .lock()
            .unwrap()
            .iter()
            .find(|(key, _)| key.identity == wallet)
            .unwrap()
            .1
            .client
            .clone();
        let logs = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(logs.reopen().unwrap())
            .finish();
        let _subscriber = tracing::subscriber::set_default(subscriber);
        for n in 0..768 {
            context.http(&provider, origin, None).unwrap();
            context
                .http(
                    &IsolationId::treasury(&format!("wallet-{n}")),
                    "https://base.example",
                    None,
                )
                .unwrap();
            let cache = context.http.lock().unwrap();
            assert_eq!(cache.len(), (n + 2).min(256));
            assert!(cache.keys().any(|key| key.identity == provider));
        }
        let cache = context.http.lock().unwrap();
        assert!(
            !cache
                .keys()
                .any(|key| key.identity == IsolationId::treasury("wallet-0"))
        );
        assert_eq!(
            Arc::strong_count(&retained),
            1,
            "eviction drops cache ownership only"
        );
        assert_eq!(retained.read_timeout(), Duration::from_secs(60));
        let text = std::fs::read_to_string(logs.path()).unwrap();
        assert!(!text.contains("WARN"), "{text}");
        assert!(
            text.contains("DEBUG")
                && text.contains("http_transport")
                && text.contains("limit_entries=256")
                && text.contains("action=\"evicted\""),
            "{text}"
        );
        assert!(
            !text.contains("PRIVATE_")
                && !text.contains("wallet-0")
                && !text.contains("base.example"),
            "{text}"
        );
        assert!(
            cache
                .keys()
                .any(|key| key.identity == IsolationId::treasury("wallet-767"))
        );
    }

    #[tokio::test]
    async fn transport_records_share_selected_handle_and_separate_keys_and_policies() {
        let context = Arc::new(NetworkContext::new(NetworkPolicy::default()).unwrap());
        let origin = "https://api.example.com/v1/resource";
        let owner = IsolationId::evm("0x0000000000000000000000000000000000000001").unwrap();
        let other_owner = IsolationId::evm("0x0000000000000000000000000000000000000002").unwrap();
        let first = context
            .http_transport(
                &owner,
                origin,
                Duration::from_secs(5),
                false,
                HttpPolicy::default(),
                SelectionPolicy::Legacy,
            )
            .unwrap();
        let repeated = context
            .http_transport(
                &owner,
                "https://api.example.com/v2/other",
                Duration::from_secs(5),
                false,
                HttpPolicy::default(),
                SelectionPolicy::Legacy,
            )
            .unwrap();
        assert!(Arc::ptr_eq(&first, &repeated));
        assert_eq!(first.identity, owner);
        assert_eq!(first.scope, owner);
        assert_eq!(first.selection(), SelectionPolicy::Legacy);

        let participating = context
            .http_transport(
                &owner,
                origin,
                Duration::from_secs(5),
                false,
                HttpPolicy::default(),
                SelectionPolicy::Participating,
            )
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &participating));
        assert_eq!(participating.selection(), SelectionPolicy::Participating);
        assert_eq!(participating.identity, owner);
        assert_eq!(participating.scope, owner);

        let variations = [
            context
                .http_transport(
                    &other_owner,
                    origin,
                    Duration::from_secs(5),
                    false,
                    HttpPolicy::default(),
                    SelectionPolicy::Legacy,
                )
                .unwrap(),
            context
                .http_transport(
                    &owner,
                    "https://other.example.com/v1",
                    Duration::from_secs(5),
                    false,
                    HttpPolicy::default(),
                    SelectionPolicy::Legacy,
                )
                .unwrap(),
            context
                .http_transport(
                    &owner,
                    origin,
                    Duration::from_secs(6),
                    false,
                    HttpPolicy::default(),
                    SelectionPolicy::Legacy,
                )
                .unwrap(),
            context
                .http_transport(
                    &owner,
                    origin,
                    Duration::from_secs(5),
                    true,
                    HttpPolicy::default(),
                    SelectionPolicy::Legacy,
                )
                .unwrap(),
            context
                .http_transport(
                    &owner,
                    origin,
                    Duration::from_secs(5),
                    false,
                    HttpPolicy::COMPATIBLE,
                    SelectionPolicy::Legacy,
                )
                .unwrap(),
        ];
        for distinct in &variations {
            assert!(!Arc::ptr_eq(&first, distinct));
            assert!(!Arc::ptr_eq(&participating, distinct));
        }

        let ctx_for_rt = context.clone();
        let owner_for_rt = owner.clone();
        let other_runtime = tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                ctx_for_rt
                    .http_transport(
                        &owner_for_rt,
                        origin,
                        Duration::from_secs(5),
                        false,
                        HttpPolicy::default(),
                        SelectionPolicy::Legacy,
                    )
                    .unwrap()
            })
        })
        .await
        .unwrap();
        assert!(!Arc::ptr_eq(&first, &other_runtime));
        assert_eq!(context.http.lock().unwrap().len(), 8);
    }

    #[tokio::test]
    async fn participating_scopes_conditional_promotion_eviction_and_restart_isolation() {
        let policy = NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some("127.0.0.1:9150".parse().unwrap()),
            ..Default::default()
        };
        let context = Arc::new(NetworkContext::new(policy.clone()).unwrap());
        let origin = "https://api.example.com/v1/resource";
        let owner = IsolationId::evm("0x00000000000000000000000000000000000000aa").unwrap();
        let owner_alias = IsolationId::evm("0x00000000000000000000000000000000000000AA").unwrap();
        let other_wallet = IsolationId::evm("0x00000000000000000000000000000000000000bb").unwrap();
        let discovery_owner = IsolationId::discovery(origin).unwrap();

        let legacy = context
            .http_transport(
                &owner,
                origin,
                Duration::from_secs(5),
                false,
                HttpPolicy::default(),
                SelectionPolicy::Legacy,
            )
            .unwrap();
        assert_eq!(legacy.identity, owner);
        assert_eq!(legacy.scope, owner);
        assert!(context.http_candidate(&legacy).is_err());

        let selected0 = context
            .http_participating(
                &owner,
                origin,
                Duration::from_secs(5),
                false,
                HttpPolicy::default(),
            )
            .unwrap();
        assert_eq!(selected0.identity, owner);
        assert_ne!(selected0.scope, owner);
        let alias_hit = context
            .http_participating(
                &owner_alias,
                "https://API.EXAMPLE.COM:443/v2/other",
                Duration::from_secs(5),
                false,
                HttpPolicy::default(),
            )
            .unwrap();
        assert!(Arc::ptr_eq(&selected0, &alias_hit));

        let other_origin_slot = context
            .http_participating(
                &owner,
                "https://second.example.com/v1",
                Duration::from_secs(5),
                false,
                HttpPolicy::default(),
            )
            .unwrap();
        let other_wallet_slot = context
            .http_participating(
                &other_wallet,
                origin,
                Duration::from_secs(5),
                false,
                HttpPolicy::default(),
            )
            .unwrap();
        let discovery_slot = context
            .http_participating(
                &discovery_owner,
                origin,
                Duration::from_secs(5),
                false,
                HttpPolicy::default(),
            )
            .unwrap();
        let timeout_slot = context
            .http_participating(
                &owner,
                origin,
                Duration::from_secs(6),
                false,
                HttpPolicy::default(),
            )
            .unwrap();

        let count_before_candidates = context.http.lock().unwrap().len();
        let cand_a = context.http_candidate(&selected0).unwrap();
        let cand_b = context.http_candidate(&selected0).unwrap();
        assert_eq!(context.http.lock().unwrap().len(), count_before_candidates);
        assert_eq!(cand_a.identity, owner);
        assert_eq!(cand_b.identity, owner);
        assert_ne!(cand_a.scope, owner);
        assert_ne!(cand_b.scope, owner);
        assert_ne!(cand_a.scope, selected0.scope);
        assert_ne!(cand_b.scope, selected0.scope);
        assert_ne!(cand_a.scope, cand_b.scope);

        // Candidate creation does not replace the selected slot.
        let still_selected = context
            .http_participating(
                &owner,
                origin,
                Duration::from_secs(5),
                false,
                HttpPolicy::default(),
            )
            .unwrap();
        assert!(Arc::ptr_eq(&still_selected, &selected0));

        // First recovery promotes cand_a; late original completion and lost race cannot overwrite.
        assert!(!context.promote_http(&selected0, &selected0));
        assert!(!context.promote_http(&legacy, &cand_a));
        assert!(!context.promote_http(&other_origin_slot, &cand_a));
        assert!(context.promote_http(&selected0, &cand_a));
        assert!(!context.promote_http(&selected0, &selected0));
        assert!(!context.promote_http(&selected0, &cand_b));
        assert_eq!(context.http.lock().unwrap().len(), count_before_candidates);

        let after_promo = context
            .http_participating(
                &owner_alias,
                "https://api.example.com:443/v3",
                Duration::from_secs(5),
                false,
                HttpPolicy::default(),
            )
            .unwrap();
        assert!(Arc::ptr_eq(&after_promo, &cand_a));

        // Sibling keys and legacy slot remain untouched by promotion.
        assert!(Arc::ptr_eq(
            &context
                .http_transport(
                    &owner,
                    origin,
                    Duration::from_secs(5),
                    false,
                    HttpPolicy::default(),
                    SelectionPolicy::Legacy,
                )
                .unwrap(),
            &legacy
        ));
        assert!(Arc::ptr_eq(
            &context
                .http_participating(
                    &owner,
                    "https://second.example.com/v1",
                    Duration::from_secs(5),
                    false,
                    HttpPolicy::default(),
                )
                .unwrap(),
            &other_origin_slot
        ));
        assert!(Arc::ptr_eq(
            &context
                .http_participating(
                    &other_wallet,
                    origin,
                    Duration::from_secs(5),
                    false,
                    HttpPolicy::default(),
                )
                .unwrap(),
            &other_wallet_slot
        ));
        assert!(Arc::ptr_eq(
            &context
                .http_participating(
                    &discovery_owner,
                    origin,
                    Duration::from_secs(5),
                    false,
                    HttpPolicy::default(),
                )
                .unwrap(),
            &discovery_slot
        ));
        assert!(Arc::ptr_eq(
            &context
                .http_participating(
                    &owner,
                    origin,
                    Duration::from_secs(6),
                    false,
                    HttpPolicy::default(),
                )
                .unwrap(),
            &timeout_slot
        ));

        // Start a candidate from cand_a, then evict owner's slot via normal LRU pressure.
        let cand_c = context.http_candidate(&cand_a).unwrap();
        for n in 0..256 {
            context
                .http_participating(
                    &IsolationId::treasury(&format!("evict-{n}")),
                    origin,
                    Duration::from_secs(5),
                    false,
                    HttpPolicy::default(),
                )
                .unwrap();
        }
        assert_eq!(context.http.lock().unwrap().len(), 256);
        assert!(
            !context
                .http
                .lock()
                .unwrap()
                .keys()
                .any(|k| k.identity == owner)
        );
        // Stale candidate cannot resurrect an evicted entry.
        assert!(!context.promote_http(&cand_a, &cand_c));
        assert!(
            !context
                .http
                .lock()
                .unwrap()
                .keys()
                .any(|k| k.identity == owner)
        );

        // Reinsertion after eviction uses a fresh scope and still rejects the stale candidate.
        let reinserted = context
            .http_participating(
                &owner,
                origin,
                Duration::from_secs(5),
                false,
                HttpPolicy::default(),
            )
            .unwrap();
        assert_eq!(context.http.lock().unwrap().len(), 256);
        assert!(!context.promote_http(&cand_a, &cand_c));
        for prior in [
            &owner,
            &selected0.scope,
            &cand_a.scope,
            &cand_b.scope,
            &cand_c.scope,
        ] {
            assert_ne!(&reinserted.scope, prior);
        }

        // Recreating the context for restart selects a fresh scope rather than any prior scope.
        let restarted_ctx = NetworkContext::new(policy).unwrap();
        let after_restart = restarted_ctx
            .http_participating(
                &owner,
                origin,
                Duration::from_secs(5),
                false,
                HttpPolicy::default(),
            )
            .unwrap();
        for prior in [
            &owner,
            &selected0.scope,
            &cand_a.scope,
            &cand_b.scope,
            &cand_c.scope,
            &reinserted.scope,
        ] {
            assert_ne!(&after_restart.scope, prior);
        }
    }
}
