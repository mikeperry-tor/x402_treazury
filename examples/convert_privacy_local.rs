//! One-time developer conversion entry for the inspected privacy-local schema12.
//! The known source fails the strict proof precondition. This utility deliberately
//! has no write path, key reader, proof override, network client or production CLI.
#[path = "convert_privacy_local/mod.rs"]
mod conversion;
use clap::Parser;
use std::path::PathBuf;
#[derive(Parser)]
#[command(
    about = "Check the strict stopped-service preconditions for the known schema12 conversion; the inspected source currently lacks required immutable destination support"
)]
struct Args {
    #[arg(long)]
    state_dir: PathBuf,
}
fn main() -> std::process::ExitCode {
    match conversion::inspect(&Args::parse().state_dir) {
        Ok(conversion::Outcome::AlreadyConverted) => {
            eprintln!("already_converted: schema16; no mutation or financial authority");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("convert-privacy-local: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
