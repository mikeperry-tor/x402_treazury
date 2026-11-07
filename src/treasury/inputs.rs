//! Exact selected owned inputs, captured before calculation (dummy actions excluded).
use crate::rotation::store::preparation::{InputPool, OwnedInput};
use anyhow::{Result, ensure};
type ProportionalFeeProposal = zcash_client_backend::proposal::Proposal<
    zcash_primitives::transaction::fees::zip317::FeeRule,
    zingolib::wallet::output::OutputRef,
>;

pub(super) fn selected_inputs(proposal: &ProportionalFeeProposal) -> Result<Vec<OwnedInput>> {
    let mut result = Vec::new();
    for step in proposal.steps() {
        if let Some(shielded) = step.shielded_inputs() {
            for note in shielded.notes() {
                let id = note.internal_note_id();
                ensure!(
                    id.pool_type() == zcash_protocol::PoolType::IRONWOOD,
                    "new shielded source inputs must use Ironwood"
                );
                let pool = InputPool::Ironwood;
                result.push(OwnedInput {
                    pool,
                    txid: id.txid().to_string(),
                    output_index: id.output_index(),
                });
            }
        }
    }
    ensure!(!result.is_empty(), "selected input inventory empty");
    Ok(result)
}
