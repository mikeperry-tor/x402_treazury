//! Pure deployment resolution compared with unauthenticated saved public metadata.
use crate::{
    deployment::MetaConfig,
    network::NetworkConfig,
    payment::SpendPolicy,
    rotation::{
        assignment::{BootstrapPolicy, GeneratedWallet, WalletBindings, WalletSummary},
        config::{ReservesConfig, WalletConfig, retirement_remainder},
        store::{PoolStatus, Status},
    },
};
use alloy_primitives::U256;
use anyhow::{Context, Result};
use serde::Serialize;
use std::{collections::BTreeMap, fmt::Write, path::Path};

#[derive(Serialize)]
pub(super) struct Report {
    schema_version: u32,
    notes: [&'static str; 4],
    configure_managed_profiles: bool,
    requested_auto_fund: Option<bool>,
    bindings: WalletBindings,
    generated: BTreeMap<String, GeneratedWallet>,
    summary: WalletSummary,
    wallet_bootstrap: Option<BootstrapPolicy>,
    managed: BTreeMap<String, Managed>,
    omitted: BTreeMap<String, Omitted>,
    pub(super) startup_conflicts: Vec<String>,
    saved: Status,
}
#[derive(Serialize)]
struct Managed {
    action: &'static str,
    saved_id: Option<String>,
    requested_policy: WalletConfig,
    changed_saved_policy_fields: Vec<String>,
}
#[derive(Serialize)]
struct Omitted {
    saved_id: String,
    action: &'static str,
}

pub(super) fn report(config_path: &Path, policy: Option<&NetworkConfig>) -> Result<Report> {
    let text = std::fs::read_to_string(config_path).with_context(|| {
        format!(
            "failed to read deployment config '{}'",
            config_path.display()
        )
    })?;
    let config = MetaConfig::parse_with_network(&text, policy)?;
    let resolved = config.validate()?;
    let mut treasury = config
        .treasury
        .context("wallet preview requires [treasury] to locate initialized state")?;
    treasury.resolve(config_path);
    let saved = crate::rotation::store::preview_status(
        &treasury.state_dir,
        (!treasury.id.is_empty()).then_some(treasury.id.as_str()),
    )
    .with_context(|| {
        format!(
            "cannot inspect initialized treasury state at '{}'; use wallet init for a new treasury",
            treasury.state_dir.display()
        )
    })?;
    let configure_managed_profiles = resolved.wallets.values().any(WalletConfig::managed);
    let mut managed = BTreeMap::new();
    let mut startup_conflicts = Vec::new();
    for (name, requested) in resolved.wallets {
        let prior = saved.pools.iter().find(|pool| pool.name == name);
        if requested.managed() {
            let action = match prior {
                None => "new",
                Some(pool) if pool.enabled => "reuse",
                Some(_) => "reactivation",
            };
            let changed_saved_policy_fields = prior
                .map(|pool| changed_policy(pool, &requested))
                .transpose()?
                .unwrap_or_default();
            managed.insert(
                name,
                Managed {
                    action,
                    saved_id: prior.map(|pool| pool.id.clone()),
                    requested_policy: requested,
                    changed_saved_policy_fields,
                },
            );
        } else if prior.is_some() {
            startup_conflicts.push(format!(
                "managed pool {name} cannot become static without explicit retirement"
            ));
        }
    }
    let omitted = saved
        .pools
        .iter()
        .filter(|pool| !managed.contains_key(&pool.name))
        .map(|pool| {
            (
                pool.name.clone(),
                Omitted {
                    saved_id: pool.id.clone(),
                    action: if !configure_managed_profiles {
                        "retained_unchanged_static_only"
                    } else if pool.enabled {
                        "would_disable"
                    } else {
                        "retained_disabled"
                    },
                },
            )
        })
        .collect();
    Ok(Report {
        schema_version: 1,
        notes: [
            "Saved metadata is unauthenticated; balances are advisory observations, not fresh spending authority.",
            "Requested sizing is nominal, not accepted funding terms or guaranteed spendable capacity.",
            "Prior auto-funding and fixed/range sizing modes are not persisted and cannot be compared.",
            "Preview neither changes state nor authorizes later work; startup and rename recheck current state.",
        ],
        configure_managed_profiles,
        requested_auto_fund: config.funding.map(|funding| funding.auto_fund),
        bindings: resolved.bindings,
        generated: resolved.generated,
        summary: resolved.summary,
        wallet_bootstrap: resolved.bootstrap,
        managed,
        omitted,
        startup_conflicts,
        saved,
    })
}

fn changed_policy(pool: &PoolStatus, requested: &WalletConfig) -> Result<Vec<String>> {
    let WalletConfig::ZcashRotation {
        reserves,
        max_api_payment_usdc,
        max_retirement_remainder_usdc,
    } = requested
    else {
        return Ok(Vec::new());
    };
    let mut changed = Vec::new();
    let limit = SpendPolicy::dollars(max_api_payment_usdc)?
        .max_atomic
        .unwrap_or(U256::MAX)
        .to_string();
    if pool.payment_limit_atomic.as_ref() != Some(&limit) {
        changed.push("payment_limit_atomic".into());
    }
    let tolerance = retirement_remainder(max_retirement_remainder_usdc)?.to_string();
    if pool.retirement_tolerance_atomic.as_ref() != Some(&tolerance) {
        changed.push("retirement_tolerance_atomic".into());
    }
    changed_reserves(pool, reserves, &mut changed)?;
    Ok(changed)
}

fn changed_reserves(
    pool: &PoolStatus,
    requested: &ReservesConfig,
    changed: &mut Vec<String>,
) -> Result<()> {
    for (slot, requested) in [
        ("small", Some(&requested.small)),
        ("large", requested.large.as_ref()),
    ] {
        let saved = pool.reserve_claims.iter().find(|claim| claim.slot == slot);
        match requested {
            Some(reserve) => {
                let minimum = reserve.minimum_atomic()?.to_string();
                let maximum = reserve.maximum_atomic()?.to_string();
                if !saved.is_some_and(|claim| {
                    claim.enabled
                        && claim.target_atomic == minimum
                        && claim.maximum_output_atomic == maximum
                }) {
                    changed.push(format!("reserves.{slot}"));
                }
            }
            None if saved.is_some_and(|claim| claim.enabled) => {
                changed.push(format!("reserves.{slot}"))
            }
            None => {}
        }
    }
    Ok(())
}

impl Report {
    pub(super) fn human(&self) -> String {
        let mut text = format!(
            "Wallet configuration preview\nTreasury ID: {}\nSnapshot revision: {}\n",
            self.saved.treasury_id, self.saved.snapshot_revision
        );
        writeln!(
            text,
            "\nEventual nominal initial reserve sizing: {}–{} USDC across {} managed groups",
            self.summary.initial_inventory_target_usdc,
            self.summary.initial_inventory_maximum_usdc,
            self.summary.managed_pool_count
        )
        .unwrap();
        if let Some(setup) = &self.summary.bootstrap_setup {
            writeln!(
                text,
                "Explicit bootstrap setup: group {}, {}–{} USDC nominal; {}",
                setup.group,
                setup.initial_inventory_target_usdc,
                setup.initial_inventory_maximum_usdc,
                setup.routing
            )
            .unwrap();
        }
        writeln!(
            text,
            "Requested auto-funding: {}",
            self.requested_auto_fund
                .map_or("unconfigured", |enabled| if enabled {
                    "enabled"
                } else {
                    "disabled"
                })
        )
        .unwrap();
        text.push_str("Sizing is nominal, not accepted funding terms or spendable capacity. Saved balances are advisory observations; no fresh reads were made.\nPrior auto-funding and fixed/range sizing modes are not saved and cannot be compared.\n");
        for (name, group) in &self.managed {
            writeln!(
                text,
                "\nManaged {name}: {} (UUID {})",
                group.action,
                group.saved_id.as_deref().unwrap_or("not yet declared")
            )
            .unwrap();
            if let WalletConfig::ZcashRotation {
                reserves,
                max_api_payment_usdc,
                max_retirement_remainder_usdc,
            } = &group.requested_policy
            {
                writeln!(text, "  Requested payment limit: {max_api_payment_usdc} USDC; retirement remainder: {max_retirement_remainder_usdc} USDC").unwrap();
                writeln!(
                    text,
                    "  Requested reserves: {}",
                    serde_json::to_string(reserves).unwrap()
                )
                .unwrap();
            }
            if !group.changed_saved_policy_fields.is_empty() {
                writeln!(
                    text,
                    "  Saved policy changes: {}",
                    group.changed_saved_policy_fields.join(", ")
                )
                .unwrap();
            }
            if let Some(saved) = self.saved.pools.iter().find(|pool| pool.name == *name) {
                write_saved_policy(&mut text, saved);
            }
        }
        for (name, group) in &self.omitted {
            writeln!(
                text,
                "\nOmitted {name}: {} (UUID {})",
                group.action, group.saved_id
            )
            .unwrap();
        }
        if !self.configure_managed_profiles {
            text.push_str("\nStatic-only startup skips managed profile configuration; saved groups keep their enabled state. Direct-swap recovery may still open the treasury.\n");
        }
        for (server, sources) in &self.bindings {
            for (source, binding) in sources {
                writeln!(
                    text,
                    "Binding {server}/{source}: {} ({})",
                    binding.wallet, binding.origin
                )
                .unwrap();
            }
        }
        for conflict in &self.startup_conflicts {
            writeln!(text, "Startup conflict: {conflict}").unwrap();
        }
        text.push_str("\nOmitted groups retain holdings and history; preview transfers or deletes nothing.\nSaved inventory and pending work:\n");
        text.push_str(&super::output::status(&self.saved, None));
        text
    }
}

fn write_saved_policy(text: &mut String, saved: &PoolStatus) {
    writeln!(
        text,
        "  Saved payment limit: {} atomic USDC; retirement remainder: {} atomic USDC",
        saved
            .payment_limit_atomic
            .as_deref()
            .unwrap_or("unconfigured"),
        saved
            .retirement_tolerance_atomic
            .as_deref()
            .unwrap_or("unconfigured")
    )
    .unwrap();
    for claim in &saved.reserve_claims {
        writeln!(
            text,
            "  Saved {} reserve: enabled={}, nominal bounds {}–{} atomic USDC",
            claim.slot, claim.enabled, claim.target_atomic, claim.maximum_output_atomic
        )
        .unwrap();
    }
}

#[cfg(test)]
mod tests;
