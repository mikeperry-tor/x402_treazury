//! Wallet administration. Only explicit reconcile --rebroadcast can submit saved bytes.
#[cfg(feature = "zcash")]
use anyhow::Context;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
pub struct WalletArgs {
    /// Deployment configuration; bootstrap also consumes wallet and funding settings.
    #[arg(long, alias = "meta-config", global = true, conflicts_with_all = ["network_config", "state_dir", "key_file", "treasury_id"])]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    network_config: Option<PathBuf>,
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    /// Defaults to wallet.key inside the state directory.
    #[arg(long, global = true)]
    key_file: Option<PathBuf>,
    /// Optional expected treasury UUID; normally read from existing state.
    #[arg(long, global = true)]
    treasury_id: Option<String>,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Fund initial managed wallet pairs and wait for confirmed USDC, without discovery.
    Bootstrap,
    /// Create a new treasury; discovers its birthday unless supplied.
    Init {
        #[arg(long)]
        birthday: Option<u32>,
        #[arg(long, conflicts_with = "birthday")]
        indexer_url_env: Option<String>,
        #[arg(long, requires = "birthday")]
        mnemonic_file: Option<PathBuf>,
    },
    /// Display saved receive addresses without deriving new ones.
    Addresses,
    /// Derive and save a new shielded receive address.
    Address,
    /// Read persisted status without unlocking or network access.
    Status,
    /// Make an offline backup, including the encryption key.
    Backup {
        #[arg(long)]
        destination: PathBuf,
    },
    /// Allocate two pool candidates without funding them.
    Pool {
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "2.00")]
        funding_amount_usdc: String,
    },
    /// Sync the configured treasury; never starts automatic funding.
    Sync {
        #[arg(long, hide = true)]
        qualification_parent_stdin: bool,
    },
    /// Observe an existing transaction, optionally resubmitting the same saved bytes.
    Reconcile {
        #[arg(long)]
        operation_id: String,
        #[arg(long)]
        rebroadcast: bool,
    },
    /// Classify existing funding jobs and perform one guarded recovery pass; never submits funds.
    Recover,
    /// Release an expired operation after proving canonical absence and unspent inputs.
    #[command(hide = true)]
    RecoverExpired {
        #[arg(long)]
        operation_id: String,
    },
    /// Prepare shielding; does not broadcast.
    ShieldRefunds {
        #[arg(long)]
        job_id: String,
    },
    /// Retry an unprepared funding job; refuses operations with signed bytes.
    #[command(hide = true)]
    RecoverUnprepared {
        #[arg(long)]
        job_id: String,
    },
}

fn resolve_args(args: WalletArgs) -> Result<Command> {
    use anyhow::{Context, ensure};
    let settings = if let Some(path) = &args.config {
        let config: crate::deployment::MetaConfig =
            toml::from_str(&std::fs::read_to_string(path)?)?;
        ensure!(config.version == 1, "unsupported deployment version");
        crate::network::install(config.network)?;
        let mut treasury = config.treasury.context("missing [treasury]")?;
        treasury.validate()?;
        treasury.resolve(path);
        Some(treasury)
    } else {
        if let Some(path) = &args.network_config {
            crate::network::install(crate::network::NetworkPolicy::load(path)?)?;
        }
        None
    };
    let configured_path = || {
        args.config
            .clone()
            .context("this wallet command requires --config FILE")
    };
    match args.command {
        Action::Bootstrap => {
            return Ok(Command::Bootstrap {
                meta_config: configured_path()?,
            });
        }
        Action::Sync {
            qualification_parent_stdin,
        } => {
            return Ok(Command::Sync {
                meta_config: configured_path()?,
                qualification_parent_stdin,
            });
        }
        Action::Reconcile {
            operation_id,
            rebroadcast,
        } => {
            return Ok(Command::Reconcile {
                meta_config: configured_path()?,
                operation_id,
                rebroadcast,
            });
        }
        Action::Recover => {
            return Ok(Command::Recover {
                meta_config: configured_path()?,
            });
        }
        Action::RecoverExpired { operation_id } => {
            return Ok(Command::RecoverExpired {
                meta_config: configured_path()?,
                operation_id,
            });
        }
        Action::ShieldRefunds { job_id } => {
            return Ok(Command::ShieldRefunds {
                meta_config: configured_path()?,
                job_id,
            });
        }
        _ => {}
    }
    let state_dir = settings
        .as_ref()
        .map(|t| t.state_dir.clone())
        .or(args.state_dir)
        .context("supply --config FILE or --state-dir PATH")?;
    let key_file = settings
        .as_ref()
        .map(|t| t.key_file.clone())
        .or(args.key_file)
        .unwrap_or_else(|| state_dir.join("wallet.key"));
    let expected = settings
        .as_ref()
        .map(|t| t.id.clone())
        .filter(|s| !s.is_empty())
        .or(args.treasury_id);
    if let Action::Init {
        birthday,
        indexer_url_env,
        mnemonic_file,
    } = args.command
    {
        ensure!(
            settings.is_none() || indexer_url_env.is_none(),
            "configure the indexer in the deployment file"
        );
        ensure!(
            expected.is_none(),
            "init creates a new treasury identity; omit the expected treasury ID"
        );
        let indexer_url =
            if birthday.is_some() {
                None
            } else if let Some(t) = &settings {
                Some(t.indexer_endpoint(|name| std::env::var(name).ok())?)
            } else if let Some(name) = indexer_url_env {
                Some(std::env::var(&name).with_context(|| {
                    format!("missing birthday indexer environment variable {name}")
                })?)
            } else {
                None
            };
        return Ok(Command::Init {
            state_dir,
            key_file,
            birthday,
            indexer_url,
            mnemonic_file,
        });
    }
    let treasury_id = if let Some(expected) = expected {
        let actual = crate::rotation::store::status(&state_dir)?.treasury_id;
        ensure!(
            expected == actual,
            "treasury ID does not match state (state identity mismatch); use the treasury_id from `wallet status` (it is a UUID, not an account number)"
        );
        expected
    } else if matches!(args.command, Action::Status | Action::Addresses) {
        String::new()
    } else {
        crate::rotation::store::status(&state_dir)?.treasury_id
    };
    Ok(match args.command {
        Action::Status => Command::Status { state_dir },
        Action::Addresses => Command::Addresses {
            state_dir,
            key_file,
            treasury_id: (!treasury_id.is_empty()).then_some(treasury_id),
        },
        Action::Address => Command::Address {
            state_dir,
            key_file,
            treasury_id,
        },
        Action::Backup { destination } => Command::Backup {
            state_dir,
            key_file,
            treasury_id,
            destination,
        },
        Action::Pool {
            name,
            funding_amount_usdc,
        } => Command::Pool {
            state_dir,
            key_file,
            treasury_id,
            name,
            funding_amount_usdc,
        },
        Action::RecoverUnprepared { job_id } => Command::RecoverUnprepared {
            state_dir,
            key_file,
            treasury_id,
            job_id,
        },
        _ => unreachable!(),
    })
}

// Keep wallet commands recognizable in reduced builds so they can report feature requirements.
#[cfg_attr(not(feature = "zcash"), allow(dead_code))]
enum Command {
    Recover {
        meta_config: PathBuf,
    },
    Bootstrap {
        meta_config: PathBuf,
    },
    /// Release an expired operation only after proving canonical absence and unspent inputs.
    RecoverExpired {
        meta_config: PathBuf,
        operation_id: String,
    },
    /// Prepare shielding for one refund address; use reconcile --rebroadcast to submit.
    ShieldRefunds {
        meta_config: PathBuf,
        job_id: String,
    },
    /// Retry an unprepared funding job; refuses every operation with signed bytes.
    RecoverUnprepared {
        state_dir: PathBuf,
        key_file: PathBuf,
        treasury_id: String,
        job_id: String,
    },
    /// Back up encrypted wallet, EVM keys, journals and encryption key offline.
    Backup {
        state_dir: PathBuf,
        key_file: PathBuf,
        treasury_id: String,
        /// New owner-only directory; existing destinations are never overwritten.
        destination: PathBuf,
    },
    /// Reconcile an existing durable transaction; never constructs a replacement.
    Reconcile {
        meta_config: PathBuf,
        operation_id: String,
        /// May submit the SAME saved bytes while quote deadline and expiry permit.
        rebroadcast: bool,
    },
    /// Sync an existing treasury using its deployment settings; no API specs are loaded.
    Sync {
        meta_config: PathBuf,
        /// Internal supervisor lifetime pipe; EOF cancels and checkpoints sync.
        qualification_parent_stdin: bool,
    },
    /// Read last persisted metadata without unlocking or network access.
    Status {
        state_dir: PathBuf,
    },
    /// Initialize an encrypted treasury; discover a new wallet birthday unless supplied.
    Init {
        state_dir: PathBuf,
        key_file: PathBuf,
        /// Required for imports; an explicit height keeps initialization offline.
        birthday: Option<u32>,
        /// Resolved birthday indexer URL; standalone initialization may use its default.
        indexer_url: Option<String>,
        /// Owner-only mnemonic file; omit to generate a new seed inside zingolib.
        mnemonic_file: Option<PathBuf>,
    },
    /// Display existing receive addresses without deriving new ones or contacting the network.
    Addresses {
        state_dir: PathBuf,
        key_file: PathBuf,
        /// Optional identity check; defaults to the treasury UUID stored in this state.
        treasury_id: Option<String>,
    },
    /// Derive and durably save a NEW shielded receive address, offline. Use addresses to display existing ones.
    Address {
        state_dir: PathBuf,
        key_file: PathBuf,
        treasury_id: String,
    },
    /// Allocate or resume a named pool's two candidates. Does not fund them.
    Pool {
        state_dir: PathBuf,
        key_file: PathBuf,
        treasury_id: String,
        name: String,
        funding_amount_usdc: String,
    },
}
pub async fn run(args: WalletArgs) -> Result<()> {
    run_args(args, crate::rotation::store::TreasuryNetwork::Mainnet).await
}
async fn run_args(
    args: WalletArgs,
    network: crate::rotation::store::TreasuryNetwork,
) -> Result<()> {
    #[cfg(not(feature = "zcash"))]
    let _ = network;
    #[cfg(not(feature = "zcash"))]
    anyhow::ensure!(
        matches!(
            &args.command,
            Action::Status | Action::Backup { .. } | Action::RecoverUnprepared { .. }
        ),
        "treasury commands require a build with --features zcash"
    );
    let command = resolve_args(args)?;
    #[cfg(feature = "zcash")]
    if let Command::Bootstrap { meta_config } = command {
        let summary = crate::deployment::bootstrap_wallets(&meta_config).await?;
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }
    #[cfg(feature = "zcash")]
    if let Command::Recover { meta_config } = command {
        return recover_funding(meta_config, network).await;
    }
    if let Command::Status { state_dir } = command {
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
    } = command
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
    } = command
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
    } = command
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
        let treasury = execute_wallet_command(command, network).await?;
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
async fn execute_wallet_command(
    command: Command,
    network: crate::rotation::store::TreasuryNetwork,
) -> Result<crate::treasury::Treasury> {
    use crate::treasury::Treasury;
    let treasury = match command {
        Command::RecoverExpired {
            meta_config,
            operation_id,
        } => recover_expired(meta_config, operation_id, network).await?,

        Command::ShieldRefunds {
            meta_config,
            job_id,
        } => shield_refunds(meta_config, job_id, network).await?,

        Command::Reconcile {
            meta_config,
            operation_id,
            rebroadcast,
        } => reconcile(meta_config, operation_id, rebroadcast, network).await?,

        Command::Sync {
            meta_config,
            qualification_parent_stdin,
        } => sync(meta_config, network, qualification_parent_stdin).await?,

        Command::Init {
            state_dir,
            key_file,
            birthday,
            indexer_url,
            mnemonic_file,
        } => init(state_dir, key_file, birthday, indexer_url, mnemonic_file).await?,

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
            funding_amount_usdc,
        } => {
            let treasury = Treasury::open(state_dir, key_file, treasury_id).await?;
            treasury.ensure_pool(name, funding_amount_usdc).await?;
            treasury
        }
        Command::Bootstrap { .. }
        | Command::Addresses { .. }
        | Command::Status { .. }
        | Command::Backup { .. }
        | Command::RecoverUnprepared { .. }
        | Command::Recover { .. } => {
            unreachable!()
        }
    };
    Ok(treasury)
}

#[cfg(feature = "zcash")]
async fn recover_funding(
    path: PathBuf,
    network: crate::rotation::store::TreasuryNetwork,
) -> Result<()> {
    let config: crate::deployment::MetaConfig =
        toml::from_str(&tokio::fs::read_to_string(&path).await?)?;
    let wallets = config.validate()?.wallets;
    let (mut treasury, settings) = configured(&path, network).await?;
    let indexer = settings.indexer_endpoint(|name| std::env::var(name).ok())?;
    let mut sender = crate::treasury::submission::GrpcSubmission::with_network(
        indexer.clone(),
        indexer,
        network,
    )?;
    let stop = tokio_util::sync::CancellationToken::new();
    let result = finish_on_shutdown(&stop, async {
        let status = treasury.status().await?;
        let mut reports = Vec::new();
        for job in status.funding_jobs.iter().filter(|job| crate::treasury::recovery::needs_recovery(job) || job.last_error.is_some()) {
            let outcome = match wallets.get(&job.pool_name) {
                Some(crate::rotation::config::WalletConfig::ZcashRotation { max_attempts, .. }) => {
                    match treasury.recover_funding_job(job.id.clone(), *max_attempts, &mut sender, &stop).await {
                        Ok(outcome) => serde_json::to_value(outcome)?,
                        Err(error) => serde_json::json!({"status":"check_failed", "reason":crate::treasury::diagnostics::sync_failure(&error)}),
                    }
                }
                _ => serde_json::json!({"status":"operator_required", "reason":"wallet_profile_missing"}),
            };
            reports.push(serde_json::json!({"job_id":job.id, "pool_name":job.pool_name, "previous_error":job.last_error, "outcome":outcome}));
            if stop.is_cancelled() { break; }
        }
        anyhow::ensure!(!stop.is_cancelled(), "wallet recovery interrupted; accepted recovery results retained");
        anyhow::Ok(reports)
    }).await;
    treasury.close().await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({"recovery":result?}))?
    );
    Ok(())
}

#[cfg(feature = "zcash")]
async fn recover_expired(
    meta_config: PathBuf,
    operation_id: String,
    network: crate::rotation::store::TreasuryNetwork,
) -> Result<crate::treasury::Treasury> {
    let (mut treasury, settings) = configured(&meta_config, network).await?;
    let indexer = settings.indexer_endpoint(|name| std::env::var(name).ok())?;
    let mut sender = crate::treasury::submission::GrpcSubmission::with_network(
        indexer.clone(),
        indexer,
        network,
    )?;
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
    Ok(treasury)
}

#[cfg(feature = "zcash")]
async fn shield_refunds(
    meta_config: PathBuf,
    job_id: String,
    network: crate::rotation::store::TreasuryNetwork,
) -> Result<crate::treasury::Treasury> {
    let (mut treasury, settings) = configured(&meta_config, network).await?;
    let stop = tokio_util::sync::CancellationToken::new();
    let result = finish_on_shutdown(
        &stop,
        treasury.shield_refund(
            job_id,
            settings
                .daily_treasury_spend_limit_zec
                .as_deref()
                .map(crate::rotation::config::zatoshis)
                .transpose()?
                .unwrap_or(i64::MAX) as u64,
            u64::try_from(crate::rotation::config::zatoshis(
                &settings.max_refund_shielding_fee_zec,
            )?)?,
            &stop,
        ),
    )
    .await;
    if let Err(error) = result {
        treasury.close().await?;
        return Err(error);
    }
    Ok(treasury)
}

#[cfg(feature = "zcash")]
async fn reconcile(
    meta_config: PathBuf,
    operation_id: String,
    rebroadcast: bool,
    network: crate::rotation::store::TreasuryNetwork,
) -> Result<crate::treasury::Treasury> {
    let (mut treasury, settings) = configured(&meta_config, network).await?;
    let indexer = settings.indexer_endpoint(|name| std::env::var(name).ok())?;
    // Read-only reconciliation needs no submission secret.
    let submission = if rebroadcast {
        settings.submission_endpoint(|name| std::env::var(name).ok())?
    } else {
        indexer.clone()
    };
    let mut sender =
        crate::treasury::submission::GrpcSubmission::with_network(submission, indexer, network)?;
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
    Ok(treasury)
}

#[cfg(feature = "zcash")]
async fn sync(
    meta_config: PathBuf,
    network: crate::rotation::store::TreasuryNetwork,
    supervised: bool,
) -> Result<crate::treasury::Treasury> {
    let mut parent = crate::supervision::Parent::from_stdin(supervised)?;
    let (mut treasury, _) = configured(&meta_config, network).await?;
    let stop = tokio_util::sync::CancellationToken::new();
    let result = {
        let work = treasury.sync_once(&stop);
        tokio::pin!(work);
        tokio::select! {
            biased;
            closed = parent.closed() => {
                eprintln!("treasury sync supervisor stopped: deliberately cancelling and checkpointing for wallet safety");
                stop.cancel();
                let _ = work.await;
                closed.and_then(|()| anyhow::bail!("treasury sync cancelled by supervisor"))
            }
            result = &mut work => result,
            signal = shutdown_signal() => {
                eprintln!("treasury sync interrupted: deliberately cancelling and checkpointing for wallet safety");
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
    if let Some(sync) = treasury.status().await?.sync
        && let (Some(height), Some(tip)) = (sync.height, sync.observed_tip_height)
        && tip > height
    {
        eprintln!(
            "treasury sync accepted with bounded tip lag: scanned={height}, observed_tip={tip}, lag={} blocks; background sync continues while serving",
            tip - height
        );
    }
    Ok(treasury)
}

#[cfg(feature = "zcash")]
async fn init(
    state_dir: PathBuf,
    key_file: PathBuf,
    birthday: Option<u32>,
    indexer_url: Option<String>,
    mnemonic_file: Option<PathBuf>,
) -> Result<crate::treasury::Treasury> {
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
            let endpoint = if let Some(endpoint) = indexer_url {
                endpoint
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
    if let Some(parent) = state_dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(parent)?;
    }
    crate::treasury::Treasury::create(state_dir, key_file, birthday, seed).await
}

#[cfg(feature = "zcash")]
pub(crate) async fn shutdown_signal() -> std::io::Result<()> {
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
    network: crate::rotation::store::TreasuryNetwork,
) -> Result<(
    crate::treasury::Treasury,
    crate::rotation::config::TreasuryConfig,
)> {
    let config: crate::deployment::MetaConfig =
        toml::from_str(&tokio::fs::read_to_string(path).await?)?;
    crate::network::install(config.network.clone())?;
    anyhow::ensure!(config.version == 1, "unsupported deployment version");
    let mut settings = config.treasury.context("missing [treasury]")?;
    settings.validate()?;
    settings.resolve(path);
    let endpoint = settings.indexer_endpoint(|name| std::env::var(name).ok())?;
    let sync = crate::treasury::SyncSettings::new(
        endpoint,
        settings.confirmations,
        settings.max_sync_age_seconds,
    )?;
    let mut treasury = crate::treasury::Treasury::open_with_network(
        settings.state_dir.clone(),
        settings.key_file.clone(),
        settings.runtime_id()?,
        network,
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

// The only non-mainnet CLI adapter exists in the unit-test executable. Enabling
// zcash-regtest on the shipped executable does not enable this entry point.
#[cfg(all(test, feature = "zcash-regtest"))]
#[tokio::test]
#[ignore = "subprocess helper invoked by treasury::regtest::recovery_cli_lifecycle"]
async fn regtest_command_child() {
    let arguments: Vec<String> = serde_json::from_str(
        &std::env::var("TREAZURY_TEST_WALLET_ARGS").expect("fixture arguments"),
    )
    .unwrap();
    let result = match WalletArgs::try_parse_from(arguments) {
        Ok(args) => run_args(args, crate::rotation::store::TreasuryNetwork::Regtest).await,
        Err(error) => Err(error.into()),
    };
    if let Err(error) = result {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
    std::process::exit(0);
}
