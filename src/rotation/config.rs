//! Deployment-only wallet and treasury settings. Validation never reads secrets.
use crate::payment::SpendPolicy;
use alloy_primitives::U256;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
pub const DEFAULT_ZCASH_INDEXER: &str = "https://zec.rocks:443";
pub const DEFAULT_BASE_RPC_URLS: [&str; 3] = [
    "https://base-rpc.publicnode.com",
    "https://base.drpc.org",
    "https://mainnet.base.org",
];
fn base_rpc_env() -> String {
    "BASE_RPC_URL".into()
}
fn enabled() -> bool {
    true
}
fn cap() -> String {
    "1.00".into()
}
fn retirement_tolerance() -> String {
    "0.001".into()
}
fn network_fee_limit() -> String {
    "0.0003".into()
}
fn attempts() -> u32 {
    3
}
fn confirmations() -> u64 {
    3
}
fn base_confirmations() -> u64 {
    12
}
fn confidentiality() -> String {
    "basic".into()
}
fn slippage() -> u32 {
    500
}
fn poll() -> u64 {
    5
}
fn deadline() -> u64 {
    1800
}
fn quote_window() -> u64 {
    7200
}
const DEFAULT_SMALL_MINIMUM_USDC: &str = "2.00";
const DEFAULT_SMALL_MAXIMUM_USDC: &str = "5.00";
const DEFAULT_LARGE_MINIMUM_USDC: &str = "10.00";
const DEFAULT_LARGE_MAXIMUM_USDC: &str = "25.00";

#[derive(Clone, Copy)]
enum SizingSlot {
    Small,
    Large,
}

impl SizingSlot {
    fn defaults(self) -> ReserveConfig {
        let (minimum, maximum) = match self {
            Self::Small => (DEFAULT_SMALL_MINIMUM_USDC, DEFAULT_SMALL_MAXIMUM_USDC),
            Self::Large => (DEFAULT_LARGE_MINIMUM_USDC, DEFAULT_LARGE_MAXIMUM_USDC),
        };
        ReserveConfig {
            funding_amount_usdc: None,
            min_funding_amount_usdc: Some(minimum.into()),
            max_funding_amount_usdc: Some(maximum.into()),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Small => "reserves.small",
            Self::Large => "reserves.large",
        }
    }
}

/// Nominal sizing policy for one reserve slot. Bounds authorize sizing only;
/// they are never a minimum credited balance or a maximum received amount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizingPolicy {
    Fixed { target: u64, maximum: u64 },
    Range { minimum: u64, maximum: u64 },
}
impl SizingPolicy {
    pub fn minimum(self) -> u64 {
        match self {
            Self::Fixed { target, .. } => target,
            Self::Range { minimum, .. } => minimum,
        }
    }
    pub fn maximum(self) -> u64 {
        match self {
            Self::Fixed { maximum, .. } => maximum,
            Self::Range { maximum, .. } => maximum,
        }
    }
    /// First nominal choice for a fresh operation: the fixed target or one
    /// unbiased inclusive draw from the configured range.
    pub fn draw_initial(self, rng: &mut impl rand::Rng) -> u64 {
        match self {
            Self::Fixed { target, .. } => target,
            Self::Range { minimum, maximum } => inclusive_draw(minimum, maximum, rng),
        }
    }
    /// Draw again after a provider floor increase. Fixed mode keeps its existing
    /// deterministic bridge-minimum adjustment instead of becoming randomized.
    pub fn draw_from_floor(self, floor: u64, rng: &mut impl rand::Rng) -> u64 {
        match self {
            Self::Fixed { target, maximum } => target.max(floor).min(maximum),
            Self::Range { minimum, maximum } => {
                inclusive_draw(minimum.max(floor).min(maximum), maximum, rng)
            }
        }
    }
}
fn inclusive_draw(minimum: u64, maximum: u64, rng: &mut impl rand::Rng) -> u64 {
    debug_assert!(minimum <= maximum);
    if minimum >= maximum {
        maximum
    } else {
        rng.random_range(minimum..=maximum)
    }
}

/// Funding preferences for one reserve slot; never payment authority.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReserveConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding_amount_usdc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_funding_amount_usdc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_funding_amount_usdc: Option<String>,
}
impl ReserveConfig {
    fn resolved(self, slot: SizingSlot) -> Result<Self> {
        match (
            self.funding_amount_usdc.is_some(),
            self.min_funding_amount_usdc.is_some(),
            self.max_funding_amount_usdc.is_some(),
        ) {
            (false, false, false) => Ok(slot.defaults()),
            (true, false, _) => Ok(self),
            (false, true, true) => Ok(self),
            (true, true, _) => anyhow::bail!(
                "{} cannot combine funding_amount_usdc with min_funding_amount_usdc",
                slot.label()
            ),
            (false, true, false) => anyhow::bail!(
                "{} min_funding_amount_usdc requires max_funding_amount_usdc",
                slot.label()
            ),
            (false, false, true) => anyhow::bail!(
                "{} max_funding_amount_usdc requires min_funding_amount_usdc or funding_amount_usdc",
                slot.label()
            ),
        }
    }
    pub fn policy(&self) -> Result<SizingPolicy> {
        match (
            &self.funding_amount_usdc,
            &self.min_funding_amount_usdc,
            &self.max_funding_amount_usdc,
        ) {
            (Some(target), None, maximum) => {
                let target = usdc_amount(target).context("invalid reserve funding_amount_usdc")?;
                let maximum = maximum
                    .as_deref()
                    .map(usdc_amount)
                    .transpose()
                    .context("invalid reserve max_funding_amount_usdc")?
                    .unwrap_or(target);
                ensure!(
                    maximum >= target,
                    "reserve max_funding_amount_usdc is below funding_amount_usdc"
                );
                Ok(SizingPolicy::Fixed { target, maximum })
            }
            (None, Some(minimum), Some(maximum)) => {
                let minimum =
                    usdc_amount(minimum).context("invalid reserve min_funding_amount_usdc")?;
                let maximum =
                    usdc_amount(maximum).context("invalid reserve max_funding_amount_usdc")?;
                ensure!(
                    maximum >= minimum,
                    "reserve max_funding_amount_usdc is below min_funding_amount_usdc"
                );
                Ok(SizingPolicy::Range { minimum, maximum })
            }
            _ => anyhow::bail!(
                "reserve must configure funding_amount_usdc or both min_funding_amount_usdc and max_funding_amount_usdc"
            ),
        }
    }
    pub fn minimum_atomic(&self) -> Result<u64> {
        Ok(self.policy()?.minimum())
    }
    pub fn maximum_atomic(&self) -> Result<u64> {
        Ok(self.policy()?.maximum())
    }
    pub fn validate(&self) -> Result<()> {
        self.policy()?;
        Ok(())
    }
}
/// Raw shape used to resolve absent or empty tables to slot-specific defaults.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReservesShape {
    #[serde(default)]
    small: Option<ReserveConfig>,
    #[serde(default)]
    large: Option<ReserveConfig>,
}
/// Named sizing slots. Slot labels are not permanent wallet classifications.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ReservesConfig {
    pub small: ReserveConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub large: Option<ReserveConfig>,
}
impl<'de> Deserialize<'de> for ReservesConfig {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let raw = ReservesShape::deserialize(d)?;
        let small = raw
            .small
            .unwrap_or_else(|| SizingSlot::Small.defaults())
            .resolved(SizingSlot::Small)
            .map_err(serde::de::Error::custom)?;
        let large = raw
            .large
            .map(|config| config.resolved(SizingSlot::Large))
            .transpose()
            .map_err(serde::de::Error::custom)?;
        Ok(Self { small, large })
    }
}
impl Default for ReservesConfig {
    fn default() -> Self {
        Self {
            small: SizingSlot::Small.defaults(),
            large: None,
        }
    }
}
impl ReservesConfig {
    pub fn validate(&self) -> Result<()> {
        self.small.validate().context("invalid reserves.small")?;
        if let Some(large) = &self.large {
            large.validate().context("invalid reserves.large")?;
        }
        Ok(())
    }
    /// Configured reserve inventory before the first payment, not current usable capacity.
    /// A randomized range has separate nominal lower and upper totals.
    pub fn initial_inventory_minimum_atomic(&self) -> Result<U256> {
        Ok(U256::from(self.small.minimum_atomic()?)
            + U256::from(
                self.large
                    .as_ref()
                    .map(ReserveConfig::minimum_atomic)
                    .transpose()?
                    .unwrap_or(0),
            ))
    }
    pub fn initial_inventory_maximum_atomic(&self) -> Result<U256> {
        Ok(U256::from(self.small.maximum_atomic()?)
            + U256::from(
                self.large
                    .as_ref()
                    .map(ReserveConfig::maximum_atomic)
                    .transpose()?
                    .unwrap_or(0),
            ))
    }
}
/// Explicit finite nonnegative tolerance, including zero. Unlike API caps,
/// unlimited spellings never grant retirement authority.
pub fn retirement_remainder(s: &str) -> Result<u64> {
    let amount = SpendPolicy::dollars(s)?
        .max_atomic
        .context("max_retirement_remainder_usdc must be a finite nonnegative USDC decimal")?;
    ensure!(
        amount <= U256::from(i64::MAX as u64),
        "max_retirement_remainder_usdc exceeds accounting limit"
    );
    Ok(amount.to::<u64>())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "mode", deny_unknown_fields)]
pub enum WalletConfig {
    #[serde(rename = "static")]
    Static {
        private_key_env: String,
        #[serde(default = "cap")]
        max_api_payment_usdc: String,
    },
    #[serde(rename = "zcash_rotation")]
    ZcashRotation {
        #[serde(default)]
        reserves: ReservesConfig,
        max_api_payment_usdc: String,
        #[serde(default = "retirement_tolerance")]
        max_retirement_remainder_usdc: String,
        max_conversion_overhead_percent: u32,
        #[serde(default = "attempts")]
        max_attempts: u32,
    },
}

impl WalletConfig {
    pub fn managed(&self) -> bool {
        matches!(self, Self::ZcashRotation { .. })
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Static {
                private_key_env,
                max_api_payment_usdc,
            } => {
                env_name(private_key_env)?;
                SpendPolicy::dollars(max_api_payment_usdc)
                    .context("invalid max_api_payment_usdc")?;
            }
            Self::ZcashRotation {
                reserves,
                max_retirement_remainder_usdc,
                max_api_payment_usdc,
                max_conversion_overhead_percent,
                max_attempts,
            } => {
                reserves.validate()?;
                retirement_remainder(max_retirement_remainder_usdc)
                    .context("invalid max_retirement_remainder_usdc")?;
                SpendPolicy::dollars(max_api_payment_usdc)
                    .context("invalid max_api_payment_usdc")?;
                ensure!(
                    *max_conversion_overhead_percent <= 100,
                    "max_conversion_overhead_percent must be an integer from 0 to 100"
                );
                ensure!(*max_attempts > 0, "invalid managed wallet limits");
            }
        }
        Ok(())
    }
}
/// Bounded exact micro-USDC amounts, shared by allocation limits and accounting.
pub fn usdc_amount(s: &str) -> Result<u64> {
    let amount = positive_usdc(s)?;
    ensure!(
        amount <= U256::from(i64::MAX as u64),
        "USDC amount exceeds accounting limit"
    );
    Ok(amount.to::<u64>())
}
pub fn positive_usdc(s: &str) -> Result<U256> {
    let n = SpendPolicy::dollars(s)?.max_atomic;
    ensure!(
        n.is_some_and(|n| n > U256::ZERO),
        "reserve funding amount must be a positive USDC decimal"
    );
    Ok(n.unwrap())
}
pub fn zatoshis(s: &str) -> Result<i64> {
    let (whole, fraction) = s.split_once('.').unwrap_or((s, ""));
    ensure!(
        !whole.is_empty()
            && whole.bytes().all(|b| b.is_ascii_digit())
            && fraction.bytes().all(|b| b.is_ascii_digit())
            && fraction.len() <= 8,
        "invalid ZEC decimal"
    );
    let n: i64 = format!("{whole}{fraction:0<8}").parse()?;
    ensure!(
        n > 0 && n <= 2_100_000_000_000_000,
        "ZEC limit out of range"
    );
    Ok(n)
}
fn env_name(s: &str) -> Result<()> {
    ensure!(
        regex::Regex::new("^[A-Za-z_][A-Za-z0-9_]*$")?.is_match(s),
        "invalid environment reference"
    );
    Ok(())
}
fn nonempty_string<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<String, D::Error> {
    let value = String::deserialize(d)?;
    if value.trim().is_empty() {
        return Err(serde::de::Error::custom(
            "omit optional values instead of setting an empty string",
        ));
    }
    Ok(value)
}
fn empty_path(path: &Path) -> bool {
    path.as_os_str().is_empty()
}
fn nonempty_path<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<PathBuf, D::Error> {
    nonempty_string(d).map(PathBuf::from)
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreasuryConfig {
    /// Optional expected identity; empty until resolved from existing state at runtime.
    #[serde(
        default,
        deserialize_with = "nonempty_string",
        skip_serializing_if = "String::is_empty"
    )]
    pub id: String,
    pub state_dir: PathBuf,
    #[serde(
        default,
        deserialize_with = "nonempty_path",
        skip_serializing_if = "empty_path"
    )]
    pub key_file: PathBuf,
    #[serde(
        default,
        deserialize_with = "nonempty_string",
        skip_serializing_if = "String::is_empty"
    )]
    pub indexer_url_env: String,
    #[serde(
        default,
        deserialize_with = "nonempty_string",
        skip_serializing_if = "String::is_empty"
    )]
    pub submission_url_env: String,
    pub indexer_url: Option<String>,
    pub submission_url: Option<String>,
    #[serde(default = "network_fee_limit")]
    pub max_funding_transaction_fee_zec: String,
    #[serde(default = "network_fee_limit")]
    pub max_refund_shielding_fee_zec: String,
    #[serde(default = "confirmations")]
    pub confirmations: u64,
}

impl TreasuryConfig {
    pub fn indexer_endpoint(&self, lookup: impl Fn(&str) -> Option<String>) -> Result<String> {
        self.endpoint(self.indexer_url.as_deref(), &self.indexer_url_env, &lookup)
            .map(|v| v.unwrap_or_else(|| DEFAULT_ZCASH_INDEXER.into()))
    }
    pub fn submission_endpoint(&self, lookup: impl Fn(&str) -> Option<String>) -> Result<String> {
        match self.endpoint(
            self.submission_url.as_deref(),
            &self.submission_url_env,
            &lookup,
        )? {
            Some(value) => Ok(value),
            None => self.indexer_endpoint(lookup),
        }
    }
    fn endpoint(
        &self,
        url: Option<&str>,
        env: &str,
        lookup: &impl Fn(&str) -> Option<String>,
    ) -> Result<Option<String>> {
        let value = if !env.is_empty() {
            Some(
                lookup(env)
                    .filter(|v| !v.trim().is_empty())
                    .with_context(|| {
                        format!("required endpoint environment variable {env} is missing or empty")
                    })?,
            )
        } else {
            url.map(str::to_owned)
        };
        if let Some(value) = &value {
            super::base::secure_endpoint(value)?;
        }
        Ok(value)
    }
    /// State inspection is runtime-only. Opening the store still authenticates this ID.
    pub fn runtime_id(&self) -> Result<String> {
        if !self.id.is_empty() {
            return Ok(self.id.clone());
        }
        super::store::treasury_id(&self.state_dir)
    }
    pub(crate) fn validate_shared(&self) -> Result<()> {
        if !self.id.is_empty() {
            uuid::Uuid::parse_str(&self.id)?;
        }
        ensure!(
            !self.state_dir.as_os_str().is_empty() && self.state_dir != self.key_file,
            "invalid state/key paths"
        );
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.validate_shared()?;
        for (url, env) in [
            (&self.indexer_url, &self.indexer_url_env),
            (&self.submission_url, &self.submission_url_env),
        ] {
            ensure!(
                url.is_none() || env.is_empty(),
                "choose an endpoint URL or environment reference, not both"
            );
            if !env.is_empty() {
                env_name(env)?;
            }
            if let Some(url) = url {
                super::base::secure_endpoint(url)?;
            }
        }
        zatoshis(&self.max_funding_transaction_fee_zec)
            .context("invalid max_funding_transaction_fee_zec")?;
        zatoshis(&self.max_refund_shielding_fee_zec)
            .context("invalid max_refund_shielding_fee_zec")?;
        ensure!(
            self.confirmations > 0 && self.confirmations <= u32::MAX as u64,
            "invalid treasury limits"
        );
        Ok(())
    }
    pub fn resolve(&mut self, file: &Path) {
        let base = file.parent().unwrap_or(Path::new("."));
        if self.state_dir.is_relative() {
            self.state_dir = base.join(&self.state_dir);
        }
        if self.key_file.as_os_str().is_empty() {
            self.key_file = self.state_dir.join("wallet.key");
        } else if self.key_file.is_relative() {
            self.key_file = base.join(&self.key_file);
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FundingConfig {
    /// Managed serving funds bootstrap and replacement wallets unless disabled.
    #[serde(default = "enabled")]
    pub auto_fund: bool,
    pub near_user_session_env: Option<String>,
    #[serde(default = "base_rpc_env")]
    pub base_rpc_url_env: String,
    #[serde(default)]
    pub base_rpc_fallback_url_envs: Option<Vec<String>>,
    #[serde(default = "base_confirmations")]
    pub base_confirmations: u64,
    #[serde(default = "confidentiality")]
    pub confidentiality: String,
    pub near_api_key_env: Option<String>,
    #[serde(default = "slippage")]
    pub slippage_bps: u32,
    #[serde(default = "poll")]
    pub poll_seconds: u64,
    /// Health warning only: reconciliation continues with a 60-second polling floor.
    #[serde(default = "deadline")]
    pub swap_timeout_seconds: u64,
    #[serde(default = "quote_window")]
    pub quote_deadline_seconds: u64,
}

impl FundingConfig {
    /// Runtime resolution shared by serving and qualification. Inspection never reads env.
    pub fn base_rpc_urls(
        &self,
        mut lookup: impl FnMut(&str) -> Option<String>,
    ) -> Result<Vec<String>> {
        self.validate_base()?;
        let required = |name: &str, lookup: &mut dyn FnMut(&str) -> Option<String>| {
            lookup(name)
                .filter(|v| !v.trim().is_empty())
                .with_context(|| {
                    format!("required environment variable {name} is missing or empty")
                })
        };
        let primary = match lookup(&self.base_rpc_url_env) {
            Some(value) if !value.trim().is_empty() => value,
            None if self.base_rpc_url_env == "BASE_RPC_URL" => DEFAULT_BASE_RPC_URLS[0].into(),
            _ => anyhow::bail!("configured Base RPC environment variable is missing or empty"),
        };
        let mut urls = vec![primary];
        if let Some(names) = &self.base_rpc_fallback_url_envs {
            for name in names {
                urls.push(required(name, &mut lookup)?);
            }
        } else {
            let primary = super::base::secure_endpoint(&urls[0])?;
            for fallback in &DEFAULT_BASE_RPC_URLS[1..] {
                if super::base::secure_endpoint(fallback)? != primary {
                    urls.push((*fallback).into());
                }
            }
        }
        let mut unique = std::collections::BTreeSet::new();
        for url in &urls {
            ensure!(
                unique.insert(super::base::secure_endpoint(url)?.to_string()),
                "duplicate Base RPC endpoint"
            );
        }
        Ok(urls)
    }
    pub fn base_rpc_policy(&self) -> serde_json::Value {
        serde_json::json!({
            "primary_env":self.base_rpc_url_env,
            "primary_default":(self.base_rpc_url_env == "BASE_RPC_URL").then_some(DEFAULT_BASE_RPC_URLS[0]),
            "fallback_defaults":self.base_rpc_fallback_url_envs.is_none().then_some(&DEFAULT_BASE_RPC_URLS[1..]),
            "fallback_envs":self.base_rpc_fallback_url_envs,
        })
    }
    pub(crate) fn validate_base(&self) -> Result<()> {
        env_name(&self.base_rpc_url_env)?;
        ensure!(
            self.base_rpc_fallback_url_envs
                .as_ref()
                .is_none_or(|v| v.len() <= 2),
            "at most two Base RPC fallbacks; list is never truncated"
        );
        let mut names = std::collections::BTreeSet::from([&self.base_rpc_url_env]);
        for name in self.base_rpc_fallback_url_envs.iter().flatten() {
            env_name(name)?;
            ensure!(names.insert(name), "duplicate Base RPC environment name");
        }
        ensure!(self.base_confirmations > 0, "invalid funding limits");
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.validate_base()?;
        if let Some(s) = &self.near_user_session_env {
            env_name(s)?;
        }
        if let Some(s) = &self.near_api_key_env {
            env_name(s)?;
        }
        ensure!(
            matches!(
                self.confidentiality.as_str(),
                "public" | "basic" | "advanced"
            ),
            "confidentiality must be public, basic or advanced"
        );
        ensure!(
            self.slippage_bps <= 1000
                && self.poll_seconds > 0
                && self.poll_seconds <= 60
                && self.swap_timeout_seconds > 0
                && self.quote_deadline_seconds > 0,
            "invalid funding limits"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};

    #[test]
    fn range_draws_are_inclusive_and_fixed_sizing_never_randomizes() {
        let mut rng = StdRng::seed_from_u64(7);
        let range = SizingPolicy::Range {
            minimum: 1,
            maximum: 2,
        };
        let draws = (0..256)
            .map(|_| range.draw_initial(&mut rng))
            .collect::<Vec<_>>();
        assert!(draws.iter().all(|draw| (1..=2).contains(draw)));
        assert!(draws.contains(&1) && draws.contains(&2));
        assert_eq!(
            SizingPolicy::Range {
                minimum: 3,
                maximum: 3
            }
            .draw_initial(&mut rng),
            3
        );
        let fixed = SizingPolicy::Fixed {
            target: 2,
            maximum: 5,
        };
        assert_eq!(fixed.draw_initial(&mut rng), 2);
        assert_eq!(fixed.draw_from_floor(4, &mut rng), 4);
        assert_eq!(fixed.draw_from_floor(6, &mut rng), 5);
        assert_eq!(range.draw_from_floor(2, &mut rng), 2);
    }

    #[test]
    fn empty_or_absent_slots_resolve_their_own_defaults() {
        let empty: ReservesConfig = toml::from_str("").unwrap();
        assert_eq!(empty.small.minimum_atomic().unwrap(), 2_000_000);
        assert_eq!(empty.small.maximum_atomic().unwrap(), 5_000_000);
        assert!(empty.large.is_none());
        let large: ReservesConfig = toml::from_str("[large]").unwrap();
        assert_eq!(
            large.large.as_ref().unwrap().minimum_atomic().unwrap(),
            10_000_000
        );
        assert_eq!(
            large.large.as_ref().unwrap().maximum_atomic().unwrap(),
            25_000_000
        );
        let wallet: WalletConfig = toml::from_str(
            "mode='zcash_rotation'\nmax_api_payment_usdc='1'\nmax_conversion_overhead_percent=5",
        )
        .unwrap();
        let WalletConfig::ZcashRotation { reserves, .. } = wallet else {
            unreachable!()
        };
        reserves.validate().unwrap();
        assert_eq!(reserves.small.minimum_atomic().unwrap(), 2_000_000);
    }
}
