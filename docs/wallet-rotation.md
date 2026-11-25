# Zcash treasury and managed wallet rotation

See
[wallet selection and network isolation](architecture.md#wallet-selection-and-network-isolation)
for pooling, cover-state, and Tor isolation tradeoffs, and
[wallet CLI](wallet-cli.md#treasury-and-managed-pools) for commands, backups,
Base RPC setup, and JSON schemas.

One embedded Zcash treasury funds named Base USDC wallet groups (pools). Each
group owns separate EVM keys, payment caps, active and retained wallets, and
small (plus optional large) reserve claims. Group names control payer sharing
across sources and listeners.

## Binding and ownership

- **Assignment:** `rotation/assignment.rs` resolves explicit source wallets,
  then listener defaults, then optional `[wallet_templates]` and
  `[wallet_assignment]` scopes (`deployment`, `server`, `source`, `binding`).
  Generated names stay stable within a treasury across template edits; changing
  names or scope creates new groups and disables absent ones while retaining
  their keys, balances, and pending work.
- **Stable group UUIDs:** EVM keys and financial records bind to internal
  group UUIDs, not lookup names. [`wallet rename`](wallet-cli.md#rename-a-managed-wallet-group)
  updates only the name under exclusive ownership; [`wallet preview`](wallet-cli.md#preview-configuration-against-saved-groups)
  compares config against public metadata without unlocking keys. Dynamic
  provider registration shares its listener's explicit wallet and never creates
  pools.
- **Exclusive process ownership:** One process owns `state_dir` exclusively. A
  serialized treasury actor (`src/treasury/actor.rs`) owns Zingolib sync,
  preparation, and submission; a dedicated Store worker owns SQLite operations.
  Database transactions never span remote calls. Accepted actor and Store work
  survives caller cancellation. Encrypted wallet snapshots and outgoing
  operations commit atomically with snapshot-revision compare-and-swap (never
  bypassing this owner with plaintext SDK saves or quick-send helpers).

### Wallet grouping, rotation, and linkability

Group endpoints by what upstream providers can learn about the caller rather
than by topic (for example, separating personal, health, and financial queries
from anonymous general research).

- Separate groups prevent payer reuse across topics, and smaller reserve
  targets create more rotation opportunities. Transport isolation keys
  connections and Tor circuits by payer identity and destination.
- Neither grouping nor Tor hides request payloads (such as images or OCR
  inputs) or timing within a single wallet's calls. Funding uses isolated Tor
  circuits and a unique shielded refund receiver per swap, leaving residual
  correlation to swap timing and size against overall ZEC-to-Base-USDC volume
  (highest during simultaneous initial funding and reduced as wallets rotate
  independently).
- Dynamic provider registrations and `raw_call` use their listener's wallet
  rather than matching a curated endpoint's wallet.

## Direct receive ownership

Opt-in [direct swap control](configuration.md#direct-swap-control) stores
native shielded ZEC requests and foreign-asset swap invoices independently of
managed Base pools and funding claims.

- **Ownership and persistence:** Each encrypted record binds the authenticated
  listener, caller `request_id` and terms, freshly derived shielded receiver,
  exact provider quote, deposit address and memo, and deadlines. Address
  derivation and snapshot persistence commit before quoting or disclosing
  instructions. Removing `swap_control` or its listener preserves background
  observation; only the original authenticated listener can inspect status.
- **Native ZEC receives:** Derive a fresh Orchard-receiver-only Unified Address
  (no NEAR calls, no deadline, no transparent-address RPCs). Addresses are
  reusable (`address_reusable = true`), partial confirmed receipts accumulate
  toward the requested amount, and scan demand ends when confirmed Ironwood
  receipts reach the target. Public status refreshes from newer SDK snapshots
  without forcing a scan; a reorg can reopen observation demand without
  creating spending authority.
- **Foreign receives (`EXACT_INPUT`):** Require the payer to send the exact
  source atomic amount in one deposit plus network fees and any mandatory memo;
  never reuse a foreign deposit address. Each new payment requires a new
  `request_id` and fresh quote (`refund_after_seconds` defaults to 7,200
  seconds as a refund-start preference). Retrying an identical `request_id`
  returns the saved operation; conflicting terms fail.
- **Status and observation:** Status returns `current_time_utc`, UTC-Z
  timestamps, and nonnegative `seconds_until_*` countdowns (zero never creates
  a local terminal state or authorizes replacement). Disclosed unsettled
  receives request 60-second ZEC scans; a provider `SUCCESS` requests one
  subsequent completed scan starting after the snapshot revision recorded with
  `SUCCESS`. Corrupt direct-operation payloads isolate during inventory scans
  while siblings continue (direct lookup fails explicitly). Receive operations
  never spend ZEC, allocate Base wallets, or trigger transparent discovery.

### Direct send ownership

Native shielded sends and foreign `EXACT_OUTPUT` swaps share the treasury's
serialized actor, accepted-input ownership, encrypted snapshots, and
signed-byte journal with managed funding.

- **Spending cap (`max_send_zec`):** Optional per-send total debit (principal
  plus SDK-calculated ZIP-317 network fee) saved with acceptance, verified
  against the proposal and final transaction, and preserved across restarts
  (omitted or `"none"` is unlimited). Before reserving a new request, the
  service checks that the cap can cover a positive principal and the minimum
  SDK fee. It uses the requested principal for native sends. For foreign sends,
  it checks the quoted principal before accepting the send. A quote that
  exceeds the cap with the minimum fee leaves a saved rejection without a
  deposit quote or spending authority. Status and identical retries report
  the rejection. Applies only to direct sends, never managed funding.
- **Non-executable previews (`swap_preview`):** Request advisory Ironwood
  spendability and ZIP-317 fee via the treasury actor using an unsigned
  proposal (for foreign sends, an internal transparent-output shape and an
  existing shielded refund receiver), then discard the proposal without saving
  snapshots or reserving inputs. `fits` compares total debit with the lesser of
  known spendable balance and `max_send_zec`.
- **Execution and recovery:** Native completion follows the Zcash transaction's
  chain state; foreign sends track source confirmation and NEAR delivery
  independently (depositing ZEC does not prove foreign delivery). Disconnects,
  disablement, and restarts preserve accepted work and transport identity.
  Uncertain submission, expiry, or refund never triggers an automatic
  replacement send.

## Funding limits and fees

Each financial control protects a distinct boundary; there is no daily or
cumulative ZEC or USDC allocation cap, per-transfer ZEC spending budget, or
local USD conversion-overhead gate:

| Control | Scope and purpose |
| --- | --- |
| `swap_control.max_send_zec` | Optional direct-send principal and fee cap (default unlimited); bound at acceptance. |
| Reserve slot bounds | Nominal USDC sizing and bridge-minimum headroom (`$2-3` small default, optional `$10-25` large); never caps API payments or ZEC spend. |
| `max_api_payment_usdc` | Per-x402-payment USDC cap (required on managed groups; static default `$1.00`; `none` or `off` for unlimited). |
| `max_retirement_remainder_usdc` | Automatic retirement remainder ceiling (default `$0.001`); never transfers funds. |
| Spendable treasury ZEC | Operator's ZEC capital limit for enabled `auto_fund` (after confirmations, reservations, liabilities, and ZIP-317 fees). |
| `funding.slippage_bps` | Executable `EXACT_INPUT` quote tolerance (default `500` bps = 5%, range `0-10000`). |
| Quote and signed expiry | External financial windows: 1,800s default quote request (7,200s foreign receive); 40-block pre-NU7 and 120-block post-NU7 Zcash consensus expiry; `min(seller_timeout, 300s)` x402 signing validity. |
| Confirmations | Zcash SDK trusted default (3 blocks, uniform override); Base `finalized` tag with no separate depth setting. |
| Precision | ZEC 8 decimals (21M domain); USDC 6 decimals (signed 64-bit micro-USDC). |
| Qualification budgets | Separately authorized test API caps (`total_test_budget_usdc`) and job counts; no separate ZEC funding budget. |

### Capital, calculation ownership, and Store supervision

- **Supported capital:** Only confirmed owned Ironwood notes count as spendable
  capital. Sapling, legacy Orchard, and transparent outputs never authorize
  spending, shielding, or fallback.
- **Offline calculation:** The SDK selects inputs and calculates ZIP-317 fees;
  the application validates exact outputs and fees, journals accepted inputs,
  and runs `create_proposed_transactions` with NU7-aware consensus expiry in
  one dedicated blocking owner that survives caller drop and runtime teardown.
  Completed signed bytes and snapshots commit atomically.
- **Publication retries and Store supervision:** If Store publication fails
  after calculation finishes, the blocking owner retains the identical bytes
  and retries local publication every 60 seconds (never recalculating or
  disclosing without a committed outgoing journal). If the Store worker dies
  fatally, that Store capability fails closed while retaining its exclusive
  file lock and unpublished bytes (other Stores and static wallets stay
  available). After an unhealthy SDK mutation, the actor reconstructs the SDK from
  the latest committed snapshot; an unpublished calculation with neither
  outgoing nor factual transaction journals closes automatically after offline
  reconstruction.

## Treasury sync coverage

- **Sync logging:** Scan starts log at `DEBUG`; persisted completions and
  30-second progress updates log saved height at `INFO`. After an SDK
  insufficient-funds proposal rejection, `funding_assessment` sums confirmed
  and unconfirmed owned Ironwood notes (excluding spent outputs and ignoring
  transparent coins): incomplete scans report unavailable assessment at `INFO`,
  sufficient owned Ironwood awaiting confirmation logs a spendability wait at
  `DEBUG`, and an assessed Ironwood shortage logs the shielded receive address
  at `INFO` (asking the operator to add ZEC). Repeated identical reports stay
  at `DEBUG`.
- **Structured sync errors:** Interruption reports include `category`, fixed
  `reason`, numeric `grpc_code` and `grpc_status` (for example, code `4`
  `DeadlineExceeded` vs. code `14` `Unavailable`), and on shard-tree errors the
  exact variant (`shard_root_conflict`, `shard_checkpoint_pruned`,
  `shard_checkpoint_out_of_order`, `shard_tree_incomplete`), `pool` (`sapling`,
  `orchard`, `ironwood`), and `operation` (`scan_merge`, `reorg_rollback`,
  `pool_recovery_rollback`) without logging addresses, note positions, or
  upstream prose.
- **Sync reuse and coverage:** Synchronization runs the public Pepper engine on
  an isolated Tokio runtime, captures migration witnesses on completion, and
  joins workers on cancellation. Unchanged completed scans reuse within their
  owner after checking the canonical tip hash and height; restart or chain
  changes rescan before preparation or submission. There is no host-clock age
  ceiling or three-block lag gate (`max_sync_age_seconds` and
  `base_max_block_age_seconds` are rejected by config parsing).

## Capacity and admission

Each managed group has one default `ACTIVE` wallet, `RETAINED` wallets with
spendable remainders, and one `READY` wallet per fulfilled enabled reserve slot
(small funds before optional large; each commits independently).

### Quote sizing and initial funding claims

- **Sizing:** Each operation draws its nominal USDC amount (fixed target or
  uniform inclusive draw in `[min, max]`) and runs a zero-slippage dry
  `EXACT_OUTPUT` preview. If NEAR reports a higher bridge minimum within the
  slot maximum, the job raises its local floor, redraws within the remaining
  range (or adjusts a fixed target), persists the choice, and retries while the
  floor rises and the deadline stays valid. If the bridge minimum exceeds the
  slot maximum, the round stops without silently raising the ceiling.
- **Executable quote:** The preview's ZEC principal becomes the fixed input of
  an executable `EXACT_INPUT` quote with `funding.slippage_bps` applied once to
  expected output. Accepted quote terms and signed bytes stay immutable;
  unprepared quotes that no longer fit updated config refresh before
  preparation.
- **One-time initial claim:** A wallet's first positive observed Base USDC
  balance permanently consumes its initial funding claim (even below the
  nominal minimum). Later low or zero balances never reopen the claim or
  authorize top-ups. Disabling a group detaches fulfilled reserves into
  `RETAINED` without cancelling observation of accepted work.

### Payment selection and journal admission

- **Selection:** `rotation/manager.rs` admits Base USDC v1 and v2 exact
  EIP-3009 challenges within the group cap (dropping unknown extensions with
  `DEBUG` logs). Initial selection uses the saved advisory balance estimate
  with no balance RPC: with a fresh observed or valid advertised per-call
  price hint, it prefers estimated-affordable `RETAINED` wallets (oldest
  first), then `ACTIVE`, then `READY`; without a hint, it prefers healthy
  positive `ACTIVE`, then `RETAINED`, then `READY`, with low, zero, and unknown
  estimates as fallbacks. After fetching a challenge, an eligible bound payer
  stays bound rather than fetching another challenge just to spend a retained
  wallet.
- **Unsigned Tor retries and handoff:** Within each payer stage in Tor mode,
  unsigned challenge acquisition may try `1 + network.tor_isolation_retries`
  scopes on eligible statuses (`400`, `403`, `408`, `429`, `500`, `502`, `503`,
  `504`, honoring `Retry-After` on `429` and `503`), promoting the winning
  scope after challenge validation. An unsigned challenge may hand off to
  another eligible payer suggested by the challenged price (visiting each
  wallet at most once with its own isolated transport).
- **Atomic promotion and journaling:** Before disclosure, the Store journal
  rechecks pool ownership, role, reserve claim, cap, payer address, and signed
  validity, records the authorization, and decrements the advisory estimate
  once (saturating at zero). Signing with a `READY` wallet atomically moves the
  prior `ACTIVE` wallet to `RETAINED`, promotes the payer to `ACTIVE`, and
  advances the reserve claim generation so background funding can refill the
  vacancy at its next opportunity.

### First-use bootstrap participants

When `[wallet_bootstrap]` is configured, uncredited deferred groups receive no
new allocations during serving until durable UUID-bound first-use demand is
recorded by a validated API call (or historical positive credit already
exists).

- Each API call captures one effective group before unsigned dispatch and stays
  within that group through signing, cover, and cancellation.
- First-use demand admits the group to the standard 30-120 second refill
  opportunity schedule (retaining its armed instant across wakes; restart
  redraws volatile timers).
- Any historical positive initial USDC credit permanently hands the group over
  to its own wallet and cap across restarts; depletion or refunds never revert
  it to bootstrap. Disabling `auto_fund` pauses allocations without deleting
  demand or recovery state.

### Early rotation and refill opportunities

Automatic funding gives each enabled eligible group an independent uniform
integer interval of 30-120 seconds (mean 75 seconds) rather than allocating
immediately on startup or exhaustion.

- **Proactive rotation:** At a due opportunity, a used `ACTIVE` wallet (with
  at least one payment journal entry) may rotate to `RETAINED` while promoting
  a funded `READY` reserve (small slot first) if its saved estimate is at or
  below a private threshold.
- **Private threshold:** For immutable first positive credit `F` and current
  cap `M`, integer micro-USDC bounds are `U = min(M, floor(F / 4))` (or
  `floor(F / 4)` when unlimited) and `L = floor(U / 5)`. The threshold is an
  unbiased integer in `[L, U]` derived via HMAC-SHA256 with the Store key over
  a versioned, length-framed treasury, pool, and wallet domain (never persisted
  or logged; preserved across renames and restarts; recomputed if `M` changes).
- **Refill and bootstrap distinction:** The new `ACTIVE` wallet must be used at
  least once before another proactive rotation. Immediate `READY` promotion on
  first payment still works whenever calls outpace funding. Explicit
  `wallet bootstrap` allocates promptly without proactive rotation;
  `auto_fund = false` disables both new allocations and proactive rotation
  while keeping observation running.

## Automatic retirement

Serving and `wallet sync` consider only low-estimate `RETAINED` wallets in
enabled groups for automatic retirement (never `ACTIVE`, `READY`, or disabled
groups).

- **Dual balance requirement:** Both the canonical `finalized` balance and the
  `latest` balance must be at or below `max_retirement_remainder_usdc` (default
  `"0.001"`, or `"0"` for exact zero); the recorded remainder is their maximum.
- **Obligation guards:** Retirement is blocked by unresolved payment
  authorizations, an active reserve claim, an unfulfilled accepted source
  calculation, pending signed source work, or conflicting canonical retirement
  evidence. For a never-credited wallet whose funding job completed, every
  possibly disclosed current or archived swap must have a bound terminal NEAR
  outcome (proven nondisclosure needs no bridge outcome) and closed local
  source work.
- **Execution and effect:** Candidate work pages outstanding authorizations (64
  per page, no total-history cap) to verify used nonces or confirmed-chain-time
  expiry, then commits exact amounts, anchors, endpoint, and evidence
  atomically with the role change. Retired wallets leave keys and any dust in
  place, stop routine polling and selection, never transfer funds, and stay
  retired (marked disputed if later contradicted). Positive tolerance applies
  independently of `auto_fund` and has no cumulative stranded-funds cap.

## Payment reconciliation

- **Insufficient-funds traversal:** Managed `PaidClient` calls may try each
  remaining eligible wallet once only after a signed attempt receives a pinned
  structured insufficient-funds `402` response (`PAYMENT-REQUIRED` with exact
  `error: insufficient_funds` or failed `PAYMENT-RESPONSE` with that exact
  `errorReason`). Each attempt uses that wallet's own transport and fresh
  challenge; all prior signed authorizations remain potentially chargeable.
  Settlement receipts (`PAYMENT-RESPONSE` and `X-PAYMENT-RESPONSE`) take
  precedence over simultaneous challenges, and duplicate protocol headers never
  permit a retry. Other HTTP errors, transport errors, malformed replies, and
  successful responses never trigger another wallet attempt.
- **Canonical Base proofs (`rotation/base.rs`):** Verify chain ID, the
  provider's `finalized` anchor (no separate depth setting), block-pinned USDC
  balances, and EIP-3009 nonce usage. Used nonces or confirmed-chain-time
  expired unused nonces release reservations atomically; unused valid
  signatures stay liabilities.
- **Canonical history changes:** If confirmed chain history diverges or a
  derived resolution is malformed, reconciliation archives old resolutions,
  reopens original attempts as potentially submitted, rebuilds exposure, and
  re-proves original nonces against the current chain. Archived proofs stay
  readable as audit history without vetoing fresh canonical proof. A newer
  `latest` height supersedes an older dual observation even when `finalized`
  height is unchanged. Reorg repair never authorizes payment replay,
  replacement deposits, reopened claims, or signer reactivation.

## Funding and source safety

Allocations and operation journals commit atomically. `rotation/funding.rs`
tracks durable outbox phases:

```text
ALLOCATED -> QUOTED -> PREPARING -> PREPARED -> DEPOSIT_PENDING
                                                      |
                                                      v
                                                   SWAPPING
                                                      |
                                                      v
                                           VERIFYING_CREDIT -> COMPLETE
```

Confirmed Base credit can skip `SWAPPING`. Refund and ambiguous states retain
`REFUND_PENDING` or `RECOVERY_REQUIRED` without discarding the operation or
auto-creating another transfer.

### Source ownership, settlement, and closure

- **Journal before disclosure:** Exact selected inputs commit before
  calculation; completed snapshots and signed bytes commit before broadcast
  authority is granted. A pending signed original keeps a durable source-work
  fence until closure (protecting against a full SDK rescan clearing pending
  SDK records).
- **Unpublished calculation recovery:** After exclusive offline SDK
  reconstruction with no live calculation owned, a calculation with neither
  outgoing nor factual transaction journals closes automatically and archives a
  nondisclosure marker so a new attempt can use a fresh operation ID. A factual
  record without outgoing bytes is treated as scoped corruption, not
  nondisclosure.
- **Quote refresh:** Requires proof of no accepted calculation and no outgoing
  bytes, archiving the old binding and allocating a fresh operation ID and
  refund receiver. Expired returned quote deadlines block new preparation and
  first submission (with no extra 5-minute margin).
- **Bound NEAR `SUCCESS`:** Validated `SUCCESS` for the exact saved quote and
  deposit binding closes local source work immediately (even if Zcash
  confirmation is pending or later reorgs), without fabricating a Zcash
  `CONFIRMED` label. First positive Base USDC credit independently consumes the
  initial claim and publishes the ready wallet; `SUCCESS` plus initial credit
  completes the job.
- **Source confirmation and expiry:** Confirmation uses the completed SDK scan
  and ZIP-244 transaction identity and effects at configured depth (allowing
  valid v5 authorization-data variants while keeping journaled bytes
  unchanged). Expiry closure requires the authenticated original signed bytes,
  finite inclusive consensus expiry (40 blocks pre-NU7, 120 post-NU7), and one
  completed canonical tip scan beyond expiry plus configured depth; the SDK
  releases expired pending spend links automatically.
- **Archived swap observation:** After a replacement attempt opens a fresh
  operation ID, archived accepted NEAR requests retain independent exact-ID
  status polling (`DEBUG` while unresolved). A late `SUCCESS` on an archived
  attempt fulfills the initial claim and prevents undisclosed replacement
  broadcasts without closing an already-disclosed replacement's source
  liability.
- **No per-job recovery cutoff:** Replacement requires safe original source
  closure, an unconsumed claim, and enabled funding authority. Malformed
  advisory counters, unknown phase labels, or unreadable polling timestamps
  route to observe-only recovery without blocking canonical confirmation,
  expiry, or settlement (automatic first submission still requires exact
  `PREPARED` and zero prior attempts).
- **Snapshot retention:** Outgoing operations keep their referenced encrypted
  snapshots as immutable audit evidence.
- **Transient read isolation and degraded serving:** Inventory scans isolate
  per-row payload conversion errors when exact job, wallet, pool, and source
  correspondence stays intact, keeping original obligations while siblings
  continue. Transient SQLite busy and locked inventory reads retry on normal
  cadence without replaying financial commits. If treasury state authenticates
  but the Zcash SDK snapshot or funding-only config (sync, fee, indexer, or
  NEAR settings) is unusable, serving falls back to a Store-only Base runtime
  so existing Base wallets keep serving while funding pauses until fixed and
  restarted.

### Funding diagnostics

- First phase logs report saved local state; later transitions report progress.
- Validated NEAR enums, confirmation polls, pending Base credit polls, and
  retryable observation errors log at `DEBUG` (entering deposit confirmation
  and observation unavailable and recovered transitions log at `INFO`).
- Terminal provider outcomes log at `INFO` after persistence with safe parsed
  refund metadata (noting that `REFUNDED` is a provider claim, not SDK
  receipt).
- Operator-required outcomes log at `WARN` once per episode (repeated identical
  checks stay `DEBUG` until cleared).

## Recovery and reconciliation policy

- **Chain and SDK are truth:** Derive monetary state from chain and SDK facts
  rather than parallel local accounting. Repair derived estimates, outcomes,
  and workflow phases without altering immutable accepted inputs, signed bytes,
  or liabilities.
- **Reorg protection:** A shorter indexer height alone never overwrites
  retained SDK observations without a proven divergent retained block. A full
  clear after deep divergence or missing checkpoints resets the effective scan
  floor to Sapling activation while preserving keys, declared birthday, signed
  bytes, and pending source liabilities.
- **Derive identity from signed bytes:** For signed Zcash operations,
  transaction ID and expiry come from the authenticated original bytes; stale
  or malformed plaintext projections cannot delay confirmation or expiry
  recovery or extend broadcast eligibility.
- **No unnecessary gates:** Stale phases, provider amount differences, fee
  deductions, and missing optional corroboration never block authorized work or
  reopen completed outcomes. Never add a financial check without naming the
  authority or liability it protects and confirming reconciliation cannot
  resolve it.
- **Unresolved `PENDING_DEPOSIT`:** Because NEAR 1Click's status contract has
  no terminal `EXPIRED` status, an expired source with an unresolved
  `PENDING_DEPOSIT` swap defers that signer's retirement automatically while
  other funding continues (not a manual-repair warning). See
  [deferred swap timing](plans/deferred/near_swap_timing_tightening.md).

## Refunds, state, and recovery

- **Shielded-only refund receivers:** Every new swap derives and persists a
  unique Orchard-receiver-only Unified Address (Ironwood) atomically with the
  SDK snapshot before quoting. Transparent-address discovery is disabled across
  creation, import, reopen, and reconstruction so NEAR refund receivers are
  never queried via transparent indexer RPCs. Historical transparent or mixed
  refund bindings remain preserved audit records and are rejected for new
  quotes.
- **Managed refund replacement sequence:** A bound `REFUNDED` status closes that
  swap's delivery obligation. Managed funding may archive the attempt and start
  a fresh operation and quote only after **all four** hold: (1) bound `REFUNDED`
  outcome, (2) original source closure, (3) confirmed positive Ironwood receipt
  at the exact saved refund receiver (even if less than deposited or later
  spent), and (4) an unconsumed initial funding claim under enabled funding
  authority. Provider status alone, receipt alone, `FAILED`, elapsed time, or
  unrelated funds never authorize replacement. Direct sends never gain
  replacement authority.
- **No transparent shielding:** Returned Ironwood receipts become spendable
  under normal SDK confirmation rules. Transparent-input shielding and
  `wallet shield-refunds` are unsupported and fail closed.
- **Encrypted storage and backups:** `rotation/store.rs` stores snapshots, EVM
  keys, funding and outgoing journals, roles, calculations, and authorizations
  in one owner-only SQLite database (schema 23), encrypting sensitive records
  with a separate 32-byte ChaCha20-Poly1305 key bound to record identities.
  Wrong keys, corruption, or mismatches never create an empty replacement
  wallet. Back up with `wallet backup` while `serve` is stopped; a Zcash seed
  alone does not recover EVM keys, pending swaps, or payment liabilities.

## Wallet observation policy

Observation scope, cadence, and stop rules belong to the work requesting them.
Intervals apply per wallet, operation, or serialized treasury scan (not a
global RPC rate limit); failed recurring observations back off up to 300
seconds (90, 180, and 300 seconds for treasury scans) without a sibling event
shortening the cooldown.

| Work or trigger | Observation and cadence | Stop rule |
| --- | --- | --- |
| Serving startup (Tor waits on 5s SOCKS check first) | One finite Base pass over eligible managed wallets and active funding recipients; one background ZEC scan when SDK is available | Captured pass finishes; creates no idle recurring watcher |
| New initial-funding preparation | SDK proposal first, then one scalar Base check of that destination (reusing recent acquisition) | Positive credit consumes the claim; advisory RPC error does not veto funding |
| Unfilled enabled reserve claim waiting for ZEC | Shared ZEC scans (60s after completion; 90s, 180s, and 300s backoff) | Stops when no enabled claim can advance and no pending source work remains |
| Accepted initial funding awaiting Base credit | Shared scalar Base recipient check (60s completion floor) | First positive credit permanently finishes the credit lane |
| Recognized signed insufficient-funds `402` | Coalesced asynchronous scalar Base refresh per wallet (60s floor) | Success (including zero) finishes the request; errors retain follow-up |
| Eligible `RETAINED` retirement | Candidate-scoped canonical authorization pages and dual balance proof (60s floor with error backoff) | Retirement or verified `latest` above tolerance finishes demand; unresolved obligations defer checks |
| Validated pending source transaction | Shared 60s ZEC scans; source checks on `poll_seconds` with jitter | Bound NEAR `SUCCESS`, canonical confirmation, or verified finite expiry closes work |
| Submitted swap awaiting NEAR outcome (including archived) | Independent per-operation NEAR status (`poll_seconds` with jitter; 60-300s error backoff) | `SUCCESS` stops; current `FAILED` keeps polling; current `REFUNDED` stops after confirmed receiver receipt; archived terminals stay stopped |
| Current managed `FAILED` or `REFUNDED` awaiting refund receipt | Shared 60s ZEC scans (even when `auto_fund = false`) | Confirmed positive Ironwood receipt at the saved refund receiver stops scans |
| Disclosed native ZEC receive request | Shared 60s ZEC scans (no NEAR status calls) | Confirmed Ironwood receipts reaching the requested amount stop scans |
| Disclosed foreign direct receive invoice | Shared 60s ZEC scans; provider status on `poll_seconds` with jitter | Terminal provider failure or refund stops; `SUCCESS` requests one subsequent completed ZEC scan |
| Explicit `wallet sync` | One ZEC scan, one managed Base pass, and one retirement attempt | Reports results and exits; no background watcher |
| Local SDK publication and reconstruction retry | 60s local retry without network scans | Succeeds or stops on shutdown |
| Fully funded idle serving | No recurring Base or ZEC polling | Resumes when new work requests observation |

## Best-effort wallet rules

Read the [wallet observation policy](#wallet-observation-policy) and
[reconciliation policy](#recovery-and-reconciliation-policy) alongside these
invariants:

- **Advisory balances:** Managed USDC balances and price hints are selection
  estimates only. Old or unavailable RPC observations never block an
  otherwise authorized signature; pool ownership, role, cap, asset and domain,
  and caller authority govern signing.
- **Journaled signing and expiry:** Every authorization is journaled before
  disclosure with signed expiry capped at `min(provider_timeout, 300s)` from
  signing (`validAfter` may be backdated for clock tolerance; expiry never
  permits replay). The journal subtracts the maximum amount once from the
  advisory estimate (saturating at zero); unresolved exposure is never
  subtracted twice. Unrepresentable payment headers fail before journaling or
  dispatch.
- **Narrow insufficient-funds fallback:** Only the
  [pinned x402 insufficient-funds encodings](../tests/fixtures/x402/insufficient-funds.json)
  on a signed `402` earn another wallet attempt (`PAYMENT-REQUIRED` with exact
  `error: insufficient_funds` or failed `PAYMENT-RESPONSE` with that exact
  `errorReason`; `PAYMENT-RESPONSE` always governs when present, and duplicate
  headers earn no retry). The unused error body is dropped immediately, the
  wallet is marked `low_balance`, an async `balanceOf(..., latest)` read is
  queued, and traversal visits each remaining eligible wallet once without
  waiting for reconciliation or funding.
- **Manual raw headers:** Nonempty manual `raw_call` headers stay pinned to the
  initial payer (disabling price handoff and cross-payer insufficient-funds
  traversal while still queueing balance refresh and keeping liability).
- **No top-ups, no replay:** First positive USDC credit consumes the initial
  funding claim forever. Exhausting eligible wallets warns that earlier
  authorizations may still charge; interrupted calls never auto-replay on
  restart. Multiple charges across insufficient-funds fallback and incomplete
  external settlement proof are accepted limitations.

## Qualification boundaries

Offline suites use local fixtures and unfunded keys; synthetic proving and
Docker regtest suites cover Zcash proofs, refunds, expiry, and recovery
separately ([testing](testing.md), [LIFECYCLE.md](../tests/LIFECYCLE.md),
[REGTEST.md](../tests/REGTEST.md)).

Public-swap live runs have exercised funding, promotion, refill, serving during
pending deposits, cross-listener concurrency, and restart; confidential swaps,
managed Permit2 `upto`, and full automated lifecycle and restart qualification
remain outside current qualification scope. See
[qualification status](testing.md#qualification-status) and
[integration technical reference](../tests/live/INTEGRATION_REFERENCE.md#planning-and-concurrency-assertions).
