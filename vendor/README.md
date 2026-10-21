# Vendored dependency patches

Both the application and `compat/zingolib` use this patch set. The independent
compatibility workspace repeats root patches because Cargo does not inherit them
from dependency manifests. The root manifests and committed lockfiles select the
actual build graph; vendored upstream lockfiles are provenance inputs.

## Current reasons and removal evidence

| Patch | Current reason / consumer | Evidence needed before removal |
| --- | --- | --- |
| Alloy `sha3` manifest changes | EVM hashing/signing must coexist with the pinned Zingo prerelease digest graph. This is dependency compatibility, independent of treasury accounting. | Unpatched resolution/build in both workspaces, independent crypto vectors and the combined compatibility suite. |
| `reqwest-transport.patch` | `src/network.rs` needs per-request post-connect header inactivity, nonempty body progress and disabled recovery of unstarted pooled requests. | Equivalent unpatched behavior through the production factory in `src/network/http_timeout_tests.rs` and `src/network/http_queue_tests.rs`, including retry policy review. |
| `hyper-transport.patch`, `h2-transport.patch` | HTTP and gRPC queued cancellation must discard unopened streams before peer capacity returns; this includes signed payment HEADERS. | Queued timeout/cancellation regressions in `src/network/http_queue_tests.rs` and `tests/network.rs` pass with equivalent unpatched behavior. |
| Zingo connector/progress patches | Treasury sync and proposal paths use the application-owned direct/Tor transport and retain progressing sync work. | Equivalent injection, per-response inactivity, startup acknowledgement and drain ownership in the full treasury call chain. See [Zingo details](ZINGO.md). |
| Pepper diagnostics patch | `src/treasury/diagnostics.rs` consumes typed pool/operation context without exposing raw upstream prose. | Equivalent sanitized diagnostics and tests; this is observability, not spendability or funding authority. |
| Pepper tip-reorg patch | SDK wallet state must recognize same-height forks because SDK/blockchain state owns spendability. | Tip-only and deeper-fork lifecycle regressions pass without the patch, preserving pending input ownership and immutable transaction facts. |
| Zingo mock-chain patch | Optional synthetic-chain tests need the indexer's tip hash to match its stored branch. | An equivalent correct upstream fixture; this patch does not affect production indexers. |
| Upstream `lightwallet-protocol` Git override | Pinned Zingo needs Ironwood protobuf fields. This is an inherited upstream requirement, not a local source patch. | Both workspaces compile and Ironwood capability tests pass with the replacement protocol dependency. |

The [wallet rules](../docs/wallet-rotation.md#best-effort-wallet-rules) make
balances advisory and the SDK authoritative for current ZEC spendability. None of
these patches implements a historical source/refund accounting gate, daily budget
or returned-principal credit. That simplification does not remove transport,
chain-reconciliation or dependency compatibility requirements. This inventory
explains the checked-in pins; it does not establish whether a newer upstream
release already supplies equivalent behavior.

Runtime contracts remain in [network policy](../docs/network-egress.md) and
[wallet ownership](../docs/wallet-rotation.md). When changing a pin or removing a
patch, review its complete consumers and retained regression coverage in both
workspaces, then update this inventory and the owning guide.

## Checkout retirement tracking

Prioritize removal of complete locally maintained crate directories over reducing
patch lines. Review the complete blocker set before investing in an upstream
submission or local behavior change. A smaller patch can be a useful prerequisite,
but does not count as a retired checkout. Moving the same fork into another
repository does not retire its maintenance responsibility.

The baseline has eight patched crate directories. The `lightwallet-protocol` Git
override is a separate dependency constraint, not a ninth local checkout. The
[retirement plan](../docs/plans/vendor_retirement.md) defines the initial execution
sequence; this table remains the maintained source of current blockers and next
actions after that plan is removed.

| Checkout | Complete local blocker set | Next action | Upstream reference and candidate |
| --- | --- | --- | --- |
| `alloy-primitives` | SHA3 dependency override. Alternative hashing features still leave an unconditional SHA3 dependency. | Qualify the Zingolib v7 migration with unpatched Alloy in both workspaces, alongside the macro crate. | [Zingolib v7 candidate](#alloy-candidate-with-zingolib-v7); source-level conflict removed, application upgrade unqualified. |
| `alloy-sol-macro-expander` | Independent unconditional SHA3 dependency and its manifest override. | Qualify macro expansion, crypto vectors and signing in the same v7 candidate graph. | [Zingolib v7 candidate](#alloy-candidate-with-zingolib-v7); source-level conflict removed, application upgrade unqualified. |
| `hyper` | Pending-open cancellation in `hyper-transport.patch`. | Prepare a minimal upstream cancellation regression/fix; qualify a version containing equivalent behavior independently of h2's status. | No retirement candidate qualified. |
| `h2` | Pending-open queue cleanup and scheduler wakeups in `h2-transport.patch`. | Prepare upstream zero-capacity/reset regressions and fix; qualify a version independently of Hyper's status. | No retirement candidate qualified. |
| `reqwest` | Post-connect header timing, body inactivity/backpressure, and disabled unstarted-request recovery. | Identify a supported upstream implementation or hooks covering every hunk before selecting contributions or a local adapter. Retain current retry behavior. | No retirement candidate qualified. |
| `zingo-netutils` | Connector constructor plus clone-preserving service injection/Tower changes; standalone manifest adaptations. | Seek a supported injected transport API and qualify source identities with every Zingo consumer. | No retirement candidate qualified. |
| `pepper-sync` | Stream-message/drain changes, typed/sanitized diagnostics and tip-reorg fix; standalone manifest adaptations. | Establish a complete upstream route for all three concerns before choosing contribution order; retain faithful lifecycle evidence. | No retirement candidate qualified. |
| `zingolib` | Indexer installation, sync startup ownership, mock-chain correctness; standalone manifest adaptations and direct compatibility-workspace dependency. Release tooling also assumes its local proving-parameter path. | Qualify the complete SDK API/fixture path and replace release parameter staging atomically with checkout removal. | No retirement candidate qualified. |

Update each affected row during dependency work and when a relevant upstream
response, accepted change or compatible release becomes known. Record actual
issue/PR links, merge revision, candidate release or Git SHA, remaining blockers
and the next action or concrete condition for reopening. An accepted PR is not a
qualified replacement. Failed candidates retain their specific blocking reason;
keep detailed build logs and comparisons under ignored `target/`, not in this
ledger. Do not invent an upstream status from a similar feature name.

A retirement is complete when both root manifests and lockfiles use the qualified
unpatched source, the local crate directory and obsolete patch/provenance entries
are removed, consumers and build/release tools work, and required regressions
remain effective. Retain verification of every remaining vendored source. When
all local blockers are gone, use ordinary upstream Git or registry sources rather
than retaining an unchanged local snapshot without a separate concrete reason.
No new monitoring service or scheduled task is needed: recheck the ledger during
relevant dependency updates and use upstream notifications when explicitly set up.

### Alloy candidate with Zingolib v7

Zingolib `zingolib_v7.0.0`, revision
[`bfb0a2156819ea7f75a9b09b1724abfd06c8925e`](https://github.com/zingolabs/zingolib/commit/bfb0a2156819ea7f75a9b09b1724abfd06c8925e),
provides a candidate for retiring both Alloy checkouts. Its
[lockfile](https://github.com/zingolabs/zingolib/blob/bfb0a2156819ea7f75a9b09b1724abfd06c8925e/Cargo.lock)
resolves `bip32 0.6.0 → hmac 0.13.0 → digest 0.11.3`, with
`crypto-common 0.2.2`. Unpatched Alloy 1.7.3 requires `sha3 0.11.0`, whose
[manifest](https://github.com/RustCrypto/hashes/blob/6c69ea988cac53319564ed90cd87671ed50d3579/sha3/Cargo.toml)
accepts stable `digest 0.11`. This removes the specific prerelease incompatibility
at the upstream dependency level.

The candidate also moves the Zcash dependency cohort to 0.31-era prereleases;
our pinned v6 sources and application graph have not migrated. Preserve required
Zingo integration/patch behavior and validate both application workspaces with
unpatched Alloy before removing either checkout. Source inspection is complete;
combined resolution, builds, crypto vectors and application regression checks remain
open. No dependency pins or runtime behavior change merely by recording this
candidate.

## Alloy compatibility patch

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

Both the application root manifest and the independent compatibility workspace
apply `[patch.crates-io]` to these exact local sources. Patches on a dependency's
manifest are not inherited, so any new workspace root must repeat them.

This replaces Alloy's Keccak dependency with the older RustCrypto implementation,
allowing `digest 0.10` to coexist with zingolib's `digest 0.11.0-pre.9`.
It does not relabel a digest version or force prerelease APIs onto stable callers.
The independent Keccak and EIP-712 vectors in `tests/crypto_vectors.rs` cover
runtime hashing and the Solidity macro expansion; existing tests cover x402
signing, including Permit2. The separate combined test suite checks these in the
Zcash dependency graph too. This is a locally maintained compatibility
combination; passing these tests does not establish upstream support or constitute a wallet security audit.

Once the removal evidence above passes, remove the Alloy entries from both
root `[patch.crates-io]` tables and update both lockfiles. Do not remove
the combined workspace's unrelated `lightwallet-protocol` patch: it comes from
zingolib v6.0.0 itself and supplies the required Ironwood protobuf fields.

## HTTP transport patches

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
timeout rejection. The patch disables Hyper's separate automatic recovery of
unstarted pooled requests, in addition to the Reqwest request-policy retries
disabled by `src/network.rs`. Both signed and unsigned HTTP requests use this
configuration; application retries, polling and financial recovery remain at
their owning layers. See [retry ownership](../docs/network-egress.md#retry-ownership)
for request categories and the connection/header timing interaction that prevents
removing only this disabling hunk without changing behavior. Zcash tonic/gRPC
submission does not use the Reqwest switch. The unenabled HTTP/3 implementation
is unchanged. Real direct/Tor DNS, SOCKS, TLS, stream contention,
sibling response progress, body stalls and consumer backpressure are tested via
the production factory. No large replacement timeout or connection-wide progress
clock is used.

## Source verification boundaries

Run from the repository root:

```sh
python3 vendor/verify.py
python3 vendor/verify_zingo.py
```

`verify.py` restores the two Alloy manifest lines in memory and compares every
file with `provenance.json`. It also calls `verify_transport.py`, which verifies
patch hashes, reverses each HTTP transport patch in temporary copies and checks
all restored files against the recorded published-source hashes.
`verify_zingo.py` checks the complete reviewed Zingo snapshot and recorded patch
files against `zingo-sources.json`, excluding generated proving parameters. It
does not independently reconstruct the original upstream tree by reversing patches.
These are source-integrity checks, not behavior tests or evidence that upstream
still needs a patch. Build/test commands and qualification boundaries are in
[development](../docs/development.md#combined-workspace-verification).
