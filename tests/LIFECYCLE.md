# Public-swap lifecycle qualification

These tests use deterministic unfunded keys, localhost fixtures, and isolated
synthetic or Docker regtest chains. None spends mainnet funds or contacts
NEAR's production API.

## Commands

```sh
# Offline default application suite:
make test CARGO_ARGS='--offline --all-targets'

# Fast Store expiry checks (no proving or Docker):
make test CARGO_ARGS='--offline --features zcash-testutils --lib' TEST_ARGS='rotation::store::expiry::'

# Synthetic proving tests (1 thread):
make test CARGO_ARGS='--offline --features zcash-testutils --lib treasury::send::proving_tests' TEST_ARGS='--test-threads=1'

# Synthetic-chain source lifecycle suite (TEST_THREADS, default 8; included in make check):
make check-lifecycle
# Serial diagnostic run:
make check-lifecycle TEST_THREADS=1

# Docker consensus scenario (run one named scenario per process):
RUST_BACKTRACE=0 make test CARGO_ARGS='--offline --features zcash-regtest --lib treasury::regtest::deposit_settlement_and_reorg_recovery' TEST_ARGS='--ignored --exact --nocapture --test-threads=1'
```

Never combine `tor_consensus_lifecycle` with other tests in one process
because it installs process-wide network policy. See [REGTEST.md](REGTEST.md)
for Docker images and [qualification status](../docs/testing.md#qualification-status)
for live mainnet scope.

## Shared fixture conventions

- **Ironwood-only fixtures:** New proposals, signed outputs, and capital
  projections follow the
  [Ironwood-only policy](../docs/zingo-integration.md#ironwood-only-policy)
  with Orchard-only UAs. Common funded fixtures share immutable raw faucet
  transactions cached per process by `(receiver, amount, height_bucket)` under
  the default synthetic activation schedule while creating fresh SDK wallets,
  encrypted Stores, and mock chains per test with no shared mutable state.
- **HTTPS policy and ownership:** The full concurrent worker shares a verified
  TLS router across its local bridge, Base RPC, and paid API. Release temporary
  `StoreHandle` observers before awaiting `Treasury::close` so the Store worker
  can exit cleanly. Direct-send fixtures synchronize on submission
  notifications and actor barriers without wall-clock deadlines.
- **Deterministic virtual time:** Scheduler, maintenance, and retirement
  fixtures await durable publication or startup drain before pausing time, and
  resume wall-clock time before blocking Store or TLS follow-ups. Native invoice
  retry tests validate each UTC clock independently and compare the remaining
  instruction object without assuming same-second calls.
- **Scan-consumption invariants:** Expiry fixtures inject an indexer fault
  after the canonical scan to assert closure completes with zero extra lookups
  or submissions. Damaged plaintext txid and expiry projections verify recovery
  uses authenticated original bytes without mutating journals or extending
  broadcast eligibility.

## Lifecycle coverage matrix

| Coverage | Test target | What it verifies |
| --- | --- | --- |
| Bootstrap -> payment -> promotion -> pending refill -> restart -> ready replacement | `managed::lifecycle::bootstrap_payment_promotion_pending_refill_restart_and_pool_isolation` | Payment admission, EVM signing, durable journals, and Base RPC adapter across five sends, two pools, one refill, and restart. |
| Refund status and pool isolation | `managed::lifecycle::failed_swap_retains_original_source_terms_while_another_pool_serves` | Provider refund status preserves original terms without inventing credit while a sibling pool keeps paying. |
| Early rotation and refill opportunities | `rotation::store::early_rotation::tests`, `rotation::funding::scheduler::opportunity_tests` | Private HMAC threshold vectors, used-wallet guard, small-first claim consumption, bound payer stability, rename and reopen stability, and prompt bootstrap. |
| Quote refresh, empty treasury, timeouts, and backoff | `tests/funding.rs` | Unprepared-only refresh, per-kind backoff, saved health, sanitized diagnostics, degraded timeout without replay. |
| Fixed and ranged `EXACT_INPUT` funding | `rotation::config::tests`, `quote_selection_tests`, `redraw_tests`, `rotation::near::tests`, `completion_tests` | Slot parsing, uniform draws, saved-choice reuse, provider-floor redraw within deadline, zero-slippage preview -> single-slippage executable quote, distinct nominal vs. source fields across restart. |
| Payment ambiguity and admission | `tests/managed.rs`, `rotation::manager::admission_tests` | Typed insufficient-funds traversal retaining liabilities, advisory hints, atomic promotion, and Unix `SIGKILL` crash and WAL fsync failure atomicity without replay. |
| Qualification lifecycle snapshots | `rotation::store::qualification::projection_tests`, `qualification::payment::tests` | Exact schema-2 fields, raw-record hashes, pinned SQLite snapshots across WAL writers, and preserved receipts on observation failure. |
| Concurrent source funding and scan reuse | `treasury::lifecycle_tests` | Two recipients sharing one treasury actor with synthetic proving, deferred preparation, same-height fork rejection after anchor loss, tip-only forks, and immutable signed bytes. |
| Full concurrent funding worker | `treasury::lifecycle_tests::worker` | Production scheduler and Backend, refund derivation, overlapping quotes, signed deposits, Base checks, insufficient-funds recovery, and single replacement on reserve consumption. |
| Shielded refund receivers and seed recovery | `treasury::lifecycle_tests::refund_addresses` | Distant Ironwood refund spendable after mnemonic-only restore, Orchard-only UAs, zero transparent RPCs across all 6 methods, and reopen override of legacy discovery-enabled snapshots. |
| Direct native sends and receives | `treasury::lifecycle_tests::shielding`, direct-send and native-receive suites | Principal-plus-fee authority, cancellation, exact-byte restart, partial and excess native receipts, zero transparent RPCs, and unconditional refusal of transparent-input shielding. |
| Blocking calculation and publication ownership | `treasury::lifecycle_tests::calculation_owner`, `treasury::send::proving_tests`, `treasury::publication::tests` | Single accepted input owner surviving cancellation, runtime drop, and concurrent MCP calls; NU7 expiry boundaries; failed publication retries of identical bytes; panic isolation and offline reconstruction. |
| Targeted destination discovery | `treasury::lifecycle_tests::discovery` | Insufficient SDK funds skip destination checks; positive destination blocks acceptance; credit handoff reuses recipient cooldown without clobbering payment decrements. |
| SDK confirmation, ZIP 244, and expiry | `treasury::lifecycle_tests::{ironwood,terminal}`, `unsubmitted_funding_expiry_uses_only_its_one_proof_scan_*` | Confirmation without indexer lookup (including v5 alternate auth), relocated Ironwood notes, buried and unsubmitted expiry in one scan, damaged plaintext projections, and preserved terminal history. |
| Managed refund replacement | `treasury::lifecycle_tests::managed_refunds` | Four-part replacement rule: bound `REFUNDED`, source closure, receiver-specific confirmed Ironwood receipt (including partial), and unused claim under enabled funding. |
| Indexer recovery and deep reorgs | `treasury::lifecycle_tests::{scan_bounds,indexer_recovery}`, `unknown_deep_fork_rescans_before_birthday_even_when_tip_remains_above_it` | Shorter same-prefix tips stay safe; full-header identity beats compact hash; missing checkpoints and deep forks rescan from Sapling activation below birthday while keeping original liabilities pending. |
| Archived swap races and scheduler saturation | `rotation::funding::scheduler::tests::{archive_status,diagnostics}`, `fulfilled_archived_request_does_not_starve_disclosed_replacement_after_credit` | Multi-page archived status polling, late archived `SUCCESS` suppressing undisclosed replacements while letting disclosed replacements confirm, and saturated scheduling-record bounds. |
| Multi-group bootstrap and Base maintenance | `deployment::bootstrap::readiness_tests`, `rotation::manager::maintenance::tests`, `rotation::funding::credit_tests` | Multi-page group credits, 70-wallet maintenance across 64-row pages and 4-reader lanes, shared startup and funding credit cancellation handoff, and Store-only fallback. |
| Explicit sync and recovery CLI | `wallet_cli::sync::tests`, `tests/treasury_sync.rs`, `tests/treasury_sync/recovery.rs`, `tests/rotation.rs` | Independent ZEC and Base sync counts, `wallet recover-expired` cancellation checkpoints, Pepper checkpoint lock freedom, and backup and restore recovery. |
| Docker consensus and CLI regtest | `treasury::regtest::*` | Zebra and Zaino settlement, reorgs, transparent-shielding refusal, expired-deposit release, Zaino non-inclusion ambiguity, recovery CLI subprocesses, and fake-SOCKS Tor lifecycle. |
