//! Required SDK trait adapters; Sapling proving is unsupported and has no parameters.
use sapling_crypto::{bundle::GrothProofBytes, circuit, keys, prover, value};
use std::convert::Infallible;

pub(super) static UNSUPPORTED_SAPLING: UnsupportedSapling = UnsupportedSapling;
pub(super) struct UnsupportedSapling;

impl prover::SpendProver for UnsupportedSapling {
    type Proof = Infallible;

    fn prepare_circuit(
        _proof_generation_key: sapling_crypto::ProofGenerationKey,
        _diversifier: sapling_crypto::Diversifier,
        _rseed: sapling_crypto::Rseed,
        _value: value::NoteValue,
        _alpha: jubjub::Fr,
        _rcv: value::ValueCommitTrapdoor,
        _anchor: jubjub::Fq,
        _merkle_path: sapling_crypto::MerklePath,
    ) -> Option<circuit::Spend> {
        None
    }

    fn create_proof<R: rand::Rng>(&self, _circuit: circuit::Spend, _rng: &mut R) -> Self::Proof {
        panic!("Sapling proving is unsupported")
    }

    fn encode_proof(proof: Self::Proof) -> GrothProofBytes {
        match proof {}
    }
}

impl prover::OutputProver for UnsupportedSapling {
    type Proof = Infallible;

    // The upstream output-prover API cannot return an error. Proposal policy
    // excludes Sapling before input acceptance; this is a fail-closed fallback.
    fn prepare_circuit(
        _esk: &keys::EphemeralSecretKey,
        _payment_address: sapling_crypto::PaymentAddress,
        _rcm: jubjub::Fr,
        _value: value::NoteValue,
        _rcv: value::ValueCommitTrapdoor,
    ) -> circuit::Output {
        panic!("Sapling proving is unsupported")
    }

    fn create_proof<R: rand::Rng>(&self, _circuit: circuit::Output, _rng: &mut R) -> Self::Proof {
        panic!("Sapling proving is unsupported")
    }

    fn encode_proof(proof: Self::Proof) -> GrothProofBytes {
        match proof {}
    }
}
