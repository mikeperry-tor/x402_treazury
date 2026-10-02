//! Wallet administration. Only explicit reconcile --rebroadcast can submit saved bytes.
#[cfg(feature = "zcash")]
use anyhow::Context;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
pub struct WalletArgs {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Back up encrypted wallet, EVM keys, journals and encryption key offline.
    Backup {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        treasury_id: String,
        /// New owner-only directory; existing destinations are never overwritten.
        #[arg(long)]
        destination: PathBuf,
    },
    /// Reconcile an existing durable transaction; never constructs a replacement.
    Reconcile {
        #[arg(long)]
        meta_config: PathBuf,
        #[arg(long)]
        operation_id: String,
        /// May submit the SAME saved bytes while quote deadline and expiry permit.
        #[arg(long)]
        rebroadcast: bool,
    },
    /// Sync an existing treasury using its deployment settings; no API specs are loaded.
    Sync {
        #[arg(long)]
        meta_config: PathBuf,
    },
    /// Read last persisted metadata without unlocking or network access.
    Status {
        #[arg(long)]
        state_dir: PathBuf,
    },
    /// Initialize an encrypted offline treasury. Never overwrites state.
    Init {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        birthday: u32,
        /// Owner-only mnemonic file; omit to generate a new seed inside zingolib.
        #[arg(long)]
        mnemonic_file: Option<PathBuf>,
    },
    /// Derive and durably save a shielded receive address, offline.
    Address {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        treasury_id: String,
    },
    /// Allocate or resume a named pool's two candidates. Does not fund them.
    Pool {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        treasury_id: String,
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "5.00")]
        deposit_size: String,
    },
}
pub async fn run() -> Result<()> {
    let args = WalletArgs::parse_from(
        std::iter::once("wallet".to_owned()).chain(std::env::args().skip(2)),
    );
    if let Command::Status { state_dir } = args.command {
        let status =
            tokio::task::spawn_blocking(move || crate::rotation::store::status(&state_dir))
                .await??;
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }
    if let Command::Backup {
        state_dir,
        key_file,
        treasury_id,
        destination,
    } = args.command
    {
        tokio::task::spawn_blocking(move || {
            let store = crate::rotation::store::Store::open(&state_dir, &key_file, &treasury_id)?;
            store.backup(&destination)
        })
        .await??;
        println!("{}", serde_json::json!({"backup_complete":true}));
        return Ok(());
    }
    #[cfg(not(feature = "zcash"))]
    {
        anyhow::bail!("treasury commands require a build with --features zcash")
    }
    #[cfg(feature = "zcash")]
    {
        use crate::treasury::Treasury;
        let treasury = match args.command {
            Command::Reconcile {
                meta_config,
                operation_id,
                rebroadcast,
            } => {
                let (mut treasury, settings) = configured(&meta_config).await?;
                let indexer = std::env::var(&settings.indexer_url_env)
                    .context("missing indexer endpoint environment variable")?;
                // Read-only reconciliation needs no submission secret.
                let submission = if rebroadcast {
                    std::env::var(&settings.submission_url_env)
                        .context("missing submission endpoint environment variable")?
                } else {
                    indexer.clone()
                };
                let mut sender =
                    crate::treasury::submission::GrpcSubmission::new(submission, indexer)?;
                let stop = tokio_util::sync::CancellationToken::new();
                let result = {
                    let work = async {
                        let presence = treasury
                            .reconcile_prepared(operation_id.clone(), &mut sender, &stop)
                            .await?;
                        if rebroadcast
                            && !matches!(
                                presence,
                                crate::rotation::transaction::TransactionPresence::Confirmed { .. }
                            )
                        {
                            treasury
                                .submit_prepared(operation_id, &mut sender, true, &stop)
                                .await?;
                        }
                        anyhow::Ok(())
                    };
                    tokio::pin!(work);
                    tokio::select! {
                        result = &mut work => result,
                        signal = shutdown_signal() => {
                            stop.cancel();
                            let result = work.await;
                            signal?;
                            result
                        }
                    }
                };
                if let Err(error) = result {
                    treasury.close().await?;
                    return Err(error);
                }
                treasury
            }
            Command::Sync { meta_config } => {
                let (mut treasury, _) = configured(&meta_config).await?;
                let stop = tokio_util::sync::CancellationToken::new();
                let result = {
                    let work = treasury.sync_once(&stop);
                    tokio::pin!(work);
                    tokio::select! {
                        result = &mut work => result,
                        signal = shutdown_signal() => {
                            stop.cancel();
                            let result = work.await;
                            signal?;
                            result
                        }
                    }
                };
                if let Err(error) = result {
                    treasury.close().await?;
                    return Err(error);
                }
                treasury
            }
            Command::Init {
                state_dir,
                key_file,
                birthday,
                mnemonic_file,
            } => {
                let seed = if let Some(path) = mnemonic_file {
                    let metadata = std::fs::symlink_metadata(&path)?;
                    anyhow::ensure!(
                        metadata.file_type().is_file(),
                        "mnemonic file must be a regular file"
                    );
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        anyhow::ensure!(
                            metadata.permissions().mode() & 0o077 == 0,
                            "mnemonic file must be owner-only"
                        );
                    }
                    Some(zeroize::Zeroizing::new(
                        std::fs::read_to_string(path).context("reading mnemonic file")?,
                    ))
                } else {
                    None
                };
                Treasury::create(state_dir, key_file, birthday, seed).await?
            }
            Command::Address {
                state_dir,
                key_file,
                treasury_id,
            } => {
                let mut treasury = Treasury::open(state_dir, key_file, treasury_id).await?;
                treasury.derive_address().await?;
                treasury
            }
            Command::Pool {
                state_dir,
                key_file,
                treasury_id,
                name,
                deposit_size,
            } => {
                let treasury = Treasury::open(state_dir, key_file, treasury_id).await?;
                treasury.ensure_pool(name, deposit_size).await?;
                treasury
            }
            Command::Status { .. } | Command::Backup { .. } => unreachable!(),
        };
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"state":treasury.status().await?,"receive_addresses":treasury.addresses().await?})
            )?
        );
        treasury.close().await
    }
}

#[cfg(feature = "zcash")]
async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! { result = tokio::signal::ctrl_c() => result?, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[cfg(feature = "zcash")]
async fn configured(
    path: &std::path::Path,
) -> Result<(
    crate::treasury::Treasury,
    crate::rotation::config::TreasuryConfig,
)> {
    let config: crate::deployment::MetaConfig =
        toml::from_str(&tokio::fs::read_to_string(path).await?)?;
    anyhow::ensure!(config.version == 1, "unsupported deployment version");
    let mut settings = config.treasury.context("missing [treasury]")?;
    settings.validate()?;
    settings.resolve(path);
    let endpoint = std::env::var(&settings.indexer_url_env)
        .context("missing indexer endpoint environment variable")?;
    let sync = crate::treasury::SyncSettings::new(
        endpoint,
        settings.confirmations,
        settings.max_sync_age_seconds,
    )?;
    let mut treasury = crate::treasury::Treasury::open(
        settings.state_dir.clone(),
        settings.key_file.clone(),
        settings.id.clone(),
    )
    .await?;
    treasury.configure_sync(sync);
    Ok((treasury, settings))
}
