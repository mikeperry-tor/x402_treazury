# Zingo integration and Pepper patches

The application and its SDK integration tests use ordinary upstream `zingolib` and
`zingo-netutils` Git sources at revision
`bfb0a2156819ea7f75a9b09b1724abfd06c8925e` (v7.0.0). Neither crate has a local
snapshot or override. The same cohort's `pepper-sync/` remains patched; the shared
workspace retains its path override. Pepper is excluded from workspace test
selection; its merge regression runs through public traits in the application
SDK integration target.

`pepper-sync-sources.json` records the upstream source archive hash, every retained
Pepper snapshot file, its ordered patches, original upstream file hashes and the
copied-code license notice.
`python3 vendor/verify_pepper.py` verifies those files and rejects unexpected Pepper
files. It reverses all patches in a temporary copy and compares the complete
restored tree with the pinned upstream Git source.
`pepper-sync-progress.patch` contains only Pepper's `client.rs` and `scan/task.rs`
changes; `pepper-sync-diagnostics.patch` covers `client/fetch.rs`, `error.rs`,
`sync.rs`, `wallet/traits.rs` and `witness.rs`. `pepper-sync-reorg.patch` keeps
the upstream 100-block rolling rollback/checkpoint/transparent-discovery horizon
and repairs derived chain state when retained rollback boundaries are unavailable. The payload accepted by the public
sync trait has a public re-export and `from_parts` constructor; the witness module
and payload fields remain private. `pepper-sync-manifest.patch` records the
standalone manifest adaptations necessary for the path dependency; upstream
unit-test dependencies are retained
but are not selected by application checks. Never edit Cargo caches or
reference checkouts to implement a dependency patch. The
[checkout ledger](../vendor/README.md#checkout-retirement-tracking) owns remaining blockers
and candidate qualification.

## Adapted application code and attribution

Two application files retain implementations adapted from the upstream revision
named above:

| Application code | Upstream origin | Purpose of the local adapter |
| --- | --- | --- |
| [`src/network/indexer.rs`](../src/network/indexer.rs) | `zingo-netutils/src/lib.rs`: `GrpcIndexer` and its `Indexer` implementation; `zingo-netutils/src/globally_public.rs`: its `TransparentIndexer` implementation | Thirteen indexer and six transparent-indexer RPC delegations use the application's identity-bound SOCKS/progress transport. Upstream's private client uses tonic `Channel` and its constructors establish the SDK's transport. The adapter retains request/response conventions while the application owns network construction and inactivity handling. |
| [`src/treasury/send/prover.rs`](../src/treasury/send/prover.rs) | `zingolib/src/wallet/utils.rs`: `LazySaplingProver` | A `OnceLock` initializes embedded Sapling parameters on first use, and `SpendProver`/`OutputProver` implementations delegate to `zcash_proofs::prover::LocalTxProver`. Upstream's adapter is crate-private; the application needs this behavior when calling backend transaction creation directly, including avoiding Sapling parameter parsing for Orchard-only sends. |

Both files retain upstream copyright and revision attribution in their source
headers. [`vendor/ZINGO-LICENSE-MIT`](../vendor/ZINGO-LICENSE-MIT) preserves the
upstream copyright and permission notice for these adapted implementations.
The notice matches the pinned upstream repository's root `LICENSE`. Its MIT terms
require retaining that notice with copies or substantial portions of the code.
These application implementations remain after removal of the Zingolib/netutils
vendor checkouts, so the notice remains relevant independently of Pepper's
checkout. Calls into ordinary upstream SDK dependencies are separate from these
locally adapted implementations.

## Application SDK boundaries

LightClient is created/restored offline and stays offline. The application does
not use SDK online constructors, URI setters, sync launching, migration
transmitters, price fetchers or optional nym workers.

`src/network.rs` alone constructs the generated gRPC client over the existing
progress transport and returns `network::GrpcIndexer`. The thirteen `Indexer`
and six `TransparentIndexer` methods in `src/network/indexer.rs` retain generated
request defaults, concrete streams, clone ownership and request metadata. Pepper
still calls deprecated aliases. Nonzero submission codes produce
`Status::Unknown` with the original message; successful transaction IDs retain
safe quote stripping. Submission validates the result against the durable exact
transaction. The adapter ignores the traits' legacy total-timeout arguments. The transport
removes any total `grpc-timeout` metadata and measures response inactivity below
protobuf decoding. Identity, authenticated SOCKS remote
DNS and no-direct-fallback rules remain in [network policy](network-egress.md).
`ping-very-insecure` and nym capabilities are not enabled consumers.

`src/treasury/mod.rs` runs one public Pepper engine task inside the existing
isolated session runtime, with local mode/progress state and finite
`shutdown_on_completion=true` configuration, including restored v6 snapshots.
Pepper can hold the wallet write lock across asynchronous recovery fetches; the
engine must remain independently polled while encrypted checkpointing waits for
that lock. Cancellation destroys the runtime and joins blocking workers before
the final snapshot. There is no SDK online handle, first-poll handshake or second
controller to maintain. Successful scans call public `capture_migration_witnesses`
before final tip validation. Both join boundaries discard panic payloads into a
fixed diagnostic. `tests/treasury_sync.rs` covers the actual lock-held recovery
interleaving, periodic/final snapshots, cancellation, teardown and reopen.

`src/treasury/send.rs` retains the exact public proposal through durable input
acceptance and calls backend `create_proposed_transactions` by reference. It uses
the SDK's original account key, system randomness, clock and sender OVK policy,
with explicit `Some(wallet::expiry::tx_expiry_height(chain,
proposal.min_target_height().into()))` for ordinary NU7-aware expiry. Leaving the
high-level SDK calculation offline would instead choose epoch-end expiry.
Public `clear_proposal` runs after releasing the wallet guard; no private proposal
manager or parallel input ledger is needed. The actor reuses canonical validation
without a redundant connection; direct preparation retains network validation.
The lazy Sapling prover uses embedded public parameter bytes only when needed.
Financial authority, retained bytes, publication and recovery remain in [wallet ownership](wallet-rotation.md) and
[lifecycle coverage](../tests/LIFECYCLE.md).

## Remaining Pepper responsibilities

The progress patch removes complete-message timers on sync streams and retains
loader/worker ownership while draining, with periodic stage diagnostics instead
of forced timeout success. Header/body inactivity remains in the application
transport. Resource-related gRPC codes (`OutOfRange` and `ResourceExhausted`)
emit safe WARNs at the existing stream-consumption points before recovery can
replace the original status. The application adapter reports the same codes for
unary, stream-opening and mempool retry failures.
These statuses may originate at the peer or the local decoder:
logs contain fixed stages and numeric codes, never upstream prose or metadata.
Read retries, protobuf size bounds and mempool drain contracts remain unchanged. Remove these hunks only with equivalent progressing/stalled response,
cancellation/restart and worker-drain evidence through the full sync chain.

The diagnostic patch retains typed shielded-pool and operation context for actual
scan merge, reorg rollback and pool-recovery rollback failures. Recovery
recommendations are unchanged. The application extracts fixed labels and discards
underlying errors before logs/status; no tree positions, hashes or upstream prose
are exposed. Fetch spans are debug-only; mempool events contain fixed labels and
numeric status codes. Rollback, automatic pool reset and mempool retry diagnostics
stay at DEBUG while recovery remains available; the owner reports the final
capability outcome. Replace these changes only with equivalent safe context and
instrumentation. The three-pool merge-conflict fixture remains executable in the
application SDK integration target without a separate SDK unit-test build:

```sh
make test CARGO_ARGS='--test sdk_compat' TEST_ARGS='sdk_merge::scan_merge_conflict_preserves_pool_context --exact'
```

See [workspace verification](development.md#combined-workspace-verification).
Upstream v7 owns same-height/deeper reorg truncation and synthetic branch hashes.
The upstream 100-block rolling horizon is a cache-retention policy, not consensus
finality. A node's larger rollback window does not require matching local retention. A rollback outside the
retained horizon clears derived SDK chain state and resumes from the effective
scan floor. Missing older shard checkpoints use upstream's existing
clear-and-rescan fallback; older snapshots need not contain additional checkpoints.
The same fallback handles a pruned checkpoint or required wallet-block boundary
that rollback can no longer reconstruct locally. Startup also repairs persisted
scanned ranges with missing boundary blocks, including interrupted older rollbacks.
New-chain metadata is not attached to an unverified old scanned prefix.
A full clear without a proven common ancestor starts at supported Sapling
activation, even when the replacement tip is above the declared birthday. Clamping
to the replacement tip or old birthday could miss replacement credits already
mined below both. An initial birthday hint beyond the observed tip uses the same
conservative rescan. Existing persisted scan ranges retain the earlier effective
floor through clearing, interruption and reopen; the declared birthday, keys and
original operation journals remain unchanged. A chain below supported activation
still reports a diagnostic rather than inventing usable history. This uses the
existing scan owner and ranges, with no additional recovery ledger or deadline.
The treasury retains keys, address derivation, accepted input claims, original
signed bytes and financial journals through either rescan. Sync readiness and
reuse are rebuilt by the existing scan owner; clearing chain state grants no
submission, release or new deposit authority. Application lifecycle tests cover
rollback beyond the rolling window
while retaining exact pending work. The patched SDK fixture
`future_birthday_rescans_supported_history_and_keeps_floor_across_clear` also
checks the activation-floor fallback with unchanged declared metadata before
the first resumed block is scanned. Mutable input/inclusion/confirmation
observations wait for canonical progress; malformed immutable transaction
identities require scoped inspection. Reorgs can move a Sapling note and change
its current position-derived nullifier. The SDK reconstructs those current inputs;
the application does not compare them with historical inputs to authorize expiry.
Confirmed source work continues the existing swap
observation rather than requesting manual review. A completed SDK observation of
the transaction's computed identity at configured depth proves confirmation without
a second indexer report. [ZIP 244](https://zips.z.cash/zip-0244) separates v5
transaction effects from authorization data: a valid authorization-data variant
can have the same transaction ID while its serialized bytes differ. Canonical SDK
matching therefore uses computed transaction identity/effects, while original
journaled bytes, transaction ID, expiry and input bindings remain strict. Legacy
v4 IDs commit to the whole transaction encoding. This comparison never replaces
the original disclosed bytes or grants replay authority.
Local broadcast counters are not part of this immutable transaction identity.
Malformed counters or unknown source-status labels conservatively retain possible
disclosure while canonical observation continues; they neither authorize another
submission nor veto an otherwise established confirmation or finite expiry.
For unfinished source expiry, the application validates the authenticated original
transaction and signed finite expiry, then uses a completed canonical SDK tip at
the configured depth. It does not reconstruct old SDK inputs or require current
unspent/alternative-spender proofs. Pepper already expires pending originals and
clears only their own spend links across all supported pools; confirmed competing
spenders remain spent. Actual SDK spendability, bridge terminal outcome and new
funding authority remain separate. Wallet format 45 and the public
v6 read/save/reopen/next-address fixture remain documented in
[wallet administration](wallet-cli.md). Server protocol 0.5.0 capability
requirements are distinct from the Rust protocol crate version.

SDK calculation is offline; only an atomically committed outgoing journal can
supply submission authority. Successful exclusive offline reconstruction, with no
completed result awaiting publication, closes a funding calculation whose outgoing
and factual transaction journals are both absent. Existing recovery archival
preserves its encrypted accepted inputs; the next calculation needs a fresh
operation and current authority. The persisted `funding_quote_refreshes` table also
marks these proven never-disclosed resets for recovery-count exclusion; its legacy
name and existing quote-refresh rows remain compatible. Raw pending originals and
committed results with lost acknowledgments retain their owner. Calculation
metadata alone does not create a global source-work fence. Full SDK clear can
discard pending transaction records and their spend links; the application
therefore retains the fence for actual unfinished signed originals until closure.
The synthetic `sdk_reorg_rescan_preserves_original_signed_work` regression covers
this gap without requiring lifetime tracking of completed source transactions.

The application does not mirror SDK reorg history into completed swap workflows.
NEAR owns the outcome of a bound swap; a validated SUCCESS closes source work
without requiring a local CONFIRMED label. Current SDK state independently owns
spendability. Actual unfinished sources retain their exact journals and existing
observation/finite-expiry recovery owner. Production deposits use the SDK's finite
expiry (40 blocks before NU7, 120 afterward); shielding uses the backend's ordinary
finite default. No production zero-expiry constructor requires a parallel recovery
framework.

Schema-21 stores still upgrade additively to schema 22 under exclusive ownership;
read-only status accepts both versions. Existing historical source dispositions,
original bytes and snapshot references remain preserved. Historical rows do not
reopen terminal work or authorize automatic disclosure; the exact-ID legacy guard
prevents previously reopened originals from being sent automatically. Multiple
legacy pending rows remain compatible and continue through the existing owner.

## Build-time parameter downloads

The upstream SDK build script obtains and embeds standard parameter-cache files
through `SAPLING_SPEND_PARAMS` and `SAPLING_OUTPUT_PARAMS`; it does not copy them
into its source checkout. Pre-populate the standard cache for an offline first
build. Deposit proving reads embedded bytes lazily. Unchanged backend shielding
uses `LocalTxProver::with_default_location` to read the provisioned runtime cache;
it does not download parameters. Legacy v6 `vendor/zingolib/zcash-params/*.params`
copies remain ignored, unused and preserved. Build-time acquisition is outside
runtime Tor routing.

The enabled `zcash_proofs` 0.31.0-pre.1 `download-params` build dependency uses
`minreq` 2.14.1 with `rustls` 0.21.12, `rustls-webpki` 0.101.7 and
`webpki-roots` 0.25.4. These older versions do not contain the
[emSign intermediate-key distrust fix](https://rustsec.org/advisories/RUSTSEC-2026-0382).
They are absent from runtime HTTP/gRPC call chains. The downloader requests only
public proving parameters and checks their hardcoded sizes and BLAKE2b-512
digests before returning them; a forged TLS peer cannot substitute accepted
parameter bytes, but can still interfere with download availability. Moving this
graph to a maintained TLS stack requires an upstream dependency migration or a
separately reviewed vendor patch, rather than a compatible lockfile update.
The [release runner](reproducible-builds.md#prepare-inputs) separately
verifies prefetched parameters and denies network access during qualification.

## Release descriptor inputs

The upstream Git build derives its embedded descriptor from checkout revision,
matching tags and dirty status. A source revision alone does not pin all those
inputs. The release runner records the resolved SDK package/source, observed Git
revision, matching tags and `diff-index` result under each isolated build
environment, then records the actual generated `git_description.txt`. It does not
predict the descriptor or modify cached sources/tags to force one. A repository-free
archive falls back to the crate version; with Git installed, the failed dirty
check can add `_dirty`. Shared-cache, same-host comparison does not qualify
reproducibility across independently populated caches. Parameter staging, integrity
checks and OS network denial remain separate release requirements.
