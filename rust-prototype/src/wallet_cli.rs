//! Local offline wallet commands. No command here submits or funds anything.
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
    #[cfg(not(feature = "zcash"))]
    {
        anyhow::bail!("offline treasury commands require a build with --features zcash")
    }
    #[cfg(feature = "zcash")]
    {
        use crate::treasury::Treasury;
        let treasury = match args.command {
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
            Command::Status { .. } => unreachable!(),
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
