//! Explicit refusal of unsupported transparent-input shielding.
use super::*;
use crate::rotation::transaction::PreparedTransaction;

impl Treasury {
    /// Unsupported admission performs no wallet mutation, Store lookup or network I/O.
    pub async fn shield_refund(
        &mut self,
        _job: String,
        _max_fee: u64,
        _stop: &CancellationToken,
    ) -> Result<PreparedTransaction> {
        anyhow::bail!("Transparent-input shielding is unsupported; treasury payments use Ironwood")
    }
}
