//! Private, disposable catalog storage and advisory prices; never payment authority.
use anyhow::{Result, ensure};
use reqwest::header::HeaderMap;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const CAPACITY: usize = 128 * 1024 * 1024;
const ENTRIES: usize = 4096;
const METADATA_LIMIT: usize = 16 * 1024;
const DATABASE_LIMIT: usize = 144 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Metadata {
    headers: std::collections::BTreeMap<String, String>,
    pub stored: u64,
    pub expires: u64,
    local_override: bool,
    // Ephemeral provenance: never written back as authority to revalidate.
    #[serde(skip)]
    direct_reuse: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub hints: Option<std::sync::Arc<crate::pricing::runtime::CatalogHints>>,
    pub metadata: Metadata,
    pub data: Vec<u8>,
}
pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(crate) const INDEFINITE_EXPIRY: u64 = i64::MAX as u64;
/// Disk opt-out leaves advertisements bound to the loaded source generation.
pub(crate) fn advertised_expiry(lifetime: crate::catalog::CacheLifetime, delay: Duration) -> u64 {
    if lifetime.disabled() {
        INDEFINITE_EXPIRY
    } else {
        lifetime.expires(now(), delay)
    }
}
/// Indefinite lifetimes never enter platform clock arithmetic.
pub(crate) fn advertised_until(expires: u64) -> Option<std::time::Instant> {
    if expires == INDEFINITE_EXPIRY {
        return None;
    }
    let now = std::time::Instant::now();
    Some(
        now.checked_add(Duration::from_secs(expires.saturating_sub(self::now())))
            .unwrap_or(now),
    )
}
fn date(value: &str) -> Option<u64> {
    httpdate::parse_http_date(value)
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}
/// RFC list splitting: commas inside quoted extension values are not directives.
fn cache_directives(value: &str) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let (mut quoted, mut escaped, mut start) = (false, false, 0);
    for (i, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ',' if !quoted => {
                parts.push(&value[start..i]);
                start = i + 1;
            }
            _ => (),
        }
    }
    if quoted || escaped {
        return None;
    }
    parts.push(&value[start..]);
    Some(parts)
}

/// All cached direct/Tor GETs use one explicit, cookie-free header profile.
/// The profile version is in the cache key. URL includes Host; no agent-supplied
/// headers, cookies or authorization are attached. Other Vary fields are absent.
pub(crate) fn discovery_get(http: &reqwest::Client, url: &str) -> reqwest::RequestBuilder {
    http.get(url)
        .header("accept", "*/*")
        .header("accept-encoding", "identity")
}

/// Private-cache storage directives, independent of receipt-time freshness.
struct CacheControl {
    max_age: Option<u64>,
    validate: bool,
}
impl CacheControl {
    fn parse(value: &str) -> Option<Self> {
        let mut max_age = None;
        let mut validate = false;
        for directive in cache_directives(value)? {
            let (name, value) = directive
                .trim()
                .split_once('=')
                .map_or((directive.trim(), None), |(n, v)| {
                    (n.trim(), Some(v.trim()))
                });
            match name.to_ascii_lowercase().as_str() {
                "no-store" => return None,
                "no-cache" => validate = true,
                "max-age" => {
                    let value = value?;
                    let value = if let Some(value) = value.strip_prefix('"') {
                        value.strip_suffix('"')?
                    } else {
                        value
                    };
                    if value.is_empty() || !value.bytes().all(|c| c.is_ascii_digit()) {
                        return None;
                    }
                    let seconds = value.parse::<u64>().unwrap_or(u64::MAX);
                    max_age = Some(max_age.map_or(seconds, |old: u64| old.min(seconds)));
                }
                // This is a private cache; private and s-maxage do not forbid
                // storage. Unknown extensions do not override known directives.
                _ => (),
            }
        }
        Some(Self { max_age, validate })
    }
    fn expires(
        &self,
        headers: &std::collections::BTreeMap<String, String>,
        delay: Duration,
        stored: u64,
    ) -> Option<u64> {
        let response_date = headers.get("date").map(|v| date(v)).transpose_option()?;
        let age = headers
            .get("age")
            .map(|v| v.parse::<u64>().ok())
            .transpose_option()?
            .unwrap_or(0);
        let explicit = self.max_age.or_else(|| {
            headers.get("expires").map(|v| {
                date(v)
                    .unwrap_or(0)
                    .saturating_sub(response_date.unwrap_or(stored))
            })
        });
        let lifetime = explicit;
        let apparent = stored.saturating_sub(response_date.unwrap_or(stored));
        let corrected = age
            .saturating_add(delay.as_secs().saturating_add(1))
            .max(apparent);
        let expires = if self.validate {
            0
        } else {
            stored.saturating_add(lifetime.unwrap_or(0).saturating_sub(corrected))
        };
        if lifetime.is_none()
            && !headers.contains_key("etag")
            && !headers.contains_key("last-modified")
        {
            return None;
        }
        Some(expires)
    }
}

impl Metadata {
    pub fn from_headers(headers: &HeaderMap, delay: Duration, old: Option<&Self>) -> Option<Self> {
        Self::parse(headers, delay, old)
    }
    pub fn from_catalog_headers(
        headers: &HeaderMap,
        delay: Duration,
        old: Option<&Self>,
        ttl_seconds: impl Into<crate::catalog::CacheLifetime>,
    ) -> Option<Self> {
        let ttl_seconds = ttl_seconds.into();
        if ttl_seconds.disabled() {
            return None;
        }
        // Catalog snapshots use local retention, independently of HTTP freshness.
        // Retain validators and variant identity, never cookies or upstream prose.
        let mut selected = HeaderMap::new();
        for name in ["etag", "last-modified", "vary"] {
            if let Some(value) = old.and_then(|m| m.headers.get(name)) {
                selected.insert(name, value.parse().ok()?);
            }
            if headers.contains_key(name) {
                selected.remove(name);
                for value in headers.get_all(name) {
                    selected.append(name, value.clone());
                }
            }
        }
        selected.insert("cache-control", "max-age=1".parse().ok()?);
        let mut metadata = Self::parse(&selected, Duration::ZERO, None)?;
        metadata.local_override = true;
        metadata.expires = ttl_seconds.expires(metadata.stored, delay);
        tracing::debug!(
            cache_policy = "local_override",
            ttl_seconds = ?ttl_seconds,
            origin_policy =
                Self::policy_label(headers, Self::from_headers(headers, delay, None).as_ref()),
            "Catalog uses configured local cache lifetime instead of origin freshness"
        );
        Some(metadata)
    }
    /// Curl does not expose its effective target request headers.
    pub fn from_relay_headers(
        headers: &HeaderMap,
        delay: Duration,
        catalog_ttl: Option<crate::catalog::CacheLifetime>,
    ) -> Option<Self> {
        if headers.contains_key("vary") {
            return None;
        }
        match catalog_ttl {
            Some(ttl) => Self::from_catalog_headers(headers, delay, None, ttl),
            None => Self::from_headers(headers, delay, None),
        }
    }
    fn parse(headers: &HeaderMap, delay: Duration, old: Option<&Self>) -> Option<Self> {
        let selected = Self::select_headers(headers, old)?;
        Self::validate_vary(&selected)?;
        let control = CacheControl::parse(
            selected
                .get("cache-control")
                .map(String::as_str)
                .unwrap_or(""),
        )?;
        let stored = now();
        let expires = control.expires(&selected, delay, stored)?;
        Some(Self {
            headers: selected,
            stored,
            expires,
            local_override: false,
            direct_reuse: false,
        })
    }
    fn select_headers(
        headers: &HeaderMap,
        old: Option<&Self>,
    ) -> Option<std::collections::BTreeMap<String, String>> {
        let mut selected = old.map(|m| m.headers.clone()).unwrap_or_default();
        // A validation response supplies a new age observation, not the age of
        // the previous download. Synthesize its receipt Date if omitted.
        if old.is_some() {
            selected.remove("age");
            selected.insert("date".into(), httpdate::fmt_http_date(SystemTime::now()));
        }
        for name in [
            "cache-control",
            "vary",
            "date",
            "age",
            "expires",
            "etag",
            "last-modified",
        ] {
            let values = headers
                .get_all(name)
                .iter()
                .map(|v| v.to_str().ok())
                .collect::<Option<Vec<_>>>()?;
            if values.is_empty() {
                continue;
            }
            let value = if matches!(name, "cache-control" | "vary") {
                values.join(", ")
            } else {
                if values.iter().any(|v| *v != values[0]) {
                    return None;
                }
                values[0].to_owned()
            };
            selected.insert(name.into(), value);
        }
        if selected.values().map(String::len).sum::<usize>() > METADATA_LIMIT {
            tracing::warn!(
                limit_bytes = METADATA_LIMIT,
                "HTTP cache metadata limit exceeded; response will not be persisted"
            );
            return None;
        }
        Some(selected)
    }
    fn validate_vary(selected: &std::collections::BTreeMap<String, String>) -> Option<()> {
        if let Some(vary) = selected.get("vary") {
            for name in vary.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                if name == "*" {
                    return None;
                }
                let name = reqwest::header::HeaderName::from_bytes(name.as_bytes()).ok()?;
                // Conditional fields vary between a full GET and validation.
                if name.as_str().starts_with("if-") {
                    return None;
                }
            }
        }
        Some(())
    }
    /// Fixed labels only: never expose validator values or upstream header prose.
    pub fn policy_label(headers: &HeaderMap, metadata: Option<&Self>) -> &'static str {
        if let Some(metadata) = metadata {
            return if metadata.fresh() {
                if metadata.local_override {
                    "local_override"
                } else {
                    "fresh"
                }
            } else if metadata.has_validator() {
                "requires_revalidation"
            } else {
                "expired_without_validator"
            };
        }
        if headers
            .get_all("vary")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .any(|v| v.trim() == "*")
        {
            return "vary_star";
        }
        let controls = headers.get_all("cache-control");
        for value in controls.iter().filter_map(|v| v.to_str().ok()) {
            for directive in value.split(',') {
                let name = directive.trim().split('=').next().unwrap_or("").trim();
                if name.eq_ignore_ascii_case("no-store") {
                    return "no_store";
                }
            }
        }
        if !["cache-control", "expires", "etag", "last-modified"]
            .iter()
            .any(|name| headers.contains_key(*name))
        {
            "no_cache_headers"
        } else {
            "unsupported_or_incomplete_cache_headers"
        }
    }
    pub fn fresh(&self) -> bool {
        let now = now();
        now >= self.stored && now < self.expires
    }
    pub fn conditional(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.direct_reuse {
            return request;
        }
        if let Some(etag) = self.headers.get("etag") {
            request.header("if-none-match", etag)
        } else if let Some(modified) = self.headers.get("last-modified") {
            request.header("if-modified-since", modified)
        } else {
            request
        }
    }
    pub fn matches_validation(&self, headers: &HeaderMap) -> bool {
        // A validator naming another representation cannot validate these bytes.
        for name in ["etag", "last-modified"] {
            if let (Some(previous), Some(current)) = (self.headers.get(name), headers.get(name))
                && current.to_str().ok() != Some(previous.as_str())
            {
                return false;
            }
        }
        true
    }
    pub fn has_validator(&self) -> bool {
        !self.direct_reuse
            && (self.headers.contains_key("etag") || self.headers.contains_key("last-modified"))
    }
}
// Option<Option<T>>: absent is fine, malformed is not.
trait TransposeOption<T> {
    fn transpose_option(self) -> Option<Option<T>>;
}
impl<T> TransposeOption<T> for Option<Option<T>> {
    fn transpose_option(self) -> Option<Option<T>> {
        match self {
            None => Some(None),
            Some(v) => v.map(Some),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Slot {
    directory: PathBuf,
    key: String,
    direct_key: Option<String>,
    resource: String,
    source: String,
    limit: usize,
    pub catalog_ttl: crate::catalog::CacheLifetime,
    revision: Option<(String, String)>,
    force_refresh: bool,
    family: Option<String>,
    replace_family: bool,
}
impl Slot {
    pub fn availability(cfg: &crate::catalog::Config, kind: &str) -> &'static str {
        if !cfg.http_cache_enabled {
            "disabled_by_config"
        } else if kind == "catalog" && cfg.catalog_cache_ttl_seconds.disabled() {
            "disabled_by_catalog_ttl"
        } else if crate::qualification::active() {
            "disabled_for_qualification"
        } else if cfg.http_cache_directory.is_none() {
            "no_cache_directory"
        } else {
            "enabled"
        }
    }
    pub fn new(cfg: &crate::catalog::Config, url: &str, kind: &str, limit: usize) -> Option<Self> {
        if !cfg.http_cache_enabled
            || crate::qualification::active()
            || (kind == "catalog" && cfg.catalog_cache_ttl_seconds.disabled())
        {
            return None;
        }
        let directory = cfg.http_cache_directory.clone()?;
        let parsed = reqwest::Url::parse(url).ok()?;
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return None;
        }
        let network = serde_json::to_vec(
            cfg.http_cache_direct_warm_target
                .as_ref()
                .unwrap_or(&crate::network::global().policy),
        )
        .ok()?;
        let key = alloy_primitives::hex::encode(Sha256::digest(
            serde_json::to_vec(&(
                "discovery-get-identity-v3",
                kind,
                url,
                network,
                cfg.allow_http1,
                cfg.allow_tls12,
                cfg.read_timeout_seconds.map(f64::to_bits),
                limit,
            ))
            .ok()?,
        ));
        let key = if kind == "catalog" {
            alloy_primitives::hex::encode(Sha256::digest(format!(
                "local-catalog-v3:{:?}:{key}",
                cfg.catalog_cache_ttl_seconds
            )))
        } else {
            key
        };
        let family = (kind == "catalog").then(|| key.clone());
        // Separate provenance: an ordinary direct-mode cache never crosses policy
        // boundaries. Only explicitly warmed entries target another configured policy.
        let direct_key =
            alloy_primitives::hex::encode(Sha256::digest(format!("explicit-direct-warm-v1:{key}")));
        let (key, direct_key) = if cfg.http_cache_direct_warm_target.is_some() {
            if crate::network::global().policy.mode != crate::network::Mode::Direct {
                return None;
            }
            (direct_key, None)
        } else {
            (key, Some(direct_key))
        };
        Some(Self {
            directory,
            key,
            direct_key,
            resource: kind.to_owned(),
            source: cfg
                .discovery_source
                .clone()
                .unwrap_or_else(|| "standalone".into()),
            limit,
            catalog_ttl: cfg.catalog_cache_ttl_seconds,
            force_refresh: cfg.catalog_cache_force_refresh,
            family,
            replace_family: false,
            revision: (kind == "catalog").then(|| {
                (
                    cfg.discovery_source
                        .clone()
                        .unwrap_or_else(|| "standalone".into()),
                    cfg.cache_revision
                        .clone()
                        .unwrap_or_else(|| configuration_revision(cfg)),
                )
            }),
        })
    }
    /// An accepted replacement supersedes provenance variants, never their active I/O.
    pub(crate) fn replacement(&self) -> Self {
        let mut slot = self.clone();
        slot.replace_family = true;
        slot
    }
    pub(crate) fn relay(mut self, identity: &str) -> Self {
        self.key = alloy_primitives::hex::encode(Sha256::digest(format!(
            "discovery-relay-v1:{identity}:{}",
            self.key
        )));
        self.direct_key = None;
        self
    }
    pub(crate) async fn check_revision(&self) {
        self.check_primary_revision().await;
        if let Some(key) = &self.direct_key {
            let mut direct = self.clone();
            direct.key = key.clone();
            direct.check_primary_revision().await;
        }
    }
    async fn check_primary_revision(&self) {
        let slot = self.clone();
        let _ = blocking(move || {
            if !slot.directory.exists() { return Ok(()); }
            let mut db = open(&slot.directory)?;
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let mut retired = Vec::new();
            if let Some((source, revision)) = &slot.revision {
                let old: Option<String> = tx.query_row("SELECT revision FROM catalog_revisions WHERE key=?1 AND source=?2", params![slot.key, source], |r| r.get(0)).optional()?;
                if old.as_ref().is_some_and(|old| old != revision) {
                    tx.execute("DELETE FROM entries WHERE key=?1", [&slot.key])?;
                    retired.push(slot.key.clone());
                }
                tx.execute("INSERT INTO catalog_revisions SELECT key,?2,?3 FROM entries WHERE key=?1 ON CONFLICT(key,source) DO UPDATE SET revision=excluded.revision", params![slot.key, source, revision])?;
            }
            commit_cache(&slot, tx, retired, None, None)?;
            Ok(())
        }).await;
    }
    pub async fn read(&self) -> Option<Entry> {
        let ordinary = self.read_primary().await;
        if ordinary
            .as_ref()
            .is_some_and(|entry| entry.metadata.fresh())
        {
            return ordinary;
        }
        if let Some(key) = &self.direct_key {
            let mut direct = self.clone();
            direct.key = key.clone();
            if let Some(mut entry) = direct
                .read_primary()
                .await
                .filter(|entry| entry.metadata.fresh())
            {
                tracing::warn!(
                    source = self.source,
                    resource = self.resource,
                    cache = "direct_warm_hit",
                    "Using explicitly directly warmed discovery cache; content was fetched with direct egress"
                );
                entry.metadata.direct_reuse = true;
                return Some(entry);
            }
        }
        // Never send a directly fetched validator through the configured network,
        // and never refresh a direct entry automatically when it expires.
        ordinary
    }
    async fn read_primary(&self) -> Option<Entry> {
        let slot = self.clone();
        let result = blocking(move || -> Result<Option<Entry>> {
            if !slot.directory.exists() {
                return Ok(None);
            }
            let mut connection = open(&slot.directory)?;
            let db =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let size: Option<(i64, i64)> = db
                .query_row(
                    "SELECT length(CAST(metadata AS BLOB)), length(data) FROM entries WHERE key=?1",
                    [&slot.key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((metadata, data)) = size else {
                return Ok(None);
            };
            if metadata > METADATA_LIMIT as i64 || data > slot.limit as i64 {
                tracing::warn!(
                    limit_bytes = slot.limit,
                    limit_metadata_bytes = METADATA_LIMIT,
                    "HTTP cache entry exceeds resource limit; discarded, fetching origin"
                );
                db.execute("DELETE FROM entries WHERE key=?1", [&slot.key])?;
                commit_cache(&slot, db, vec![slot.key.clone()], None, None)?;
                return Ok(None);
            }
            let (metadata, data): (String, Vec<u8>) = db.query_row(
                "SELECT metadata,data FROM entries WHERE key=?1",
                [&slot.key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let metadata: Metadata = match serde_json::from_str(&metadata) {
                Ok(metadata) => metadata,
                Err(_) => {
                    tracing::debug!("HTTP cache metadata is incompatible; fetching origin");
                    return Ok(None);
                }
            };
            let hints = if slot.resource == "catalog" {
                hydrate(&slot, &db, &metadata)?
            } else {
                None
            };
            Ok(Some(Entry {
                metadata,
                data,
                hints,
            }))
        })
        .await;
        match result {
            Ok(Ok(value)) => value,
            failure => {
                report_failure(self, "read", &failure);
                None
            }
        }
    }
    pub async fn write(
        &self,
        entry: Option<Entry>,
    ) -> Option<std::sync::Arc<crate::pricing::runtime::CatalogHints>> {
        let slot = self.clone();
        let result = blocking(move || -> Result<(bool, Option<std::sync::Arc<crate::pricing::runtime::CatalogHints>>)> {
            // Never create a treasury state directory, including after concurrent removal.
            if entry.is_none() && !slot.directory.exists() { retire(&slot); return Ok((false,None)); }
            ensure!(slot.directory.parent().is_some_and(Path::is_dir), "cache parent missing");
            let mut db = open(&slot.directory)?;
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let mut retired = if slot.replace_family { delete_family(&slot, &tx, entry.is_some())? } else { Vec::new() };
            let Some(entry) = entry else {
                tx.execute("DELETE FROM entries WHERE key=?1", [&slot.key])?;
                retired.push(slot.key.clone());
                commit_cache(&slot, tx, retired, None, None)?;
                return Ok((false,None));
            };
            let metadata = serde_json::to_string(&entry.metadata)?;
            let size = metadata.len().saturating_add(entry.data.len());
            if entry.data.len() > slot.limit || metadata.len() > METADATA_LIMIT || size > CAPACITY {
                tx.execute("DELETE FROM entries WHERE key=?1",[&slot.key])?;
                retired.push(slot.key.clone());
                commit_cache(&slot, tx, retired, None, None)?;
                tracing::warn!(limit_bytes = slot.limit.min(CAPACITY), limit_metadata_bytes = METADATA_LIMIT, "HTTP cache entry exceeds resource limit; complete response used without persistence");
                return Ok((false,None));
            }
            let unchanged = !slot.force_refresh && tx.query_row("SELECT data=?2 AND EXISTS(SELECT 1 FROM catalogs WHERE catalogs.key=entries.key AND expires>?3) FROM entries WHERE key=?1",params![slot.key,entry.data,i64::try_from(now())?],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
            if !unchanged {
                tx.execute("DELETE FROM entries WHERE key=?1", [&slot.key])?;
                retired.push(slot.key.clone());
            }
            loop {
                let (bytes, count): (i64, i64) = tx.query_row("SELECT coalesce(sum(length(CAST(metadata AS BLOB))+length(data)),0),count(*) FROM entries WHERE key!=?1", [&slot.key], |r| Ok((r.get(0)?, r.get(1)?)))?;
                if bytes + size as i64 <= CAPACITY as i64 && count < ENTRIES as i64 { break; }
                let key: String = tx.query_row("DELETE FROM entries WHERE key=(SELECT key FROM entries WHERE key!=?1 ORDER BY stored,key LIMIT 1) RETURNING key", [&slot.key], |r| r.get(0))?;
                retired.push(key);
                tracing::debug!(limit_bytes = CAPACITY, limit_entries = ENTRIES, "HTTP cache capacity reached; oldest disposable entry evicted");
            }
            tx.execute("INSERT INTO entries VALUES(?1,?2,?3,?4) ON CONFLICT(key) DO UPDATE SET metadata=excluded.metadata,data=excluded.data,stored=excluded.stored", params![slot.key, metadata, entry.data, i64::try_from(entry.metadata.stored)?])?;
            if let Some(family) = &slot.family {
                tx.execute("INSERT INTO catalog_families VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET family=excluded.family", params![slot.key, family])?;
            }
            let hints = if slot.resource == "catalog" {
                tx.execute("INSERT INTO catalogs VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET expires=excluded.expires", params![slot.key, uuid::Uuid::new_v4().to_string(), i64::try_from(entry.metadata.expires)?])?;
                prepare_hints(&slot,&tx,&entry.metadata)?
            } else { None };
            if let Some((source,revision)) = &slot.revision { tx.execute("INSERT OR IGNORE INTO catalog_revisions VALUES(?1,?2,?3)",params![slot.key,source,revision])?; }
            let hints = commit_cache(&slot, tx, retired, hints, Some(&entry.metadata))?;
            Ok((true,hints))
        }).await;
        if matches!(result, Ok(Ok((true, _)))) {
            tracing::debug!(
                source = self.source,
                resource = self.resource,
                cache = "stored",
                "HTTP disk cache entry stored"
            );
        }
        report_failure(self, "update", &result);
        result
            .ok()
            .and_then(Result::ok)
            .and_then(|(_, hints)| hints)
    }
}
// Local error types supply safe causes; never format arbitrary error prose,
// database messages, paths or cached/upstream content into persistence diagnostics.
fn report_failure<T>(
    slot: &Slot,
    operation: &'static str,
    result: &std::result::Result<Result<T>, tokio::task::JoinError>,
) {
    let error = match result {
        Ok(Ok(_)) => return,
        Ok(Err(error)) => Some(error),
        Err(_) => None,
    };
    let io = error.and_then(|error| error.downcast_ref::<std::io::Error>());
    let sqlite = error.and_then(|error| match error.downcast_ref::<rusqlite::Error>() {
        Some(rusqlite::Error::SqliteFailure(code, _)) => Some(code),
        _ => None,
    });
    let cause = match result {
        Err(error) if error.is_panic() => "local cache worker panicked",
        Err(_) => "local cache worker was cancelled",
        _ if io.is_some() => "filesystem access failed",
        _ if sqlite.is_some() => "SQLite access failed",
        _ => "local cache validation failed; specific cause unavailable",
    };
    let impact = if operation == "read" {
        "fetching origin without cached data"
    } else {
        "response remains usable without persistence"
    };
    tracing::warn!(
        source = slot.source,
        resource = slot.resource,
        limit_database_bytes = DATABASE_LIMIT,
        io_kind = io.map(|error| format!("{:?}", error.kind())),
        sqlite_code = sqlite.map(|error| format!("{:?}", error.code)),
        sqlite_extended_code = sqlite.map(|error| error.extended_code),
        "HTTP cache {operation} failed: {cause}; {impact}"
    );
}

fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> tokio::task::JoinHandle<Result<T>> {
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    tokio::task::spawn_blocking(move || tracing::dispatcher::with_default(&dispatch, work))
}

fn open(directory: &Path) -> Result<Connection> {
    match crate::private_fs::create_dir(directory) {
        Ok(()) => (),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::AlreadyExists) =>
        {
            crate::private_fs::directory(directory)?;
        }
        Err(error) => return Err(error),
    }
    let path = directory.join("discovery-v1.sqlite");
    match crate::private_fs::create_file(&path) {
        Ok(_) => (),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::AlreadyExists) =>
        {
            crate::private_fs::open_file(&path, true)?;
        }
        Err(error) => return Err(error),
    }
    for suffix in ["-journal", "-wal", "-shm"] {
        let sidecar = directory.join(format!("discovery-v1.sqlite{suffix}"));
        match crate::private_fs::open_file(&sidecar, false) {
            Ok(_) => (),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    let db = Connection::open(path)?;
    db.busy_timeout(Duration::from_secs(2))?;
    let page_size: i64 = db.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    let page_count: i64 = db.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    ensure!(
        page_size == 4096 && page_count <= 36864,
        "cache database exceeds storage limit"
    );
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA max_page_count=36864; CREATE TABLE IF NOT EXISTS entries(key TEXT PRIMARY KEY,metadata TEXT NOT NULL,data BLOB NOT NULL,stored INTEGER NOT NULL);")?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS catalogs(key TEXT PRIMARY KEY REFERENCES entries(key) ON DELETE CASCADE,generation TEXT NOT NULL,expires INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS prices(key TEXT NOT NULL REFERENCES catalogs(key) ON DELETE CASCADE,operation TEXT NOT NULL,amount TEXT NOT NULL,version INTEGER NOT NULL,PRIMARY KEY(key,operation)); CREATE TABLE IF NOT EXISTS catalog_revisions(key TEXT NOT NULL REFERENCES entries(key) ON DELETE CASCADE,source TEXT NOT NULL,revision TEXT NOT NULL,PRIMARY KEY(key,source)); CREATE TABLE IF NOT EXISTS catalog_families(key TEXT PRIMARY KEY REFERENCES entries(key) ON DELETE CASCADE,family TEXT NOT NULL); CREATE INDEX IF NOT EXISTS catalog_family ON catalog_families(family);")?;
    Ok(db)
}

fn delete_family(
    slot: &Slot,
    tx: &rusqlite::Transaction<'_>,
    keep_primary: bool,
) -> Result<Vec<String>> {
    let Some(family) = &slot.family else {
        return Ok(Vec::new());
    };
    let keys = tx
        .prepare("SELECT key FROM catalog_families WHERE family=?1 AND (?2=0 OR key!=?3)")?
        .query_map(params![family, keep_primary, slot.key], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for key in &keys {
        tx.execute("DELETE FROM entries WHERE key=?1", [key])?;
    }
    Ok(keys)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod retry_after_tests;

// Weak ownership follows published catalogs; aliases reuse the same live entry.
type Registry = std::collections::HashMap<
    (PathBuf, String),
    std::sync::Weak<crate::pricing::runtime::CatalogHints>,
>;
fn registry() -> &'static std::sync::Mutex<Registry> {
    static REGISTRY: std::sync::OnceLock<std::sync::Mutex<Registry>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}
fn existing(slot: &Slot) -> Option<std::sync::Arc<crate::pricing::runtime::CatalogHints>> {
    registry()
        .lock()
        .expect("catalog registry poisoned")
        .get(&(slot.directory.clone(), slot.key.clone()))
        .and_then(std::sync::Weak::upgrade)
}
fn retire(slot: &Slot) {
    retire_locked(
        slot,
        &slot.key,
        &mut registry().lock().expect("catalog registry poisoned"),
    );
}
fn retire_locked(slot: &Slot, key: &str, registry: &mut Registry) {
    if let Some(hints) = registry
        .remove(&(slot.directory.clone(), key.to_owned()))
        .and_then(|h| h.upgrade())
    {
        hints.invalidate();
    }
}

// SQLite owns mutation ordering. Take the registry only after BEGIN IMMEDIATE;
// keep it through commit and memory publication so a later writer cannot publish
// first. No fallible SQL or filesystem work follows a successful commit.
fn commit_cache(
    slot: &Slot,
    tx: rusqlite::Transaction<'_>,
    retired: Vec<String>,
    hints: Option<std::sync::Arc<crate::pricing::runtime::CatalogHints>>,
    metadata: Option<&Metadata>,
) -> Result<Option<std::sync::Arc<crate::pricing::runtime::CatalogHints>>> {
    let mut registry = registry().lock().expect("catalog registry poisoned");
    tx.commit()?;
    for key in retired {
        retire_locked(slot, &key, &mut registry);
    }
    // An expired entry has no new attachment, but its committed expiry still
    // applies to handles already attached to this unchanged generation.
    if hints.is_none()
        && let Some(metadata) = metadata
        && let Some(previous) = registry
            .get(&(slot.directory.clone(), slot.key.clone()))
            .and_then(std::sync::Weak::upgrade)
    {
        previous
            .state
            .lock()
            .expect("catalog hints poisoned")
            .metadata = metadata.clone();
    }
    Ok(hints.map(|hints| {
        publish_hints(
            slot,
            hints,
            metadata.expect("catalog metadata"),
            &mut registry,
        )
    }))
}
impl Slot {
    pub(crate) async fn persist_price(
        &self,
        generation: &str,
        operation: &str,
        amount: alloy_primitives::U256,
        version: i64,
    ) {
        let (slot, generation, operation) =
            (self.clone(), generation.to_owned(), operation.to_owned());
        let result = blocking(move || {
            if !slot.directory.exists() { return Ok(()); }
            let db = open(&slot.directory)?;
            db.execute("INSERT INTO prices(key,operation,amount,version) SELECT key,?3,?4,?5 FROM catalogs WHERE key=?1 AND generation=?2 AND expires>?6 ON CONFLICT(key,operation) DO UPDATE SET amount=excluded.amount,version=excluded.version WHERE excluded.version>prices.version", params![slot.key,generation,operation,amount.to_string(),version,i64::try_from(now())?])?;
            Ok(())
        }).await;
        if !matches!(result, Ok(Ok(()))) {
            tracing::debug!(
                "Catalog price estimate could not be saved; in-memory estimate remains usable"
            );
        }
    }
}

fn hydrate(
    slot: &Slot,
    db: &Connection,
    metadata: &Metadata,
) -> Result<Option<std::sync::Arc<crate::pricing::runtime::CatalogHints>>> {
    let hints = prepare_hints(slot, db, metadata)?;
    let mut registry = registry().lock().expect("catalog registry poisoned");
    Ok(hints.map(|hints| publish_hints(slot, hints, metadata, &mut registry)))
}

fn prepare_hints(
    slot: &Slot,
    db: &Connection,
    metadata: &Metadata,
) -> Result<Option<std::sync::Arc<crate::pricing::runtime::CatalogHints>>> {
    use crate::pricing::runtime::{CatalogHints, Hint, State};
    let row: Option<(String, i64)> = db
        .query_row(
            "SELECT generation,expires FROM catalogs WHERE key=?1",
            [&slot.key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((generation, _expires)) = row else {
        return Ok(None);
    };
    if !metadata.fresh() {
        return Ok(None);
    }
    if let Some(hints) = existing(slot)
        && hints.generation == generation
    {
        return Ok(Some(hints));
    }
    let mut prices = std::collections::BTreeMap::new();
    let mut statement = db.prepare("SELECT operation,amount,version FROM prices WHERE key=?1")?;
    for row in statement.query_map([&slot.key], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })? {
        let (operation, amount, version) = row?;
        if let Ok(amount) = amount.parse() {
            prices.insert(
                operation,
                Hint {
                    amount,
                    version,
                    receipt: None,
                },
            );
        }
    }
    let hints = std::sync::Arc::new(CatalogHints {
        slot: slot.clone(),
        generation,
        state: std::sync::Mutex::new(State {
            metadata: metadata.clone(),
            valid: true,
            prices,
        }),
    });
    Ok(Some(hints))
}

fn publish_hints(
    slot: &Slot,
    hints: std::sync::Arc<crate::pricing::runtime::CatalogHints>,
    metadata: &Metadata,
    registry: &mut Registry,
) -> std::sync::Arc<crate::pricing::runtime::CatalogHints> {
    registry.retain(|_, value| value.strong_count() != 0);
    let identity = (slot.directory.clone(), slot.key.clone());
    if let Some(previous) = registry.get(&identity).and_then(std::sync::Weak::upgrade) {
        if previous.generation == hints.generation {
            previous
                .state
                .lock()
                .expect("catalog hints poisoned")
                .metadata = metadata.clone();
            return previous;
        }
        previous.invalidate();
    }
    registry.insert(identity, std::sync::Arc::downgrade(&hints));
    hints
}

pub(crate) fn configuration_revision(cfg: &crate::catalog::Config) -> String {
    alloy_primitives::hex::encode(Sha256::digest(
        serde_json::to_vec(cfg).expect("serializable catalog configuration"),
    ))
}
impl Slot {
    pub(crate) async fn record_configuration(&self, cfg: &crate::catalog::Config) {
        let mut slot = self.clone();
        slot.revision = Some((
            cfg.discovery_source
                .clone()
                .unwrap_or_else(|| "standalone".into()),
            cfg.cache_revision
                .clone()
                .unwrap_or_else(|| configuration_revision(cfg)),
        ));
        slot.check_primary_revision().await;
    }
}

#[cfg(test)]
mod price_tests;
