# Zingo vendored patches

`zingolib/`, `zingo-netutils/` and `pepper-sync/` contain source from zingolib revision
`bfb0a2156819ea7f75a9b09b1724abfd06c8925e` (v7.0.0). Upstream licenses are
included. `zingo-connector.patch` records the connector Rust source changes:

- `GrpcIndexer::from_channel` wraps an application-owned tonic channel.
- `LightClient::set_indexer` installs that indexer without creating a direct socket.

`zingo-progress.patch` applies after the connector and diagnostic patches. It
adds `GrpcIndexer::from_transport` for clone-preserving service middleware
(constructed only by `src/network.rs`), removes
complete-message timers on sync streams, replaces sync-start polling/abort with
an owned first-poll acknowledgement, and drains loaders/workers with periodic
stage diagnostics instead of forced timeout success. Application transport owns
header/body inactivity; legacy constructors retain library request metadata.
Read retries, protobuf size bounds, mempool drain and financial authority are
unchanged. The added Tower utility dependency supports cloneable injected services.

`pepper-sync-diagnostics.patch` records the additional sync diagnostic changes.
Returned shard-tree errors retain their pool and operation (scan merge, reorg
rollback, or pool-recovery rollback) in a typed error. Existing recovery
recommendations are preserved. No tree insertion, truncation, retention,
serialization, request, or retry policy changes. The application extracts fixed
labels and discards the underlying error before logs and saved status; no tree
positions or hashes are exposed. The synthetic `scan_merge_conflict_preserves_pool_context`
unit fixture exercises actual conflicting merges in all three shielded pools,
without wallet keys or network requests. Pepper Sync retains its upstream unit-test
dependencies for this fixture.
It is selected explicitly by `scripts/check.sh` through Pepper's root workspace
membership; ordinary root tests alone do not execute dependency unit tests. See
[workspace verification](../docs/development.md#combined-workspace-verification).
The diagnostic patch also removes raw `err` events from metadata, latest-block
and tree-state fetch instrumentation and places those spans at debug level.
Mempool failures emit fixed events (a numeric gRPC code when available), not
status prose or cause chains. The application owns
sanitized failure reporting and describes retries according to its caller's schedule;
no upstream status prose is emitted by those spans, including at debug level.

Upstream v7 detects a changed same-height tip and truncates wallet data below
the verification range before scanning its replacement. Deeper seam mismatches
continue backward verification and truncation. Its synthetic indexer reports the
actual stored branch's `tip_hash()`. These upstream behaviors replace the local
tip-reorg and mock-chain patches. The application's
[lifecycle regressions](../tests/LIFECYCLE.md) still cover tip-only and deeper
forks, immutable source facts, retained pending input claims and canonical return.
Historical source facts do not veto new preparation: SDK/blockchain state owns
current spendability, while accepted unresolved inputs remain fenced.

Every existing RPC, cloned indexer and pepper-sync stream uses the supplied channel.
The application constructs both direct and authenticated SOCKS channels through
`network.rs`; it creates LightClient offline and installs that indexer for SDK
sync. Deposit selection and backend calculation keep the client offline. It does not use upstream online constructors, URI setters,
migration transmitters, price fetchers or optional nym workers.

Cargo manifests expand workspace-inherited dependencies, pin sibling Git crates to
the same revision and omit upstream standalone test/example/bench targets and dev
dependencies, except Pepper Sync’s unit-test dependencies. Library sources
(including testutils used by our consensus suite),
build scripts and licenses are preserved; the upstream root MIT license is copied
into the Zingolib and netutils directories. The combined compatibility manifest uses
these same patches. Run `python3 vendor/verify_zingo.py` to check all
reviewed files against `zingo-sources.json`; compare `zingo-connector.patch` with the
pinned upstream commit when updating; review the diagnostic and progress patches
alongside it. `zingo-sources.json` also records the upstream source archive hash.
Never edit a reference checkout or Cargo's Git cache to implement these patches.

## Current consumers and removal criteria

The [complete inventory](README.md#current-reasons-and-removal-evidence) separates
runtime requirements, diagnostics and test-only fixes. None of these changes adds
application monetary accounting. Relying on SDK/blockchain spendability makes
correct SDK reorg handling necessary; it does not require revalidating all historical
source operations before allowing new work.

`src/network.rs` constructs `GrpcIndexer::from_transport`;
`src/treasury/mod.rs` installs it on the offline LightClient for SDK sync.
`src/treasury/send.rs` retains the public proposal and calls backend calculation
without installing an SDK indexer. Its lazy Sapling prover uses embedded public
parameter bytes; copied prover/RPC code retains attribution in
`vendor/ZINGO-LICENSE-MIT`.
Remove connector injection only when an upstream API preserves that same transport
across every RPC and clone, including Tor remote DNS and no direct fallback.
`tests/network_audit.rs`, `tests/network.rs` and `tests/treasury_sync.rs`
cover the relevant factory/consumer boundaries.

Remove progress changes only with equivalent response inactivity below protobuf
assembly, first-poll startup acknowledgement and retained loader/worker ownership
until draining finishes. Inspect `src/network/grpc.rs`,
`tests/network.rs`, `tests/treasury_sync.rs` and the startup implementation in
`vendor/zingolib/src/lightclient/sync.rs`; restoring whole-message or forced drain deadlines
would change current policy even if monetary accounting stays unchanged.

The diagnostic patch is a deliberate observability choice. Its typed context is
consumed by `src/treasury/diagnostics.rs`, whose tests cover fixed labels and
redaction; the Pepper fixture covers real merge conflicts. Replace it only with
equivalent context and safe instrumentation, or explicitly revise the documented
diagnostic contract. It does not change tree recovery recommendations or authority.

Treasury scans explicitly select `shutdown_on_completion=true` before launching
sync, including snapshots read from v6. This preserves one-pass observation and
the application checkpoint/cancellation owner despite the SDK's continuous-sync
default. v7's `go_offline` waits for the current batch, so cancellation first
destroys the scan's owned runtime and joins its blocking workers, then clears
the SDK handle before saving the final snapshot. This preserves cancellation
while a network read is stalled and leaves interrupted scan ranges recoverable
on the next scan; it does not authorize financial replay.
Wallet format 45 adds nested sync settings and scan-state fields; the
public v6 upgrade fixture in `tests/treasury.rs` exercises the old reader, retained
scan targets, save/reopen and next-address continuity. See
[wallet administration](../docs/wallet-cli.md) for upgrade and downgrade boundaries.
The SDK additionally requires a server-advertised lightwallet protocol of at
least 0.5.0; [indexer requirements](../docs/wallet-cli.md#treasury-and-managed-pools)
describe the operational failure behavior. This is distinct from the Rust crate
version selected in the lockfiles.

## Two-checkout replacement boundary

The pinned v7 public APIs provide a candidate exit for `zingolib` and
`zingo-netutils` together. Backend deposit calculation already uses this public
route; the sync, adapter and source transitions remain to be implemented. The [checkout ledger](README.md#checkout-retirement-tracking)
owns its decision and remaining qualification; the
[implementation plan](../docs/plans/vendor_retirement.md) owns commit and review gates.

| Maintained adaptation | Replacement or retained responsibility |
| --- | --- |
| Zingolib `set_indexer` and netutils channel/service injection | Application `Indexer`/`TransparentIndexer` implementation over the generated client, constructed only in `src/network.rs`. All 13 indexer and six transparent methods are required; Pepper still calls deprecated aliases. Keep generated defaults, concrete streaming types, nonzero send-code rejection as `Status::Unknown` and quote-wrapped transaction-ID parsing. `ping-very-insecure` and nym capabilities are not enabled consumers. |
| Zingolib sync startup/first-poll patch | One owned Pepper engine task inside the existing isolated `SyncSession` runtime, with a local mode/progress pair and existing checkpoints. No launch deadline, first-poll handshake or second controller is needed. Keep successful public `capture_migration_witnesses`, protocol checks and finite sync. |
| Connected high-level deposit calculation | Keep the exact public proposal returned by `LightClient::propose_send` through authority acceptance, then call backend `create_proposed_transactions` by reference. Clear through public `LightClient::clear_proposal` after dropping the wallet guard. Private proposal management is unnecessary. |
| Expanded standalone manifests, licenses and source hashes | Ordinary upstream Git sources for both crates at the exact cohort revision in both graphs, including the compatibility workspace's direct Zingolib dependency. Retain the Pepper override and coherent sibling identities; remove only obsolete verification/provenance entries with implementation. |
| Pepper progress/drain and diagnostics patches | Retain all changes in `client.rs`, `scan/task.rs`, `error.rs`, `sync.rs`, `client/fetch.rs` and `wallet/traits.rs`, plus manifest adaptations, unit-test dependencies and explicit root merge-fixture selection. The adapter does not replace them. |

Pepper can hold the wallet write lock across asynchronous fetches. Polling its
future inline in a select that then awaits checkpoint acquisition can deadlock.
An independently scheduled engine task preserves progress while a checkpoint waits;
the session runtime still stops detached I/O and joins blocking workers before the
final snapshot. No additional cancellation or mode-reset framework is needed for
a session-local engine atomic that is never reused.

Ordinary deposits need an explicit
`Some(wallet::expiry::tx_expiry_height(chain, proposal.min_target_height().into()))`.
Neither the backend default nor offline high-level calculation preserves the
connected SDK's NU7-aware ordinary expiry. The accepted single transparent-payment
shape cannot be a canonical ZIP 318 crossing, so no general crossing controller is
needed. Keep original spending keys, `SystemClock`, the SDK CSPRNG and sender OVK.
Preserve lazy Sapling proving from public embedded `SaplingParams` bytes; using
shielding's runtime parameter-cache lookup would change deposit availability.

The backend already scans calculated transactions into SDK pending state.
Current send proposal selection uses no SDK output locks; proposal cleanup releases
the stored proposal/pause, not a separate input-lock ledger. Existing snapshot
publication marks the wallet dirty. Keep accepted-input journaling, retained
publication, restart uncertainty and canonical recovery unchanged rather than
duplicating these SDK/application responsibilities.

The payoff is removal of two checkout snapshots, manifest expansion and connector/
startup patch synchronization. It adds ownership of nineteen RPC delegations and
a small lazy-prover trait adapter; it is not a reduction in handwritten behavioral
lines. Retain upstream attribution for copied adapter/prover code when removing
the checkout licenses. Upstream public injection or a public lazy prover could
reduce that burden later, but neither is required to use the pinned
public-trait/backend route.

The upstream Git build also derives its embedded descriptor from checkout tags,
revision and dirty status. Pinning source revision alone does not pin those Git
metadata inputs. Release evidence must identify them and the actual generated
descriptor when replacing the current vendored-archive fallback. That fallback
uses the crate version and can add `_dirty` when Git finds no repository.
Shared-cache, same-host comparisons do not prove
reproducibility across independently populated caches. Do not alter Cargo caches
to force a descriptor. Parameter staging and network denial remain separate checks.

## Build-time parameter downloads

Sapling parameters are obtained and embedded by the upstream build script, not
fetched by runtime wallet operations. v7 embeds the standard parameter-cache
files through `SAPLING_SPEND_PARAMS` and `SAPLING_OUTPUT_PARAMS`; it does not copy
them into the source checkout. Legacy v6 `zcash-params/*.params` copies remain
ignored and excluded from snapshot verification. Build-time network acquisition is
outside runtime Tor routing. Pre-populate the standard Sapling parameter cache for
an offline first build.

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
The [release runner](../docs/reproducible-builds.md#prepare-inputs) separately
verifies prefetched parameters and denies network access during qualification.
