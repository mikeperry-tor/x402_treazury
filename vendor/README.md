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
python3 vendor/verify.py
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

# HTTP transport patches

`hyper` 1.11.1 and `h2` 0.4.19 are the exact published crates.io archives
with the narrow patches in `hyper-transport.patch` and `h2-transport.patch`. Pending-open HTTP/2 requests now observe response-waiter
cancellation and reset their stream before capacity returns, removing queued
HEADERS as well as preventing body transmission. Previously the cancellation
callback was not polled until after stream capacity returned. h2 also retained
HEADERS ahead of an explicit reset; its scheduler now discards cancelled unopened
streams without emitting an illegal RST_STREAM on an idle stream. This applies to
explicit and implicit cancellation and reclaims cancelled head entries even with
zero peer capacity.

`transport-sources.json` records the archive SHA-256, every original file hash,
and the reviewed patch hash. `python3 vendor/verify.py` also reverses the patches
and checks every file against its upstream hash. Both application and independent
compatibility workspace pin the same patch. No protocol downgrade, retry or
change to financial authority is introduced. Application real TLS/SOCKS tests
check raw HEADERS absence, GET/POST cancellation and timeout, and reuse of the
healthy multiplexed connection. Remove the patch only after equivalent upstream
behavior passes those regressions.

`reqwest` 0.13.5 is also preserved from its published archive, with
`reqwest-transport.patch`. HTTP/1 and HTTP/2 header inactivity begins at the
per-request Hyper connection capture (after DNS/SOCKS/TLS), body inactivity
ignores empty DATA, and backpressured consumers can read available bytes before
timeout rejection. The separate Hyper stale-connection automatic retry is
disabled to match the application's no-automatic-retry policy; explicit unsigned
Base RPC retries remain at their existing owning layer. The unenabled HTTP/3
implementation is unchanged. Real direct/Tor DNS, SOCKS, TLS, stream contention,
sibling response progress, body stalls and consumer backpressure are tested via
the production factory. No large replacement timeout or connection-wide progress
clock is used.
