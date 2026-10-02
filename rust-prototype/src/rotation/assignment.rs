//! Pure wallet assignment: inspectable without keys, state, catalogs or networking.
use super::config::{WalletConfig, positive_usdc};
use crate::deployment::MetaConfig;
use alloy_primitives::U256;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const AUTO_PREFIX: &str = "auto_v1_";
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Deployment,
    Server,
    Source,
    Binding,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    pub scope: Scope,
    pub template: String,
}
#[derive(Clone, Serialize)]
pub struct WalletBinding {
    pub wallet: String,
    pub origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
}
pub type WalletBindings = BTreeMap<String, BTreeMap<String, WalletBinding>>;
#[derive(Serialize)]
pub struct GeneratedWallet {
    pub template: String,
    pub scope: Scope,
}
#[derive(Serialize)]
pub struct WalletSummary {
    pub managed_pool_count: usize,
    pub generated_pool_count: usize,
    pub active_and_standby_target_atomic: String,
    pub active_and_standby_target_usdc: String,
}
pub struct Resolution {
    pub wallets: BTreeMap<String, WalletConfig>,
    pub bindings: WalletBindings,
    pub generated: BTreeMap<String, GeneratedWallet>,
    pub summary: WalletSummary,
}
impl Scope {
    fn identity(self, server: &str, source: &str) -> String {
        // Binding names use a length prefix, so (a_b,c) and (a,b_c) cannot collide.
        // Never include template contents, template name, file path or ordering.
        match self {
            Self::Deployment => format!("{AUTO_PREFIX}deployment"),
            Self::Server => format!("{AUTO_PREFIX}server_{server}"),
            Self::Source => format!("{AUTO_PREFIX}source_{source}"),
            Self::Binding => format!("{AUTO_PREFIX}binding_{}_{server}_{source}", server.len()),
        }
    }
}
pub fn resolve(config: &MetaConfig) -> Result<Resolution> {
    for name in config.wallets.keys() {
        ensure!(
            !name.starts_with(AUTO_PREFIX),
            "wallet name prefix {AUTO_PREFIX} is reserved for automatic pools"
        );
    }
    for (name, template) in &config.wallet_templates {
        ensure!(
            template.managed(),
            "wallet template {name}: only zcash_rotation is supported"
        );
        template
            .validate()
            .with_context(|| format!("wallet template {name}"))?;
    }
    if let Some(assignment) = &config.wallet_assignment {
        ensure!(
            config.wallet_templates.contains_key(&assignment.template),
            "unknown wallet template {}",
            assignment.template
        );
    }
    let mut wallets = config.wallets.clone();
    let mut generated = BTreeMap::new();
    let mut bindings = BTreeMap::new();
    for (server_name, server) in &config.servers {
        let mut server_bindings = BTreeMap::new();
        for source_name in &server.sources {
            let source = config.sources.get(source_name).context("unknown source")?;
            let explicit = source
                .wallet
                .as_ref()
                .map(|w| (w, format!("sources.{source_name}.wallet")))
                .or_else(|| {
                    server
                        .wallet
                        .as_ref()
                        .map(|w| (w, format!("servers.{server_name}.wallet")))
                });
            let binding = if let Some((wallet, origin)) = explicit {
                ensure!(
                    config.wallets.contains_key(wallet),
                    "unknown wallet {wallet}"
                );
                WalletBinding {
                    wallet: wallet.clone(),
                    origin,
                    template: None,
                    scope: None,
                }
            } else {
                let assignment = config.wallet_assignment.as_ref().with_context(||format!("server {server_name}, source {source_name}: wallet required on source or server, or configure wallet_assignment"))?;
                let wallet = assignment.scope.identity(server_name, source_name);
                wallets
                    .entry(wallet.clone())
                    .or_insert_with(|| config.wallet_templates[&assignment.template].clone());
                generated
                    .entry(wallet.clone())
                    .or_insert_with(|| GeneratedWallet {
                        template: assignment.template.clone(),
                        scope: assignment.scope,
                    });
                WalletBinding {
                    wallet,
                    origin: "wallet_assignment".into(),
                    template: Some(assignment.template.clone()),
                    scope: Some(assignment.scope),
                }
            };
            server_bindings.insert(source_name.clone(), binding);
        }
        bindings.insert(server_name.clone(), server_bindings);
    }
    let mut total = U256::ZERO;
    let mut count = 0;
    for wallet in wallets.values() {
        if let WalletConfig::ZcashRotation { deposit_size, .. } = wallet {
            let target = positive_usdc(deposit_size)?;
            total = total
                .checked_add(
                    target
                        .checked_mul(U256::from(2))
                        .context("combined funding target overflow")?,
                )
                .context("combined funding target overflow")?;
            count += 1;
        }
    }
    let scale = U256::from(1_000_000);
    let summary = WalletSummary {
        managed_pool_count: count,
        generated_pool_count: generated.len(),
        active_and_standby_target_atomic: total.to_string(),
        active_and_standby_target_usdc: format!(
            "{}.{:06}",
            total / scale,
            u64::try_from(total % scale)?
        ),
    };
    Ok(Resolution {
        wallets,
        bindings,
        generated,
        summary,
    })
}
