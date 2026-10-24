# Zingo integration and Pepper patches

Both the application and `compat/zingolib` use ordinary upstream `zingolib` and
`zingo-netutils` Git sources at revision
`bfb0a2156819ea7f75a9b09b1724abfd06c8925e` (v7.0.0). Neither crate has a local
snapshot or override. The same cohort's `pepper-sync/` remains patched; both
workspaces retain its path override. The root workspace includes Pepper only so
its private merge regression can execute with the application lockfile.

`zingo-sources.json` records the upstream source archive hash, every retained
Pepper snapshot file, its patches and the copied-code license notice.
`python3 vendor/verify_zingo.py` verifies those files and rejects unexpected Pepper
files. `zingo-progress.patch` contains only Pepper's `client.rs` and `scan/task.rs`
changes; `pepper-sync-diagnostics.patch` covers `client/fetch.rs`, `error.rs`,
`sync.rs` and `wallet/traits.rs`. Standalone manifest adaptations and Pepper's
upstream unit-test dependencies remain necessary. Never edit Cargo caches or
reference checkouts to implement a dependency patch. The
[checkout ledger](README.md#checkout-retirement-tracking) owns remaining blockers
and candidate qualification.

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
DNS and no-direct-fallback rules remain in [network policy](../docs/network-egress.md).
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
Copied RPC/prover code retains its exact upstream revision and the full notice in
`ZINGO-LICENSE-MIT`. Financial authority, retained bytes, publication and recovery
remain in [wallet ownership](../docs/wallet-rotation.md) and
[lifecycle coverage](../tests/LIFECYCLE.md).

## Remaining Pepper responsibilities

The progress patch removes complete-message timers on sync streams and retains
loader/worker ownership while draining, with periodic stage diagnostics instead
of forced timeout success. Header/body inactivity remains in the application
transport. Resource-related gRPC codes (`OutOfRange` and `ResourceExhausted`)
emit safe WARNs at the existing stream-consumption points before recovery can
replace the original status. The application adapter reports the same codes for
unary and stream-opening failures; mempool consumption retains its existing safe
status warning. These statuses may originate at the peer or the local decoder:
logs contain fixed stages and numeric codes, never upstream prose or metadata.
Read retries, protobuf size bounds and mempool drain contracts remain unchanged. Remove these hunks only with equivalent progressing/stalled response,
cancellation/restart and worker-drain evidence through the full sync chain.

The diagnostic patch retains typed shielded-pool and operation context for actual
scan merge, reorg rollback and pool-recovery rollback failures. Recovery
recommendations are unchanged. The application extracts fixed labels and discards
underlying errors before logs/status; no tree positions, hashes or upstream prose
are exposed. Fetch spans are debug-only; mempool events contain fixed labels and
numeric status codes. Replace these changes only with equivalent safe context and
instrumentation. The private three-pool merge-conflict fixture must remain
executable, not just copied in a dependency:

```sh
make test CARGO_ARGS='-p pepper-sync --lib wallet::traits::diagnostic_tests::scan_merge_conflict_preserves_pool_context' TEST_ARGS='--exact'
```

See [workspace verification](../docs/development.md#combined-workspace-verification).
Upstream v7 owns same-height/deeper reorg truncation and synthetic branch hashes;
application lifecycle tests retain exact historical facts and current SDK
spendability without a historical capital veto. Wallet format 45 and the public
v6 read/save/reopen/next-address fixture remain documented in
[wallet administration](../docs/wallet-cli.md). Server protocol 0.5.0 capability
requirements are distinct from the Rust protocol crate version.

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
The [release runner](../docs/reproducible-builds.md#prepare-inputs) separately
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
