# Vendored dependency patches

Both the application and `compat/zingolib` use this patch set. The independent
compatibility workspace repeats root patches because Cargo does not inherit them
from dependency manifests. The root manifests and committed lockfiles select the
actual build graph; vendored upstream lockfiles are provenance inputs.

## Current reasons and removal evidence

| Patch | Current reason / consumer | Evidence needed before removal |
| --- | --- | --- |
| `reqwest-transport.patch` | `src/network.rs` needs per-request post-connect header inactivity, nonempty body progress and disabled recovery of unstarted pooled requests. | Equivalent unpatched behavior through the production factory in `src/network/http_timeout_tests.rs` and `src/network/http_queue_tests.rs`, including retry policy review. |
| `hyper-transport.patch`, `h2-transport.patch` | HTTP and gRPC queued cancellation must discard unopened streams before peer capacity returns; this includes signed payment HEADERS. | Queued timeout/cancellation regressions in `src/network/http_queue_tests.rs` and `tests/network.rs` pass with equivalent unpatched behavior. |
| `zingo-progress.patch` (Pepper only) | Direct Pepper synchronization retains progressing stream responses and loader/worker drain ownership. RPC adaptation lives in the application. | Equivalent response inactivity, progress/stall, cancellation/restart and truthful drain behavior through the full sync chain. See [Zingo details](ZINGO.md). |
| Pepper diagnostics patch | `src/treasury/diagnostics.rs` consumes typed pool/operation context without exposing raw upstream prose. | Equivalent sanitized diagnostics and tests; this is observability, not spendability or funding authority. |

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

Four patched crate directories remain. Alloy uses unpatched registry sources;
`zingolib` and `zingo-netutils` use the coherent pinned upstream Git cohort, and
`lightwallet-protocol` 0.4.0 supplies Ironwood fields without a Git override.
Pepper retains its path override and private merge-fixture workspace membership.
The [unfinished assessment plan](../docs/plans/vendor_retirement.md) preserves
remaining assessment requirements; this ledger owns current blockers, next actions
and upstream follow-up.

| Checkout | Complete local blocker set | Next action | Upstream reference and candidate |
| --- | --- | --- | --- |
| `hyper` | Pending-open cancellation in `hyper-transport.patch`. | Prepare a minimal upstream cancellation regression/fix; qualify a version containing equivalent behavior independently of h2's status. | No retirement candidate qualified. |
| `h2` | Pending-open queue cleanup and scheduler wakeups in `h2-transport.patch`. | Prepare upstream zero-capacity/reset regressions and fix; qualify a version independently of Hyper's status. | No retirement candidate qualified. |
| `reqwest` | Post-connect header timing, body inactivity/backpressure, and disabled unstarted-request recovery. | Identify a supported upstream implementation or hooks covering every hunk before selecting contributions or a local adapter. Retain current retry behavior. | No retirement candidate qualified. |
| `pepper-sync` | Stream-message/drain changes and typed/sanitized diagnostics; standalone manifest adaptations and root membership for its private merge regression. | Establish a complete upstream route for both concerns before choosing contribution order; preserve lifecycle/merge regression evidence and remove local test membership when retiring the checkout. | No retirement candidate qualified. |

The application owns nineteen public-trait RPC delegations and a narrow lazy
prover adapter instead of SDK/netutils source snapshots and manifest/patch
synchronization. Handwritten behavior has not disappeared: retain copied-code
attribution, complete factory/financial consumer coverage and actual SDK Git/
descriptor release inputs. Direct Pepper execution keeps the existing runtime,
checkpoint and cancellation owner; do not reintroduce a second controller or
inline polling that deadlocks a wallet-lock checkpoint. Current contracts live in
[Zingo integration](ZINGO.md), network policy and wallet ownership.

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

## Combined dependency compatibility

Both workspaces use unpatched Alloy core 1.7.3 and the pinned Zingolib v7 cohort.
Alloy's `sha3 0.11.0` and Zingo's `bip32 0.6.0 → hmac 0.13.0` accept stable
`digest 0.11.3`; no local hashing manifest override is needed. Independent
Keccak/EIP-712 vectors, payment signing and the compatibility suite remain the
acceptance evidence for this combination. A compatible dependency graph is not a
wallet security audit.

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

`verify.py` calls `verify_transport.py`, which verifies
patch hashes, reverses each HTTP transport patch in temporary copies and checks
all restored files against the recorded published-source hashes.
`verify_zingo.py` checks the retained Pepper snapshot, patches and copied-code
notice against `zingo-sources.json`, rejecting every unexpected Pepper file. It
does not independently reconstruct the original upstream tree by reversing patches.
These are source-integrity checks, not behavior tests or evidence that upstream
still needs a patch. Build/test commands and qualification boundaries are in
[development](../docs/development.md#combined-workspace-verification).
