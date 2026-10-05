# Public-swap lifecycle qualification

These tests use public deterministic keys, localhost fixtures and isolated regtest
chains. None spends mainnet funds or contacts NEAR's production API.

| Coverage | Test | Evidence and limits |
| --- | --- | --- |
| Bootstrap → payment → promotion → pending refill → restart → ready replacement | `managed::lifecycle::bootstrap_payment_promotion_pending_refill_restart_and_pool_isolation` | Real payment admission, EVM signing, durable journals and Base RPC adapter. Deterministic funding backend; proves five distinct sends for two bootstrapped pools plus one refill, with stable operation identity across restart. |
| Refund status and pool isolation | `managed::lifecycle::failed_swap_keeps_source_expense_while_another_pool_serves` | API refund status cannot manufacture treasury credit; another funded pool keeps paying. |
| Quote refresh, empty treasury, timeouts and backoff | `tests/funding.rs` | Unprepared-only bounded refresh, independent persisted status-error streaks, sanitized diagnostics, degraded timeout with continued reconciliation and no repeated send. |
| Payment ambiguity and admission | `tests/managed.rs` | One signed attempt, durable liabilities across restart/cancellation, canonical balance checks, nonce reconciliation, pool isolation and atomic promotion. |
| Refund accounting and expiry journal | Store unit tests and `tests/rotation.rs` | Idempotent capped refund credit, fees retained, snapshot revision guards, operation replacement, backup/restore and crash recovery. |
| Deposit settlement and reorgs | `treasury::regtest::deposit_settlement_and_reorg_recovery` | Actual proofs, configured confirmation depth, same-byte rebroadcast and fail-closed reorg handling against pinned Zebra/Zaino. |
| High-index refunds and shielding | `treasury::regtest::high_index_refunds_and_separate_shielding` | Derives 32 distinct refund addresses, restores the encrypted wallet, discovers the two high-index funded addresses and shields each independently. |
| Expired ambiguous source operation | `treasury::regtest::expired_ambiguous_deposit_releases_only_after_chain_proof` | Crashes after send intent, rejects early release and missing inputs, then recovers from synced unspent inputs beyond expiry. Old bytes cannot be broadcast again. |
| Recovery CLI commands | `treasury::regtest::recovery_cli_lifecycle` | Test-only subprocess adapter drives the production parser/dispatcher: shielding, read-only reconciliation, explicit same-byte broadcast and successful expiry. Confirmed rebroadcast and early expiry refuse without changing accounting. |
| Indexer ambiguity | `treasury::regtest::indexer_non_inclusion_response` | Pins Zaino's generic Internal response for a missing transaction; production lookup leaves it Unknown rather than inventing proof of absence. |

Run the offline application suite:

```sh
scripts/zcash.sh test --offline --all-targets
```

Fast synthetic wallet checks use the test-utilities feature without proving or Docker:

```sh
scripts/zcash.sh test --offline --features zcash-testutils --lib treasury::expiry
scripts/zcash.sh test --offline --features zcash-testutils --lib refund_observations
```

These check output evidence for transparent, Sapling, Orchard and Ironwood inputs,
plus refund confirmation/reorg recognition. Actual prepared-transaction input
selection and chain-based release remain the expiry consensus scenario. The
shielding consensus scenario also injects fee-cap and preparation-commit failures,
checks unchanged mempool/outgoing state and reopens before retrying. Compilation
alone does not qualify those Docker-dependent assertions.

Synthetic proving also checks a calculated transaction whose journal commit fails:
no outgoing transaction becomes durable, further preparation requires reopen, and
the pre-calculation input state is recovered. See [COVERAGE.md](COVERAGE.md) for
the exact proving command and recorded qualification results.

Run consensus qualification separately; [REGTEST.md](REGTEST.md) describes the
pinned images and isolated networking:

```sh
RUST_BACKTRACE=0 scripts/zcash.sh test --offline \
  --features zcash-regtest --lib \
  treasury::regtest::deposit_settlement_and_reorg_recovery \
  -- --ignored --exact --nocapture --test-threads=1
```

Run the other named scenarios separately using the same exact-filter pattern.
Never combine the global-policy Tor case with other tests in one process.

These checks establish local implementation behavior. A funded public swap still
needs explicit qualification of actual NEAR routing, source fees, destination
settlement and a real seller's payment acceptance. Confidential authentication is
outside the public-edition test scope.

For the planned all-provider mainnet run, including Tor observations, two-pool
rotation/refill and stage-specific failures, see [qualification scope](../docs/testing.md#qualification-status).
