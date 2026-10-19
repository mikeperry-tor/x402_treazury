//! Compatibility boundary for the pinned x402-chain-eip155 2.0.2 client.
use alloy_signer_local::PrivateKeySigner;
use std::sync::Arc;
use x402_chain_eip155::V1Eip155ExactClient;
use x402_types::{
    proto::PaymentRequired,
    scheme::{
        X402SchemeId,
        client::{PaymentCandidate, X402SchemeClient},
    },
};

pub(crate) struct V1ExactClient(V1Eip155ExactClient<Arc<PrivateKeySigner>>);

impl V1ExactClient {
    pub(crate) fn new(signer: Arc<PrivateKeySigner>) -> Self {
        Self(V1Eip155ExactClient::new(signer))
    }
}

impl X402SchemeId for V1ExactClient {
    fn x402_version(&self) -> u8 {
        1
    }
    fn namespace(&self) -> &str {
        "eip155"
    }
    fn scheme(&self) -> &str {
        "exact"
    }
}

impl X402SchemeClient for V1ExactClient {
    fn accept(&self, required: &PaymentRequired) -> Vec<PaymentCandidate> {
        if !matches!(required, PaymentRequired::V1(_)) {
            return Vec::new();
        }
        // The SDK's v1 client inherits the trait default version (2), including
        // in its candidates. Correct metadata only for this v1-only delegate;
        // the SDK still owns signing, and managed payload validation is unchanged.
        let mut candidates = self.0.accept(required);
        for candidate in &mut candidates {
            candidate.x402_version = 1;
        }
        candidates
    }
}
