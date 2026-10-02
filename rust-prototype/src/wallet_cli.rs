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
    /// Release an expired operation only after proving canonical absence and unspent inputs.
    RecoverExpired {
        #[arg(long)]
        meta_config: PathBuf,
        #[arg(long)]
        operation_id: String,
    },
    /// Prepare shielding for one refund address; use reconcile --rebroadcast to submit.
    ShieldRefunds {
        #[arg(long)]
        meta_config: PathBuf,
        #[arg(long)]
        job_id: String,
    },
    /// Retry an unprepared funding job; refuses every operation with signed bytes.
    RecoverUnprepared {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        treasury_id: String,
        #[arg(long)]
        job_id: String,
    },
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
    /// Initialize an encrypted treasury; discover a new wallet birthday unless supplied.
    Init {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        key_file: PathBuf,
        /// Required for imports; an explicit height keeps initialization offline.
        #[arg(long)]
        birthday: Option<u32>,
        /// Environment variable holding the birthday indexer URL. Defaults to
        /// ZCASH_INDEXER_URL when set, otherwise https://zec.rocks:443.
        #[arg(long, conflicts_with = "birthday")]
        indexer_url_env: Option<String>,
        /// Owner-only mnemonic file; omit to generate a new seed inside zingolib.
        #[arg(long, requires = "birthday")]
        mnemonic_file: Option<PathBuf>,
    },
    /// Display existing receive addresses without deriving new ones or contacting the network.
    Addresses {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        key_file: PathBuf,
        /// Optional identity check; defaults to the treasury UUID stored in this state.
        #[arg(long)]
        treasury_id: Option<String>,
    },
    /// Derive and durably save a NEW shielded receive address, offline. Use addresses to display existing ones.
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
    #[cfg(feature = "zcash")]
    if let Command::Addresses {
        state_dir,
        key_file,
        treasury_id,
    } = args.command
    {
        let output =
            crate::treasury::Treasury::inspect_addresses(state_dir, key_file, treasury_id).await?;
        println!("{}", serde_json::to_string_pretty(&output)?);
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
    if let Command::RecoverUnprepared {
        state_dir,
        key_file,
        treasury_id,
        job_id,
    } = args.command
    {
        tokio::task::spawn_blocking(move || {
            let mut store =
                crate::rotation::store::Store::open(&state_dir, &key_file, &treasury_id)?;
            store.recover_unprepared_funding(&job_id)
        })
        .await??;
        println!("{}", serde_json::json!({"funding_job_reset":true}));
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
            Command::RecoverExpired {
                meta_config,
                operation_id,
            } => {
                let (mut treasury, settings) = configured(&meta_config).await?;
                let indexer =
                    std::env::var(&settings.indexer_url_env).context("missing indexer endpoint")?;
                let mut sender =
                    crate::treasury::submission::GrpcSubmission::new(indexer.clone(), indexer)?;
                let stop = tokio_util::sync::CancellationToken::new();
                let result = finish_on_shutdown(
                    &stop,
                    treasury.recover_expired(operation_id, &mut sender, &stop),
                )
                .await;
                if let Err(error) = result {
                    treasury.close().await?;
                    return Err(error);
                }
                treasury
            }
            Command::ShieldRefunds {
                meta_config,
                job_id,
            } => {
                let (mut treasury, settings) = configured(&meta_config).await?;
                let stop = tokio_util::sync::CancellationToken::new();
                let result = finish_on_shutdown(
                    &stop,
                    treasury.shield_refund(
                        job_id,
                        u64::try_from(crate::rotation::config::zatoshis(
                            &settings.daily_input_zec,
                        )?)?,
                        u64::try_from(crate::rotation::config::zatoshis(
                            &settings.shield_max_fee_zec,
                        )?)?,
                        &stop,
                    ),
                )
                .await;
                if let Err(error) = result {
                    treasury.close().await?;
                    return Err(error);
                }
                treasury
            }
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
                indexer_url_env,
                mnemonic_file,
            } => {
                anyhow::ensure!(
                    !state_dir.exists() && !key_file.exists(),
                    "treasury state or key already exists; init only creates a new wallet. Use `wallet addresses --state-dir PATH --key-file PATH` to display existing receive addresses, or `wallet status --state-dir PATH` for its treasury ID"
                );
                // Imports must never infer a recent birthday and skip historical funds.
                anyhow::ensure!(
                    mnemonic_file.is_none() || birthday.is_some(),
                    "seed import requires --birthday"
                );
                let birthday = match birthday {
                    Some(height) => height,
                    None => {
                        let endpoint = if let Some(name) = indexer_url_env {
                            std::env::var(name)
                                .context("missing birthday indexer environment variable")?
                        } else {
                            match std::env::var("ZCASH_INDEXER_URL") {
                                Ok(endpoint) => endpoint,
                                Err(std::env::VarError::NotPresent) => {
                                    crate::treasury::birthday::DEFAULT_INDEXER.into()
                                }
                                Err(_) => {
                                    anyhow::bail!("invalid birthday indexer environment variable")
                                }
                            }
                        };
                        crate::treasury::birthday::discover(&endpoint).await?
                    }
                };
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
            Command::Addresses { .. }
            | Command::Status { .. }
            | Command::Backup { .. }
            | Command::RecoverUnprepared { .. } => {
                unreachable!()
            }
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

#[cfg(feature = "zcash")]
async fn finish_on_shutdown<T>(
    stop: &tokio_util::sync::CancellationToken,
    work: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::pin!(work);
    tokio::select! {
        result = &mut work => result,
        signal = shutdown_signal() => { stop.cancel(); let result = work.await; signal?; result }
    }
}
