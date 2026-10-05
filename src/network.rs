//! All runtime egress is constructed here. Policy is immutable after startup.
use anyhow::{Context, Result, ensure};
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
    /// Tor-only floor for a complete request, including connection and body.
    pub request_timeout_seconds: Option<u64>,
}
impl NetworkPolicy {
    pub fn cover_enabled(&self) -> bool {
        self.cover_traffic_enabled.unwrap_or(self.mode == Mode::Tor)
    }

    pub fn validate(&self) -> Result<()> {
        self.cover_limits.validate()?;
        if self.mode == Mode::Direct {
            ensure!(
                self.socks_endpoint.is_none()
                    && self.isolation_namespace.is_none()
                    && self.socks_auth.is_none()
                    && self.connect_timeout_seconds.is_none()
                    && self.request_timeout_seconds.is_none(),
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
            ensure!(
                (1..=300).contains(&self.connect_timeout_seconds.unwrap_or(120)),
                "network connect timeout must be 1..300 seconds"
            );
            ensure!(
                (1..=86400).contains(&self.request_timeout_seconds.unwrap_or(240))
                    && self.request_timeout_seconds.unwrap_or(240) >= self.timeout().as_secs(),
                "network request timeout must be 1..86400 seconds and at least the connect timeout"
            );
        }
        Ok(())
    }
    fn namespace(&self) -> &str {
        self.isolation_namespace
            .as_deref()
            .unwrap_or("x402_treazury")
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(self.connect_timeout_seconds.unwrap_or(match self.mode {
            Mode::Direct => 15,
            Mode::Tor => 120,
        }))
    }
    pub fn inspection(&self) -> serde_json::Value {
        serde_json::json!({"cover_traffic_enabled":self.cover_enabled(),"cover_limits":self.cover_limits,"mode":self.mode,"socks_endpoint":self.socks_endpoint,"isolation_namespace":self.namespace(),"socks_auth":self.socks_auth.as_ref().unwrap_or(&SocksAuth::TorExtended),"connect_timeout_seconds":self.timeout().as_secs(),"request_timeout_seconds":self.request_timeout_seconds.or((self.mode == Mode::Tor).then_some(240)),"identity_scopes":["evm_address","treasury_uuid","discovery_origin","bootstrap_invocation"]})
    }
    pub fn load(path: &Path) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct File {
            network: NetworkPolicy,
        }
        let file: File =
            toml::from_str(&std::fs::read_to_string(path).context("reading network config")?)
                .context("invalid network config")?;
        file.network.validate()?;
        Ok(file.network)
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
}
/// Provider HTTPS defaults; infrastructure clients retain their existing policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HttpPolicy {
    pub allow_http1: bool,
    pub allow_tls12: bool,
}
impl HttpPolicy {
    const COMPATIBLE: Self = Self {
        allow_http1: true,
        allow_tls12: true,
    };
}

/// Only protocol metadata: never log URLs, credentials or payment headers.
pub fn log_http(response: &reqwest::Response, stage: &str) {
    tracing::info!(target: "x402_treazury::network", stage,
        http_version = ?response.version(), "HTTP response protocol");
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct HttpKey {
    transport: HttpPolicy,
    public_only: bool,
    runtime: Option<tokio::runtime::Id>,
    identity: IsolationId,
    origin: String,
    timeout_ms: u64,
}
pub struct NetworkContext {
    pub cover: Option<Arc<crate::cover::runtime::Engine>>,
    pub policy: NetworkPolicy,
    http: Mutex<HashMap<HttpKey, reqwest::Client>>,
    #[cfg(test)]
    test_root: Option<&'static [u8]>,
    #[cfg(test)]
    test_dns: Option<Arc<dyn reqwest::dns::Resolve>>,
    #[cfg(feature = "zcash")]
    grpc: tokio::sync::Mutex<
        HashMap<(tokio::runtime::Id, IsolationId, String), zingo_netutils::GrpcIndexer>,
    >,
}
impl NetworkContext {
    pub fn new(policy: NetworkPolicy) -> Result<Self> {
        policy.validate()?;
        Ok(Self {
            cover: policy
                .cover_enabled()
                .then(|| crate::cover::runtime::Engine::new(policy.cover_limits.clone())),
            policy,
            http: Mutex::new(HashMap::new()),
            #[cfg(test)]
            test_root: None,
            #[cfg(test)]
            test_dns: None,
            #[cfg(feature = "zcash")]
            grpc: tokio::sync::Mutex::new(HashMap::new()),
        })
    }
    /// Preserve direct-mode budgets; Tor needs room for circuit/stream setup.
    /// Callers may request longer deadlines, but not shorten the Tor policy floor.
    pub fn request_timeout(&self, requested: Duration) -> Duration {
        if self.policy.mode == Mode::Tor {
            requested.max(Duration::from_secs(
                self.policy.request_timeout_seconds.unwrap_or(240),
            ))
        } else {
            requested
        }
    }
    /// For outer connection guards; do not undercut the connector's Tor budget.
    pub fn connection_timeout(&self, direct: Duration) -> Duration {
        if self.policy.mode == Mode::Tor {
            self.policy.timeout()
        } else {
            direct
        }
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
    pub(crate) fn with_test_root(mut self, root: &'static [u8]) -> Self {
        self.test_root = Some(root);
        self
    }
    pub fn http(&self, id: &IsolationId, url: &str, timeout: Duration) -> Result<reqwest::Client> {
        self.http_policy(id, url, timeout, false, HttpPolicy::COMPATIBLE)
    }
    pub fn http_public(
        &self,
        id: &IsolationId,
        url: &str,
        timeout: Duration,
    ) -> Result<reqwest::Client> {
        public_url(url)?;
        self.http_policy(id, url, timeout, true, HttpPolicy::COMPATIBLE)
    }
    pub fn http_policy(
        &self,
        id: &IsolationId,
        url: &str,
        timeout: Duration,
        public_only: bool,
        transport: HttpPolicy,
    ) -> Result<reqwest::Client> {
        if public_only {
            public_url(url)?;
        }
        let timeout = self.request_timeout(timeout);
        let parsed = reqwest::Url::parse(url).context("invalid network URL")?;
        ensure!(
            matches!(parsed.scheme(), "http" | "https"),
            "unsupported network URL scheme"
        );
        let key = HttpKey {
            transport,
            public_only,
            runtime: tokio::runtime::Handle::try_current().ok().map(|h| h.id()),
            identity: id.clone(),
            origin: parsed.origin().ascii_serialization(),
            timeout_ms: timeout.as_millis().try_into()?,
        };
        let mut clients = self.http.lock().expect("network cache poisoned");
        if let Some(client) = clients.get(&key) {
            return Ok(client.clone());
        }
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .retry(reqwest::retry::never())
            .timeout(timeout)
            .connect_timeout(self.policy.timeout())
            .redirect(reqwest::redirect::Policy::none());
        if parsed.scheme() == "https" {
            if !transport.allow_http1 {
                builder = builder.http2_prior_knowledge();
            }
            if !transport.allow_tls12 {
                builder = builder.min_tls_version(reqwest::tls::Version::TLS_1_3);
            }
        }
        if public_only && self.policy.mode == Mode::Direct {
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
            let (user, pass) = self.credentials(id);
            let proxy =
                reqwest::Proxy::all(format!("socks5h://{}", self.policy.socks_endpoint.unwrap()))?
                    .basic_auth(&user, &pass);
            builder = builder.proxy(proxy);
        }
        let client = builder
            .build()
            .context("network client construction failed")?;
        // Eviction drops only cache ownership. Existing requests keep their connections.
        if clients.len() >= 256 {
            clients.clear();
        }
        clients.insert(key, client.clone());
        Ok(client)
    }
    pub fn discovery(&self, url: &str, timeout: Duration) -> Result<reqwest::Client> {
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
    timeout: Duration,
    policy: HttpPolicy,
) -> Result<reqwest::Client> {
    global().http_policy(&IsolationId::discovery(url)?, url, timeout, false, policy)
}
pub fn discovery(url: &str, timeout: Duration) -> Result<reqwest::Client> {
    global().discovery(url, timeout)
}

#[cfg(feature = "zcash")]
impl NetworkContext {
    pub async fn release_grpc(&self, id: &IsolationId) {
        let runtime = tokio::runtime::Handle::current().id();
        self.grpc
            .lock()
            .await
            .retain(|(owner, identity, _), _| *owner != runtime || identity != id);
    }
    pub async fn grpc(&self, id: &IsolationId, url: &str) -> Result<zingo_netutils::GrpcIndexer> {
        use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
        let key = (
            tokio::runtime::Handle::current().id(),
            id.clone(),
            url.to_owned(),
        );
        if let Some(client) = self.grpc.lock().await.get(&key) {
            return Ok(client.clone());
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
            .connect_timeout(self.policy.timeout())
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
                .connect_with_connector(connector)
                .await
                .map_err(|_| anyhow::anyhow!("tor_grpc_connection_failed"))?
        } else {
            endpoint
                .connect()
                .await
                .map_err(|_| anyhow::anyhow!("grpc_connection_failed"))?
        };
        let client = zingo_netutils::GrpcIndexer::from_channel(uri, channel);
        let mut cache = self.grpc.lock().await;
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(key, client.clone());
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

fn validate_public_addresses(addresses: &[std::net::SocketAddr]) -> std::io::Result<()> {
    if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
        return Err(std::io::Error::other("non-public DNS destination rejected"));
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
            .context("local control connection deadline exceeded")??,
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
            Err(_) => serde_json::json!({"reached":false,"denied":false,"error_kind":"timeout"}),
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
