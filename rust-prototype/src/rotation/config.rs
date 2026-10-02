//! Deployment-only wallet and treasury settings. Validation never reads secrets.
use crate::payment::SpendPolicy;
use alloy_primitives::U256;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
fn cap() -> String {
    "1.00".into()
}
fn deposit() -> String {
    "5.00".into()
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
            self.confirmations > 0 && self.max_sync_age_seconds > 0,
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
    pub base_rpc_url_env: String,
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
    pub fn validate(&self) -> Result<()> {
        env_name(&self.base_rpc_url_env)?;
        if let Some(s) = &self.near_api_key_env {
            env_name(s)?;
        }
        ensure!(
            matches!(self.confidentiality.as_str(), "basic" | "advanced"),
            "confidentiality must be basic or advanced"
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
