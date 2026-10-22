//! Explicit resource rejection; never return a partial document as successful data.
use anyhow::{Result, bail};
/// Public inventory page selection; not a total-result or acquisition bound.
pub const DEFAULT_PAGE_SIZE: usize = 25;
pub const MCP_REQUEST_BYTES: usize = 4 * 1024 * 1024;
pub const RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub const HELP_BYTES: usize = 4 * 1024 * 1024;
pub const SPEC_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const API_CACHE_TTL_SECONDS: u64 = 24 * 60 * 60;
pub fn response_default() -> usize {
    RESPONSE_BYTES
}
pub fn help_default() -> usize {
    HELP_BYTES
}

/// Reject a resource bound with safe diagnostics, never caller-supplied content.
pub(crate) fn check(
    actual: usize,
    limit: usize,
    reason: &'static str,
    setting: &'static str,
) -> Result<()> {
    if actual > limit {
        tracing::warn!(
            reason,
            actual,
            limit,
            setting,
            "resource limit exceeded; operation rejected"
        );
        bail!("{reason}: actual={actual}, limit={limit}, setting={setting}");
    }
    Ok(())
}

pub(crate) fn busy(reason: &'static str, limit: usize) -> &'static str {
    tracing::warn!(
        reason,
        limit,
        "resource capacity exhausted; operation rejected"
    );
    reason
}

#[derive(Debug)]
pub(crate) struct DownloadLimit {
    pub kind: &'static str,
    pub setting: &'static str,
    pub limit: usize,
}
impl std::fmt::Display for DownloadLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} exceeds {}={} bytes; content rejected, no partial content returned. Ask the operator to raise {} if appropriate.",
            self.kind, self.setting, self.limit, self.setting
        )
    }
}
impl std::error::Error for DownloadLimit {}

pub fn exceeded(kind: &'static str, setting: &'static str, limit: usize) -> anyhow::Error {
    // Do not include URLs, headers, document excerpts or credentials in logs.
    tracing::warn!(
        resource = kind,
        setting,
        limit_bytes = limit,
        "download limit exceeded; content rejected, no partial content returned"
    );
    DownloadLimit {
        kind,
        setting,
        limit,
    }
    .into()
}
pub async fn read(
    mut response: reqwest::Response,
    limit: usize,
    kind: &'static str,
    setting: &'static str,
) -> Result<Vec<u8>> {
    if limit == 0 {
        bail!("{setting} must be positive");
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(exceeded(kind, setting, limit));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(exceeded(kind, setting, limit));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
