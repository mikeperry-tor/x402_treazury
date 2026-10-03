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
    #[serde(default)]
    pub mode: Mode,
    pub socks_endpoint: Option<SocketAddr>,
    pub isolation_namespace: Option<String>,
    pub socks_auth: Option<SocksAuth>,
    pub connect_timeout_seconds: Option<u64>,
}
impl NetworkPolicy {
    pub fn validate(&self) -> Result<()> {
        if self.mode == Mode::Direct {
            ensure!(
                self.socks_endpoint.is_none()
                    && self.isolation_namespace.is_none()
                    && self.socks_auth.is_none()
                    && self.connect_timeout_seconds.is_none(),
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
                (1..=300).contains(&self.connect_timeout_seconds.unwrap_or(30)),
                "network connect timeout must be 1..300 seconds"
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
            Mode::Tor => 30,
        }))
    }
    pub fn inspection(&self) -> serde_json::Value {
        serde_json::json!({"mode":self.mode,"socks_endpoint":self.socks_endpoint,"isolation_namespace":self.namespace(),"socks_auth":self.socks_auth.as_ref().unwrap_or(&SocksAuth::TorExtended),"connect_timeout_seconds":self.timeout().as_secs(),"identity_scopes":["evm_address","treasury_uuid","discovery_origin","bootstrap_invocation"]})
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
#[derive(Clone, PartialEq, Eq, Hash)]
struct HttpKey {
    public_only: bool,
    runtime: Option<tokio::runtime::Id>,
    identity: IsolationId,
    origin: String,
    timeout_ms: u64,
}
pub struct NetworkContext {
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
        self.http_policy(id, url, timeout, false)
    }
    pub fn http_public(
        &self,
        id: &IsolationId,
        url: &str,
        timeout: Duration,
    ) -> Result<reqwest::Client> {
        public_url(url)?;
        self.http_policy(id, url, timeout, true)
    }
    fn http_policy(
        &self,
        id: &IsolationId,
        url: &str,
        timeout: Duration,
        public_only: bool,
    ) -> Result<reqwest::Client> {
        let parsed = reqwest::Url::parse(url).context("invalid network URL")?;
        ensure!(
            matches!(parsed.scheme(), "http" | "https"),
            "unsupported network URL scheme"
        );
        let key = HttpKey {
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
