//! Deployment-only wallet and treasury settings. Validation never reads secrets.
use crate::payment::SpendPolicy;
use alloy_primitives::U256;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
pub const DEFAULT_BASE_RPC_URLS: [&str; 3] = [
    "https://base-rpc.publicnode.com",
    "https://base.drpc.org",
    "https://mainnet.base.org",
];
fn base_rpc_env() -> String {
    "BASE_RPC_URL".into()
}
fn cap() -> String {
    "1.00".into()
}
fn deposit() -> String {
    "2.00".into()
}
fn wait() -> u64 {
    30
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
#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "mode", deny_unknown_fields)]
pub enum WalletConfig {
    #[serde(rename = "static")]
    Static {
        private_key_env: String,
        #[serde(default = "cap")]
        max_price_usd: String,
    },
    #[serde(rename = "zcash_rotation")]
    ZcashRotation {
        #[serde(default = "deposit")]
        deposit_size: String,
        #[serde(default = "cap")]
        max_price_usd: String,
        max_input_zec: String,
        max_fee_bps: u32,
        #[serde(default = "wait")]
        wait_seconds: u64,
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
                max_price_usd,
            } => {
                env_name(private_key_env)?;
                SpendPolicy::dollars(max_price_usd).context("invalid max_price_usd")?;
            }
            Self::ZcashRotation {
                deposit_size,
                max_price_usd,
                max_input_zec,
                max_fee_bps,
                wait_seconds,
                max_attempts,
            } => {
                positive_usdc(deposit_size)?;
                SpendPolicy::dollars(max_price_usd).context("invalid max_price_usd")?;
                zatoshis(max_input_zec)?;
                ensure!(
                    *max_fee_bps <= 10000
                        && *wait_seconds > 0
                        && *wait_seconds <= 3600
                        && *max_attempts > 0,
                    "invalid managed wallet limits"
                );
            }
        }
        Ok(())
    }
}
pub fn positive_usdc(s: &str) -> Result<U256> {
    let n = SpendPolicy::dollars(s)?.max_atomic;
    ensure!(
        n.is_some_and(|n| n > U256::ZERO),
        "deposit_size must be a positive USDC decimal"
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
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TreasuryConfig {
    pub id: String,
    pub state_dir: PathBuf,
    pub key_file: PathBuf,
    pub indexer_url_env: String,
    pub submission_url_env: String,
    pub daily_input_zec: String,
    pub shield_max_fee_zec: String,
    #[serde(default = "confirmations")]
    pub confirmations: u64,
    #[serde(default = "sync_age")]
    pub max_sync_age_seconds: u64,
}
impl TreasuryConfig {
    pub fn validate(&self) -> Result<()> {
        uuid::Uuid::parse_str(&self.id)?;
        ensure!(
            !self.state_dir.as_os_str().is_empty()
                && !self.key_file.as_os_str().is_empty()
                && self.state_dir != self.key_file,
            "invalid state/key paths"
        );
        env_name(&self.indexer_url_env)?;
        env_name(&self.submission_url_env)?;
        zatoshis(&self.daily_input_zec)?;
        zatoshis(&self.shield_max_fee_zec)?;
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
        if self.key_file.is_relative() {
            self.key_file = base.join(&self.key_file);
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FundingConfig {
    /// Explicit opt-in: starts source deposits, not merely chain reconciliation.
    #[serde(default)]
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
    #[serde(default = "deadline")]
    pub swap_timeout_seconds: u64,
    #[serde(default = "deadline")]
    pub quote_deadline_seconds: u64,
}
impl FundingConfig {
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
                && self.quote_deadline_seconds >= 300,
            "invalid funding limits"
        );
        Ok(())
    }
}
