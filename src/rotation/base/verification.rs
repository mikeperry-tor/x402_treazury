//! Opaque runtime verification values. Only completed Base acquisition constructs proofs.
use super::{Anchor, AuthorizationOutcome, PendingAuthorization, evidence::BalanceObservation};
use std::collections::BTreeMap;

#[derive(Clone, PartialEq, Eq)]
pub struct EvidenceCapture {
    pub(crate) namespace: uuid::Uuid,
    pub(crate) wallet: String,
    pub(crate) generation: u64,
    pub(crate) endpoint_generations: BTreeMap<String, u64>,
    pub(crate) checkpoint: Option<Anchor>,
    pub(crate) previous: Option<BalanceObservation>,
    pub(crate) previous_confirmed_covered: bool,
    /// Fixed per-wallet durable readiness dependencies, independent of cache retention.
    pub(crate) durable_balance_anchors: Vec<Anchor>,
}
#[derive(Clone)]
pub struct BalanceCertificate {
    capture: EvidenceCapture,
    observation: BalanceObservation,
    verified_resolution_anchor: Anchor,
    endpoint_generation: u64,
}
#[derive(Clone)]
pub struct ResolutionCertificate {
    capture: EvidenceCapture,
    endpoint: String,
    endpoint_generation: u64,
    authorization: PendingAuthorization,
    anchor: Anchor,
    outcome: AuthorizationOutcome,
    block_time: u64,
}

/// Equal heights alone do not establish lineage. Coverage of an older anchor is
/// available only because this private proof verified it with its capture.
fn compatible(captured: Option<&Anchor>, proof: &Anchor, current: Option<&Anchor>) -> bool {
    captured == current
        || current == Some(proof)
        || captured.is_some_and(|checkpoint| proof.height <= checkpoint.height)
}
impl BalanceCertificate {
    pub(super) fn verified(
        capture: EvidenceCapture,
        observation: BalanceObservation,
        verified_resolution_anchor: Anchor,
    ) -> Self {
        let endpoint_generation = capture
            .endpoint_generations
            .get(&observation.endpoint)
            .copied()
            .unwrap_or(0);
        Self {
            capture,
            observation,
            verified_resolution_anchor,
            endpoint_generation,
        }
    }
    pub(crate) fn capture(&self) -> &EvidenceCapture {
        &self.capture
    }
    pub(crate) fn observation(&self) -> &BalanceObservation {
        &self.observation
    }
    pub(crate) fn endpoint_generation(&self) -> u64 {
        self.endpoint_generation
    }
    /// Reducing validity cannot create authority; equivalent state cannot renew age.
    pub(crate) fn restrict_validity(&mut self, valid_until: u64) {
        self.observation.valid_until = self.observation.valid_until.min(valid_until);
    }
    pub(crate) fn compatible(&self, checkpoint: Option<&Anchor>) -> bool {
        checkpoint == Some(&self.verified_resolution_anchor)
            || compatible(
                self.capture.checkpoint.as_ref(),
                &self.observation.confirmed.anchor,
                checkpoint,
            )
    }
}
impl ResolutionCertificate {
    pub(super) fn verified(
        capture: EvidenceCapture,
        endpoint: String,
        authorization: PendingAuthorization,
        anchor: Anchor,
        outcome: AuthorizationOutcome,
        block_time: u64,
    ) -> Self {
        let endpoint_generation = capture
            .endpoint_generations
            .get(&endpoint)
            .copied()
            .unwrap_or(0);
        Self {
            capture,
            endpoint,
            endpoint_generation,
            authorization,
            anchor,
            outcome,
            block_time,
        }
    }
    pub(crate) fn capture(&self) -> &EvidenceCapture {
        &self.capture
    }
    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub(crate) fn endpoint_generation(&self) -> u64 {
        self.endpoint_generation
    }
    pub(crate) fn authorization(&self) -> &PendingAuthorization {
        &self.authorization
    }
    pub(crate) fn anchor(&self) -> &Anchor {
        &self.anchor
    }
    pub(crate) fn outcome(&self) -> &AuthorizationOutcome {
        &self.outcome
    }
    pub(crate) fn block_time(&self) -> u64 {
        self.block_time
    }
    pub(crate) fn compatible(&self, checkpoint: Option<&Anchor>) -> bool {
        compatible(self.capture.checkpoint.as_ref(), &self.anchor, checkpoint)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct EvidenceConflict {
    pub wallet: String,
    pub endpoint: String,
    pub anchor: Anchor,
    pub latest_only: bool,
    pub financial_checkpoint: bool,
}
impl std::fmt::Display for EvidenceConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.latest_only {
            "latest_anchor_replacement"
        } else {
            "confirmed_anchor_replacement"
        })
    }
}
impl std::error::Error for EvidenceConflict {}

#[cfg(test)]
mod tests {
    use super::*;
    fn anchor(height: u64) -> Anchor {
        Anchor {
            height,
            hash: format!("0x{height:064x}"),
        }
    }
    #[test]
    fn common_older_checkpoint_does_not_cover_two_newer_anchors() {
        let old = anchor(10);
        let proof = anchor(12);
        let current = anchor(11);
        assert!(!compatible(Some(&old), &proof, Some(&current)));
        assert!(compatible(Some(&old), &proof, Some(&old)));
        assert!(compatible(Some(&current), &old, Some(&proof)));
        assert!(compatible(Some(&old), &proof, Some(&proof)));
        assert!(compatible(None, &proof, None));
        assert!(!compatible(None, &proof, Some(&old)));
    }
}

#[derive(Clone, Debug)]
pub(crate) struct EndpointMismatch {
    pub endpoint: String,
}
impl std::fmt::Display for EndpointMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("wrong Base chain")
    }
}
impl std::error::Error for EndpointMismatch {}
