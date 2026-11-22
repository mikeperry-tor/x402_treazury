# Zingo integration and Pepper patches

The application and its SDK integration tests depend on upstream `zingolib` and
`zingo-netutils` at Git revision `81d799775f11e882896e8710e8af12d6d4d29a41`
(v8.0.0) with no local checkout or override. `pepper-sync` (0.7.0) from the
same cohort remains patched in `vendor/pepper-sync/` via a workspace path
override.

- **Pinned Zcash cohort:** `zcash_client_backend` 0.25.0, `zcash_primitives`
  and `zcash_proofs` 0.31.0, `zcash_protocol` 0.11.0, `zcash_keys` 0.17.0,
  `pepper-sync` 0.7.0, and `zingo-netutils` 7.0.0.
- **Mainnet NU7 schedule:** Activates at height 3,543,405 with branch ID
  `0x77190ad9` (verified on both sides of activation by `tests/sdk_compat.rs`).
- **Patch verification:** `vendor/pepper-sync-sources.json` pins the upstream
  archive hash, retained files, ordered patches, and license notice. Run
  `python3 vendor/verify_pepper.py` to reverse all patches into a temporary
  tree and confirm an exact match against the pinned upstream source. Never
  edit Cargo caches or reference checkouts to patch dependencies. See
  [checkout retirement tracking](../vendor/README.md#checkout-retirement-tracking)
  for remaining blockers.

| Patch | Files covered | Purpose |
| --- | --- | --- |
| `pepper-sync-progress.patch` | `client.rs`, `scan.rs`, `scan/task.rs`, `scan/task_publication.rs` | Removes complete-message timers, preserves worker drain on shutdown, and publishes busy state before enqueueing. |
| `pepper-sync-diagnostics.patch` | `client/fetch.rs`, `error.rs`, `sync.rs`, `wallet/traits.rs`, `witness.rs` | Retains typed pool and operation context on merge and rollback failures and keeps fetch spans at `DEBUG`. |
| `pepper-sync-reorg.patch` | Reorg, checkpoint, and scan-floor handling | Keeps upstream's 1,000-block rolling horizon while repairing derived chain state when rollback boundaries are missing or chains diverge beyond the horizon. |
| `pepper-sync-manifest.patch` | `Cargo.toml` | Minimal manifest adjustments for the workspace path dependency. |

## Adapted application code and attribution

`src/network/indexer.rs` retains a local adapter derived from `zingo-netutils`
v7 revision `bfb0a2156819ea7f75a9b09b1724abfd06c8925e`, verified against the
current v8 cohort:

- **Origin:** `GrpcIndexer` and `Indexer` (`zingo-netutils/src/lib.rs`) plus
  `TransparentIndexer` (`zingo-netutils/src/globally_public.rs`).
- **Behavior:** Delegates the 13 shielded `Indexer` RPC methods through
  `src/network.rs`'s identity-bound SOCKS and progress transport, and fails
  all 6 `TransparentIndexer` methods locally without opening a connection.
- **Attribution:** `src/network/indexer.rs` carries the upstream copyright and
  revision header, and [`vendor/ZINGO-LICENSE-MIT`](../vendor/ZINGO-LICENSE-MIT)
  retains the upstream MIT notice independently of `vendor/pepper-sync/`.

## Ironwood-only policy

Ironwood is the only supported Zcash shielded pool for managed funding, direct
swaps, and wallet administration.

- **Unsupported pools:** Sapling and legacy Orchard transfers, pool fallbacks,
  transparent wallet receiving, transparent-input shielding, and
  transparent-address RPC queries are rejected. Balances in unsupported pools
  never count as spendable capital or delay funding.
- **Address format:** New addresses are Unified Addresses containing only the
  Orchard-format receiver used by Ironwood (no Sapling or transparent
  receivers). Transaction proposals use the backend's Ironwood-only policy and
  validate Ironwood outputs and fees before accepting input ownership.
- **Sole transparent-output exception:** A NEAR-issued transparent deposit
  address bound to an accepted swap quote, paid from Ironwood inputs. All six
  `TransparentIndexer` methods fail locally, and wallet transparent discovery
  is disabled across creation, import, reopen, and reconstruction.
- **No runtime Sapling prover:** Backend transaction APIs require Sapling
  prover trait types, so a parameter-free adapter supplies those types and
  fails closed if ever called. The application has no runtime Sapling parameter
  parser, cache, or downloader.
- **SDK decoding boundary:** The pinned SDK still derives unused keys and can
  decode older wallet formats or unsupported outputs while scanning. Raw
  snapshots and accepted journals stay intact, while application balance
  summaries and `wallet notes` project Ironwood only.
- **Build-time exception:** Upstream `zingolib` `81d7997` unconditionally
  invokes `get_zcash_params` in `build.rs` and embeds Sapling parameter bytes
  in `lib.rs` with no Cargo feature to disable it. See
  [build-time parameter downloads](#build-time-parameter-downloads).

## Application SDK boundaries

`LightClient` is constructed and restored offline and stays offline throughout
its lifetime. The application never calls SDK online constructors, URI setters,
sync launchers, migration transmitters, price fetchers, or nym workers.

### Indexer transport (`src/network/indexer.rs`)

- `src/network.rs` is the sole constructor of `network::GrpcIndexer`. Clones
  share the identity-bound channel.
- The 13 `Indexer` methods keep generated request defaults, streaming types,
  and deprecated aliases called by Pepper; the 6 `TransparentIndexer` methods
  return a fixed local error without network I/O.
- Nonzero submission codes map to `Status::Unknown` with the original message;
  successful txids strip surrounding quotes safely and validate against the
  journaled transaction.
- Legacy total-timeout arguments are ignored; the transport strips
  `grpc-timeout` metadata and measures per-response inactivity below protobuf
  decoding. See [network egress](network-egress.md).

### Sync engine (`src/treasury/mod.rs`)

- Runs one public Pepper engine task inside an isolated Tokio runtime with
  `shutdown_on_completion = true`.
- Because Pepper can hold the wallet write lock across asynchronous recovery
  fetches, the engine stays independently polled while periodic encrypted
  checkpoints wait for that lock.
- Successful scans call `capture_migration_witnesses` before final tip
  validation. Cancellation destroys the sync runtime and joins blocking workers
  before saving the final snapshot. Both join boundaries replace panic payloads
  with a fixed diagnostic (`tests/treasury_sync.rs`).

### Offline transaction calculation (`src/treasury/send.rs`)

- Validates the public Ironwood-only proposal, journals accepted inputs, and
  calls `create_proposed_transactions` by reference inside one dedicated
  blocking owner that survives caller drop and normal runtime teardown.
- Supplies explicit NU7-aware consensus expiry
  (`wallet::expiry::tx_expiry_height(chain, proposal.min_target_height().into())`:
  40 blocks before NU7, 120 blocks afterward) rather than the offline
  high-level API's epoch-end expiry.
- Uses an empty change memo rather than the SDK's advisory full-UA serializer,
  and calls `clear_proposal` after releasing the wallet guard.
- Retries only local Store publication on transient failure (never recalculating
  or disclosing without a committed outgoing journal). After a crash, exclusive
  offline reconstruction can close an unpublished calculation only when both
  outgoing and factual transaction journals are absent, marking the reset in
  `funding_quote_refreshes`.

## Remaining Pepper responsibilities

### Progress, worker drain, and diagnostics

- **Publication order:** Both dispatch paths publish busy before enqueueing so
  a fast worker cannot have its idle state overwritten (matching upstream fix
  [`3bc86bec`](https://github.com/zingolabs/zingolib/commit/3bc86bec73db704e5fba242dc6221497da73e429);
  upstream's broader scanner protocol
  [`4f411a57`](https://github.com/zingolabs/zingolib/commit/4f411a573c792925aa4a58ad9fca166e72e7a379)
  is a future upgrade candidate).
- **Drain and resource codes:** Loader and scan workers drain on shutdown with
  periodic stage diagnostics rather than aborting on a timeout. `OutOfRange`
  and `ResourceExhausted` log `WARN` with fixed stage and numeric status codes
  before recovery can replace the error.
- **Typed failure context:** Merge and rollback errors keep pool (`sapling`,
  `orchard`, `ironwood`) and operation (`scan_merge`, `reorg_rollback`,
  `pool_recovery_rollback`) labels without exposing tree positions, hashes, or
  upstream prose. Rollback, pool reset, and mempool retries stay at `DEBUG`.
  Run the merge-conflict regression via:

  ```sh
  make test CARGO_ARGS='--test sdk_compat' TEST_ARGS='sdk_merge::scan_merge_conflict_preserves_pool_context --exact'
  ```

### Reorgs, rollback horizons, and transaction identity

- **Rolling 1,000-block horizon:** Upstream's 1,000-block window is a cache
  policy, not consensus finality. Recovery depth uses the previous session's
  chain height.
- **Shortened endpoints and divergence checks:** If an endpoint reports a tip
  below protected wallet facts (retained blocks, scanned ranges, or confirmed
  transactions), Pepper compares a retained block served by the endpoint (using
  the computed header hash when a valid full header is present, else an exact
  32-byte compact hash). A mismatch triggers recovery; matching or non-overlapping
  heights preserve wallet observations and report a recoverable sync failure.
  Unscanned scheduled tails beyond the tip are trimmed without lowering the
  scan floor or touching tree leaves, roots, or witnesses.
- **Full clear and activation floor:** When a chain diverges outside the
  retained horizon or a required checkpoint or boundary block is missing, full
  clear resets the effective scan floor to supported Sapling activation (even
  if the new tip is above the declared birthday) so credits mined below the
  birthday are still found. Keys, declared birthday, accepted input claims,
  signed bytes, and operation journals survive the rescan unchanged.
- **Transaction identity and ZIP 244:** Under
  [ZIP 244](https://zips.z.cash/zip-0244), valid v5 authorization-data variants
  share a txid while their serialized bytes can differ. Canonical confirmation
  matches computed transaction effects and txid at configured depth, while
  journaled bytes, expiry, and input bindings remain immutable.
- **Unfinished source expiry:** Validates the authenticated original
  transaction and signed finite expiry against one completed canonical SDK tip
  scan at configured depth. Pepper expires pending originals and releases only
  their own spend links; full clear can drop SDK pending records, so the
  application retains its own pending-source fence until closure. Bound NEAR
  `SUCCESS` closes swap source work without requiring local confirmation or
  reopening after later reorgs.
- **Wallet and database formats:** Application storage requires database schema
  23 and SDK wallet format 45 (independent of JSON output versions; see
  [versioned output](wallet-cli.md#versioned-capacity-output)). Server
  lightwallet protocol >= 0.5.0 is independent of the Rust
  `lightwallet-protocol` crate version.

## Build-time parameter downloads

Upstream `zingolib`'s `build.rs` obtains and embeds `SAPLING_SPEND_PARAMS` and
`SAPLING_OUTPUT_PARAMS` via `zcash_proofs` 0.31.0 (`download-params`, which
uses `minreq` 2 and legacy `rustls` 0.21).

- Pre-populate the standard parameter cache for offline builds. Build-time
  downloads run outside runtime Tor routing and carry no runtime client
  secrets. Legacy `vendor/zingolib/zcash-params/*.params` files remain ignored
  and unused.
- The downloader verifies BLAKE2b-512 digests and fixed file sizes; the known
  `rustls` build-time advisory is tracked and accepted in
  [dependency disposition](development.md#locked-graph-advisory-disposition)
  rather than maintaining a downloader fork. The
  [release runner](reproducible-builds.md#prepare-inputs) prefetches parameters
  and blocks network access during qualification builds.

## Refund receivers and seed recovery

### Query suppression and refund derivation

- Every generated, imported, reopened, and reconstructed wallet enforces
  `TransparentAddressDiscovery::disabled()`, preventing known-address RPCs and
  gap scans so a refund address disclosed to NEAR cannot be correlated via
  transparent-address queries to the Zcash indexer.
- New refund addresses use
  `generate_unified_address(ReceiverSelection::orchard_only(), AccountId::ZERO)`
  (Ironwood's Orchard-format receiver only, no Sapling or transparent
  components) and commit with the SDK snapshot and job binding before quoting.
  Saved historical transparent or mixed refund bindings remain preserved audit
  records and are rejected for new quotes.
- SDK regressions in
  [`src/treasury/lifecycle_tests/refund_addresses.rs`](../src/treasury/lifecycle_tests/refund_addresses.rs)
  assert zero transparent RPC calls across all six transaction-history,
  balance, and UTXO methods and verify seed-only recovery of distant shielded
  refunds via account viewing keys (no address list or gap scan required).
  Preserve these negative controls on SDK upgrades.
- Public bridge references (`bridge-sdk-rs`
  `d1361ac1c68fa579ed1a498d30f2124bc16e09cc`, `zcash.rs` L612 and
  `get_orchard_raw_with_memo`) and Zodl's shielded-UA usage show format support
  for Orchard-only UAs on refunds and shielded withdrawals, though dry quotes
  alone do not qualify live funded settlement. See
  [refund policy](wallet-rotation.md#refunds-state-and-recovery).

### Direct sends, previews, and receives

- **Native sends:** Accept shielded-only recipients and verify one positive
  OVK-recovered Ironwood output against the recipient and amount (`OvkPolicy::Sender`
  excludes implicit change from outgoing inventory). Optional `max_send_zec`
  checks principal plus SDK network fee at proposal and publication.
- **Previews:** Reuse the Ironwood-only proposal and SDK `total_fee` with
  `lock_inputs = None`, then discard the proposal without saving snapshots or
  reserving inputs. When the proposal reports `ScanRequired`, preview reports
  unknown spendability instead of the SDK's fallback zero. Foreign previews
  estimate fees against an internal transparent-output shape and an existing
  shielded refund receiver without creating persistent bindings.
- **Native receives:** Derive one fresh shielded-only UA per new request ID
  with no NEAR traffic or transparent discovery, matching incoming Ironwood
  note receivers against SDK confirmations and spent links.

## Release descriptor inputs

The upstream SDK build script computes `git_description.txt` from the checkout
revision, matching tags, and Git dirty state (falling back to crate version or
`_dirty` when Git metadata is absent). The release runner records the resolved
SDK source, revision, tags, `diff-index`, and generated `git_description.txt`
for each isolated build rather than mutating caches to force a match.
