#[cfg(test)]
mod contract_tests;
mod contracts;
mod credit_pricing;
mod input;
pub(crate) mod media;
pub(crate) use contracts::omission_instructions;
pub use contracts::{
    CatalogBuild, IncompleteCatalog, OperationIssue, Unsupported, append_omissions,
};
pub use input::BodyArgument;
pub(crate) mod diagnostics;
pub(crate) mod expansion;
#[cfg(test)]
mod expansion_tests;
mod sanitize;
pub use credit_pricing::CreditPricing;

use anyhow::{Context, Result, bail, ensure};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Fixed, content-free context for catalog failures. Retains the original error
/// chain (including numeric HTTP status) without parsing human error messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadStage {
    Headers,
    Body,
    LocalRead,
    Parse,
    Configuration,
    Generation,
    Selection,
    Publication,
}
impl LoadStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Headers => "headers",
            Self::Body => "body",
            Self::LocalRead => "local_read",
            Self::Parse => "parse",
            Self::Configuration => "configuration",
            Self::Generation => "generation",
            Self::Selection => "selection",
            Self::Publication => "publication",
        }
    }
}
impl std::fmt::Display for LoadStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "catalog {} failed", self.label())
    }
}
impl std::error::Error for LoadStage {}

/// Authored selection/configuration errors cannot be suppressed as an optional
/// remote outage. Keep the detailed cause for the operator without classifying
/// failures by their human-readable text.
#[derive(Debug)]
pub(crate) struct ToolConfigurationError {
    error: anyhow::Error,
    reason: &'static str,
}
impl std::fmt::Display for ToolConfigurationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid provider tool configuration: {}", self.error)
    }
}
impl std::error::Error for ToolConfigurationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.error.as_ref())
    }
}

const DEFAULT_PRICE: &str = "Cost: unknown.";
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Deployment source identity for discovery diagnostics, independent of tool prefixes.
    #[serde(skip)]
    pub discovery_source: Option<String>,
    #[serde(skip)]
    pub cache_revision: Option<String>,
    #[serde(skip)]
    pub relay_fallback: Option<std::sync::Arc<crate::discovery_relay::Relay>>,
    /// Only deferred serving discovery waits for its local SOCKS prerequisite.
    #[serde(skip)]
    #[doc(hidden)]
    pub wait_for_startup_socks: bool,
    pub http_cache_enabled: bool,
    /// Local catalog snapshot lifetime; zero disables catalog disk caching.
    pub catalog_cache_ttl_seconds: u64,
    #[serde(skip)]
    #[doc(hidden)]
    pub http_cache_directory: Option<std::path::PathBuf>,
    /// Only the explicit cache-warming command sets this; serving never changes egress.
    #[serde(skip)]
    #[doc(hidden)]
    pub http_cache_direct_warm_target: Option<crate::network::NetworkPolicy>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reliability_tags: Vec<crate::provider_status::ReliabilityTag>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reliability_note: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub response_mappings: BTreeMap<String, crate::output::ResponseMapping>,
    #[serde(skip_serializing_if = "crate::output::ImageLimits::is_default")]
    pub image_limits: crate::output::ImageLimits,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_traffic: Option<crate::cover::Config>,
    /// Explicit provider/source opt-out, including inherited cover settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_traffic_enabled: Option<bool>,
    pub allow_http1: bool,
    pub allow_tls12: bool,
    pub read_timeout_seconds: Option<f64>,
    pub max_response_bytes: usize,
    pub max_help_bytes: usize,
    pub max_spec_bytes: usize,
    pub spec: String,
    /// Optional protocol metadata for providers selected as relay adapters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_format: Option<String>,
    pub name: Option<String>,
    /// Searchable provider capabilities, separate from agent instructions.
    pub description: Option<String>,
    pub base_url: Option<String>,
    /// Opt into absolute request URLs carried by an OpenAPI operation extension.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_url_extension: Option<String>,
    pub prefix: Option<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub include_tools: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exclude_tools: Vec<String>,
    /// Advisory generated-name selectors; selection requires explicit opt-in.
    pub recommended_tools: Vec<String>,
    pub use_recommended_tools: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub include_operations: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exclude_operations: Vec<String>,
    /// Internal escape hatch for integrations that need both same-path methods.
    pub prefer_post: bool,
    pub tags: Vec<String>,
    pub exclude_tags: Vec<String>,
    pub pricing_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_pricing: Option<CreditPricing>,
    pub overrides: Value,
    pub additional_properties: Option<bool>,
    pub instructions_text: Option<String>,
    pub help_url: Option<String>,
    pub max_description_chars: Option<usize>,
}

impl Config {
    /// Compile advice once for an operator inventory; tool contracts stay unchanged.
    pub fn recommendation_matcher(&self) -> Result<impl Fn(&str) -> bool> {
        let patterns = self
            .recommended_tools
            .iter()
            .map(|selector| crate::deployment::pattern(selector))
            .collect::<Result<Vec<_>>>()?;
        Ok(move |name: &str| patterns.iter().any(|pattern| pattern.is_match(name)))
    }

    pub fn transport(&self) -> crate::network::HttpPolicy {
        crate::network::HttpPolicy {
            allow_http1: self.allow_http1,
            allow_tls12: self.allow_tls12,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            discovery_source: None,
            cache_revision: None,
            relay_fallback: None,
            wait_for_startup_socks: false,
            http_cache_enabled: true,
            catalog_cache_ttl_seconds: crate::limits::API_CACHE_TTL_SECONDS,
            http_cache_directory: None,
            http_cache_direct_warm_target: None,
            reliability_tags: vec![],
            reliability_note: String::new(),
            response_mappings: BTreeMap::new(),
            image_limits: Default::default(),
            cover_traffic: None,
            cover_traffic_enabled: None,
            allow_http1: false,
            allow_tls12: false,
            read_timeout_seconds: None,
            max_response_bytes: crate::limits::RESPONSE_BYTES,
            max_help_bytes: crate::limits::HELP_BYTES,
            max_spec_bytes: crate::limits::SPEC_BYTES,
            spec: String::new(),
            relay_format: None,
            name: None,
            description: None,
            base_url: None,
            operation_url_extension: None,
            prefix: None,
            include: vec![],
            exclude: vec![],
            include_tools: vec![],
            exclude_tools: vec![],
            recommended_tools: vec![],
            use_recommended_tools: false,
            include_operations: vec![],
            exclude_operations: vec![],
            prefer_post: true,
            tags: vec![],
            exclude_tags: vec![],
            pricing_key: None,
            credit_pricing: None,
            overrides: json!({}),
            additional_properties: None,
            instructions_text: None,
            help_url: None,
            max_description_chars: None,
        }
    }
}

/// Operator-only metadata; MCP publication continues to use ToolSpec directly.
#[derive(Serialize)]
pub struct ToolInspection<'a> {
    #[serde(flatten)]
    pub tool: &'a ToolSpec,
    pub recommended: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeaderParameter {
    pub name: String,
    pub required: bool,
    pub explode: bool,
}

pub(crate) mod headers;

#[derive(Clone, Debug)]
pub struct DescriptionRendering {
    pub base: String,
    pub max_chars: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolSpec {
    #[serde(skip)]
    pub catalog_hints: Option<std::sync::Arc<crate::pricing::runtime::CatalogHints>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_mapping: Option<crate::output::ResponseMapping>,
    #[serde(
        default,
        skip_serializing_if = "crate::output::ImageLimits::is_default"
    )]
    pub image_limits: crate::output::ImageLimits,
    pub name: String,
    pub description: String,
    #[serde(skip)]
    pub description_rendering: Option<DescriptionRendering>,
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_url: Option<String>,
    pub input_schema: Value,
    pub param_routes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub header_params: BTreeMap<String, HeaderParameter>,
    pub has_body: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub wire_names: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whole_body: Option<BodyArgument>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub body_closed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_media_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RoutedRequest {
    pub method: String,
    pub url: String,
    pub query: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    pub body: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_media_type: Option<String>,
}

impl ToolSpec {
    /// Resolve an existing parameter-description target for compilation and tooling.
    pub fn parameter_note_pointer(&self, key: &str) -> Option<String> {
        let escape = |name: &str| name.replace('~', "~0").replace('/', "~1");
        let properties = self.input_schema.get("properties")?.as_object()?;
        if properties.get(key).is_some_and(Value::is_object) {
            return Some(format!("/properties/{}", escape(key)));
        }
        let body = self.whole_body.as_ref()?;
        let nested = properties.get(&body.name)?.get("properties")?.as_object()?;
        let wire = if nested.contains_key(key) {
            key
        } else {
            key.strip_suffix("_body")
                .filter(|wire| nested.contains_key(*wire))?
        };
        nested.get(wire)?.is_object().then(|| {
            format!(
                "/properties/{}/properties/{}",
                escape(&body.name),
                escape(wire)
            )
        })
    }

    /// Present a current scoped observation without changing canonical catalog text.
    pub fn render_description(&self, observed: Option<crate::output::ObservedPrice>) -> String {
        let Some(observed) = observed else {
            return self.description.clone();
        };
        let (base, max) = self
            .description_rendering
            .as_ref()
            .map_or((self.description.as_str(), None), |rendering| {
                (rendering.base.as_str(), rendering.max_chars)
            });
        let separator = if base.is_empty() { "" } else { " " };
        truncate_description(
            &format!("{base}{separator}Observed cost: ~${observed}/call."),
            max,
        )
    }

    pub fn route(&self, base_url: &str, args: &Map<String, Value>) -> Result<RoutedRequest> {
        let mut path = self.request_url.as_ref().unwrap_or(&self.path).clone();
        let mut query = BTreeMap::new();
        let mut body = Map::new();
        let mut whole_body = None;
        let mut assigned = BTreeSet::new();
        let headers = headers::route(&self.header_params, args)?;
        for (key, value) in args {
            if self
                .whole_body
                .as_ref()
                .is_some_and(|body| body.name == *key)
            {
                whole_body = Some(value.clone());
                continue;
            }
            if (self.whole_body.is_some() || self.body_closed)
                && !self.param_routes.contains_key(key)
                && !self.header_params.contains_key(key)
            {
                bail!("unknown argument outside the declared JSON body and parameters");
            }
            if self.header_params.contains_key(key) {
                continue;
            }
            let wire = self.wire_names.get(key).unwrap_or(key);
            let location = self
                .param_routes
                .get(key)
                .map(String::as_str)
                .unwrap_or(if self.has_body { "body" } else { "query" });
            ensure!(
                matches!(location, "path" | "query" | "body"),
                "unsupported routed parameter location"
            );
            ensure!(
                location != "body" || self.has_body,
                "body route has no request body"
            );
            ensure!(
                assigned.insert((location, wire)),
                "duplicate routed parameter identity"
            );
            match location {
                "path" => {
                    if value.is_null() {
                        continue;
                    }
                    ensure!(
                        value.is_string() || value.is_number() || value.is_boolean(),
                        "path parameter values must be scalars"
                    );
                    let value = arg_text(value);
                    // URL parsing removes dot segments, including percent-encoded
                    // dots. Reject them before they can escape the selected route.
                    ensure!(value != "." && value != "..", "dot path parameter rejected");
                    path = path.replace(
                        &format!("{{{wire}}}"),
                        &utf8_percent_encode(&value, PATH_SEGMENT).to_string(),
                    )
                }
                "query" => {
                    if !value.is_null() {
                        query.insert(wire.clone(), value.clone());
                    }
                }
                _ => {
                    body.insert(wire.clone(), value.clone());
                }
            }
        }
        ensure!(
            self.whole_body
                .as_ref()
                .is_none_or(|body| !body.required || whole_body.is_some()),
            "required JSON body argument is missing"
        );
        ensure!(
            !path.contains('{'),
            "missing path parameter for {}",
            self.name
        );
        // A template literal and an empty argument can also form a dot
        // segment. Inspect the assembled path, with one URL decoding pass.
        ensure!(
            !path
                .split(['?', '#'])
                .next()
                .unwrap_or("")
                .split('/')
                .any(|segment| {
                    let decoded = percent_encoding::percent_decode_str(segment).collect::<Vec<_>>();
                    decoded == b"." || decoded == b".."
                }),
            "dot path segment rejected"
        );
        // httpx appends relative paths to base_url, including a gateway prefix.
        let url = if self.request_url.is_some()
            || path.starts_with("https://")
            || path.starts_with("http://")
        {
            path
        } else {
            format!(
                "{}/{}",
                base_url.trim_end_matches('/'),
                path.trim_start_matches('/')
            )
        };
        reqwest::Url::parse(&url).context("invalid request URL")?;
        Ok(RoutedRequest {
            headers,
            method: self.method.clone(),
            url,
            query,
            body: if self.whole_body.is_some() {
                whole_body
            } else {
                self.has_body.then_some(Value::Object(body))
            },
            json_media_type: self.json_media_type.clone(),
        })
    }
}

pub fn arg_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}

pub fn slug(text: &str) -> String {
    static RE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new("[^0-9a-z]+").expect("constant regex"));
    RE.replace_all(&text.to_lowercase(), "_")
        .trim_matches('_')
        .to_string()
}

pub fn default_prefix(base: &str) -> Result<String> {
    let url = reqwest::Url::parse(base)?;
    let mut labels: Vec<_> = url.host_str().unwrap_or("api").split('.').collect();
    if labels.len() > 1 && matches!(labels[0], "api" | "www") {
        labels.remove(0);
    }
    if labels.len() > 1 {
        labels.pop();
    }
    Ok(slug(&labels.join(".")))
}

fn anchor<'a>(path: &str, prefixes: &'a [String]) -> Option<&'a str> {
    prefixes
        .iter()
        .filter(|p| path == p.as_str() || path.starts_with(&format!("{p}/")))
        .max_by_key(|p| p.len())
        .map(String::as_str)
}

pub async fn load_json(source: &str, http: &reqwest::Client) -> Result<Value> {
    load_json_with_limit(source, http, crate::limits::SPEC_BYTES).await
}
pub async fn load_json_with_limit(
    source: &str,
    http: &reqwest::Client,
    limit: usize,
) -> Result<Value> {
    if !source.starts_with("https://") && !source.starts_with("http://") {
        tracing::debug!(cache = "local_file", "Catalog loading local file");
        let bytes = tokio::fs::read(source)
            .await
            .context(LoadStage::LocalRead)?;
        return parse_document(&bytes);
    }
    crate::network::provider_url(source)?;
    let started = std::time::Instant::now();
    let response = crate::http_cache::discovery_get(http, source)
        .send()
        .await
        .map_err(reqwest::Error::without_url)
        .context(LoadStage::Headers)?
        .error_for_status()
        .map_err(reqwest::Error::without_url)
        .context(LoadStage::Headers)?;
    tracing::debug!(target: "x402_treazury::startup",
        headers_ms = started.elapsed().as_millis() as u64,
        "Catalog response headers received; reading body");
    crate::network::log_http(&response, "catalog");
    let (_, _, bytes) = read_catalog_body(response, limit)
        .await
        .context(LoadStage::Body)?;
    parse_document(&bytes)
}
/// An unsigned load can retain its exact relay continuation without repeating
/// origin I/O. The continuation owns its configuration and shared relay state.
#[derive(Clone, Debug)]
pub struct LoadedCatalog {
    pub document: std::sync::Arc<Value>,
    pub hints: Option<std::sync::Arc<crate::pricing::runtime::CatalogHints>>,
}
impl std::ops::Deref for LoadedCatalog {
    type Target = Value;
    fn deref(&self) -> &Value {
        &self.document
    }
}
impl From<std::sync::Arc<LoadedCatalog>> for LoadedCatalog {
    fn from(value: std::sync::Arc<LoadedCatalog>) -> Self {
        (*value).clone()
    }
}
impl From<std::sync::Arc<Value>> for LoadedCatalog {
    fn from(document: std::sync::Arc<Value>) -> Self {
        Self {
            document,
            hints: None,
        }
    }
}
fn loaded(
    document: Value,
    hints: Option<std::sync::Arc<crate::pricing::runtime::CatalogHints>>,
) -> LoadedCatalog {
    LoadedCatalog {
        document: std::sync::Arc::new(document),
        hints,
    }
}
pub(crate) enum CatalogLoad {
    Ready(LoadedCatalog),
    RelayPending(Box<RelayContinuation>),
}
pub(crate) struct RelayContinuation {
    config: Config,
    origin_error: anyhow::Error,
    retry_after: Option<crate::network::RetryAfter>,
}
impl RelayContinuation {
    pub(crate) async fn resume(&self) -> Result<LoadedCatalog> {
        self.resume_mode(None, None, false).await
    }
    async fn resume_bounded(
        &self,
        gate: std::sync::Arc<tokio::sync::Semaphore>,
    ) -> Result<LoadedCatalog> {
        self.resume_mode(Some(gate), None, false).await
    }
    pub(crate) async fn resume_when_ready(
        &self,
        gate: std::sync::Arc<tokio::sync::Semaphore>,
        operator_status: Option<tokio::sync::watch::Sender<bool>>,
    ) -> Result<LoadedCatalog> {
        self.resume_mode(Some(gate), operator_status, true).await
    }
    #[tracing::instrument(name = "catalog_relay", skip_all, fields(
        source = self.config.discovery_source.as_deref().or(self.config.prefix.as_deref()).unwrap_or("standalone")
    ))]
    async fn resume_mode(
        &self,
        gate: Option<std::sync::Arc<tokio::sync::Semaphore>>,
        operator_status: Option<tokio::sync::watch::Sender<bool>>,
        wait_for_ready: bool,
    ) -> Result<LoadedCatalog> {
        let cfg = &self.config;
        let error = &self.origin_error;
        let http_error = error.downcast_ref::<reqwest::Error>();
        let reason = if http_error.is_some_and(|e| e.is_timeout() && e.is_connect()) {
            "connection_timeout"
        } else if http_error.is_some_and(reqwest::Error::is_timeout) {
            "read_inactivity"
        } else if http_error.is_some_and(reqwest::Error::is_connect) {
            "connection"
        } else if http_error.and_then(reqwest::Error::status).is_some() {
            "http_status"
        } else if http_error.is_some() {
            "transport"
        } else {
            error
                .downcast_ref::<LoadStage>()
                .map(|s| s.label())
                .unwrap_or("fetch")
        };
        crate::discovery_relay::log_fallback(
            cfg,
            "catalog",
            reason,
            http_error
                .and_then(reqwest::Error::status)
                .map(|s| s.as_u16()),
        );
        let relay = cfg.relay_fallback.as_ref().expect("captured relay");
        let response = if wait_for_ready {
            relay
                .fetch_when_ready_bounded(
                    &cfg.spec,
                    cfg.max_spec_bytes,
                    "max_spec_bytes",
                    gate,
                    operator_status,
                    self.retry_after,
                )
                .await
        } else {
            relay
                .fetch_with_capacity(
                    &cfg.spec,
                    cfg.max_spec_bytes,
                    "max_spec_bytes",
                    gate.into_iter().collect(),
                    self.retry_after,
                )
                .await
        }
        .inspect_err(|error| {
            crate::discovery_relay::log_failure(cfg, "catalog", error);
        })?;
        // The relay has already validated complete JSON before caching the response.
        let document = parse_document(&response.body)?;
        let hints = if let Some(slot) = crate::discovery_relay::slot_for(
            cfg,
            &cfg.spec,
            "catalog",
            cfg.max_spec_bytes,
            &response.cache_key,
        ) {
            let metadata = crate::http_cache::Metadata::from_relay_headers(
                &response.headers,
                response.catalog_cache_age(),
                Some(cfg.catalog_cache_ttl_seconds),
            );
            tracing::debug!(
                cache = "relay_response",
                cache_policy =
                    crate::http_cache::Metadata::policy_label(&response.headers, metadata.as_ref()),
                persist = metadata.as_ref().is_some_and(|m| m.fresh()),
                "Catalog relay response cache policy"
            );
            slot.write(
                metadata
                    .filter(|m| m.fresh())
                    .map(|metadata| crate::http_cache::Entry {
                        hints: None,
                        metadata,
                        data: response.body.clone(),
                    }),
            )
            .await
        } else {
            None
        };
        Ok(loaded(document, hints))
    }
}

pub async fn load_json_discovery(cfg: &Config) -> Result<Value> {
    load_json_discovery_with(cfg, crate::network::global()).await
}

pub(crate) async fn load_json_discovery_with(
    cfg: &Config,
    network: &crate::network::NetworkContext,
) -> Result<Value> {
    Ok((*load_json_discovery_bounded(cfg, network, None)
        .await?
        .document)
        .clone())
}

pub async fn load_json_discovery_bounded(
    cfg: &Config,
    network: &crate::network::NetworkContext,
    gate: Option<std::sync::Arc<tokio::sync::Semaphore>>,
) -> Result<LoadedCatalog> {
    match prepare_json_discovery_bounded(cfg, network, gate.clone()).await? {
        CatalogLoad::Ready(document) => Ok(document),
        CatalogLoad::RelayPending(pending) => match gate {
            Some(gate) => pending.resume_bounded(gate).await,
            None => pending.resume().await,
        },
    }
}

#[cfg(test)]
pub(crate) async fn prepare_json_discovery_with(
    cfg: &Config,
    network: &crate::network::NetworkContext,
) -> Result<CatalogLoad> {
    prepare_json_discovery_bounded(cfg, network, None).await
}

#[tracing::instrument(name = "catalog", skip_all, fields(source = cfg.discovery_source.as_deref().unwrap_or("standalone")))]
pub(crate) async fn prepare_json_discovery_bounded(
    cfg: &Config,
    network: &crate::network::NetworkContext,
    gate: Option<std::sync::Arc<tokio::sync::Semaphore>>,
) -> Result<CatalogLoad> {
    if cfg.spec.starts_with("http://") || cfg.spec.starts_with("https://") {
        crate::network::provider_url(&cfg.spec)?;
    }
    tracing::debug!(
        cache = crate::http_cache::Slot::availability(cfg, "catalog"),
        "Catalog disk cache configuration"
    );
    for slot in crate::discovery_relay::slots(cfg, &cfg.spec, "catalog", cfg.max_spec_bytes) {
        slot.check_revision().await;
        let Some(entry) = slot.read().await.filter(|e| e.metadata.fresh()) else {
            continue;
        };
        if let Ok(document) = parse_document(&entry.data) {
            tracing::debug!(
                cache = "relay_disk_hit",
                "Catalog JSON read from fresh relay cache; content supplied by configured relay; tool generation and validation still required"
            );
            return Ok(CatalogLoad::Ready(loaded(document, entry.hints)));
        }
        slot.write(None).await;
    }
    let result = load_json_discovery_origin(
        cfg,
        network,
        crate::http_cache::Slot::new(cfg, &cfg.spec, "catalog", cfg.max_spec_bytes),
        gate.as_deref(),
    )
    .await;
    let failure = match result {
        Ok(doc) => return Ok(CatalogLoad::Ready(doc)),
        Err(failure) => failure,
    };
    if !failure.fallback {
        return Err(failure.error);
    }
    let error = failure.error;
    let retry_after = failure.retry_after;
    let Some(_) = &cfg.relay_fallback else {
        return Err(error);
    };
    if !(cfg.spec.starts_with("https://") || cfg.spec.starts_with("http://")) {
        return Err(error);
    }
    Ok(CatalogLoad::RelayPending(Box::new(RelayContinuation {
        config: cfg.clone(),
        origin_error: error,
        retry_after,
    })))
}

async fn load_json_discovery_origin(
    cfg: &Config,
    network: &crate::network::NetworkContext,
    slot: Option<crate::http_cache::Slot>,
    gate: Option<&tokio::sync::Semaphore>,
) -> std::result::Result<LoadedCatalog, crate::network::acquisition::Failure> {
    let source = &cfg.spec;
    if !source.starts_with("https://") && !source.starts_with("http://") {
        tracing::debug!(cache = "local_file", "Catalog loading local file");
        let bytes = tokio::fs::read(source)
            .await
            .context(LoadStage::LocalRead)
            .map_err(crate::network::acquisition::Failure::local)?;
        return Ok(loaded(
            parse_document(&bytes).map_err(crate::network::acquisition::Failure::validation)?,
            None,
        ));
    }

    if let Some(slot) = &slot {
        slot.check_revision().await;
    }
    let cached = read_cached_catalog(slot.as_ref()).await;
    if cached
        .as_ref()
        .is_some_and(|(entry, _)| entry.metadata.fresh())
    {
        tracing::debug!(
            cache = "disk_hit",
            "Catalog JSON read from fresh HTTP disk cache; tool generation and validation still required"
        );
        return Ok({
            let (entry, document) = cached.expect("fresh cached document");
            loaded(document, entry.hints)
        });
    }

    if cfg.wait_for_startup_socks {
        network.wait_for_startup_socks().await;
    }
    let conditional = cached
        .as_ref()
        .map(|(entry, _)| entry)
        .filter(|e| e.metadata.has_validator());
    let cache = if slot.is_none() {
        "unavailable"
    } else if conditional.is_some() {
        "revalidate"
    } else if cached.is_some() {
        "expired"
    } else {
        "miss"
    };
    tracing::debug!(
        cache,
        conditional = conditional.is_some(),
        "Catalog fetching origin"
    );

    match fetch_catalog_origin(cfg, network, conditional, slot.as_ref(), gate).await {
        Ok(document) => Ok(document),
        Err(err) => {
            if let Some(slot) = &slot {
                slot.write(None).await;
            }
            Err(err)
        }
    }
}

async fn fetch_catalog_origin(
    cfg: &Config,
    network: &crate::network::NetworkContext,
    conditional: Option<&crate::http_cache::Entry>,
    slot: Option<&crate::http_cache::Slot>,
    gate: Option<&tokio::sync::Semaphore>,
) -> std::result::Result<LoadedCatalog, crate::network::acquisition::Failure> {
    use crate::network::acquisition::{self, Failure, FallbackPolicy};
    let policy = FallbackPolicy::RemoteFailures;
    let source = &cfg.spec;
    let id = crate::network::IsolationId::discovery(source).map_err(Failure::local)?;
    let timeout = cfg
        .read_timeout_seconds
        .map(std::time::Duration::from_secs_f64);
    let selected = network
        .http_participating(&id, source, timeout, false, cfg.transport())
        .map_err(Failure::local)?;
    let acquired = acquisition::run(
        network,
        selected.clone(),
        cfg.http_cache_direct_warm_target.is_some(),
        |current, _, _| async move {
            let _permit = match gate {
                Some(gate) => Some(
                    crate::limits::acquire_capacity(
                        gate,
                        "catalog_downloads_busy",
                        None,
                        "startup.catalog_concurrency",
                    )
                    .await
                    .expect("owned catalog gate"),
                ),
                None => None,
            };
            let started = std::time::Instant::now();
            let response = catalog_request(&current.client, source, conditional)
                .send()
                .await
                .map_err(reqwest::Error::without_url)
                .map_err(|error| {
                    let mut failure = Failure::transport(error);
                    failure.error = failure.error.context(LoadStage::Headers);
                    failure
                })?;
            tracing::debug!(target: "x402_treazury::startup",
                headers_ms = started.elapsed().as_millis() as u64,
                "Catalog response headers received; reading body");
            crate::network::log_http(&response, "catalog");
            let status = response.status();
            if status == reqwest::StatusCode::NOT_MODIFIED {
                if response
                    .headers()
                    .contains_key(reqwest::header::CONTENT_RANGE)
                {
                    return Err(Failure::validation(
                        anyhow::Error::new(IncompleteRelayCatalog).context(LoadStage::Body),
                    ));
                }
                let hints = revalidate_cached_catalog(response.headers(), conditional, slot)
                    .await
                    .map_err(Failure::validation)?;
                return Ok(loaded(
                    serde_json::from_slice(&conditional.expect("validated cached entry").data)
                        .map_err(|error| Failure::validation(error.into()))?,
                    hints,
                ));
            }
            // An unfollowed redirect may still carry a valid legacy catalog body.
            if status.is_client_error() || status.is_server_error() {
                let headers = response.headers().clone();
                let error = response
                    .error_for_status()
                    .expect_err("non-success status")
                    .without_url();
                return Err(Failure::status(
                    anyhow::Error::new(error).context(LoadStage::Headers),
                    status,
                    &headers,
                    policy,
                ));
            }
            let (status, headers, bytes) = read_catalog_body(response, cfg.max_spec_bytes)
                .await
                .map_err(|error| {
                    let mut failure = if error.is::<IncompleteRelayCatalog>() {
                        Failure::validation(error)
                    } else {
                        Failure::body(error)
                    };
                    failure.error = failure.error.context(LoadStage::Body);
                    failure
                })?;
            let document =
                parse_document(&bytes).map_err(|error| Failure::remote_content(error, policy))?;
            let hints = persist_catalog_response(status, &headers, bytes, slot).await;
            Ok(loaded(document, hints))
        },
    )
    .await?;
    network.promote_http(&selected, &acquired.current);
    Ok(acquired.value)
}

fn catalog_request(
    http: &reqwest::Client,
    url: &str,
    conditional: Option<&crate::http_cache::Entry>,
) -> reqwest::RequestBuilder {
    let request = crate::http_cache::discovery_get(http, url);
    match conditional {
        Some(entry) => entry.metadata.conditional(request),
        None => request,
    }
}

#[derive(Debug)]
struct IncompleteRelayCatalog;
impl std::fmt::Display for IncompleteRelayCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("catalog partial_response")
    }
}
impl std::error::Error for IncompleteRelayCatalog {}

async fn read_catalog_body(
    response: reqwest::Response,
    limit: usize,
) -> Result<(reqwest::StatusCode, reqwest::header::HeaderMap, Vec<u8>)> {
    let status = response.status();
    let headers = response.headers().clone();
    if status == reqwest::StatusCode::PARTIAL_CONTENT
        || headers.contains_key(reqwest::header::CONTENT_RANGE)
    {
        return Err(IncompleteRelayCatalog.into());
    }
    let body_started = std::time::Instant::now();
    let bytes = crate::limits::read(response, limit, "static URL spec", "max_spec_bytes").await?;
    tracing::debug!(target: "x402_treazury::startup",
        body_ms = body_started.elapsed().as_millis() as u64, bytes = bytes.len(),
        "Catalog response body complete");
    Ok((status, headers, bytes))
}

async fn read_cached_catalog(
    slot: Option<&crate::http_cache::Slot>,
) -> Option<(crate::http_cache::Entry, Value)> {
    let slot = slot?;
    let entry = slot.read().await?;
    match parse_document(&entry.data) {
        Ok(document) => Some((entry, document)),
        Err(_) => {
            tracing::debug!("Invalid HTTP cached catalog; fetching origin");
            slot.write(None).await;
            None
        }
    }
}

async fn revalidate_cached_catalog(
    headers: &reqwest::header::HeaderMap,
    conditional: Option<&crate::http_cache::Entry>,
    slot: Option<&crate::http_cache::Slot>,
) -> Result<Option<std::sync::Arc<crate::pricing::runtime::CatalogHints>>> {
    let Some(entry) = conditional else {
        return Err(anyhow::anyhow!("unsolicited catalog HTTP 304"))
            .context(diagnostics::Problem(
                "The catalog server returned HTTP 304 but there is no cached document to validate",
            ))
            .context(LoadStage::Headers);
    };
    if !entry.metadata.matches_validation(headers) {
        return Err(anyhow::anyhow!(
            "catalog HTTP 304 did not validate cached representation"
        ))
        .context(diagnostics::Problem(
            "The catalog server's HTTP 304 response does not match the cached document",
        ))
        .context(LoadStage::Headers);
    }
    // A 304 may refresh only a representation that still meets the catalog contract.
    parse_document(&entry.data)?;
    let metadata = crate::http_cache::Metadata::from_catalog_headers(
        headers,
        std::time::Duration::ZERO,
        Some(&entry.metadata),
        slot.map_or(0, |s| s.catalog_ttl),
    );
    tracing::debug!(
        cache_policy = crate::http_cache::Metadata::policy_label(headers, metadata.as_ref()),
        "Catalog revalidation response cache policy"
    );
    let hints = if let Some(slot) = slot {
        slot.write(metadata.map(|metadata| crate::http_cache::Entry {
            hints: None,
            metadata,
            data: entry.data.clone(),
        }))
        .await
    } else {
        None
    };
    tracing::debug!(
        cache = "revalidated",
        http_status = 304,
        "Catalog HTTP disk cache revalidated by origin"
    );
    Ok(hints)
}

async fn persist_catalog_response(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    bytes: Vec<u8>,
    slot: Option<&crate::http_cache::Slot>,
) -> Option<std::sync::Arc<crate::pricing::runtime::CatalogHints>> {
    let metadata = if status == reqwest::StatusCode::OK {
        crate::http_cache::Metadata::from_catalog_headers(
            headers,
            std::time::Duration::ZERO,
            None,
            slot.map_or(0, |s| s.catalog_ttl),
        )
    } else {
        None
    };
    tracing::debug!(
        cache = if slot.is_some() {
            "enabled"
        } else {
            "unavailable"
        },
        cache_policy = crate::http_cache::Metadata::policy_label(headers, metadata.as_ref()),
        http_status = status.as_u16(),
        "Catalog origin response cache policy"
    );
    if let Some(slot) = slot {
        slot.write(metadata.map(|metadata| crate::http_cache::Entry {
            hints: None,
            metadata,
            data: bytes,
        }))
        .await
    } else {
        None
    }
}
pub(crate) fn parse_document(bytes: &[u8]) -> Result<Value> {
    let started = std::time::Instant::now();
    let document: Value = serde_json::from_slice(bytes).context(LoadStage::Parse)?;
    // Validate actual conversion before cache publication, preserving the original
    // document and accepting generic JSON used by the shared document loader.
    if document.get("paths").is_some() || document.get("operations").is_some() {
        operations_with_credits(&document, None, None)?;
    } else {
        expansion::shape(&document)?;
    }
    tracing::debug!(target: "x402_treazury::startup", parse_ms = started.elapsed().as_millis() as u64,
        bytes = bytes.len(), "Catalog JSON parse complete");
    Ok(document)
}

pub fn sanitize(node: &Value) -> Value {
    sanitize::sanitize(node)
}

pub fn operations(root: &Value, pricing_key: Option<&str>) -> Result<Vec<Value>> {
    operations_with_credits(root, pricing_key, None)
}

pub(crate) fn operations_with_credits(
    root: &Value,
    pricing_key: Option<&str>,
    credit_key: Option<&str>,
) -> Result<Vec<Value>> {
    expansion::shape(root)?;
    let mut budget = expansion::Budget::default();
    if root.get("paths").is_none() {
        return digest_operations(root, credit_key, &mut budget);
    }
    let mut ops = Vec::new();
    for (path, item) in root["paths"]
        .as_object()
        .context("paths must be an object")?
    {
        for (method, op) in item.as_object().context("invalid path item")? {
            if !["get", "post", "put", "patch", "delete"].contains(&method.as_str()) {
                continue;
            }
            ops.push(openapi_operation(
                root,
                path,
                method,
                op,
                pricing_key,
                credit_key,
                &mut budget,
            )?);
        }
    }
    Ok(ops)
}

// Copy opaque digest metadata once, excluding schemas that are projected below.
fn digest_fields(
    source: &Value,
    excluded: &[&str],
    budget: &mut expansion::Budget,
) -> Result<Map<String, Value>> {
    budget.schema_node(source, 0)?;
    let mut output = Map::new();
    for (key, value) in source.as_object().context("digest object required")? {
        if excluded.contains(&key.as_str()) {
            continue;
        }
        budget.copying(value)?;
        output.insert(key.clone(), value.clone());
    }
    Ok(output)
}
fn digest_parameter(root: &Value, source: &Value, budget: &mut expansion::Budget) -> Result<Value> {
    let source = match contracts::reference(root, source, budget) {
        Ok(source) => source,
        Err(error) => {
            let Some(reason) = error.downcast_ref::<Unsupported>() else {
                return Err(error);
            };
            return parameter_issue(source, *reason, budget);
        }
    };
    if !source.is_object() {
        return Ok(json!({"issue":Unsupported::InputSchema}));
    }
    let mut parameter = digest_fields(source, &["schema", "issue"], budget)?;
    parameter.entry("in").or_insert(json!("query"));
    if parameter.get("in") == Some(&Value::Null) {
        parameter.insert("in".into(), json!("query"));
    }
    if parameter["in"] == "cookie" {
        parameter.insert("issue".into(), json!(Unsupported::CookieParameter));
    } else if let Some(source) = source.get("schema") {
        match contracts::schema(root, source, budget) {
            Ok(schema) => {
                parameter.insert("schema".into(), schema);
            }
            Err(error) => {
                let Some(reason) = error.downcast_ref::<Unsupported>() else {
                    return Err(error);
                };
                parameter.insert("issue".into(), json!(reason));
            }
        }
    }
    Ok(Value::Object(parameter))
}
fn digest_body(root: &Value, source: &Value, budget: &mut expansion::Budget) -> Result<Value> {
    if !source.is_object() {
        return Ok(json!({"issue":Unsupported::InputSchema}));
    }
    let mut body = digest_fields(source, &["schema", "issue"], budget)?;
    let media = source
        .get("media_type")
        .and_then(Value::as_str)
        .unwrap_or("application/json");
    if media::json_type(media).is_err() {
        body.insert("issue".into(), json!(Unsupported::RequestMedia));
    } else {
        match contracts::schema(root, source.get("schema").unwrap_or(&json!({})), budget) {
            Ok(schema) => {
                body.insert("schema".into(), schema);
                body.insert("media_type".into(), json!(media));
            }
            Err(error) => {
                let Some(reason) = error.downcast_ref::<Unsupported>() else {
                    return Err(error);
                };
                body.insert("issue".into(), json!(reason));
            }
        }
    }
    Ok(Value::Object(body))
}
fn digest_operations(
    root: &Value,
    credit_key: Option<&str>,
    budget: &mut expansion::Budget,
) -> Result<Vec<Value>> {
    let source = root
        .get("operations")
        .and_then(Value::as_array)
        .context("expected OpenAPI paths or operations digest")?;
    let mut operations = Vec::new();
    for source in source {
        if !source.is_object() {
            budget.copying(source)?;
            operations.push(source.clone());
            continue;
        };
        let mut operation = digest_fields(source, &["params", "body"], budget)?;
        if let Some(params) = source.get("params") {
            let value = if let Some(params) = params.as_array() {
                Value::Array(
                    params
                        .iter()
                        .map(|p| digest_parameter(root, p, budget))
                        .collect::<Result<_>>()?,
                )
            } else {
                budget.copying(params)?;
                params.clone()
            };
            operation.insert("params".into(), value);
        }
        if let Some(body) = source.get("body") {
            operation.insert(
                "body".into(),
                if body.is_null() {
                    Value::Null
                } else {
                    digest_body(root, body, budget)?
                },
            );
        }
        if let Some(value) = credit_key.and_then(|key| source.get(key)) {
            budget.copying(value)?;
            operation.insert("credit_pricing".into(), value.clone());
        }
        operations.push(Value::Object(operation));
    }
    Ok(operations)
}

fn openapi_operation(
    root: &Value,
    path: &str,
    method: &str,
    op: &Value,
    pricing_key: Option<&str>,
    credit_key: Option<&str>,
    budget: &mut expansion::Budget,
) -> Result<Value> {
    // Operation parameters override inherited parameters by (location, name),
    // not by name alone. Resolve references before comparing their identities.
    let mut params: Vec<Value> = Vec::new();
    for parameter_source in root["paths"][path]["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(op["parameters"].as_array().into_iter().flatten())
    {
        let Some(parameter) = openapi_parameter(root, parameter_source, budget)? else {
            continue;
        };
        if let Some(previous) = params.iter_mut().find(|p| {
            p["in"] == parameter["in"]
                && if p["in"] == "header" {
                    p["name"]
                        .as_str()
                        .unwrap_or("")
                        .eq_ignore_ascii_case(parameter["name"].as_str().unwrap_or(""))
                } else {
                    p["name"] == parameter["name"]
                }
        }) {
            *previous = parameter;
        } else {
            params.push(parameter);
        }
    }
    let body = op
        .get("requestBody")
        .filter(|value| !value.is_null())
        .map(|source| media::body(root, source, budget))
        .transpose()?;
    budget.generated_node(
        &[
            "path",
            "method",
            "summary",
            "description",
            "tags",
            "pricing",
            "credit_pricing",
            "params",
            "body",
        ],
        0,
    )?;
    budget.copying_text(path, 1)?;
    budget.copying_text(method, 1)?;
    let tags = op.get("tags").unwrap_or(&Value::Null);
    for value in [
        &op["summary"],
        &op["description"],
        tags,
        pricing_key
            .and_then(|key| op.get(key))
            .unwrap_or(&Value::Null),
        credit_key
            .and_then(|key| op.get(key))
            .unwrap_or(&Value::Null),
    ] {
        budget.copying_at(value, 1)?;
    }
    // Parameters and body have already been charged while projected; move them.
    budget.generated_node(&[], 1)?; // params array
    if body.is_none() {
        budget.copying_at(&Value::Null, 1)?;
    }
    let mut operation = json!({"path":path,"method":method,"summary":op["summary"],"description":op["description"],
        "tags":op.get("tags").unwrap_or(&json!([])),"pricing":pricing_key.and_then(|key|op.get(key)),"credit_pricing":credit_key.and_then(|key|op.get(key))});
    operation["params"] = Value::Array(params);
    operation["body"] = body.unwrap_or(Value::Null);
    Ok(operation)
}

fn parameter_issue(
    source: &Value,
    reason: Unsupported,
    budget: &mut expansion::Budget,
) -> Result<Value> {
    budget.generated_node(&["name", "in", "issue"], 0)?;
    budget.copying_at(&source["name"], 1)?;
    budget.copying_at(&source["in"], 1)?;
    let issue = serde_json::to_value(reason)?;
    budget.copying_at(&issue, 1)?;
    Ok(json!({"name":source["name"],"in":source["in"],"issue":issue}))
}

fn openapi_parameter(
    root: &Value,
    parameter_source: &Value,
    budget: &mut expansion::Budget,
) -> Result<Option<Value>> {
    let raw = match contracts::reference(root, parameter_source, budget) {
        Ok(raw) => raw,
        Err(error) => {
            let Some(reason) = error.downcast_ref::<Unsupported>() else {
                return Err(error);
            };
            return parameter_issue(parameter_source, *reason, budget).map(Some);
        }
    };
    if raw["in"] == "cookie" {
        return parameter_issue(raw, Unsupported::CookieParameter, budget).map(Some);
    }
    let p = raw;
    if !p["name"].is_string() {
        return Ok(None);
    }
    let ignored = p["in"] == "header"
        && p["name"].as_str().is_some_and(|name| {
            headers::ignored(&name.to_ascii_lowercase())
                || headers::payment(&name.to_ascii_lowercase())
        });
    let s = match if ignored {
        let schema = p.get("schema").unwrap_or(&Value::Null);
        budget.copying(schema).map(|_| schema.clone())
    } else {
        contracts::schema(
            root,
            p.get("schema").unwrap_or(&json!({"type":"string"})),
            budget,
        )
    } {
        Ok(schema) => schema,
        Err(error) => {
            let Some(reason) = error.downcast_ref::<Unsupported>() else {
                return Err(error);
            };
            return parameter_issue(p, *reason, budget).map(Some);
        }
    };
    let description = p
        .get("description")
        .filter(|v| !v.is_null())
        .unwrap_or(&s["description"]);
    let serialization = ["style", "explode", "content", "allowReserved"];
    let mut keys = vec!["name", "in", "required", "description", "schema"];
    keys.extend(
        serialization
            .iter()
            .copied()
            .filter(|key| p.get(*key).is_some()),
    );
    budget.generated_node(&keys, 0)?;
    budget.copying_at(description, 1)?;
    budget.copying_at(&Value::Bool(p["required"].as_bool().unwrap_or(false)), 1)?;
    budget.copying_at(&p["name"], 1)?;
    budget.copying_at(&p["in"], 1)?;
    // Copy only emitted parameter metadata; schema annotations move below.
    for key in serialization {
        if let Some(value) = p.get(key) {
            budget.copying_at(value, 1)?;
        }
    }
    let mut parameter = json!({"name": p["name"], "in": p["in"],
    "required": p["required"].as_bool().unwrap_or(false),
    "description": description,
    });
    parameter["schema"] = s;
    // Serialization belongs to the original parameter contract.
    {
        for key in serialization {
            if let Some(value) = p.get(key) {
                parameter[key] = value.clone();
            }
        }
    }
    if p["in"] == "header"
        && p["name"]
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case("idempotency-key"))
    {
        // This managed header never becomes a caller argument. Keep its authored
        // constraints instead of the lossy tool-schema projection so automatic
        // generation and digest input use the same authoritative schema field.
        let original = headers::original_key_schema(root, parameter_source, budget)?;
        budget.copying(original)?;
        parameter["schema"] = original.clone();
    }
    Ok(Some(parameter))
}

/// Listener tags match the exact path and case-insensitive HTTP method. Tag values
/// are case-sensitive; exclusions win, and positive filters reject untagged tools.
/// Keep listener name selectors and wallet binding at the call site.
pub(crate) fn matches_operation_tags(
    operations: &[Value],
    tool: &ToolSpec,
    include: &[String],
    exclude: &[String],
) -> bool {
    if include.is_empty() && exclude.is_empty() {
        return true;
    }
    let tags: BTreeSet<_> = operations
        .iter()
        .filter(|op| {
            op["path"] == tool.path
                && op["method"]
                    .as_str()
                    .is_some_and(|method| method.eq_ignore_ascii_case(&tool.method))
        })
        .flat_map(|op| {
            op["tags"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
        })
        .collect();
    (include.is_empty() || include.iter().any(|tag| tags.contains(tag.as_str())))
        && !exclude.iter().any(|tag| tags.contains(tag.as_str()))
}

/// Counts from the source before selection, including tags on excluded operations.
pub fn tag_counts(root: &Value) -> Result<BTreeMap<String, usize>> {
    let mut counts = BTreeMap::new();
    for op in operations(root, None)? {
        let tags: BTreeSet<_> = op["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if tags.is_empty() {
            *counts.entry("(untagged)".into()).or_default() += 1;
        }
        for tag in tags {
            *counts.entry(tag.to_owned()).or_default() += 1;
        }
    }
    Ok(counts)
}

fn money(value: &Value) -> Option<String> {
    if !(value.is_string() || value.is_u64() || value.is_i64()) {
        return None;
    }
    let value = arg_text(value).trim().to_owned();
    static RE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"^\d+(?:\.\d+)?$").expect("constant regex"));
    if !RE.is_match(&value) {
        return None;
    }
    Some(if let Some((a, b)) = value.split_once('.') {
        let b = b.trim_end_matches('0');
        format!("{a}.{b:0<2}")
    } else {
        value
    })
}

fn price(op: &Value) -> String {
    let p = &op["pricing"];
    if p.is_null() || p == &json!({}) {
        return DEFAULT_PRICE.into();
    }
    if p["authMode"] == "free" && p.get("price").is_none() {
        return "Cost: free [spec].".into();
    }
    let block = p.get("price").filter(|v| v.is_object()).unwrap_or(p);
    let block = block
        .get("fixed")
        .filter(|v| v.is_object())
        .unwrap_or(block);
    let unit = block["unit"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("call");
    let show = |amount: String| match block["currency"].as_str() {
        Some(c) if c.eq_ignore_ascii_case("USD") || c.eq_ignore_ascii_case("USDC") => {
            format!("${amount}")
        }
        Some(c) => format!("{amount} {c}"),
        None => amount,
    };
    if let Some(amount) = money(&block["amount"]) {
        let dynamic = block["mode"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("dynamic"));
        format!(
            "Cost: ~{}/{unit} [spec{}].",
            show(amount),
            if dynamic { ", dynamic" } else { "" }
        )
    } else if let (Some(lo), Some(hi)) = (money(&block["min"]), money(&block["max"])) {
        format!("Cost: ~{}–{}/{unit} [spec].", show(lo), show(hi))
    } else {
        format!("Pricing: {p} [spec].")
    }
}

pub fn build_tools(cfg: &Config, root: &Value, prefix: &str) -> Result<Vec<ToolSpec>> {
    build_catalog(cfg, root, prefix)?.complete()
}

pub fn build_catalog(cfg: &Config, root: &Value, prefix: &str) -> Result<CatalogBuild> {
    if let Some(credits) = &cfg.credit_pricing {
        credits.validate().map_err(|error| ToolConfigurationError {
            error,
            reason: "Provider credit-pricing settings are invalid",
        })?;
    }
    let OperationSelection {
        operations: selected,
        issues,
    } = selected_operations(cfg, root, prefix)?;
    // A help document cannot disguise the absence of every selected API contract.
    // Active tool-name selectors still require an actual compatible inventory.
    if selected.is_empty() && !issues.is_empty() {
        if !cfg.include_tools.is_empty()
            || !cfg.exclude_tools.is_empty()
            || cfg.use_recommended_tools
        {
            filter_tool_names(cfg, &mut Vec::new(), true).map_err(|error| ToolConfigurationError {
                error: error.context(IncompleteCatalog(issues.clone())),
                reason: "Provider tool-name selector matches no compatible tools; inspect omitted operations and operation selections",
            })?;
        }
        return Ok(CatalogBuild {
            tools: Vec::new(),
            issues,
        });
    }
    let mut credit_diagnostics = credit_pricing::Diagnostics::default();
    let mut tools = selected
        .into_iter()
        .map(|(op, name, input)| {
            operation_tool(cfg, root, &op, name, input, &mut credit_diagnostics)
        })
        .collect::<Result<Vec<_>>>()?;
    if let Some(url) = &cfg.help_url {
        tools.push(ToolSpec { catalog_hints: None, response_mapping: None, image_limits: cfg.image_limits.clone(), name: format!("{prefix}_help"), description_rendering: None, description: format!(
            "Extended documentation for all {prefix}_* tools: API-wide usage guidance, pricing notes, and workflows published by the vendor (llms.txt). Takes no arguments and returns the full document. Call this before other {prefix}_* tools when unsure how to use them."),
            method:"GET".into(), path:url.clone(), request_url:None, input_schema:json!({"type":"object","properties":{}}),
            param_routes:BTreeMap::new(), header_params:BTreeMap::new(), has_body:false, help_url:Some(url.clone()), wire_names:BTreeMap::new(),whole_body:None,body_closed:false,json_media_type:None });
    }
    // Authored overrides also apply to help and always follow generated text/schema.
    for tool in &mut tools {
        apply_overrides(cfg, tool).map_err(|error| ToolConfigurationError {
            error,
            reason: "Provider pricing-description overrides are invalid",
        })?;
    }
    let names: BTreeSet<_> = tools.iter().map(|t| &t.name).collect();
    if names.len() != tools.len() {
        bail!("duplicate generated tool names");
    }
    filter_tool_names(cfg, &mut tools, !issues.is_empty()).map_err(|error| ToolConfigurationError {
        error: if issues.is_empty() {error} else {error.context(IncompleteCatalog(issues.clone()))},
        reason: if issues.is_empty() {"Provider tool-name filters are invalid or match no tools; check include_tools, exclude_tools and recommendation settings"} else {"Provider tool-name selector matches no compatible tools; inspect omitted operations and operation selections"},
    })?;
    if let Some(credits) = &cfg.credit_pricing {
        credit_diagnostics.report(prefix, credits, &tools);
    }
    Ok(CatalogBuild { tools, issues })
}

fn filter_tool_names(cfg: &Config, tools: &mut Vec<ToolSpec>, partial: bool) -> Result<()> {
    // Every selector sees the same compatible named inventory, including help.
    let compile = |selectors: &[String], advisory: bool| -> Result<Vec<regex::Regex>> {
        selectors.iter().map(|selector| {
            let pattern = crate::deployment::pattern(selector)?;
            if !tools.iter().any(|tool| pattern.is_match(&tool.name)) {
                if advisory || selector.contains(['*', '?']) {
                    tracing::warn!(selector, advisory, "Provider tool selector matches nothing");
                } else {
                    if partial {bail!("provider tool selector {selector} matches no compatible tools; inspect omitted operations and operation selections");}
                    bail!("unknown provider tool selector {selector}; check path, operation and tag filters");
                }
            }
            Ok(pattern)
        }).collect()
    };
    let include = compile(&cfg.include_tools, false)?;
    let exclude = compile(&cfg.exclude_tools, false)?;
    let recommended = compile(&cfg.recommended_tools, !cfg.use_recommended_tools)?;
    if cfg.use_recommended_tools {
        ensure!(
            tools
                .iter()
                .any(|tool| recommended.iter().any(|p| p.is_match(&tool.name))),
            "no tools matched recommended_tools; supply recommendations or disable use_recommended_tools"
        );
    }
    tools.retain(|tool| {
        (include.is_empty() || include.iter().any(|p| p.is_match(&tool.name)))
            && !exclude.iter().any(|p| p.is_match(&tool.name))
            && (!cfg.use_recommended_tools || recommended.iter().any(|p| p.is_match(&tool.name)))
    });
    ensure!(
        !tools.is_empty(),
        "no tools matched provider tool-name filters"
    );
    Ok(())
}

/// Names after operation selection and collision naming, before tool-name filters.
/// Uses the same compatible inventory; no pricing/tool-description generation.
pub(crate) fn tool_names_before_filtering(
    cfg: &Config,
    root: &Value,
    prefix: &str,
) -> Result<Vec<String>> {
    let OperationSelection {
        operations: selected,
        issues,
    } = selected_operations(cfg, root, prefix)?;
    if !issues.is_empty() {
        return Err(IncompleteCatalog(issues).into());
    }
    let mut names: Vec<_> = selected.into_iter().map(|(_, name, _)| name).collect();
    if cfg.help_url.is_some() {
        names.push(format!("{prefix}_help"));
    }
    Ok(names)
}

// Selection and collision naming depend on the entire sorted inventory.
// Compatible named operations and their omissions belong to the same selection.
struct OperationSelection {
    operations: Vec<(Value, String, input::OperationInput)>,
    issues: Vec<OperationIssue>,
}

fn selected_operations(cfg: &Config, root: &Value, prefix: &str) -> Result<OperationSelection> {
    let mut ops = operations_with_credits(
        root,
        cfg.pricing_key.as_deref(),
        cfg.credit_pricing
            .as_ref()
            .map(|c| c.credit_cost_key.as_str()),
    )?;
    for op in &ops {
        ensure!(
            op["path"].is_string() && op["method"].is_string(),
            "invalid operation"
        );
    }
    ops.sort_by_key(|o| {
        (
            o["path"].as_str().unwrap().to_owned(),
            o["method"].as_str().unwrap().to_owned(),
        )
    });
    let mut selected = Vec::new();
    let had_operations = !ops.is_empty();
    for op in ops {
        if let Some(base) = selected_operation_base(cfg, &op, prefix) {
            selected.push((op, base));
        }
    }
    if selected.is_empty() {
        let error = anyhow::anyhow!("no operations matched");
        return Err(if had_operations {
            ToolConfigurationError {
                error,
                reason: "Provider path, method or tag filters select no API operations",
            }
            .into()
        } else {
            error
        });
    }
    let mut issues = Vec::new();
    let mut compatible = Vec::new();
    for (op, base) in selected {
        match input::prepare(&op) {
            Ok(input) => compatible.push((op, base, input)),
            Err(error) => {
                let Some(reason) = error.downcast_ref::<Unsupported>() else {
                    return Err(error);
                };
                issues.push(OperationIssue {
                    method: op["method"].as_str().unwrap().to_ascii_uppercase(),
                    path: op["path"].as_str().unwrap().into(),
                    reason: *reason,
                });
            }
        }
    }
    let mut selected = compatible;
    // Exact wire paths define twins; later name filters cannot resurrect GET.
    if cfg.prefer_post {
        let post_paths: BTreeSet<_> = selected
            .iter()
            .filter(|(op, _, _)| op["method"].as_str().unwrap().eq_ignore_ascii_case("POST"))
            .map(|(op, _, _)| op["path"].as_str().unwrap().to_owned())
            .collect();
        selected.retain(|(op, _, _)| {
            !op["method"].as_str().unwrap().eq_ignore_ascii_case("GET")
                || !post_paths.contains(op["path"].as_str().unwrap())
        });
        // Intentionally suppressed GET twins are outside the selected inventory,
        // including its omissions; unsupported POSTs never enter post_paths.
        issues.retain(|issue| {
            !issue.method.eq_ignore_ascii_case("GET") || !post_paths.contains(&issue.path)
        });
    }
    let mut counts = BTreeMap::<String, usize>::new();
    for (_, base, _) in &selected {
        *counts.entry(base.clone()).or_default() += 1;
    }
    Ok(OperationSelection {
        operations: name_selected_operations(selected, &counts),
        issues,
    })
}

// Filters precede collision counting and retain the most specific include anchor.
fn selected_operation_base(cfg: &Config, op: &Value, prefix: &str) -> Option<String> {
    let path = op["path"].as_str().unwrap();
    let operation = format!(
        "{} {path}",
        op["method"].as_str().unwrap().to_ascii_uppercase()
    );
    if cfg.exclude_operations.contains(&operation)
        || (!cfg.include_operations.is_empty() && !cfg.include_operations.contains(&operation))
    {
        return None;
    }
    if anchor(path, &cfg.exclude).is_some() {
        return None;
    }
    let anchored = anchor(path, &cfg.include);
    if !cfg.include.is_empty() && anchored.is_none() {
        return None;
    }
    let tags: BTreeSet<_> = op["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if cfg.exclude_tags.iter().any(|t| tags.contains(t.as_str())) {
        return None;
    }
    if !cfg.tags.is_empty() && !cfg.tags.iter().any(|t| tags.contains(t.as_str())) {
        return None;
    }
    let suffix = slug(path.strip_prefix(anchored.unwrap_or("")).unwrap_or(path));
    let base = format!(
        "{prefix}_{}",
        if suffix.is_empty() { "root" } else { &suffix }
    );
    Some(base)
}

fn name_selected_operations(
    selected: Vec<(Value, String, input::OperationInput)>,
    counts: &BTreeMap<String, usize>,
) -> Vec<(Value, String, input::OperationInput)> {
    let mut seen = BTreeMap::<String, usize>::new();
    let mut named = Vec::new();
    for (op, base, input) in selected {
        let method = op["method"].as_str().unwrap();
        let name = if counts[&base] > 1 {
            format!("{base}_{}", method.to_lowercase())
        } else {
            base
        };
        let count = seen.entry(name.clone()).or_default();
        *count += 1;
        let name = if *count > 1 {
            format!("{name}_{count}")
        } else {
            name
        };
        named.push((op, name, input));
    }
    named
}

fn operation_description(
    cfg: &Config,
    op: &Value,
    credit_estimate: Option<String>,
) -> (String, DescriptionRendering) {
    let method = op["method"].as_str().unwrap();
    let text = op["description"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| op["summary"].as_str().filter(|s| !s.is_empty()))
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{} {}", method.to_uppercase(), op["path"].as_str().unwrap()));
    let mut price = price(op);
    if let Some(estimate) = credit_estimate {
        price = if price == DEFAULT_PRICE {
            estimate
        } else {
            format!("{price} {estimate}")
        };
    }
    // Dollar amounts elsewhere in vendor prose do not establish price provenance.
    // Only omit a suffix that is already present verbatim.
    let base = text
        .strip_suffix(&price)
        .map_or_else(|| text.clone(), |base| base.trim_end().to_owned());
    let description = if text.ends_with(&price) {
        text
    } else {
        format!("{text} {price}")
    };
    if let Some(max) = cfg.max_description_chars
        && description.chars().count() > max
    {
        tracing::warn!(
            limit_chars = max,
            "tool description truncated by max_description_chars"
        );
    }
    (
        truncate_description(&description, cfg.max_description_chars),
        DescriptionRendering {
            base,
            max_chars: cfg.max_description_chars,
        },
    )
}

fn truncate_description(description: &str, max: Option<usize>) -> String {
    match max {
        Some(max) if description.chars().count() > max => format!(
            "{}\n[truncated by max_description_chars={max}]",
            description.chars().take(max).collect::<String>()
        ),
        _ => description.to_owned(),
    }
}

fn operation_tool(
    cfg: &Config,
    root: &Value,
    op: &Value,
    name: String,
    input: input::OperationInput,
    credit_diagnostics: &mut credit_pricing::Diagnostics,
) -> Result<ToolSpec> {
    let method = op["method"].as_str().unwrap();
    let credit_estimate = cfg.credit_pricing.as_ref().map(|credits| {
        let (description, reason) = credits.describe(&op["credit_pricing"]);
        credit_diagnostics.record(&name, reason);
        description
    });
    let (description, description_rendering) = operation_description(cfg, op, credit_estimate);
    Ok(ToolSpec {
        catalog_hints: None,
        response_mapping: cfg
            .response_mappings
            .get(&format!(
                "{} {}",
                method.to_uppercase(),
                op["path"].as_str().unwrap()
            ))
            .cloned(),
        image_limits: cfg.image_limits.clone(),
        name,
        description,
        description_rendering: Some(description_rendering),
        method: method.to_uppercase(),
        path: op["path"].as_str().unwrap().into(),
        request_url: operation_request_url(cfg, root, op)?,
        input_schema: input.schema,
        param_routes: input.routes,
        header_params: input.header_params,
        has_body: input.has_body,
        help_url: None,
        wire_names: input.wire_names,
        whole_body: input.whole_body,
        body_closed: input.body_closed,
        json_media_type: input.json_media_type,
    })
}

fn operation_request_url(cfg: &Config, root: &Value, op: &Value) -> Result<Option<String>> {
    let Some(key) = &cfg.operation_url_extension else {
        return Ok(None);
    };
    let source = if root.get("paths").is_some() {
        &root["paths"][op["path"].as_str().unwrap()][op["method"].as_str().unwrap()]
    } else {
        op
    };
    let Some(value) = source.get(key) else {
        return Ok(None);
    };
    let template = value
        .as_str()
        .context("operation URL extension must contain a URL string")?;
    let url = crate::network::public_url(template)?;
    ensure!(
        !url.host_str().unwrap_or_default().contains(['{', '}']),
        "operation URL authority must not contain parameters"
    );
    Ok(Some(template.into()))
}

#[cfg(test)]
mod operation_url_tests;
#[cfg(test)]
mod selection_tests;

fn apply_overrides(cfg: &Config, tool: &mut ToolSpec) -> Result<()> {
    if let Some(ap) = cfg.additional_properties {
        tool.input_schema["additionalProperties"] = json!(ap && tool.whole_body.is_none());
    }
    if let Some(patch) = cfg.overrides.get(&tool.name) {
        let pricing = patch
            .get("pricing_description")
            .map(|pricing| {
                ensure!(
                    patch["description"].is_string(),
                    "pricing_description requires a description override for {}",
                    tool.name
                );
                pricing.as_str().with_context(|| {
                    format!("pricing_description must be a string for {}", tool.name)
                })
            })
            .transpose()?;
        if let Some(desc) = patch["description"].as_str() {
            tool.description =
                pricing.map_or_else(|| desc.to_owned(), |price| format!("{desc} {price}"));
            tool.description_rendering = Some(DescriptionRendering {
                base: desc.into(),
                max_chars: cfg.max_description_chars,
            });
        }
        for (key, value) in patch["params"].as_object().into_iter().flatten() {
            if !value.is_string() {
                continue;
            }
            if let Some(pointer) = tool.parameter_note_pointer(key) {
                tool.input_schema
                    .pointer_mut(&pointer)
                    .expect("resolved parameter-description target")["description"] = value.clone();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "catalog/description_tests.rs"]
mod description_tests;

#[cfg(test)]
#[path = "catalog/conversion_tests.rs"]
mod conversion_tests;

#[cfg(test)]
#[path = "catalog/input_tests.rs"]
mod input_tests;

#[cfg(test)]
#[path = "catalog/recommendation_tests.rs"]
mod recommendation_tests;
