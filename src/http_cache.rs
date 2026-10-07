//! Private, disposable discovery cache. Never used by payment or dynamic imports.
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
    // Ephemeral provenance: never written back as authority to revalidate.
    #[serde(skip)]
    direct_reuse: bool,
}
#[derive(Clone)]
pub(crate) struct Entry {
    pub metadata: Metadata,
    pub data: Vec<u8>,
}
pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn date(value: &str) -> Option<u64> {
    httpdate::parse_http_date(value)
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}
impl Metadata {
    pub fn from_headers(headers: &HeaderMap, delay: Duration, old: Option<&Self>) -> Option<Self> {
        // Conservatively decline all variants and cookie-bearing responses. We do not
        // persist request headers, cookies, challenges or arbitrary response headers.
        if headers.contains_key("vary") || headers.contains_key("set-cookie") {
            return None;
        }
        let mut selected = old.map(|m| m.headers.clone()).unwrap_or_default();
        for name in [
            "cache-control",
            "date",
            "age",
            "expires",
            "etag",
            "last-modified",
        ] {
            let values = headers.get_all(name).iter().collect::<Vec<_>>();
            if values.len() > 1 {
                return None;
            }
            if let Some(value) = values.first() {
                selected.insert(name.into(), value.to_str().ok()?.to_owned());
            }
        }
        if selected.values().map(String::len).sum::<usize>() > METADATA_LIMIT {
            tracing::warn!(
                limit_bytes = METADATA_LIMIT,
                "HTTP cache metadata limit exceeded; response will not be persisted"
            );
            return None;
        }
        let mut max_age = None;
        let mut validate = false;
        for directive in selected
            .get("cache-control")
            .map(String::as_str)
            .unwrap_or("")
            .split(',')
        {
            let (name, value) = directive
                .trim()
                .split_once('=')
                .map_or((directive.trim(), None), |(n, v)| {
                    (n.trim(), Some(v.trim()))
                });
            match name.to_ascii_lowercase().as_str() {
                "no-store" | "private" => return None,
                "no-cache" => validate = true,
                "max-age" => {
                    if max_age.is_some() {
                        return None;
                    }
                    let value = value?;
                    let value = if let Some(quoted) = value.strip_prefix('"') {
                        quoted.strip_suffix('"')?
                    } else {
                        value
                    };
                    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                        return None;
                    }
                    max_age = Some(value.parse::<u64>().ok()?);
                }
                _ if value.is_some_and(|v| v.contains('"')) => return None,
                _ => (),
            }
        }
        let stored = now();
        let response_date = selected.get("date").map(|v| date(v)).transpose_option()?;
        let age = selected
            .get("age")
            .map(|v| v.parse::<u64>().ok())
            .transpose_option()?
            .unwrap_or(0);
        let lifetime = max_age.or_else(|| {
            selected
                .get("expires")
                .and_then(|v| date(v))
                .map(|v| v.saturating_sub(response_date.unwrap_or(stored)))
        });
        let apparent = stored.saturating_sub(response_date.unwrap_or(stored));
        let corrected = age
            .saturating_add(delay.as_secs().saturating_add(1))
            .max(apparent);
        let expires = if validate || (old.is_some() && !headers.contains_key("date")) {
            0
        } else {
            stored.saturating_add(lifetime.unwrap_or(0).saturating_sub(corrected))
        };
        if lifetime.is_none()
            && !selected.contains_key("etag")
            && !selected.contains_key("last-modified")
        {
            return None;
        }
        Some(Self {
            headers: selected,
            stored,
            expires,
            direct_reuse: false,
        })
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
        !headers.contains_key("vary")
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

#[derive(Clone)]
pub(crate) struct Slot {
    directory: PathBuf,
    key: String,
    direct_key: Option<String>,
    resource: String,
    limit: usize,
}
impl Slot {
    pub fn new(cfg: &crate::catalog::Config, url: &str, kind: &str, limit: usize) -> Option<Self> {
        if !cfg.http_cache_enabled || crate::qualification::active() {
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
        let key = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    1,
                    kind,
                    url,
                    network,
                    cfg.allow_http1,
                    cfg.allow_tls12,
                    cfg.timeout.to_bits(),
                    cfg.probe_timeout.to_bits(),
                    cfg.probe_ttl_seconds.to_bits(),
                    limit
                ))
                .ok()?
            )
        );
        // Separate provenance: an ordinary direct-mode cache never crosses policy
        // boundaries. Only explicitly warmed entries target another configured policy.
        let direct_key = format!(
            "{:x}",
            Sha256::digest(format!("explicit-direct-warm-v1:{key}"))
        );
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
            limit,
        })
    }
    pub(crate) fn relay(mut self, identity: &str) -> Self {
        self.key = format!(
            "{:x}",
            Sha256::digest(format!("discovery-relay-v1:{identity}:{}", self.key))
        );
        self.direct_key = None;
        self
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
                    resource = self.resource,
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
            let db = connection.transaction()?;
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
                db.commit()?;
                return Ok(None);
            }
            let (metadata, data): (String, Vec<u8>) = db.query_row(
                "SELECT metadata,data FROM entries WHERE key=?1",
                [&slot.key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            Ok(Some(Entry {
                metadata: serde_json::from_str(&metadata)?,
                data,
            }))
        })
        .await;
        match result {
            Ok(Ok(value)) => value,
            _ => {
                tracing::warn!(
                    limit_database_bytes = DATABASE_LIMIT,
                    "HTTP cache read failed; fetching origin without cached data"
                );
                None
            }
        }
    }
    pub async fn write(&self, entry: Option<Entry>) {
        let slot = self.clone();
        let result = blocking(move || -> Result<()> {
            // Never create a treasury state directory, including after concurrent removal.
            ensure!(slot.directory.parent().is_some_and(Path::is_dir), "cache parent missing");
            if entry.is_none() && !slot.directory.exists() { return Ok(()); }
            let mut db = open(&slot.directory)?;
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute("DELETE FROM entries WHERE key=?1", [&slot.key])?;
            if let Some(entry) = entry {
                let metadata = serde_json::to_string(&entry.metadata)?;
                let size = metadata.len().saturating_add(entry.data.len());
                if entry.data.len() > slot.limit || metadata.len() > METADATA_LIMIT || size > CAPACITY {
                    tracing::warn!(limit_bytes = slot.limit.min(CAPACITY), limit_metadata_bytes = METADATA_LIMIT, "HTTP cache entry exceeds resource limit; complete response used without persistence");
                } else {
                    loop {
                        let (bytes, count): (i64, i64) = tx.query_row("SELECT coalesce(sum(length(CAST(metadata AS BLOB))+length(data)),0),count(*) FROM entries", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
                        if bytes + size as i64 <= CAPACITY as i64 && count < ENTRIES as i64 { break; }
                        tx.execute("DELETE FROM entries WHERE key=(SELECT key FROM entries ORDER BY stored,key LIMIT 1)", [])?;
                        tracing::info!(limit_bytes = CAPACITY, limit_entries = ENTRIES, "HTTP cache capacity reached; oldest disposable entry evicted");
                    }
                    tx.execute("INSERT INTO entries VALUES(?1,?2,?3,?4)", params![slot.key, metadata, entry.data, i64::try_from(entry.metadata.stored)?])?;
                }
            }
            tx.commit()?;
            Ok(())
        }).await;
        if !matches!(result, Ok(Ok(()))) {
            tracing::warn!(
                limit_database_bytes = DATABASE_LIMIT,
                "HTTP cache update failed; response remains usable without persistence"
            );
        }
    }
}
fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> tokio::task::JoinHandle<Result<T>> {
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    tokio::task::spawn_blocking(move || tracing::dispatcher::with_default(&dispatch, work))
}

fn open(directory: &Path) -> Result<Connection> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        if !directory.exists() {
            match std::fs::DirBuilder::new().mode(0o700).create(directory) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(e.into()),
            }
        }
        let metadata = std::fs::symlink_metadata(directory)?;
        ensure!(
            metadata.is_dir() && metadata.permissions().mode() & 0o077 == 0,
            "cache directory must be private"
        );
    }
    #[cfg(not(unix))]
    if !directory.exists() {
        std::fs::create_dir(directory)?;
    }
    let path = directory.join("discovery-v1.sqlite");
    if !path.exists() {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
    }
    let metadata = std::fs::symlink_metadata(&path)?;
    ensure!(metadata.is_file(), "cache must be an ordinary file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.mode() & 0o077 == 0 && metadata.nlink() == 1,
            "cache must be private and singly linked"
        );
    }
    let db = Connection::open(path)?;
    db.busy_timeout(Duration::from_secs(2))?;
    let page_size: i64 = db.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    let page_count: i64 = db.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    ensure!(
        page_size == 4096 && page_count <= 36864,
        "cache database exceeds storage limit"
    );
    db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA max_page_count=36864; CREATE TABLE IF NOT EXISTS entries(key TEXT PRIMARY KEY,metadata TEXT NOT NULL,data BLOB NOT NULL,stored INTEGER NOT NULL);")?;
    Ok(db)
}

#[cfg(test)]
mod tests;
