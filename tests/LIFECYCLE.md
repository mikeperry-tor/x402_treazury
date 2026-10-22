# Public-swap lifecycle qualification

These tests use public deterministic keys, localhost fixtures and isolated regtest
chains. None spends mainnet funds or contacts NEAR's production API.

| Coverage | Test | Evidence and limits |
| --- | --- | --- |
| Bootstrap → payment → promotion → pending refill → restart → ready replacement | `managed::lifecycle::bootstrap_payment_promotion_pending_refill_restart_and_pool_isolation` | Real payment admission, EVM signing, durable journals and Base RPC adapter. Deterministic funding backend; proves five distinct sends for two bootstrapped pools plus one refill, with stable operation identity across restart. |
| Refund status and pool isolation | `managed::lifecycle::failed_swap_keeps_source_expense_while_another_pool_serves` | API refund status cannot manufacture treasury credit; another funded pool keeps paying. |
| Quote refresh, empty treasury, timeouts and backoff | `tests/funding.rs` | Unprepared-only bounded refresh, independent per-kind backoff and saved health reporting, sanitized diagnostics, degraded timeout with continued reconciliation and no repeated send. |
| Payment ambiguity and admission | `tests/managed.rs` | Typed insufficient-funds traversal retains every signed attempt; other uncertainty grants no paid replay. Advisory balance hints, durable liabilities, qualification nonce proofs, pool isolation and atomic promotion. |
| Direct payment cancellation, process loss and WAL failures | `rotation::manager::admission_tests::{ownership_tests,crash_tests,io_failure_tests}` | Actual signing/HTTP and journal checks at acquired/private-signature/journaled boundaries. Owned subprocess SIGKILL preserves exact committed authorizations without replay; unavailable balance RPC does not block independent peer payment. Real WAL write/fsync failures preserve attempt/evidence/estimate atomicity and disclose no signature. Process/VFS tests run on Unix. Unsigned candidate lookup rejects all role/claim/journal writes and proceeds while the payment gate is held. |
| Concurrent source funding and scan reuse | `treasury::lifecycle_tests` | Two managed recipients share the real treasury actor; real proving, submission and scans on a synthetic local chain. Covers deferred competing preparation, exact quote/pool and auto-funding checks, unchanged observation age/revision, confirmation depth, pending-byte restart, dropped submission waiter and lost send response without replay. Also covers a funded same-height fork: prepared bytes validate before the fork, lose their anchor afterward, still reach the strict mock validator and remain owned after rejection, blocking another preparation. Exact signed bytes and original transaction facts remain immutable. Historical source reorgs update SDK state without a monetary-history gate. Wallets and the mock use the same upstream synthetic activation schedule. Bridge quotes are fixture inputs; strict mock checks do not establish consensus validity. |
| Full concurrent funding worker | `treasury::lifecycle_tests::worker` | Real scheduler, production Backend, refund derivation, overlapping local HTTP quotes, signed source transactions, unsigned current Base balance observations and actual payment admission. Two groups bootstrap after reopening historical insufficient-funds failures, pending-source passes reuse scans, and consuming a reserve triggers exactly one funded replacement while the prior payment liability remains. A client-scoped test-only bridge codec supports synthetic-chain addresses; production stays mainnet-only. |
| Automatic owned-transparent shielding | `treasury::lifecycle_tests::shielding` | Real pinned SDK proposals/proofs, per-address selected inputs and decrypted owned shielded notes, unattributed returns, uneconomic addresses, actual fee rejection, disabled/unconfirmed inputs, automatic actor trigger and cancelled submission. Exact-fee deposits reject before accepting inputs and do not block recovery at other addresses. Lost send response and restart retain identical bytes without replay; mined outputs become SDK spendable. Automatic expiry uses actual selected-input manifests and two completed tip-covering scans; an isolated SDK snapshot with a missing input rejects despite enough remaining value. Only proven expiry permits a later calculation. Synthetic chain does not validate consensus. Store tests separately reject jobless preparation/submission under qualification permits and verify stable address isolation plus explicit EVM identity precedence. |
| Owner reconstruction/replacement | `treasury::reconstruction`, `treasury::actor`, `exited_funding_task`, `exited_reconciliation_task` | Failed SDK mutation restores the latest committed address range through the same Store. Lost acknowledgments preserve uncertain input claims/restrictions; cancelled driver waiters retain queued work. Distinct joined-task IDs prove replacement through existing leases/gates. A scheduler panic while its source is held preserves exact bytes, attempt count and backoff, excludes overlapping replacement, then resumes after release; explicit shutdown prevents replacement. Forced process death cannot recover unpublished bytes. |
| Due treasury observations under capacity traffic | `treasury::actor::tests::due_observation_precedes_ready_capacity_notifications` | The real treasury owner starts an overdue scan before consuming an already-ready capacity notification. Concurrent payment notifications cannot repeatedly overtake due discovery; shutdown and serialized commands retain their existing ownership. The fixture performs no submission or transaction lookup. |
| Backend deposit calculation and cleanup | `treasury::send::proving_tests` | Real Orchard and Sapling calculations retain the exact selected proposal, ordinary pre/at-NU7 expiry and upgrade-boundary behavior, lazy embedded parameters, public proposal cleanup and identical bytes through failed publication/reopen. A production preparation failure after input acceptance checks the exact no-stored-proposal error, no publication or replay, and retained input ownership after reopen. Synthetic proofs do not establish live settlement. |
| Targeted initial destination discovery | `treasury::lifecycle_tests::discovery` | Production treasury proposal and shared Base owner verify that insufficient SDK funds issue no destination request, a previously positive destination prevents source acceptance, failed positive-credit publication blocks acceptance through the cached cooldown, and credit handoff reuses the recipient cooldown without restoring an estimate decremented by a new payment journal. It also exercises new-claim wakeup and shared minute-cadence capital discovery of a later confirmed deposit. The preparation regression evicts the canonical-validation transport while counting accepted sockets, proving no extra actor connection as well as no extra RPC; direct wrong-network failure accepts no inputs. Synthetic/local observations do not qualify live settlement. |
| Deposit after insufficient funds | `treasury::lifecycle_tests::worker::production_preparation_scans_new_deposit_without_a_readiness_balance_gate` | Readiness accepts a stale low estimate without SDK I/O or input claims. Actual actor preparation scans a newly confirmed synthetic deposit, calculates with SDK-selected funds and retains signed bytes without broadcasting. |
| Unstarted SDK rejection and later funding | `treasury::lifecycle_tests::worker::unstarted_sdk_failures_keep_the_quote_and_recovery_allowance_until_funded` | Production Backend and scheduler retain the original quote and zero recovery charges through repeated actual SDK shortages beyond the configured recovery allowance, then fee rejection and a newly mined deposit. The same operation prepares without a new quote or broadcast. Store regression `preparation_phase_and_input_owner_commit_together` injects a phase-publication failure and verifies atomic rollback, successful acceptance and retained ownership after reopen. |
| Tip-only fork and canonical return | `treasury::lifecycle_tests::same_height_tip_only_reorg_updates_sdk_without_historical_source_gate` | A one-block same-height fork triggers an SDK scan while original confirmation records/bytes remain immutable. No historical source-balance veto or automatic rebroadcast is introduced. Re-mining the exact transaction updates SDK state. |
| Replacement credit with unresolved old payments | `managed::replacement_credit_survives_restart_without_releasing_old_authorizations` | Concurrent refill allocates one generation; local Base credit survives restart and promotes the replacement while all older signed liabilities remain. |
| Multi-group bootstrap completion | `deployment::bootstrap::readiness_tests` | Concurrent destination credits across a group-page boundary, incomplete final-page claims, selected-group scope, stable summaries after reopen and captured claims across generation advancement. Both bootstrap and serving ignore historical capital shortages until the current worker reproduces them; stale callbacks cannot block startup. |
| Source settlement after partial credit | `rotation::store::funding::completion_tests` | Clean unprepared work closes once; accepted calculations and exact prepared bytes remain owned after credit. Stale operation callbacks cannot settle replacement work. These are Store fixtures, not chain settlement proofs. |
| Managed ownership cleanup | `deployment::{initialization_tests,store_only_tests}`, `deployment::bootstrap::tests` | Concrete store-only and treasury capability paths retain restrictions and release ownership on failure; rejected store-only bootstrap permits immediate reopening. |
| Balance maintenance across inventory pages | `rotation::manager::maintenance::tests::inventory` | Seventy retained wallets exceed the four-reader lane and 64-row page. A held read permits the finite startup pass to reach independent wallets; a new insufficiency during that read survives for another observation. Requested failures retry across later pages without duplicate active reads or financial writes. Local RPC fixtures do not qualify network availability. |
| Requested refresh and finite startup | `rotation::manager::maintenance::tests` | Production scalar owners cover zero success stopping work, no idle polling, 59-second notification bursts without cooldown renewal, newer requests during reads, cancellation/restart, scoped inventory failure, retired exclusion and first-positive uncredited claim consumption. No payment or source work is created by discovery. |
| Shared startup and funding credit | `rotation::funding::credit_tests` | Production startup reconciliation and funding credit share one blocked Base read after cancellation of the startup caller. A newer insufficient-funds notification survives durable publication and cached credit reuse until the next minute's acquisition; no allocation or source work occurs. The real funding worker also adopts failed shared acquisitions without recounting cached errors or adding another backoff, and later zero success clears its credit failure. |
| Observation cadence and local handoffs | `rotation::funding::scheduler::tests`, `tests/treasury_sync.rs` | Paused-time and actual endpoint counts cover independent minute floors, source/provider/credit failure isolation, local quote→preparation→submission progress without minute delays, blocked or successful no-op preparation retaining the source floor, finite treasury startup, shared scans and accepted jobless ownership with disabled funding. |
| Explicit deployment sync | `wallet_cli::sync::tests`, `tests/treasury_sync.rs`, `tests/integration_preparation.rs` | Existing configured recipients and accepted removed-profile recipients are observed without allocation/provider/NEAR credentials. Asset failures report truthful independent JSON counts and nonzero completion failure; usable scopes still run. Exclusive ownership and reopen checks retain existing financial state. |
| Exact preparation ownership and expiry journal | Store unit tests and `tests/rotation.rs` | No refund-credit ledger; accepted input ownership, retained publication, snapshot revision guards, expiry-proved operation replacement, backup/restore and crash recovery. |
| Never-submitted source expiry | `treasury::lifecycle_tests::unsubmitted_funding_expiry_uses_only_its_two_proof_scans` | Actual SDK-managed source bytes with zero attempts expire through exactly two completed canonical scans, without a preliminary forecast scan. Disabled reauthorization preserves original operation/bytes and performs no submission. Synthetic chain does not validate consensus. |
| Deposit settlement and reorgs | `treasury::regtest::deposit_settlement_and_reorg_recovery` | Actual proofs, configured confirmation depth, same-byte rebroadcast and fail-closed reorg handling against pinned Zebra/Zaino. |
| High-index refunds and shielding | `treasury::regtest::high_index_refunds_and_separate_shielding` | Derives 32 distinct refund addresses, restores the encrypted wallet, discovers the two high-index funded addresses and shields each independently. |
| Expired ambiguous source operation | `treasury::regtest::expired_ambiguous_deposit_releases_only_after_chain_proof` | Crashes after send intent, rejects early release and missing inputs, then recovers from synced unspent inputs beyond expiry. Old bytes cannot be broadcast again. |
| Recovery CLI commands | `treasury::regtest::recovery_cli_lifecycle` | Test-only subprocess adapter drives the production parser/dispatcher: shielding, read-only reconciliation, explicit same-byte broadcast and successful expiry. Confirmed rebroadcast and early expiry refuse without changing accounting. |
| Indexer ambiguity | `treasury::regtest::indexer_non_inclusion_response` | Pins Zaino's generic Internal response for a missing transaction; production lookup leaves it Unknown rather than inventing proof of absence. |

Scheduling fixtures must await the owner's actual publication before advancing
time. HTTP callback or event-channel arrival can precede durable publication and
cooldown assignment; persisted health written after scheduling supplies the
existing barrier in scheduler tests. Resume real time before awaiting existing
fixture failure timeouts when an idle blocking Store worker would prevent paused
automatic time advancement. These tests verify production owners and forbidden
side effects, without introducing runtime deadlines or a separate test framework.

Run the offline application suite:

```sh
scripts/zcash.sh test --offline --all-targets
```

Fast synthetic wallet checks use the test-utilities feature without proving or Docker:

```sh
scripts/zcash.sh test --offline --features zcash-testutils --lib treasury::expiry
```

These check output evidence for transparent, Sapling, Orchard and Ironwood inputs,
including a missing small original input even when the remaining value is sufficient.
Actual transaction selection and dummy actions are covered by synthetic proving;
consensus-valid chain release remains the separate expiry consensus scenario. The
shielding consensus scenario also injects fee-cap and preparation-commit failures,
checks unchanged mempool and retention of the same completed bytes through local
publication and reopen, without another calculation. Compilation
alone does not qualify those Docker-dependent assertions.

Synthetic proving also checks a calculated transaction whose journal publication
fails. The existing owner retains completed bytes and the durable accepted-input
claim; after local publication succeeds, reopen preserves those identical bytes.
No second calculation or broadcast occurs. Run it with:

```sh
scripts/zcash.sh test --offline --features zcash-testutils --lib treasury::send::proving_tests -- --test-threads=1
```

`scripts/check.sh` includes the synthetic-chain source lifecycle. Run it separately
without Docker or external services:

```sh
scripts/zcash.sh test --offline --features zcash-testutils --lib treasury::lifecycle_tests -- --test-threads=1
```

This opt-in test uses public deterministic keys and a test-only network namespace
that production wallet opening rejects. It complements the default bootstrap and
replacement-credit fixtures; it does not establish end-to-end bridge settlement.

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
