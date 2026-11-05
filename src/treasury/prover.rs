//! Sapling parameters are parsed only when a Sapling proof is requested.
//!
//! Adapted from zingolib `src/wallet/utils.rs` at
//! bfb0a2156819ea7f75a9b09b1724abfd06c8925e, licensed under MIT.
// Copyright (c) 2022-present Zingolabs
// SPDX-License-Identifier: MIT
// See vendor/ZINGO-LICENSE-MIT for the full upstream license.

#[cfg(all(test, feature = "zcash-testutils"))]
static PARSES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(all(test, feature = "zcash-testutils"))]
pub(super) fn parse_count() -> usize {
    PARSES.load(std::sync::atomic::Ordering::Acquire)
}

// One adapter owns immutable PUBLIC parameters for this process. The static
// requires LocalTxProver: Send + Sync through OnceLock; no unsafe impl is used.
pub(super) static SAPLING_PROVER: LazySaplingProver = LazySaplingProver {
    prover: std::sync::OnceLock::new(),
};

use sapling_crypto::{
    bundle as sapling_bundle, circuit as sapling_circuit, keys as sapling_keys,
    prover as sapling_prover, value as sapling_value,
};
use zcash_proofs::prover;
pub(super) struct LazySaplingProver {
    prover: std::sync::OnceLock<prover::LocalTxProver>,
}

impl LazySaplingProver {
    /// - Parses the embedded public parameters on the process adapter's first use.
    fn prover(&self) -> &prover::LocalTxProver {
        self.prover.get_or_init(|| {
            #[cfg(all(test, feature = "zcash-testutils"))]
            PARSES.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
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

    /// - Parses the embedded Sapling parameters if the process has not yet.
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

    /// - Parses the embedded Sapling parameters if the process has not yet.
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

#[cfg(all(test, feature = "zcash-testutils"))]
pub(super) fn isolated_scenario(test: &str) -> bool {
    const CHILD: &str = "X402_TEST_SAPLING_PROVER_SCENARIO";
    if std::env::var(CHILD).as_deref() == Ok(test) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--test-threads=1", "--nocapture"])
        .env(CHILD, test)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "isolated prover scenario failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"),
        "isolated prover scenario did not execute its exact test"
    );
    true
}

#[cfg(all(test, feature = "zcash-testutils"))]
pub(super) fn assert_concurrent_first_use() {
    use std::sync::{Arc, Barrier};
    assert!(SAPLING_PROVER.prover.get().is_none());
    assert_eq!(parse_count(), 0);
    let barrier = Arc::new(Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                SAPLING_PROVER.prover() as *const prover::LocalTxProver as usize
            })
        })
        .collect();
    let identities: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(
        identities[0], identities[1],
        "concurrent callers share the parsed instance"
    );
    assert_eq!(parse_count(), 1);
}
