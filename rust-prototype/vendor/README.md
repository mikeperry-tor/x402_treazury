# Experimental Alloy compatibility patch

These are the crates.io source archives for `alloy-primitives` 1.7.3 and
`alloy-sol-macro-expander` 1.7.3, with **one dependency line changed per crate**:

```diff
 [dependencies.sha3]
-version = "0.11.0"
+version = "=0.10.9"
```

No Rust source or wallet cryptography is modified. `Cargo.toml.orig` remains
upstream's original workspace manifest; Cargo builds from the patched normalized
`Cargo.toml`. The upstream `.cargo_vcs_info.json` is retained. `provenance.json`
records SHA-256 hashes of the published crate archives and every original file.
Cargo's extraction marker `.cargo-ok` is omitted. The dual MIT/Apache license
texts are copied from upstream commit `87080853083c909b34f8c8cff3622570c114985d`.

Verify that no other source changes have occurred:

```sh
python3 rust-prototype/vendor/verify.py
```

Both the prototype's root manifest and the separate zingolib experiment root
apply `[patch.crates-io]` to these exact local sources. Patches on a dependency's
manifest are not inherited, so any new workspace root must repeat them.

This replaces Alloy's Keccak dependency with the older RustCrypto implementation,
allowing `digest 0.10` to coexist with zingolib's `digest 0.11.0-pre.9`.
It does not relabel a digest version or force prerelease APIs onto stable callers.
The independent Keccak and EIP-712 vectors in `tests/crypto_vectors.rs` cover
runtime hashing and the Solidity macro expansion; existing tests cover x402
signing, including Permit2. The separate combined test suite checks these in the
Zcash dependency graph too. This remains a compatibility experiment, not an
upstream-supported dependency combination or a wallet security audit.

To remove the patch, remove both root `[patch.crates-io]` Alloy entries and
update both lockfiles after upstream resolves the incompatibility. Do not remove
the combined workspace's unrelated `lightwallet-protocol` patch: it comes from
zingolib v6.0.0 itself and supplies the required Ironwood protobuf fields.
