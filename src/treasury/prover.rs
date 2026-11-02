//! Sapling parameters are parsed only when a Sapling proof is requested.
//!
//! Adapted from zingolib `src/wallet/utils.rs` at
//! bfb0a2156819ea7f75a9b09b1724abfd06c8925e, licensed under MIT.
// Copyright (c) 2022-present Zingolabs
// SPDX-License-Identifier: MIT
// See vendor/ZINGO-LICENSE-MIT for the full upstream license.

#[cfg(all(test, feature = "zcash-testutils"))]
std::thread_local! {
    static PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(all(test, feature = "zcash-testutils"))]
pub(super) fn parse_count() -> usize {
    PARSES.with(std::cell::Cell::get)
}

use sapling_crypto::{
    bundle as sapling_bundle, circuit as sapling_circuit, keys as sapling_keys,
    prover as sapling_prover, value as sapling_value,
};
use zcash_proofs::prover;
#[derive(Default)]
pub(super) struct LazySaplingProver {
    prover: std::sync::OnceLock<prover::LocalTxProver>,
}

impl LazySaplingProver {
    /// - Parses the embedded Sapling parameters on this adapter's first call.
    fn prover(&self) -> &prover::LocalTxProver {
        self.prover.get_or_init(|| {
            #[cfg(all(test, feature = "zcash-testutils"))]
            PARSES.with(|count| count.set(count.get() + 1));
            prover::LocalTxProver::from_bytes(
                zingolib::SaplingParams::SPEND,
                zingolib::SaplingParams::OUTPUT,
            )
        })
    }
}

impl sapling_prover::SpendProver for LazySaplingProver {
    type Proof = <prover::LocalTxProver as sapling_prover::SpendProver>::Proof;

    fn prepare_circuit(
        proof_generation_key: sapling_crypto::ProofGenerationKey,
        diversifier: sapling_crypto::Diversifier,
        rseed: sapling_crypto::Rseed,
        value: sapling_value::NoteValue,
        alpha: jubjub::Fr,
        rcv: sapling_value::ValueCommitTrapdoor,
        anchor: jubjub::Fq,
        merkle_path: sapling_crypto::MerklePath,
    ) -> Option<sapling_circuit::Spend> {
        <prover::LocalTxProver as sapling_prover::SpendProver>::prepare_circuit(
            proof_generation_key,
            diversifier,
            rseed,
            value,
            alpha,
            rcv,
            anchor,
            merkle_path,
        )
    }

    /// - Parses the embedded Sapling parameters if this adapter has not yet.
    fn create_proof<R: rand10::Rng>(
        &self,
        circuit: sapling_circuit::Spend,
        rng: &mut R,
    ) -> Self::Proof {
        sapling_prover::SpendProver::create_proof(self.prover(), circuit, rng)
    }

    fn encode_proof(proof: Self::Proof) -> sapling_bundle::GrothProofBytes {
        <prover::LocalTxProver as sapling_prover::SpendProver>::encode_proof(proof)
    }
}

impl sapling_prover::OutputProver for LazySaplingProver {
    type Proof = <prover::LocalTxProver as sapling_prover::OutputProver>::Proof;

    fn prepare_circuit(
        esk: &sapling_keys::EphemeralSecretKey,
        payment_address: sapling_crypto::PaymentAddress,
        rcm: jubjub::Fr,
        value: sapling_value::NoteValue,
        rcv: sapling_value::ValueCommitTrapdoor,
    ) -> sapling_circuit::Output {
        <prover::LocalTxProver as sapling_prover::OutputProver>::prepare_circuit(
            esk,
            payment_address,
            rcm,
            value,
            rcv,
        )
    }

    /// - Parses the embedded Sapling parameters if this adapter has not yet.
    fn create_proof<R: rand10::Rng>(
        &self,
        circuit: sapling_circuit::Output,
        rng: &mut R,
    ) -> Self::Proof {
        sapling_prover::OutputProver::create_proof(self.prover(), circuit, rng)
    }

    fn encode_proof(proof: Self::Proof) -> sapling_bundle::GrothProofBytes {
        <prover::LocalTxProver as sapling_prover::OutputProver>::encode_proof(proof)
    }
}
