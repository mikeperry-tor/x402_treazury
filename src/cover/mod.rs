//! Optional, bounded provider cover traffic. Disabled operation owns no tasks or RNG.
pub mod budget;
pub mod episode;
pub mod metrics;
pub mod range;
pub mod registry;
pub mod runtime;
pub mod sampling;
/// Operator-selected listener/source binding for cover diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Scope {
    pub listener: String,
    pub source: String,
}
use anyhow::{Result, ensure};
use sampling::{Distribution, Unit};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_active_episodes: usize,
    pub max_active_streams: usize,
    pub window_ms: u64,
    pub max_requests_per_window: usize,
    pub max_cover_body_bytes_per_window: u64,
    pub max_padding_value_bytes_per_window: u64,
    pub max_owners: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_active_episodes: 8,
            max_active_streams: 6,
            window_ms: 60_000,
            max_requests_per_window: 64,
            max_cover_body_bytes_per_window: 4_194_304,
            max_padding_value_bytes_per_window: 65_536,
            max_owners: 1024,
        }
    }
}
impl Limits {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=4096).contains(&self.max_active_episodes)
                && (1..=4096).contains(&self.max_active_streams),
            "cover active limits must be 1..4096"
        );
        ensure!(
            (1..=86_400_000).contains(&self.window_ms)
                && (1..=1_000_000).contains(&self.max_requests_per_window),
            "invalid cover window/request limit"
        );
        ensure!(
            (1..=1_048_576).contains(&self.max_owners),
            "cover max_owners must be 1..1048576"
        );
        ensure!(
            self.max_cover_body_bytes_per_window > 0
                && self.max_cover_body_bytes_per_window <= 1 << 40
                && self.max_padding_value_bytes_per_window > 0
                && self.max_padding_value_bytes_per_window <= 1 << 40,
            "cover byte limits must be 1..2^40"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeMode {
    ExtraBody,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_ranges_enabled")]
    pub ranges_enabled: bool,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_url: Option<String>,
    pub volume_mode: VolumeMode,
    pub concurrency: usize,
    pub max_requests_per_episode: usize,
    pub max_cover_body_bytes_per_episode: u64,
    pub max_episode_ms: u64,
    pub qualification_range_bytes: u64,
    pub max_resource_bytes: u64,
    pub volume: Distribution,
    pub ranges: Distribution,
    pub start_delay: Distribution,
    pub request_gap: Distribution,
    pub tail: Distribution,
    pub padding: Option<Padding>,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Padding {
    pub header_name: String,
    pub on_api_requests: bool,
    pub on_cover_requests: bool,
    pub max_value_bytes_per_request: u64,
    pub max_value_bytes_per_episode: u64,
    pub max_total_header_list_bytes: u64,
    pub size: Distribution,
}
impl Padding {
    pub fn validate(&self) -> Result<()> {
        let name = self.header_name.to_ascii_lowercase();
        reqwest::header::HeaderName::from_bytes(name.as_bytes())?;
        // A narrow extension namespace protects standard, proxy, payment and future fields.
        ensure!(
            name.starts_with("x-")
                && name.len() <= 64
                && !name.starts_with("x-payment")
                && !name.starts_with("x-forwarded")
                && !name.starts_with("x-x402")
                && !matches!(
                    name.as_str(),
                    "x-api-key" | "x-auth-token" | "x-http-method-override" | "x-real-ip"
                ),
            "padding requires a non-reserved x- extension header"
        );
        ensure!(
            self.on_api_requests || self.on_cover_requests,
            "padding must select at least one request kind"
        );
        ensure!(
            (1..=8192).contains(&self.max_value_bytes_per_request)
                && self.max_value_bytes_per_episode >= self.max_value_bytes_per_request
                && self.max_value_bytes_per_episode <= 1 << 30,
            "invalid padding value budget"
        );
        ensure!(
            self.max_total_header_list_bytes > self.max_value_bytes_per_request
                && self.max_total_header_list_bytes <= 65536,
            "invalid padding header-list budget"
        );
        self.size.validate(Unit::Bytes, true)?;
        ensure!(
            self.size.bounds(Unit::Bytes)?.1 <= self.max_value_bytes_per_request,
            "padding distribution exceeds request budget"
        );
        Ok(())
    }
}
fn default_ranges_enabled() -> bool {
    true
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.ranges_enabled || self.padding.as_ref().is_some_and(|p| p.on_api_requests),
            "cover must enable ranges or API request padding"
        );
        if let Some(fallback) = &self.fallback_url {
            let mut candidate = self.clone();
            candidate.url = fallback.clone();
            candidate.fallback_url = None;
            candidate.validate()?;
            candidate.validate_origin(&self.url)?;
            ensure!(
                candidate.url != self.url,
                "cover fallback must differ from primary URL"
            );
        }
        let url = reqwest::Url::parse(&self.url)?;
        ensure!(
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none(),
            "cover URL must be credential-free HTTPS without a fragment"
        );
        ensure!(
            (1..=3).contains(&self.concurrency)
                && (1..=65536).contains(&self.max_requests_per_episode),
            "invalid cover concurrency/request cap"
        );
        ensure!(
            (1..=86_400_000).contains(&self.max_episode_ms),
            "cover episode duration must be 1..86400000ms"
        );
        ensure!(
            self.max_cover_body_bytes_per_episode > 0
                && self.max_cover_body_bytes_per_episode <= 1 << 40,
            "invalid cover episode byte cap"
        );
        ensure!(
            self.qualification_range_bytes > 0
                && self.qualification_range_bytes <= self.max_cover_body_bytes_per_episode
                && self.qualification_range_bytes <= self.max_resource_bytes
                && self.max_resource_bytes <= 1 << 40,
            "invalid cover qualification/resource size"
        );
        for d in [&self.volume, &self.ranges] {
            d.validate(Unit::Bytes, false)?;
        }
        for d in [&self.start_delay, &self.request_gap, &self.tail] {
            d.validate(Unit::Milliseconds, true)?;
            ensure!(
                d.bounds(Unit::Milliseconds)?.1 <= self.max_episode_ms,
                "cover timing exceeds episode deadline"
            );
        }
        ensure!(
            self.volume.bounds(Unit::Bytes)?.1 <= self.max_cover_body_bytes_per_episode
                && self.volume.bounds(Unit::Bytes)?.0 >= self.qualification_range_bytes,
            "cover volume outside episode/qualification budget"
        );
        ensure!(
            self.ranges.bounds(Unit::Bytes)?.1 <= self.max_cover_body_bytes_per_episode,
            "cover range exceeds episode budget"
        );
        if let Some(p) = &self.padding {
            p.validate()?;
        }
        Ok(())
    }
    pub fn validate_origin(&self, api: &str) -> Result<()> {
        let cover = reqwest::Url::parse(&self.url)?;
        let api = reqwest::Url::parse(api)?;
        ensure!(
            cover.origin() == api.origin(),
            "cover URL must have the API's exact HTTPS origin"
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod runtime_tests;

#[cfg(test)]
mod protocol_tests;

#[cfg(test)]
mod matrix_tests;

#[cfg(test)]
mod rejection_tests;

impl crate::catalog::Config {
    /// Resolve once after the API origin is known. Remote catalogs never supply
    /// this policy: only authored source settings enter this method.
    pub fn resolve_cover(&mut self, base: &str, process_enabled: bool) -> Result<()> {
        // Invalid authored settings remain errors even when cover is disabled.
        if let Some(config) = &self.cover_traffic {
            config.validate()?;
            config.validate_origin(base)?;
        }
        if !process_enabled || self.cover_traffic_enabled == Some(false) {
            self.cover_traffic = None;
            return Ok(());
        }
        if self.cover_traffic.is_none() {
            let origin = reqwest::Url::parse(base)?;
            let eligible = |value: &str| {
                reqwest::Url::parse(value)
                    .ok()
                    .filter(|u| {
                        u.scheme() == "https"
                            && u.origin() == origin.origin()
                            && u.username().is_empty()
                            && u.password().is_none()
                            && u.fragment().is_none()
                    })
                    .map(|u| u.to_string())
            };
            let catalog = eligible(&self.spec);
            let help = self.help_url.as_deref().and_then(eligible);
            if let Some(url) = catalog.clone().or_else(|| help.clone()) {
                let mut profile = Config::catalog_default(url.clone());
                profile.fallback_url = help.filter(|h| *h != url);
                self.cover_traffic = Some(profile);
            } else {
                tracing::warn!(code = "cover_no_same_origin_resource", source = ?self.prefix,
                    "Cover unavailable: configure a same-origin HTTPS catalog/help/cover URL or disable cover for this provider");
            }
        }
        Ok(())
    }
}

impl Config {
    /// Conservative range-only profile; the endpoint must still qualify at runtime.
    pub fn catalog_default(url: String) -> Self {
        let mut config: Self =
            toml::from_str(include_str!("default.toml")).expect("tested built-in cover profile");
        config.url = url;
        config
    }
}
