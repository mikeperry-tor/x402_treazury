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
fn deposit() -> String {
    "2.00".into()
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
fn sync_age() -> u64 {
    300
}
fn base_confirmations() -> u64 {
    12
}
fn block_age() -> u64 {
    120
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
#[derive(Clone, Deserialize, Serialize)]
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
        #[serde(default = "deposit")]
        funding_amount_usdc: String,
        #[serde(default = "cap")]
        max_api_payment_usdc: String,
        /// Compatibility default; false opts into the explicit API cap and available balance.
        #[serde(default = "enabled")]
        limit_payments_to_funding_target: bool,
        #[serde(default)]
        max_funding_amount_usdc: Option<String>,
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
                funding_amount_usdc,
                max_funding_amount_usdc,
                max_api_payment_usdc,
                max_funding_spend_zec,
                max_conversion_overhead_percent,
                wait_seconds,
                max_attempts,
                ..
            } => {
                let target =
                    usdc_amount(funding_amount_usdc).context("invalid funding_amount_usdc")?;
                if let Some(max) = max_funding_amount_usdc {
                    ensure!(
                        usdc_amount(max).context("invalid max_funding_amount_usdc")? >= target,
                        "max_funding_amount_usdc is below funding_amount_usdc"
                    );
                }
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
#[derive(Clone, Deserialize, Serialize)]
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
    #[serde(default)]
    pub daily_treasury_spend_limit_zec: Option<String>,
    #[serde(default = "network_fee_limit")]
    pub max_funding_transaction_fee_zec: String,
    #[serde(default = "network_fee_limit")]
    pub max_refund_shielding_fee_zec: String,
    #[serde(default = "confirmations")]
    pub confirmations: u64,
    #[serde(default = "sync_age")]
    pub max_sync_age_seconds: u64,
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
        Ok(super::store::status(&self.state_dir)?.treasury_id)
    }
    pub fn validate(&self) -> Result<()> {
        if !self.id.is_empty() {
            uuid::Uuid::parse_str(&self.id)?;
        }
        ensure!(
            !self.state_dir.as_os_str().is_empty() && self.state_dir != self.key_file,
            "invalid state/key paths"
        );
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
        if let Some(limit) = &self.daily_treasury_spend_limit_zec {
            zatoshis(limit).context("invalid daily_treasury_spend_limit_zec")?;
        }
        zatoshis(&self.max_funding_transaction_fee_zec)
            .context("invalid max_funding_transaction_fee_zec")?;
        zatoshis(&self.max_refund_shielding_fee_zec)
            .context("invalid max_refund_shielding_fee_zec")?;
        ensure!(
            self.confirmations > 0
                && self.confirmations <= u32::MAX as u64
                && self.max_sync_age_seconds > 0,
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
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FundingConfig {
    /// Gross USDC allocated across all pools. Omission leaves this budget uncapped.
    #[serde(default)]
    pub daily_funding_limit_usdc: Option<String>,
    #[serde(default)]
    pub total_funding_limit_usdc: Option<String>,
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
    #[serde(default = "block_age")]
    pub base_max_block_age_seconds: u64,
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
    pub fn allocation_limits(&self) -> Result<FundingBudgetLimits> {
        Ok(FundingBudgetLimits {
            daily: self
                .daily_funding_limit_usdc
                .as_deref()
                .map(usdc_amount)
                .transpose()
                .context("invalid daily_funding_limit_usdc")?,
            total: self
                .total_funding_limit_usdc
                .as_deref()
                .map(usdc_amount)
                .transpose()
                .context("invalid total_funding_limit_usdc")?,
        })
    }
    /// Runtime resolution shared by serving and qualification. Inspection never reads env.
    pub fn base_rpc_urls(
        &self,
        mut lookup: impl FnMut(&str) -> Option<String>,
    ) -> Result<Vec<String>> {
        self.validate()?;
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
    pub fn validate(&self) -> Result<()> {
        self.allocation_limits()?;
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
            self.base_confirmations > 0
                && self.base_max_block_age_seconds > 0
                && self.slippage_bps <= 1000
                && self.poll_seconds > 0
                && self.poll_seconds <= 60
                && self.swap_timeout_seconds > 0
                && self.quote_deadline_seconds >= super::transaction::MIN_QUOTE_VALIDITY_SECONDS,
            "invalid funding limits"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FundingBudgetLimits {
    pub daily: Option<u64>,
    pub total: Option<u64>,
}
