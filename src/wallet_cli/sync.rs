//! Explicit one-pass discovery under the existing exclusive treasury owner.
use super::*;
use crate::{
    deployment::MetaConfig,
    rotation::{base::BaseRpc, config::TreasuryConfig, store::StoreHandle},
    treasury::{OpenedTreasury, Treasury},
};
use serde::Serialize;
use std::collections::BTreeSet;
use tokio_util::sync::CancellationToken;

type BalanceScope = BTreeSet<(String, String)>;

#[derive(Serialize)]
struct AssetObservation {
    status: &'static str,
    attempted: usize,
    completed: usize,
    failed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
}
impl AssetObservation {
    fn failed(error: &'static str) -> Self {
        Self {
            status: "failed",
            attempted: 0,
            completed: 0,
            failed: 0,
            error: Some(error),
        }
    }
}
#[derive(Serialize)]
struct Observations {
    zec: AssetObservation,
    usdc: AssetObservation,
}
impl Observations {
    fn complete(&self) -> bool {
        self.zec.status != "failed" && self.usdc.status != "failed"
    }
}

pub(super) async fn run(
    path: PathBuf,
    network: crate::rotation::store::TreasuryNetwork,
    supervised: bool,
    json: bool,
) -> Result<()> {
    let mut parent = crate::supervision::Parent::from_stdin(supervised)?;
    let config: MetaConfig = toml::from_str(&tokio::fs::read_to_string(&path).await?)?;
    anyhow::ensure!(config.version == 1, "unsupported deployment version");
    crate::network::install(config.network.clone())?;
    let mut settings = config.treasury.clone().context("missing [treasury]")?;
    settings.validate_shared()?;
    settings.resolve(&path);
    tracing::info!("wallet sync: loading configuration and opening encrypted wallet");
    // Open once, before either asset resolves its transport. No funding owner or
    // provider loader is started; both passes use the same exclusive Store.
    let shared = Treasury::open_shared_state(
        settings.state_dir.clone(),
        settings.key_file.clone(),
        settings.runtime_id()?,
        network,
    )
    .await?;
    let (mut treasury, store, worker) = match Treasury::reconstruct(shared).await? {
        OpenedTreasury::Ready(treasury) => {
            let store = treasury.store_handle();
            (Some(treasury), store, None)
        }
        OpenedTreasury::SnapshotUnavailable(shared) => {
            let (store, worker) = StoreHandle::spawn(*shared);
            (None, store, Some(worker))
        }
    };
    tracing::info!("wallet sync: wallet opened; starting synchronization");
    let stop = CancellationToken::new();
    let observed = {
        let work = async {
            let addresses = match treasury.as_ref() {
                Some(treasury) => treasury.addresses().await?,
                None => serde_json::json!([]),
            };
            let zec = async {
                match treasury.as_mut() {
                    Some(treasury) => observe_zec(treasury, &settings, &stop).await,
                    None => AssetObservation::failed("zec_snapshot_unavailable"),
                }
            };
            let (zec, usdc) = tokio::join!(zec, observe_usdc(&config, store.clone(), &stop),);
            let state = store.call(|store| store.status()).await?;
            Ok((Observations { zec, usdc }, state, addresses))
        };
        tokio::pin!(work);
        tokio::select! {
            biased;
            closed = parent.closed() => {
                eprintln!("treasury sync supervisor stopped: deliberately cancelling and checkpointing for wallet safety");
                stop.cancel();
                crate::server::drain_or_force("wallet_sync", work).await?;
                closed.and_then(|()| anyhow::bail!("treasury sync cancelled by supervisor"))
            }
            result = &mut work => result,
            signal = shutdown_signal() => {
                eprintln!("treasury sync interrupted: deliberately cancelling and checkpointing for wallet safety");
                stop.cancel();
                let result = crate::server::drain_or_force("wallet_command", work).await;
                signal?;
                result
            }
        }
    };
    drop(store);
    if !observed
        .as_ref()
        .is_err_and(|error| error.is::<crate::server::ForcedShutdown>())
    {
        if let Some(treasury) = treasury {
            treasury.close().await?;
        } else if let Some(worker) = worker {
            worker.await?;
        }
    }
    let (observed, state, addresses) = observed?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "state":state, "receive_addresses":addresses, "observations":observed,
            }))?
        );
    } else {
        println!(
            "Wallet synchronization: ZEC {}; managed USDC {} ({} attempted, {} completed, {} failed).\n\n{}\nPrimary ZEC address:\n{}",
            observed.zec.status,
            observed.usdc.status,
            observed.usdc.attempted,
            observed.usdc.completed,
            observed.usdc.failed,
            output::status(&state),
            addresses
                .get(0)
                .and_then(|address| address["encoded_address"].as_str())
                .unwrap_or("unavailable")
        );
    }
    anyhow::ensure!(
        observed.complete(),
        "wallet synchronization incomplete; see per-asset observations"
    );
    Ok(())
}

async fn observe_zec(
    treasury: &mut Treasury,
    settings: &TreasuryConfig,
    stop: &CancellationToken,
) -> AssetObservation {
    if settings.indexer_url.is_some() && !settings.indexer_url_env.is_empty() {
        return AssetObservation::failed("zec_configuration_unavailable");
    }
    let resolved = settings
        .indexer_endpoint(|name| std::env::var(name).ok())
        .and_then(|endpoint| crate::treasury::SyncSettings::new(endpoint, settings.confirmations));
    let sync = match resolved {
        Ok(sync) => sync,
        Err(_) => return AssetObservation::failed("zec_configuration_unavailable"),
    };
    treasury.configure_sync(sync);
    match treasury.sync_once(stop).await {
        Ok(()) => AssetObservation {
            status: "completed",
            attempted: 1,
            completed: 1,
            failed: 0,
            error: None,
        },
        Err(error) => {
            crate::treasury::diagnostics::warn_sync_failure(&error);
            AssetObservation {
                attempted: 1,
                failed: 1,
                ..AssetObservation::failed(crate::treasury::diagnostics::sync_failure(&error))
            }
        }
    }
}

/// Snapshot only existing rows; neither missing configured groups nor new source
/// registrations create allocations. Accepted destinations survive removal.
async fn scope(config: &MetaConfig, store: &StoreHandle) -> Result<(BalanceScope, usize)> {
    let names = crate::rotation::assignment::managed_names(config);
    store
        .call(move |s| {
            let mut wallets = BTreeSet::new();
            let mut malformed = 0;
            for pool in s.advisory_pools(&names)? {
                let through = s.wallet_boundary(&pool)?;
                let mut cursor = -1;
                loop {
                    let page = s.advisory_wallet_page(&pool, cursor, through)?;
                    if page.is_empty() {
                        break;
                    }
                    for (sequence, wallet) in page {
                        cursor = sequence;
                        match wallet {
                            Some(wallet) => {
                                wallets.insert((pool.clone(), wallet));
                            }
                            None => malformed += 1,
                        }
                    }
                }
            }
            let through = s.funding_work_boundary()?;
            let mut cursor = String::new();
            loop {
                let page = s.funding_work_page(&cursor, through)?;
                if page.is_empty() {
                    break;
                }
                for (id, work) in page {
                    cursor = id;
                    match work {
                        Ok(work) if work.facts.credit_needed() => {
                            wallets.insert((work.job.pool_id, work.job.wallet_id));
                        }
                        Ok(_) => {}
                        Err(_) => malformed += 1,
                    }
                }
            }
            Ok((wallets, malformed))
        })
        .await
}

async fn observe_usdc(
    config: &MetaConfig,
    store: StoreHandle,
    stop: &CancellationToken,
) -> AssetObservation {
    let (wallets, malformed) = match scope(config, &store).await {
        Ok(scope) => scope,
        Err(_) => return AssetObservation::failed("usdc_inventory_unavailable"),
    };
    let report = AssetObservation {
        status: "inapplicable",
        attempted: malformed,
        completed: 0,
        failed: malformed,
        error: None,
    };
    if wallets.is_empty() && malformed == 0 {
        return report;
    }
    let base = config
        .funding
        .as_ref()
        .context("missing Base settings")
        .and_then(|funding| {
            BaseRpc::with_fallbacks(
                &funding.base_rpc_urls(|name| std::env::var(name).ok())?,
                funding.base_confirmations,
            )
        });
    let base = match base {
        Ok(base) => base,
        Err(_) => return AssetObservation::failed("usdc_configuration_unavailable"),
    };
    refresh_usdc(store, wallets, report, base, stop).await
}

async fn refresh_usdc(
    store: StoreHandle,
    wallets: BalanceScope,
    mut report: AssetObservation,
    base: BaseRpc,
    stop: &CancellationToken,
) -> AssetObservation {
    for (pool, wallet) in wallets {
        if stop.is_cancelled() {
            report.error = Some("cancelled");
            break;
        }
        report.attempted += 1;
        match store.observe_balance(pool, wallet, base.clone()).await {
            Ok(_) => report.completed += 1,
            Err(_) => report.failed += 1,
        }
    }
    report.status = if report.failed > 0 || report.error.is_some() {
        "failed"
    } else {
        "completed"
    };
    report
}

#[cfg(test)]
mod tests;
