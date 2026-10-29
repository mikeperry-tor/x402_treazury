# Public-swap lifecycle qualification

These tests use public deterministic keys, localhost fixtures and isolated regtest
chains. None spends mainnet funds or contacts NEAR's production API.

`treasury::lifecycle_tests::refund_addresses` proves a distant shielded refund
becomes SDK-spendable after restoring only its mnemonic and early birthday into a
new wallet. No original snapshot, address metadata or financial workflow is
imported. Binding/reopen tests preserve legacy transparent addresses and prove an
explicit shielded-refund shielding request performs no RPC, calculation or new
operation. Persisted mixed T+Orchard and T+Sapling bindings pass through
`shield_refund` into real SDK shielding proposals and proofs, selecting only the
funded owned transparent input while preserving the exact binding and leaving
submission attempts at zero. Persisted TEX and invalid encodings reject before
sync or preparation instead of reporting that shielding is unnecessary. The
ignored Docker refund fixture still explicitly creates legacy transparent
bindings to retain high-index discovery and per-address shielding coverage;
ordinary new refund derivation emits shielded-only UAs.

Expiry maintenance must consume its completed same-owner canonical scan without
another indexer request. The shielding-expiry fixture makes the next indexer
request fail after successful scanning and requires closure with the fault still
unconsumed, no lookup, no submission and the original bytes/attempts preserved.

The late-archived-success fixture in `treasury::lifecycle_tests::terminal` uses the
real actor to retain a disclosed replacement after destination completion. A
following independent command must finish after maintenance without duplicating
the replacement's lookup; later canonical confirmation closes its original owner
without submission or another lookup.

Canonical recovery coverage must distinguish Base finality from confirmation
depth: finalized lag/unavailability, incoherent headers and mid-read reorgs must
not produce a depth-only balance, nonce or receipt proof. Replaced historical
anchors must recover through original authorization pages after atomic archival
and reopening of derived resolutions, including still-valid signatures and
unburied expiry. Exercise restart and stale concurrent publication at the reset
boundary, preserve retired signer exclusion and consumed initial funding claims,
and assert no payment replay or replacement deposit. Historical resolution/balance
support must remain audit data without vetoing fresh canonical proof. Equal
finalized heights with newer latest balances must preserve the newer observation
through out-of-order publication and forbid retirement on an older zero balance.
Delivery guards must cover every accepted current/archived bridge operation,
including after partial credit: provider terminality and local unfinished work
have separate meanings. Source expiry alone cannot close pending bridge delivery.
SDK/indexer discrepancies
must converge through further observations; deeper-than-rolling-history forks
require full rescan. A full clear without a proven common ancestor must scan from supported
activation, including replacement credits below the old birthday even when the
new tip remains above it. The `unknown_deep_fork_rescans_before_birthday_even_when_tip_remains_above_it`
fixture covers replacement credit below the birthday with a higher replacement
tip. The earlier floor must survive clearing before its
first scanned block, interruption and reopen while declared metadata and original
journals remain unchanged. Initial future birthday hints use that same conservative
coverage; preactivation chains remain diagnostic failures. The patched SDK
`future_birthday_rescans_supported_history_and_keeps_floor_across_clear` fixture
checks the unstarted floor's persistence. These fixtures do not establish live
consensus finality. SDK reconstruction must preserve keys, original bytes, inputs
and genuinely unfinished owners without resurrecting terminal source work. Bound
NEAR SUCCESS must close source work regardless of SDK confirmation, retain delayed
credit observation and survive restart/reorg without submission or credit
fabrication. Exercise settlement before acceptance, during calculation, before
disclosure and after a disclosed request; accepted calculations and I/O retain
their original completion owners.

The orphaned-input shielding expiry fixture counts completed scans across the
post-reorg sync and automatic maintenance together, requiring one canonical scan
while retaining original bytes, accepted inputs and the single submission attempt.

Actual SDK confirmation and signed-expiry fixtures also damage saved plaintext
txid/expiry projections, including missing fields, wrong types, zero and far-future
expiry. Recovery must use authenticated original bytes without rewriting the
journal, replaying a send or altering original authority. Scheduler tests exercise
the same distinction before routing; first disclosure of an expired original
must fail despite a permissive projection. Corrupt ciphertext and required facts
remain scoped failures while a sibling original can still be observed.

| Coverage | Test | Evidence and limits |
| --- | --- | --- |
| Bootstrap → payment → promotion → pending refill → restart → ready replacement | `managed::lifecycle::bootstrap_payment_promotion_pending_refill_restart_and_pool_isolation` | Real payment admission, EVM signing, durable journals and Base RPC adapter. Deterministic funding backend; proves five distinct sends for two bootstrapped pools plus one refill, with stable operation identity across restart. |
| Refund status and pool isolation | `managed::lifecycle::failed_swap_retains_original_source_terms_while_another_pool_serves` | API refund status cannot manufacture treasury credit; bound fulfillment preserves original source terms without inventing native confirmation, while another funded pool keeps paying. |
| Quote refresh, empty treasury, timeouts and backoff | `tests/funding.rs` | Unprepared-only bounded refresh, independent per-kind backoff and saved health reporting, sanitized diagnostics, degraded timeout with continued reconciliation and no repeated send. |
| Payment ambiguity and admission | `tests/managed.rs` | Typed insufficient-funds traversal retains every signed attempt; other uncertainty grants no paid replay. Advisory balance hints, durable liabilities, qualification nonce proofs, pool isolation and atomic promotion. |
| Direct payment cancellation, process loss and WAL failures | `rotation::manager::admission_tests::{ownership_tests,crash_tests,io_failure_tests}` | Actual signing/HTTP and journal checks at acquired/private-signature/journaled boundaries. Owned subprocess SIGKILL preserves exact committed authorizations without replay; unavailable balance RPC does not block independent peer payment. Real WAL write/fsync failures preserve attempt/evidence/estimate atomicity and disclose no signature. Process/VFS tests run on Unix. Unsigned candidate lookup rejects all role/claim/journal writes and proceeds while the payment gate is held. |
| Concurrent source funding and scan reuse | `treasury::lifecycle_tests` | Two managed recipients share the real treasury actor; real proving, submission and scans on a synthetic local chain. Covers deferred competing preparation, exact quote/pool and auto-funding checks, unchanged observation age/revision, confirmation depth, pending-byte restart, dropped submission waiter and lost send response without replay. Also covers a funded same-height fork: prepared bytes validate before the fork, lose their anchor afterward, still reach the strict mock validator and remain owned after rejection, blocking another preparation. Exact signed bytes and original transaction facts remain immutable. Historical source reorgs update SDK state without a monetary-history gate. Wallets and the mock use the same upstream synthetic activation schedule. Bridge quotes are fixture inputs; strict mock checks do not establish consensus validity. |
| Full concurrent funding worker | `treasury::lifecycle_tests::worker` | Real scheduler, production Backend, refund derivation, overlapping local HTTP quotes, signed source transactions, unsigned current Base balance observations and actual payment admission. Two groups bootstrap after reopening historical insufficient-funds failures, pending-source passes reuse scans, and consuming a reserve triggers exactly one funded replacement while the prior payment liability remains. A client-scoped test-only bridge codec supports synthetic-chain addresses; production stays mainnet-only. |
| Automatic owned-transparent shielding | `treasury::lifecycle_tests::shielding` | Real pinned SDK proposals/proofs, per-address selected inputs and decrypted owned shielded notes, unattributed returns, uneconomic addresses, actual fee rejection, disabled/unconfirmed inputs, automatic actor trigger and cancelled submission. Exact-fee deposits reject before accepting inputs and do not block recovery at other addresses. Lost send response and restart retain identical bytes without replay; mined outputs become SDK spendable. Automatic expiry uses exact signed finite expiry and completed canonical tip coverage at configured depth. Current absent, moved or differently spent inputs remain SDK spendability facts; they do not veto closure. A later calculation uses current funding authority and actual SDK funds. Synthetic chain does not validate consensus. Store tests separately reject jobless preparation/submission under qualification permits and verify stable address isolation plus explicit EVM identity precedence. |
| Inclusion after a wallet scan | `treasury::lifecycle_tests::inclusion_after_wallet_scan_waits_without_resubmitting_or_releasing_inputs` | A real synthetic-chain deposit is mined inside lookup after the wallet scan. Pending confirmation retains exact signed bytes, reservations and the original attempt; a later scan confirms at the configured depth without another submission. Unknown scan progress and contradictory current observations retain original ownership while further canonical scans resolve the discrepancy; established SDK confirmation needs no indexer agreement. |
| Owner reconstruction/replacement | `treasury::reconstruction`, `treasury::actor`, `exited_funding_task`, `exited_reconciliation_task` | Failed SDK mutation restores the latest committed address range through the same Store. Lost acknowledgments preserve committed originals and restrictions; unpublished calculation recovery requires successful offline reconstruction and absent outgoing/factual journals, preserves exact accepted inputs and uses fresh authority without consuming a transaction recovery attempt; cancelled driver waiters retain queued work. Distinct joined-task IDs prove replacement through existing leases/gates. A scheduler panic while its source is held preserves exact bytes, attempt count and backoff, excludes overlapping replacement, then resumes after release; explicit shutdown prevents replacement. Forced process death does not recover unpublished bytes or replay their original operation. |
| Due treasury observations under capacity traffic | `treasury::actor::tests::due_observation_precedes_ready_capacity_notifications` | The real treasury owner starts an overdue scan before consuming an already-ready capacity notification. Concurrent payment notifications cannot repeatedly overtake due discovery; shutdown and serialized commands retain their existing ownership. The fixture performs no submission or transaction lookup. |
| Upstream SDK source integration | Both Cargo workspaces, `tests/treasury.rs`, `tests/crypto_vectors.rs`, application/treasury suites and `scripts/tests/test_reproducible.py` | Unmodified pinned Zingolib/netutils preserve wallet read/save/reopen, signing, ordinary expiry and all financial/network consumers. Pepper's real merge fixture and mutation-rejecting source verification remain selected; release-tool tests retain actual descriptor/Git evidence and parameter isolation without a local SDK source path. |
| Application RPC adapter | `network::indexer`, `tests/network.rs`, `tests/submission.rs`, `tests/network_audit.rs` and network unit tests | Active public-trait RPCs use the factory transport through sync and exact-byte submission. Parser cases retain nonzero-code rejection and bare/quoted/lone-quote handling; transport fixtures cover identity/clone/cache ownership, remote DNS, metadata and progress versus stalls. Constructor auditing includes generated clients. |
| Direct Pepper checkpoint ownership | `tests/treasury_sync.rs::checkpoint_waiting_for_pepper_wallet_lock_does_not_stop_engine_progress` | Real pool-recovery fetch holds the wallet write lock across the periodic checkpoint tick. Releasing the response permits failure, periodic/final snapshots and runtime teardown; no submission or surviving I/O remains, and truthful reopen reaches Ready. The inline-engine mutation must stall this interleaving. |
| Backend deposit calculation and cleanup | `treasury::send::proving_tests` | Real Orchard and Sapling calculations retain the exact selected proposal, ordinary pre/at-NU7 expiry and upgrade-boundary behavior, lazy embedded parameters, public proposal cleanup and identical bytes through failed publication/reopen. A production preparation failure after input acceptance checks the exact no-stored-proposal error, no publication or replay, and retained input ownership after reopen. Synthetic proofs do not establish live settlement. |
| Targeted initial destination discovery | `treasury::lifecycle_tests::discovery` | Production treasury proposal and shared Base owner verify that insufficient SDK funds issue no destination request, a previously positive destination prevents source acceptance, failed positive-credit publication blocks acceptance through the cached cooldown, and credit handoff reuses the recipient cooldown without restoring an estimate decremented by a new payment journal. It also exercises new-claim wakeup and shared minute-cadence capital discovery of a later confirmed deposit. The preparation regression evicts the canonical-validation transport while counting accepted sockets, proving no extra actor connection as well as no extra RPC; direct wrong-network failure accepts no inputs. Synthetic/local observations do not qualify live settlement. |
| Deposit after insufficient funds | `treasury::lifecycle_tests::worker::production_preparation_scans_new_deposit_without_a_readiness_balance_gate` | Readiness accepts a stale low estimate without SDK I/O or input claims. Actual actor preparation scans a newly confirmed synthetic deposit, calculates with SDK-selected funds and retains signed bytes without broadcasting. |
| Unstarted SDK rejection and later funding | `treasury::lifecycle_tests::worker::unstarted_sdk_failures_keep_the_quote_and_recovery_allowance_until_funded` | Production Backend and scheduler retain the original quote and zero recovery charges through repeated actual SDK shortages beyond the configured recovery allowance, then fee rejection and a newly mined deposit. The same operation prepares without a new quote or broadcast. Store regression `preparation_phase_and_input_owner_commit_together` injects a phase-publication failure and verifies atomic rollback, successful acceptance and retained ownership after reopen. |
| Tip-only fork and canonical return | `treasury::lifecycle_tests` | SDK chain replacement updates spendability without reopening completed source work. Exact original bytes, inputs and attempt count survive; genuinely pending originals remain owned without replay. |
| SDK source proof without indexer agreement | `treasury::lifecycle_tests::sdk_confirmation_finishes_original_source_without_indexer_agreement_or_replay`, source expiry lifecycle tests | Completed SDK confirmation with matching computed identity/effects finishes without redundant indexer lookup, including a v5 authorization-only encoding variant while the original journaled bytes remain unchanged. Buried signed finite expiry and completed SDK coverage require no indexer-presence or current-input equality veto. Real SDK confirmation and expiry also proceed with malformed local broadcast sequencing, preserving its stored counter and original bytes without a new submission; authenticated original corruption remains scoped. No automatic replay or replacement authority is created. |
| Relocated Sapling input | `treasury::lifecycle_tests::sapling` | A replacement chain moves the identical received transaction behind new Sapling commitments. Actual SDK position/nullifier changes cannot prevent expiry recovery for the same owned output, either unspent or spent by another confirmed transaction. Original bytes, selected inputs and zero submission attempts remain unchanged; early expiry cannot close ownership. |
| Terminal source preservation | `treasury::lifecycle_tests::terminal`, `rotation::store::source_reconciliation::tests` | Real SDK reconstruction preserves completed historical source journals without resurrection; ordinary reconciliation rejects terminal originals. A retained authenticated original exercises canonical-proof/Store confirmation after expiry and restart, then failed/refunded recovery at the exact allowance counts it once, with no broadcasts and repeated genuine-limit denials. Bound fulfillment remains closed without fabricated confirmation/credit. Store fixtures preserve schema-22 history, snapshot pins and multiple legacy pending originals, while preventing their automatic disclosure. |
| Interrupted SDK scan boundaries | `treasury::lifecycle_tests::scan_bounds` | A saved scanned range missing its required block boundary rebuilds after restart. Automatic reconstruction preserves historical terminal journals and genuinely unfinished ownership without replay or loss of keys, signed bytes, inputs or attempts. |
| Replacement credit with unresolved old payments | `managed::replacement_credit_survives_restart_without_releasing_old_authorizations` | Concurrent refill allocates one generation; local Base credit survives restart and promotes the replacement while all older signed liabilities remain. |
| Provider report reconciliation | `rotation::store::funding::completion_tests::positive_completion_recovers_prior_provider_reports_after_restart`, `rotation::store::bootstrap::tests::provider_reports_do_not_obstruct_live_original_observation` | Earlier incomplete, failed or refunded reports remain recorded, but bound fulfillment plus initial credit settles saved recovery-required work after restart without a source-confirmation conjunction. Delayed negative reports cannot revoke that proof or authorize another deposit. |
| Multi-group bootstrap completion | `deployment::bootstrap::readiness_tests` | Concurrent destination credits across a group-page boundary, incomplete final-page claims, selected-group scope, stable summaries after reopen and captured claims across generation advancement. Both bootstrap and serving ignore historical capital shortages until the current worker reproduces them; stale callbacks cannot block startup. |
| Source settlement after partial credit | `rotation::store::funding::completion_tests` | Clean unprepared work closes once; accepted calculations and exact prepared bytes remain owned after credit. Stale source callbacks cannot replace the captured original identity. Bound archived SUCCESS can fulfill the initial claim while a different disclosed source retains its own owner. Partial credit retains unresolved bridge delivery. Fulfilled intent-only restart preserves accepted inputs while allowing unrelated preparation and exact late-result publication. These are Store fixtures, not chain settlement proofs. |
| Archived swap observation and replacement races | `rotation::funding::scheduler::tests::archive_status`, `treasury::lifecycle_tests::terminal::fulfilled_archived_request_does_not_starve_disclosed_replacement_after_credit` | More than one page of attempts under one job retains exact status identities and quotes across restart, independent completion floors and terminal delivery closure. Status keeps its request identity without borrowing a replacement phase or health; its retry floor survives later current-job reporting failures (`completion_storage_failure_is_fatal`). Held old SUCCESS suppresses untouched/private replacements without changing their provider flags; a disclosed replacement still receives scheduler work after job completion. The real SDK test then confirms that replacement without replay, byte loss or reauthorization. Local fixtures do not qualify live NEAR settlement. |
| Managed ownership cleanup | `deployment::{initialization_tests,store_only_tests}`, `deployment::bootstrap::tests` | Concrete store-only and treasury capability paths retain restrictions and release ownership on failure; rejected store-only bootstrap permits immediate reopening. |
| Balance maintenance across inventory pages | `rotation::manager::maintenance::tests::inventory` | Seventy retained wallets exceed the four-reader lane and 64-row page. A held read permits the finite startup pass to reach independent wallets; a new insufficiency during that read survives for another observation. Requested failures retry across later pages without duplicate active reads or financial writes. Local RPC fixtures do not qualify network availability. |
| Requested refresh and finite startup | `rotation::manager::maintenance::tests` | Production scalar owners cover zero success stopping work, no idle polling, 59-second notification bursts without cooldown renewal, newer requests during reads, cancellation/restart, scoped inventory failure, retired exclusion and first-positive uncredited claim consumption. No payment or source work is created by discovery. |
| Shared startup and funding credit | `rotation::funding::credit_tests` | Production startup reconciliation and funding credit share one blocked Base read after cancellation of the startup caller. A newer insufficient-funds notification survives durable publication and cached credit reuse until the next minute's acquisition; no allocation or source work occurs. The real funding worker also adopts failed shared acquisitions without recounting cached errors or adding another backoff, and later zero success clears its credit failure. |
| Observation cadence and local handoffs | `rotation::funding::scheduler::tests`, `tests/treasury_sync.rs` | Paused-time and actual endpoint counts cover independent minute floors, source/provider/credit failure isolation, local quote→preparation→submission progress without minute delays, blocked or successful no-op preparation retaining the source floor, finite treasury startup, shared scans and accepted jobless ownership with disabled funding. Legacy bound SUCCESS closes previously reopened source work before a held credit read and even for completed work with no eligible observation lane; unrelated preparation proceeds without source/status I/O. |
| Resolution-observation diagnostics | `rotation::funding::scheduler::tests::diagnostics::{canonical_disagreement_reports_automatic_resolution_once_per_error_episode,retained_request_debug_matches_actual_status_acquisitions_without_shortening_cadence}` | DEBUG identifies each actual retained-request lookup and why checks continue; due/capacity deferral and ordinary current polling add no such logs or requests. Canonical disagreement reports automatic resolution at INFO on a changed error episode; ordinary depth waits stay DEBUG and genuine operator outcomes retain WARN. Trace checks also preserve original bytes/attempts and reject provider prose. |
| Explicit deployment sync | `wallet_cli::sync::tests`, `tests/treasury_sync.rs`, `tests/integration_preparation.rs` | Existing configured recipients and accepted removed-profile recipients are observed without allocation/provider/NEAR credentials. Asset failures report truthful independent JSON counts and nonzero completion failure; usable scopes still run. Exclusive ownership and reopen checks retain existing financial state. |
| Exact preparation ownership and expiry journal | Store unit tests and `tests/rotation.rs` | No refund-credit ledger; accepted input ownership, retained publication, snapshot revision guards, expiry-proved operation replacement, backup/restore and crash recovery. |
| Never-submitted source expiry | `treasury::lifecycle_tests::unsubmitted_funding_expiry_uses_only_its_one_proof_scan` | Actual SDK-managed source bytes with zero attempts expire through one fully completed canonical tip scan, without a preliminary forecast scan or redundant second scan. Original signed expiry burial and final unchanged Ready snapshot coverage remain required. Disabled reauthorization preserves original operation/bytes and performs no submission. Synthetic chain does not validate consensus. |
| Deposit settlement and reorgs | `treasury::regtest::deposit_settlement_and_reorg_recovery` | Actual proofs, configured confirmation depth, same-byte rebroadcast and fail-closed reorg handling against pinned Zebra/Zaino. |
| Legacy high-index refunds and shielding | `treasury::regtest::high_index_refunds_and_separate_shielding` | Derives 32 distinct legacy transparent refund addresses, restores the encrypted wallet, discovers the two high-index funded addresses and shields each independently. |
| Expired ambiguous source operation | `treasury::regtest::expired_ambiguous_deposit_releases_only_after_chain_proof` | Crashes after send intent, rejects early release, then recovers using authenticated finite expiry and completed canonical SDK coverage at the configured depth. Current input disposition is SDK-owned; original journals remain preserved and old bytes cannot be broadcast again. |
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
make test CARGO_ARGS='--offline --all-targets'
```

Fast Store expiry checks run without proving or Docker:

```sh
make test CARGO_ARGS='--offline --features zcash-testutils --lib' TEST_ARGS='rotation::store::expiry::'
```

These check revision-guarded expiry publication while preserving the original job,
bytes and accepted inputs. The [recovery policy](../docs/wallet-rotation.md#recovery-and-reconciliation-policy)
owns the signed-expiry and canonical-coverage requirements. The SDK lifecycle cases
in the matrix cover completed scans, missing or relocated inputs, reorg/restart
and same-command scan reuse; run them with `make check-lifecycle`.
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
make test CARGO_ARGS='--offline --features zcash-testutils --lib treasury::send::proving_tests' TEST_ARGS='--test-threads=1'
```

`make check` includes the synthetic-chain source lifecycle. Run it separately
without Docker or external services:

```sh
make check-lifecycle
```

The lifecycle stage runs its independent fixtures on eight test threads while
feature builds remain sequential. For a serial diagnostic run, use
`make test CARGO_ARGS='--offline --features zcash-testutils --lib treasury::lifecycle_tests' TEST_THREADS=1`.
These opt-in tests use public deterministic keys and a test-only network namespace
that production wallet opening rejects. They complement the default bootstrap and
replacement-credit fixtures; they do not establish end-to-end bridge settlement.

Run consensus qualification separately; [REGTEST.md](REGTEST.md) describes the
pinned images and isolated networking:

```sh
RUST_BACKTRACE=0 make test CARGO_ARGS='--offline --features zcash-regtest --lib treasury::regtest::deposit_settlement_and_reorg_recovery' TEST_ARGS='--ignored --exact --nocapture --test-threads=1'
```

Run the other named scenarios separately using the same exact-filter pattern.
Never combine the global-policy Tor case with other tests in one process.

These checks establish local implementation behavior. A funded public swap still
needs explicit qualification of actual NEAR routing, source fees, destination
settlement and a real seller's payment acceptance. Confidential authentication is
outside the public-edition test scope.

For the planned all-provider mainnet run, including Tor observations, two-pool
rotation/refill and stage-specific failures, see [qualification scope](../docs/testing.md#qualification-status).
