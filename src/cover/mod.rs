//! Optional, bounded provider cover traffic. Disabled operation owns no tasks or RNG.
pub mod budget;
pub mod episode;
pub mod range;
pub mod registry;
pub mod runtime;
pub mod sampling;
pub mod status;
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
    pub url: String,
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
impl Config {
    pub fn validate(&self) -> Result<()> {
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
