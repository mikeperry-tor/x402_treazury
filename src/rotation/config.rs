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
fn no_remainder() -> String {
    "0".into()
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
    100
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
/// Funding preferences for one reserve slot; never payment authority.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReserveConfig {
    pub funding_amount_usdc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_funding_amount_usdc: Option<String>,
}
impl ReserveConfig {
    pub fn target_atomic(&self) -> Result<u64> {
        usdc_amount(&self.funding_amount_usdc).context("invalid reserve funding_amount_usdc")
    }
    pub fn maximum_atomic(&self) -> Result<u64> {
        self.max_funding_amount_usdc
            .as_deref()
            .map(usdc_amount)
            .transpose()
            .context("invalid reserve max_funding_amount_usdc")?
            .map_or_else(|| self.target_atomic(), Ok)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.maximum_atomic()? >= self.target_atomic()?,
            "reserve max_funding_amount_usdc is below funding_amount_usdc"
        );
        Ok(())
    }
}
/// Named sizing slots. Slot labels are not permanent wallet classifications.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReservesConfig {
    pub small: ReserveConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub large: Option<ReserveConfig>,
}
impl ReservesConfig {
    pub fn validate(&self) -> Result<()> {
        self.small.validate().context("invalid reserves.small")?;
        if let Some(large) = &self.large {
            large.validate().context("invalid reserves.large")?;
            ensure!(
                large.target_atomic()? >= self.small.target_atomic()?,
                "reserves.large funding_amount_usdc is below reserves.small"
            );
        }
        Ok(())
    }
    /// Configured reserve inventory before the first payment, not current usable capacity.
    pub fn initial_inventory_target_atomic(&self) -> Result<U256> {
        Ok(U256::from(self.small.target_atomic()?)
            + U256::from(
                self.large
                    .as_ref()
                    .map(ReserveConfig::target_atomic)
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

// Derive the schema once, while inspecting removed keys before typed parsing.
// Remote Self generates inherent serde helpers; these trait adapters retain the
// migration errors even when an obsolete setting has a malformed value.
macro_rules! impl_config_serde {
    ($config:ident) => {
        impl Serialize for $config {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                Self::serialize(self, serializer)
            }
        }
        impl<'de> Deserialize<'de> for $config {
            fn deserialize<D: serde::Deserializer<'de>>(
                deserializer: D,
            ) -> std::result::Result<Self, D::Error> {
                let value = serde_json::Value::deserialize(deserializer)?;
                reject_replaced_settings::<D::Error>(&value, stringify!($config))?;
                Self::deserialize(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(remote = "Self", tag = "mode", deny_unknown_fields)]
pub enum WalletConfig {
    #[serde(rename = "static")]
    Static {
        private_key_env: String,
        #[serde(default = "cap")]
        max_api_payment_usdc: String,
    },
    #[serde(rename = "zcash_rotation")]
    ZcashRotation {
        reserves: ReservesConfig,
        max_api_payment_usdc: String,
        #[serde(default = "no_remainder")]
        max_retirement_remainder_usdc: String,
        #[serde(default)]
        max_funding_spend_zec: Option<String>,
        max_conversion_overhead_percent: u32,
        /// Compatibility only: admission no longer has an elapsed-time deadline.
        #[serde(default, skip_serializing)]
        wait_seconds: Option<u64>,
        #[serde(default = "attempts")]
        max_attempts: u32,
    },
}
impl_config_serde!(WalletConfig);

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
                max_funding_spend_zec,
                max_conversion_overhead_percent,
                wait_seconds,
                max_attempts,
                ..
            } => {
                reserves.validate()?;
                retirement_remainder(max_retirement_remainder_usdc)
                    .context("invalid max_retirement_remainder_usdc")?;
                SpendPolicy::dollars(max_api_payment_usdc)
                    .context("invalid max_api_payment_usdc")?;
                if let Some(max) = max_funding_spend_zec {
                    zatoshis(max).context("invalid max_funding_spend_zec")?;
                }
                ensure!(
                    *max_conversion_overhead_percent <= 100,
                    "max_conversion_overhead_percent must be an integer from 0 to 100"
                );
                if wait_seconds.is_some() {
                    tracing::warn!(
                        setting = "wait_seconds",
                        "Obsolete wallet setting ignored; payment admission has no total timeout"
                    );
                }
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
        "funding_amount_usdc must be a positive USDC decimal"
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
#[serde(remote = "Self", deny_unknown_fields)]
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
impl_config_serde!(TreasuryConfig);

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
#[serde(remote = "Self", deny_unknown_fields)]
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
impl_config_serde!(FundingConfig);

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

fn reject_replaced_settings<E: serde::de::Error>(
    value: &serde_json::Value,
    scope: &str,
) -> std::result::Result<(), E> {
    let Some(fields) = value.as_object() else {
        return Ok(());
    };
    for name in [
        "daily_funding_limit_usdc",
        "total_funding_limit_usdc",
        "daily_treasury_spend_limit_zec",
    ] {
        if fields.contains_key(name) {
            return Err(E::custom(format!(
                "{name} was removed; remove this aggregate budget setting. Actual spendable treasury funds govern enabled funding; retain per-operation safeguards and separately authorized qualification limits"
            )));
        }
    }
    for name in ["max_sync_age_seconds", "base_max_block_age_seconds"] {
        if fields.contains_key(name) {
            return Err(E::custom(format!(
                "{name} was removed; remove this local balance-age setting. SDK input selection and protocol transaction validity govern spending"
            )));
        }
    }
    if scope == "WalletConfig"
        && fields.get("mode").and_then(serde_json::Value::as_str) == Some("zcash_rotation")
    {
        for name in ["funding_amount_usdc", "max_funding_amount_usdc"] {
            if fields.contains_key(name) {
                return Err(E::custom(format!(
                    "managed {name} moved to reserves.small (and optional reserves.large); configure explicit max_api_payment_usdc and migrate stopped wallet state separately"
                )));
            }
        }
        if fields.contains_key("limit_payments_to_funding_target") {
            return Err(E::custom(
                "limit_payments_to_funding_target was removed; choose explicit max_api_payment_usdc preserving your intended per-payment authority; reserve targets no longer cap payments",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod retired_age_tests {
    use super::*;
    #[test]
    fn removed_age_settings_fail_explicitly_before_value_parsing() {
        for value in [
            serde_json::json!(0),
            serde_json::json!(300),
            serde_json::json!("invalid"),
        ] {
            for key in ["max_sync_age_seconds", "base_max_block_age_seconds"] {
                let mut document = serde_json::json!({});
                document[key] = value.clone();
                let error = if key == "max_sync_age_seconds" {
                    serde_json::from_value::<TreasuryConfig>(document)
                        .err()
                        .unwrap()
                } else {
                    serde_json::from_value::<FundingConfig>(document)
                        .err()
                        .unwrap()
                };
                assert!(error.to_string().contains(&format!("{key} was removed")));
            }
        }
    }
}
