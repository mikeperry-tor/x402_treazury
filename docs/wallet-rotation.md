# Zcash treasury and managed wallet rotation

See [Wallet selection and network isolation](architecture.md#wallet-selection-and-network-isolation)
for the selected pooling, cover-state and isolated Tor reuse tradeoffs.

A single embedded Zcash treasury funds named virtual EVM wallets on Base. Each
virtual wallet is a durable pool with its own keys, balance reservations, payment
cap, active and retained wallets, and exact small/optional-large reserve claims. Names determine sharing across sources and MCP
listeners; they are not interchangeable labels for one global payer.

Use the [wallet CLI reference](wallet-cli.md#treasury-and-managed-pools) for TOML,
initialization, receive addresses, status, backup and recovery commands. The
[public demo runbook](public-swap-demo.md) supplies bounded operator examples.
This document explains the implementation and its invariants, not permission to
execute a funded test.

## Binding and ownership

`rotation/assignment.rs` resolves explicit source wallets before listener defaults,
then expands optional managed-wallet templates for unassigned bindings. Automatic
scopes are deployment, server, source or binding. Generated names are stable
within treasury state; changing a template's settings does not change existing
pool identity. Changing names or sharing scope can allocate different pools.
`config show` exposes these effective bindings without opening wallet state.

`deployment.rs` initializes the selected static payers and all declared managed
pools. Removing a profile disables its pool while retaining keys, balances and
work. Agent source registration uses an already configured named profile and
cannot create a new pool or choose arbitrary signing credentials.

A process owns the treasury state directory exclusively. A serialized treasury
owner manages zingolib sync, preparation and submission; a bounded blocking store
worker owns SQLite operations. Financial transactions do not hold a database
transaction across remote calls. Accepted actor/store work survives cancellation
of its caller. Independent EVM pools share source funds, not payment reservations.
Encrypted wallet snapshots and outgoing operations commit atomically; snapshot
revision compare-and-swap prevents stale saves. Do not use upstream plaintext
wallet saves or SDK quick-send helpers to bypass this owner and journal path.

## Funding limits and fees

Reserve targets describe USDC requested for new allocations; payment caps and
spendable treasury capacity supply separate authority. The complete syntax and
migration rules are in [managed configuration](configuration.md).

```toml
[funding]
auto_fund = true

[wallets.research]
mode = "zcash_rotation"
max_api_payment_usdc = "1.00"
max_retirement_remainder_usdc = "0.001"
max_funding_spend_zec = "0.02"
max_conversion_overhead_percent = 5

[wallets.research.reserves.small]
funding_amount_usdc = "2.00"
max_funding_amount_usdc = "3.00"

[wallets.research.reserves.large]
funding_amount_usdc = "10.00"
```

| Setting | Meaning |
| --- | --- |
| `reserves.small.funding_amount_usdc` | Required positive target for the small reserve. |
| `reserves.large.funding_amount_usdc` | Optional large target, at least as large as small. |
| Slot `max_funding_amount_usdc` | Maximum accepted output after a bridge-minimum increase; omission permits no increase above that slot's target. |
| `max_api_payment_usdc` | Required explicit cap for each payment by every group wallet, independently of reserve sizes. |
| `max_retirement_remainder_usdc` | Nonnegative automatic-retirement tolerance, default $0.001 per group; it never authorizes transfers or replacement allocation. |
| `max_conversion_overhead_percent` | Integer percentage (0–100) by which quoted USD input valuation may exceed output; the Zcash transaction fee has its own safeguard. |

Enabled automatic funding uses actual spendable treasury funds after confirmation,
reservations, liabilities and transaction fees. Displayed total balance is not
spendable capacity. There are no daily/cumulative USDC allocation or daily ZEC
spending ceilings, and UTC rollover changes no funding authority. Adding spendable
funds supplies capacity without another budget-setting step. Removed aggregate
fields are rejected; historical charges and exact reservations remain intact.

A group cap may exceed its reserve targets, and targets may exceed the cap.
Neither case promises that initial funding can serve every allowed price. The
fresh initial-inventory target is `S + L`, or `S` without large: $2/$10 requests
$12 before accepted minimum increases and fees. Report actual credits, output
ceilings, retained holdings and costs separately. Smaller reserve sizes offer more
cheap-call rotation opportunities; larger sizes support expensive calls but can
keep more traffic on an address. See [sizing and selection limits](configuration.md).

`auto_fund=false` pauses new funding, while independently permitted retirement and
reconciliation continue. Financial completion and current payment usability are
separate: a consumed wallet need not rebuild its original funding balance to close
the accepted source operation. First positive credit permanently consumes the initial claim.
Already accepted work retains its bytes, reservations and original identities
across disablement. A valid successful operation may receive excess input later;
SDK-observed returns become ordinary owned capital without reopening funding.

Per-operation safeguards remain:

```toml
[treasury]
max_funding_transaction_fee_zec = "0.0003"
max_refund_shielding_fee_zec = "0.0003"

# Within an existing managed profile:
# max_funding_spend_zec = "0.006" # One transfer including its network fee.
```

Immutable transaction facts report confirmed principal and actual fees. Shielding
records principal zero and charges only its fee. There is no mutable gross/net cost,
returned-principal credit or historical monetary equality gate. Missing swap
attribution does not invalidate owned SDK funds. Enabled automatic funding also shields confirmed spendable owned transparent
outputs after a successful persisted sync, including unattributed returns. Each
proposal uses one address and the actual standard fee under
`max_refund_shielding_fee_zec`; the SDK determines confirmations, maturity and input
availability. Selected inputs must exceed the actual proposed fee before accepting
input ownership: the pinned SDK permits fee-only proposals, which cannot recover
capital. Uneconomic/empty addresses do not block independent addresses. No
principal reservation, attribution requirement or funding-claim reopening is added.
The explicit per-address command remains available.

Zingolib selects inputs and calculates standard network fees. The application
checks the actual proposal against configured per-operation fee/spend safeguards,
then persists the exact input owner before calculation. Backend calculation
borrows that same public proposal and retains ordinary consensus expiry, including
the SDK's NU7 delta, rather than the offline high-level API's epoch-end expiry.
Sapling proving uses lazily parsed embedded parameters. Proposal cleanup happens
after releasing the wallet guard; accepted input ownership remains durable.
Canonical actor validation supplies the network/anchor check without an additional
connection for local calculation; direct preparation keeps its network validation.
A configured maximum is a
rejection ceiling, not a capital reservation. The serialized SDK owner retains live
calculations and completed results until publication. The preparation phase commits
with accepted input evidence; known proposal shortages and fee rejections leave the quoted operation
unstarted without consuming transaction recovery attempts. The
SDK owns current balances, confirmations, witnesses, spent outputs and chain
corrections. No full historical source/refund traversal gates publication.

The bridge's USD valuation is not an independent exchange-rate oracle. Reserve output
and conversion-overhead ceilings do not replace a per-operation ZEC safeguard when
one is required. All financial limits still apply alongside qualification permits;
no configuration value authorizes replay or bypasses a zero-new-funding restriction.
Positive funded qualification still requires an explicit `max_funding_spend_zec`
to bound its native-asset permits.

Treasury command and sync drivers retain their cleanup owner when callers disconnect.
After an unhealthy SDK mutation, they discard that SDK and reconstruct it from the
latest committed snapshot through the same serialized Store. Queued commands remain
queued; an interrupted command is never replayed. Accepted input evidence, funding
restrictions and Base handles remain preserved. Successful offline reconstruction
can close an unpublished calculation whose outgoing and factual transaction journals
are both absent. Completed bytes take precedence:
only identical local publication repeats on the existing 60-second cadence, including
shutdown. SDK reconstruction also retries unavailable snapshots on that cadence.
Exited funding and reconciliation tasks reacquire their existing Store/group gates
on their ordinary polling cadence; retained source work keeps its existing lease
and backoff state. Explicit shutdown never starts a replacement. An unavailable
shared Store or authentication state can still require restart; listeners and usable
Base payers remain independent. Forced process death or irrecoverable storage can
lose unpublished bytes; the durable input claim still prevents duplicate calculation.

## Treasury sync coverage

Scan starts log at DEBUG; successful persisted completion logs at INFO. Long
scans also report saved progress at INFO every 30 seconds. These INFO messages
show only the saved scanned height; tip and block-count diagnostics remain in
status and DEBUG output. A new spendable-ZEC shortage reports the treasury deposit
address, asks the operator to add ZEC and allow confirmations, and explains that
synchronization and funding retry automatically while demand remains.

Recurring `treasury sync unavailable` warnings include a broad `category` and,
for Zingolib scan/launch failures, a fixed `reason`. Indexer request failures also
include numeric `grpc_code` and a fixed `grpc_status`, for example
`reason="indexer_request_failed" grpc_code=14 grpc_status="Unavailable"`.
Code 4 (`DeadlineExceeded`) distinguishes a request deadline from code 14
(`Unavailable`); neither alone establishes Tor as the cause.
Other reasons distinguish tree-size mismatches, missing checkpoints, invalid
indexer data and mempool shutdown timeouts. Shard-tree errors identify the precise
variant: for example, `shard_root_conflict`, `shard_checkpoint_pruned`,
`shard_checkpoint_out_of_order` or `shard_tree_incomplete`. These describe the
failed tree operation, not its root cause; they do not by themselves establish
wallet corruption or authorize a reset/rescan. Returned sync-engine tree failures
also name `pool` (`sapling`, `orchard`, `ironwood`) and `operation`
(`scan_merge`, `reorg_rollback`, `pool_recovery_rollback`). These fields survive in
CLI errors and saved status, so a failed retry need not be reproduced just to
recover the context. Tree addresses, note positions, upstream messages, metadata
and wallet identifiers are omitted. These diagnostics do not change retry or funding
policy; a successful sync is still required before new treasury preparation.


The Zcash treasury is exclusively spent by this application. Synchronization calls
the public Pepper engine on the existing isolated session runtime. One spawned
engine task owns scan progress while periodic encrypted checkpoints wait for the
wallet lock; successful scans capture migration witnesses through the public SDK
API. Cancellation tears down that runtime and joins its workers before the final
snapshot. Task failures retain fixed diagnostics without panic payloads. Direct and Tor sync
retain completed work without a local age ceiling or ordinary three-block lag
policy. SDK proposal selection enforces actual confirmations, witnesses, spend
links and scan coverage. Snapshot revision and the serialized actor bind mutation
to its owning SDK state. Cancellation cannot turn an interrupted scan into a
completed observation. An unchanged completed scan can be reused within its owner
after checking its exact canonical tip height/hash; restart or chain changes scan
again for an actual preparation or submission. Scheduled source observations share
the last successful current-owner scan without fresh tip probes between due scans;
see [wallet observation policy](#wallet-observation-policy).

Status keeps `target_height`, scanned `height`, final `observed_tip_height` and
the original `checked_at` for diagnostics. Completion never relabels observation
time. Confirmation and expiry decisions use scanned coverage. Expiry recovery
requires one fully completed scan to reach its observed canonical tip and validate
the authenticated original transaction and its buried signed expiry. Final publication checks the same Ready snapshot revision and coverage.
No host clock age or ordinary lag allowance supplies that proof.
`max_sync_age_seconds` and `base_max_block_age_seconds` are removed settings;
configuration explicitly rejects them. Base balance and receipt verification have
no block-age or host-future gate, and receipt logs use the existing byte bound.


Base observations and historical authorization resolution provide reporting facts.
They do not certify affordability or grant signing permission. Unavailable balance
or nonce reads do not prevent a permitted payment; existing attempt records remain
owned even when their outcome is unknown.

## Capacity and admission

A group has one default `ACTIVE` identity, `RETAINED` wallets with spendable
remainders, and one `READY` wallet for each fulfilled enabled reserve claim.
Initial and small-reserve allocations use the explicit small target; an optional
large reserve has its own target. Each allocation commits independently. A failed
large fill cannot roll back successful small funding. Destination credit makes a
wallet usable independently of source accounting and provider completion.

The funding adapter validates an exact-output quote and accepts a bridge-minimum
increase only within that slot's explicit `max_funding_amount_usdc`. Original
allocation targets remain immutable, accepted quote terms remain bound to their
operation, and configuration changes govern new work. Actual spendable treasury
funds, outstanding liabilities, per-operation safeguards and explicit qualification
permits govern funding. Reserve targets do not cap individual API payments.

Each wallet has one initial funding claim. Its first positive observed Base USDC
balance consumes that claim, even below the target; later zero or insufficient
balances never reopen it or authorize a top-up. The saved scalar is an advisory
estimate, without historical block certificates. Existing claim generation and
operation bindings publish the wallet role atomically with the first credit.
Disablement preserves observation of accepted work and detaches a fulfilled reserve
into RETAINED inventory; it forbids a new deposit. Known prepared source bytes keep
the destination from automatic retirement until their original outcome is resolved.
A partial credit never releases those source inputs or permits another deposit.

`rotation/manager.rs` admits supported Base USDC v1 and v2 exact EIP-3009 offers. It
validates the envelope, domain, amount and configured challenge-policy cap before
acquisition. Payment preparation checks the current Store cap before retrieving
the signing key, and the authorization journal rechecks it before disclosure.
The local v1 SDK adapter corrects protocol-version metadata in the pinned client's
candidates; signing remains in the SDK and the signed payload must still match
the selected protocol, payer, payee, amount and validity. V1 uses `X-PAYMENT`;
v2 uses `PAYMENT-SIGNATURE`. Standard body-only v1 challenges are normalized at
the same bounded response boundary used for header challenges.
Advertised extensions outside the managed payment contract are omitted
with DEBUG diagnostics during challenge preparation; this does not establish
signing, submission or the cause of a later failure. Seller rejection remains
possible. Terminal payment-call failures are WARN events; see
[logging and diagnostics](development.md#logging-and-diagnostics).

Selection uses the saved wallet estimate as described in the
[best-effort wallet rules](#best-effort-wallet-rules). It performs no balance RPC,
nonce scan, historical readiness validation or certificate publication. Price hints
can choose an existing identity before its own unsigned challenge is fetched.
Wallets with malformed stored EVM addresses are skipped before transport construction;
healthy sibling identities remain eligible. Unsigned selection is read-only: it
neither promotes a retained wallet nor consumes
a reserve claim, and it does not acquire the payment gate. `selection_amount` is
only a ranking input. Actual challenge amounts and current ownership/caps are
checked by payment preparation and the authorization journal; balances are not
spending permission. No additional hint or authority state is maintained.

Runtime hints expire 24 hours after the unsigned response was received and
retain at most 65,535 fixed-size entries per process. Their lifetime shares the
API catalog disk-cache default; per-source catalog overrides remain independent.
See [local resource limits](local-resource-limits.md). Their trusted key includes
source/revision, method/OpenAPI template, wallet group and transport scope; request
arguments cannot choose that scope. Receipt order controls updates, including lower
prices. Restart, expiry and eviction lose only a selection estimate. They cause
no probe, cached-challenge reuse or new payment authority.
A valid observed offer can update a hint even when current Store authority refuses
financial admission; that hint grants no permission to sign.

The invocation captures its existing inventory boundary. It may hand off an unsigned
challenge to an eligible payer suggested by the actual price, and may traverse on
the narrow insufficient-funds exception below. Each identity is visited at most
once. Each handoff creates that payer's own isolated transport and unsigned
challenge while preserving the original request bytes. Within each payer stage in
Tor mode, unsigned challenge acquisition may try
`1 + network.tor_isolation_retries` isolation scopes (default 3) on
HTTP `400`, `403`, `408`, `429`, `500`, `502`, `503`, or `504` (honoring valid `Retry-After` headers on `429`
and `503`) with that same pinned payer—never switching wallets, allocating,
funding or waiting for balance reconciliation during the unsigned retry loop—and
conditionally promotes the winning scope after valid challenge validation before
signing and signed submission on that exact pinned record.

Signing produces private bytes. The journal transaction rechecks current pool
ownership, eligible role, actual reserve claim, amount cap, payer address and signed
validity. It records the possibly submitted authorization and decrements the
current advisory estimate once. Concurrent estimate replacement or READY-to-ACTIVE
promotion does not invalidate another permitted authorization.

A READY payer consumes its exact reserve claim atomically: the former active
becomes RETAINED, the payer becomes ACTIVE, and the reserve generation advances.
Ordinary background funding can fill the vacancy. Admission never allocates or
retires wallets. An expired unpublished signature fails before disclosure; no
local evidence/signature recovery loop remains. Unknown journal outcomes grant no replay.

The serialized Store owns commits after waiter cancellation. Signing and network
I/O do not hold a group turn to stabilize advisory balances.

## Automatic retirement

Ordinary serving and finite explicit sync consider only eligible RETAINED wallets
in enabled groups for canonical retirement. Low saved estimates select candidate
work; they neither prove retirement nor exclude an otherwise permitted payer.
Retirement requires compatible confirmed and latest balances. Both must be at or below
`max_retirement_remainder_usdc`: the recorded bound is their maximum, not their
minimum. The default `"0.001"` permits at most 1,000 atomic units per wallet;
each group can override it, and explicit `"0"` requires both observations to be zero. Pending
payment exposure, a current enabled reserve claim, an unfulfilled accepted source calculation, pending signed source operation or conflicting canonical retirement evidence
prevents retirement. A prior partial credit does not close a still-pending NEAR
delivery obligation. Never-credited wallets remain protected while any original
delivery obligation is unresolved, including abandoned reserve predecessors whose
source transaction is confirmed. Source confirmation does not prove destination
delivery or permanent loss. For a completed funding job, each current/archived
possibly disclosed swap must have its exact bound NEAR terminal outcome, and no
unfulfilled accepted calculation or pending source transaction may remain.
Proven nondisclosure needs no bridge terminal result, including a privately held
signed transaction after verified source closure. Source expiry alone does not prove terminal bridge
outcome. Missing original provenance retains the guard. Closure creates no credit
or funding authority; possible late credit stays attached to the original identity.

Candidate work pages only that wallet's original outstanding authorizations, then
publishes its dual balance proof through the existing canonical transaction. A
used nonce or confirmed-chain-time expiry with an unused nonce resolves its
original authorization; local time, seller receipts and request failures cannot.
More than 64 authorizations continue through pages without a total-history cap or
reads of unrelated wallets. Publication rechecks current role, reserve claim,
source obligations and exact evidence after remote acquisition.

Retirement records exact amounts, anchors, endpoint and supporting evidence in the
same transaction that changes the role. It removes the wallet from routine balance
polling and payment selection; keys, funds and historical authorization work remain.
It sends no transfer and creates no replacement allocation. A later contradiction
marks the saved retirement disputed without silently reactivating the wallet.

The configured tolerance supplies ordinary retirement authority independently of
`auto_fund`, treasury capacity and reserve vacancies. Qualification restrictions
can deny that mutation; funding-only permits do not grant it. Positive tolerance
accepts stranded purchasing power per retirement and has no cumulative loss cap.
Status reports recorded amounts rather than claiming the funds were spent. Manual
retirement and incident-specific repair remain [deferred](plans/deferred/managed_wallet_followups.md#manual-retirement-and-incident-repair).

## Payment reconciliation

Managed `PaidClient` calls may repeat an operation on another captured wallet only
when the signed attempt receives a supported structured insufficient-funds failure.
Both authorizations can charge or execute. Other HTTP failures, transport loss,
malformed replies, successful responses and output conversion failures grant no
retry. The low-level `ManagedPool::pay_with_cover` API dispatches an already-bound
challenge/transport once; use `PaidClient::managed` for complete routed calls.
Settlement receipts take precedence over simultaneous insufficient-funds challenges.
`X-PAYMENT-RESPONSE` (v1, network `base`) and `PAYMENT-RESPONSE` (v2, network
`eip155:8453`) share one ambiguity boundary: duplicates, including both names
together, grant no retry. Qualification normalizes valid legacy receipt networks
to `eip155:8453` for existing canonical-debit recovery; seller claims remain
unverified until the chain checks succeed.

`rotation/base.rs` verifies Base chain identity, finalized anchors,
block-pinned USDC balances and EIP-3009 authorization use. Its confirmed anchor
is the lower of the provider's finalized height and the latest height minus
`funding.base_confirmations` (default 12). Both finality and configured depth must
hold; depth alone does not establish finality. Missing or inconsistent finalized
evidence retains the existing observation retry and configured fallback behavior.
A used authorization
or an expired authorization proven unused can release its reservation through the
store's canonical reconciliation transaction. An unused but still-valid signature
remains a liability. Receipt inclusion must satisfy the same finalized/depth bound.

Changed canonical history invalidates affected derived evidence. In one Store
transaction, reconciliation archives the current authorization resolutions,
reopens their original attempts as potentially submitted, rebuilds their exposure,
clears derived financial/debit checkpoints and invalidates old captures. Existing
candidate pages then prove the original nonces against the current chain. An
unused signature that is still valid becomes an outstanding liability again;
expiry requires the current confirmed chain time. Historical proofs remain
available for inspection rather than becoming permanent compatibility gates.
Malformed derived resolutions encountered during proof publication are rebuilt
through the same path, without a routine scan of lifetime resolution history.
Archived resolution and balance-support rows retain their exact audit bytes but
never veto fresh canonical proof, including well-formed old contradictions.
Retirement history does not force indefinite revalidation of old block anchors.
Current authorization bindings, generation fences and debit ordering still
protect publication; an active cached contradiction invalidates its scope for a
fresh observation. When confirmed/finalized height is unchanged, newer latest
height still supersedes an older dual balance observation. A late older callback
cannot replace that newer latest balance or make retirement eligible.
Disputed retirement stays retired. This correction grants no payment replay,
replacement deposit, reopened initial funding claim or signer reactivation.

Ordinary advisory refresh performs scalar unsigned latest `balanceOf` reads.
Eligible retained-wallet retirement separately acquires candidate-scoped canonical
authorization and dual balance proofs. It does not run qualification's full-pool
historical traversal. Historical balance, exposure and repair fields do not gate
current signing; advisory RPC failure policy is unchanged.
Retired wallets remain retired. Original attempts and canonical proof records stay
readable without a historical capacity-repair pass.

## Funding and source safety

Allocations and their exact operation journals commit atomically. Inventory reads
never create missing journals or substitute operation IDs; missing provenance
requires repair. The runtime funding owner schedules work through captured,
paged inventory and independent observation lanes.
The funding owner reconciles previously reopened source work from a bound NEAR
SUCCESS before choosing a lane, including completed jobs. A delayed destination
read cannot keep that source work open or block unrelated funding.

`rotation/funding.rs` drives the durable outbox through the states defined in
`rotation/store/funding.rs`:

```text
ALLOCATED → QUOTED → PREPARING → PREPARED → DEPOSIT_PENDING
                                                 ↓
                                              SWAPPING
                                                 ↓
                                         VERIFYING_CREDIT → COMPLETE
```

A confirmed credit path can skip the separate swapping observation. These phases
describe source progress; they do not serialize independent observations. Refund and
ambiguous outcomes retain `REFUND_PENDING` or `RECOVERY_REQUIRED` evidence; they
do not discard the candidate or automatically create another transfer. The saved
operation ID and recipient remain stable across restart.

`rotation/near.rs` validates token metadata, quote bindings, confidentiality mode,
amounts, deadlines and cost limits. Public swaps are explicitly selected and were
qualified without partner authentication. Confidential settings exist but funded
authenticated confidential execution remains unqualified; there is no fallback
from confidential to public mode. Current requests do not add application
commissions. Do not infer a commercial partner agreement from an API key.

The SDK selects available confirmed inputs and calculates standard fees. The
application checks exact output/fee and current operation/run permission, then
journals the exact selected inputs before calculation. The serialized SDK owner
prevents overlapping calculations. Completed snapshots and exact signed bytes
commit atomically; failed publication retains the identical completed result and
does not grant another calculation. A pending signed original remains a durable
source-work fence. Historical calculation intents are preserved evidence rather
than an independent global capital reservation. A full SDK rescan can discard
unconfirmed calculated transaction records and their input reservations. The
retained signed original therefore keeps its source-work fence until closure;
actor serialization alone cannot prevent later input reselection after that reset.
This protects genuinely unfinished work without auditing completed transactions.

Production calculation is offline. Broadcasting requires committed outgoing bytes
and factual transaction metadata before the Store can grant disclosure authority.
After successful exclusive offline SDK reconstruction, with no completed result
still owned, a funding calculation with neither journal can close automatically.
The original accepted inputs remain encrypted under the original operation ID;
existing recovery archival records the never-disclosed reset. Another calculation
uses a fresh operation ID, current claim and funding authority, and SDK spendable
funds. A jobless shielding calculation uses its normal fresh identity. This local
recovery needs no blockchain absence proof or extra observation. An outgoing
commit with a lost acknowledgment stays owned, and a factual transaction record
without its outgoing journal is corruption, not proof of non-disclosure.

Bound NEAR SUCCESS independently closes source work. Its preserved calculation
intent cannot block unrelated work or retirement. The serialized SDK owner still
journals any live calculation's late result before processing its next mutation,
without disclosing a new deposit.

A returned quote deadline that has actually expired prevents a new preparation or
first submission. Untouched quote refresh requires proof of no accepted calculation
and no outgoing bytes, then archives the old binding without consuming a signed
transaction recovery attempt. No local five-minute margin applies.

Recovery eligibility comes from the original operation and its bound closure
archive, with liabilities rechecked before reuse. Saved error messages grant no
recovery permission. Startup obstructions use current typed observations and current
funding restrictions; reopening does not turn historical diagnostic prose into a
new policy obstruction.

An uncertain broadcast retains the saved bytes and transaction identity. Recovery
reconciles that operation; explicit rebroadcast uses the same bytes. A timeout or
indexer omission does not prove non-submission. For an unfinished source operation,
confirmation and expiry use completed canonical SDK observations at the configured
depth. Base credit alone cannot prove source confirmation.

Submission counters and status labels do not establish the transaction's monetary
effects. Observation treats malformed counters or unknown labels as possibly
disclosed, preserving the original columns while canonical confirmation, finite
expiry or exact bound settlement can resolve the work. Immutable transaction
facts and signed bytes still require authentication. Inspection and disclosure
remain strict: automatic first submission requires the original PREPARED label
and a valid zero counter. An observation fallback never grants submission or
rewrites missing history.
Saved quote-observation counts, polling times and diagnostic health fields are
also advisory. Their decoding cannot stop runtime recovery or authorize spending;
unreadable values use ordinary unknown/default scheduling observations without
rewriting fields merely on read. An unrecognized runtime phase routes to the
existing recovery path; it does not authorize a new calculation, satisfy a phase
mutation guard or invent a replacement state. Full inspection still reports
malformed stored data. These fields are separate from original source disclosure history and the
configured recovery authority enforced through archived operations.

NEAR owns cross-chain swap settlement. A validated SUCCESS for the exact accepted
quote/deposit binding closes local source work, without requiring local Zcash
confirmation or independently proving the bridge's destination transfer. Original
bytes, inputs, submission attempts and the last observed source status remain
available; settlement does not fabricate a Zcash CONFIRMED label. An already
accepted calculation finishes journaling its exact result as resolved. No new
calculation or disclosure starts for that fulfilled operation, and already accepted
network I/O drains under its original owner. Restart applies the same rule to
previously retained pending rows. Later Zcash reorgs do not reopen settled swaps.
The SDK independently repairs chain state and determines current spendability.

Destination readiness remains separate: the first positive Base USDC observation
consumes the initial claim and can publish the usable wallet before settlement.
This scalar is advisory readiness, not finalized transfer evidence. Bound provider
SUCCESS plus that initial credit completes the existing funding job. A successful
operation awaiting credit remains in VERIFYING_CREDIT without source-chain work.
Earlier negative reports cannot veto success or reopen the claim. FAILED/REFUNDED
ends bridge-status observation; a genuinely pending original source transaction
keeps its own confirmation/finite-expiry observation. Unknown, processing and
incomplete bridge outcomes remain unresolved automatic waits; they do not
manufacture an integrity error or operator-review requirement. Source expiry or
receipt of returned coins cannot by itself prove the bridge has no delivery
obligation. A confirmed deposit with unresolved settlement waits for that outcome.
Verified finite expiry can permit a fresh attempt under current unconsumed claim,
funding, fee and recovery authority and actual SDK spendable funds. The old NEAR
request remains independently observable until its own terminal outcome.

Status observations retain each accepted request's exact operation and saved quote,
including archived attempts after the funding job changes identity or completes.
Their scheduler identity contains the accepted job, operation and wallet identifiers,
without inventing a funding phase or borrowing replacement health. They share the
existing independent status lane and cadence; one slow old request
cannot serialize the newer request's observation. A late bound SUCCESS fulfills
the same initial claim and prevents any replacement deposit not yet disclosed.
It does not rewrite another operation's provider result or close that operation's
already-disclosed source liability. Initial credit can finish the job regardless
of which attempt was current when the scalar observation was saved; unresolved
external requests and disclosed originals keep their existing observation owners.
Each actual status lookup for a retained unresolved request emits DEBUG explaining
that its final outcome is still unknown and additional checks continue. The log
identifies the original job, operation and wallet; it does not log provider prose
or signed data. Admission deferral and ordinary current-swap polls remain DEBUG.

One retained funding owner schedules independent status, credit and prerequisite-
complete unsigned quote observations in bounded lanes, alongside serialized treasury
commands. Per-operation/kind backoff cannot stall another group's observations.
Advisory reads cannot restore a consumed initial claim. Completion storage failures still
stop the shared owner; rejected proofs never release liabilities or authorize retries.
Historical work uses captured boundaries and keyset pages, not full reporting
snapshots; wallet count is not a deadline. Read-only readiness and quote prerequisites
precede proposal mutation or reservation. The same treasury actor and worker survive
bootstrap into serving, and accepted work drains after waiter cancellation.

The scheduler's response waiter does not own the funding lease. Accepted mutable
source work retains it even if observation publication fails and the scheduler
returns an error; another worker cannot enter until that source task finishes.
Once accepted source work has drained, stopping drops any remaining unsigned
observations without accepting their results or consuming transaction recovery
attempts. A single pass uses its captured
inventory boundary; continuous observation lanes can refresh their boundaries
independently while source work remains occupied. Focused scheduler tests use
explicit completion barriers and the durable store to exercise these boundaries;
they do not exhaust all possible async interleavings.

Funding logs distinguish progress from failed checks. Entering deposit confirmation
emits an informational message; ordinary confirmation-depth and pending Base-credit
polls log at debug level, clear stale error status and use the normal polling interval.
They do not release reservations or authorize another deposit. Retained retryable
observation failures (including sync, RPC and storage failures) and retained-owner
reacquisition attempts log at DEBUG with existing retry behavior. Meaningful transitions into delayed or degraded funding
belong at INFO; operator-required outcomes belong at WARN and explain the required
review while observation continues. Repeated observations of the same operator
requirement log at DEBUG even when height details change; successful observation
clears that diagnostic episode. Repetition alone does not raise severity; see
[log severity](development.md#log-severity).

Source confirmation uses the completed SDK scan and computed transaction identity
bound to the original journaled bytes under the same owner. An SDK-confirmed transaction
at the configured depth completes source reconciliation without an additional
indexer lookup or agreement with stale, different or unknown indexer reports.
Canonical SDK records match the transaction's committed effects, allowing valid
v5 authorization-data variants; the original disclosed bytes remain unchanged.
See [SDK transaction identity](zingo-integration.md) for the ZIP 244 boundary.
Completed source work is not re-audited or reopened after restart or SDK chain
replacement. Schema-22 historical source dispositions and referenced snapshots
remain readable audit evidence; they do not drive a lifetime reconciliation pass.
For records reopened by an earlier implementation, the existing read-only history
guard still forbids automatic disclosure of previously closed originals. Actual
unfinished originals keep their existing owner and exact journals.

Source confirmation disagreement has the same classification in ordinary
reconciliation and recovery. Before the SDK establishes sufficient confirmation,
an unconfirmed wallet record is pending when its
known highest scanned block is below the lookup's inclusion height: the lookup
can observe a new block after the wallet scan. This is an ordinary confirmation
poll at DEBUG, not an operator requirement. Unknown scan progress, an unconfirmed
record after scanning the inclusion height, and unequal reported heights retain
the original work for fresh observation. A sufficiently deep coherent SDK record
with the original computed transaction identity completes confirmation without
requiring another lookup or agreement with its reported height.
Diagnostics retain the
lookup confirmation height, wallet confirmation height (or `unconfirmed`), and
highest scanned block (or `unknown`); these facts do not establish a root cause.
Entering a recoverable canonical-disagreement observation emits INFO with the
safe rejection context. Unchanged checks and ordinary confirmation-depth waits
remain DEBUG; a disagreement alone does not require operator repair.
Existing work, inputs and reservations remain owned, and observations continue on
their existing schedule. An observation discrepancy grants no replacement deposit;
source confirmation still requires the configured depth. The SDK's upstream 100-block
rolling history is a synchronization recovery horizon, not consensus finality.
A deeper fork requires a full SDK rescan rather than a permanent historical gate.
Missing required SDK block boundaries and pruned rollback checkpoints use that
same automatic rescan, including for already-persisted interrupted state. These
derived-cache failures do not imply lost original transaction provenance.
A full clear without a proven common ancestor moves the effective scan floor
back to supported activation, including when the new tip remains above the
declared birthday, so earlier replacement credits are discovered. Existing scan ranges preserve that
floor through interruption and reopen without changing the declared birthday,
keys or accepted operation journals; see [SDK chain recovery](zingo-integration.md).

Bootstrap and the serving funding worker recover eligible jobs automatically.
The operator can run `wallet recover --config FILE` with serving stopped for one
pass through the same classifier, without starting funding or submitting transactions.
Reports distinguish recovered, waiting, failed checks and operator-required cases.
Unpublished calculations recover at the exclusive reconstructed SDK owner;
missing immutable records required to resolve genuinely disclosed work still
require scoped review before replacing that funding attempt. A confirmed source with an
unresolved swap remains automatic pending work for the existing provider/credit
lanes; a refund alone cannot authorize a replacement deposit. Ordinary discovery and shielding
of owned outputs can continue under the current funding authority.

A prepared deposit retains its exact bytes and input owner when its external quote
deadline expires. Local source closure uses its authenticated original signed
transaction, finite consensus expiry, and one fully completed canonical SDK tip
scan beyond expiry plus the configured confirmation depth. A canonical confirmed
original follows ordinary confirmation instead. A source observation that completed
that scan in the same command also supplies the expiry proof, including the
treasury maintenance pass immediately following a successful scan; a cached or
failed observation does not replace the fresh scan. The final Store publication checks
the same unchanged Ready snapshot and coverage. No current-input equality,
unspent-input inventory or alternative-spender proof is required: the SDK already
expires pending transactions, releases only their own spend links, and owns actual
spendability. Missing or relocated old inputs do not create another local financial
liability. Original bytes, accepted inputs and snapshots remain preserved.

Expiry height N allows inclusion through N. Production deposits use ordinary SDK
expiry (40 blocks before NU7, 120 afterward), and shielding uses the finite backend
default. A genuinely retained zero-expiry original cannot use this expiry path;
production does not construct one. Submitted/uncertain operations are observed,
never automatically replayed. Closing source work does not close an unresolved
bridge operation. A fresh attempt after expiry requires the original's stored
canonical closure proof, not an EXPIRED label alone, plus current unconsumed claim,
funding, fee and recovery authority. An unresolved NEAR request does not veto that
source-expiry proof; it retains its own status observation. Returned coins are
ordinary SDK funds, without per-job refund attribution.
The profile's `max_attempts` bounds transaction recovery resets using durable
history; neither process restart nor operation replacement erases this count.
Replacement checks this allowance once, after idempotent archival in the Store
transaction. An archived original confirmed through the canonical-proof/Store
boundary (including compatible library or legacy state) does not consume another
entry for the same operation. Ordinary reconciliation remains pending-only;
scanning a replacement chain does not reopen completed historical source journals.
Disabled automatic funding is an ordinary wait after safe source closure, not a
manual-repair warning; a later enabled configuration can resume eligible work.
An exhausted recovery limit or qualification restriction still requires its
separate authority before a new attempt.
Proven never-disclosed resets, including untouched quote refreshes and unpublished
calculations closed after offline reconstruction, are marked in durable history
and excluded from that count. Historical unclassified records still count. Earlier binaries reject the newer state schema.
Unsigned quote outages use bounded per-pass work and backoff without consuming
transaction recovery attempts.


The treasury receives/synchronizes Ironwood with capability checks on the injected
lightwalletd transport. All workspace packages use ordinary upstream Zingolib/netutils
sources; the application's public-trait RPC adapter preserves the factory transport
without SDK-owned direct networking. Direct Pepper sync retains its reviewed
progress/drain and diagnostic patches. The
[egress inventory](network-egress.md) explains treasury versus immutable recipient
identities and the remaining upstream timeout boundaries.

## Recovery and reconciliation policy

Financial state supports recovery of accepted work and current spending authority.
SDK/blockchain state owns actual funds, spent inputs and chain corrections.
NEAR owns settlement of the exact accepted cross-chain swap; source reorgs do not
reopen its validated successful outcome. Use
authoritative observations to repair derived estimates, outcomes and workflow
phases, and finish work whose outcome is established. Preserve immutable accepted
inputs, signed bytes, original identities and liabilities while doing so.

Reuse existing validated observations, exact operation bindings and confirmed
journals. Acquire a missing fact only when it can advance the affected operation,
through the existing network/SDK owners and observation policy. Do not add fresh
agreement checks, historical sweeps, refund watchers, cross-chain accounting
ledgers or redundant balance certificates to certify an already established
outcome. Failure to acquire optional corroboration must not invalidate that outcome.

Stale phases, provider amount differences, fee deductions and missing optional
attribution are reporting or reconciliation matters. They must not create
financial-repair blockers, pause otherwise authorized payments or funding, or
reopen proven completed work. For these derived observations, use the actual amount
without requiring equality to a quote, target, provider report or locally calculated
refund total. This does not relax quote acceptance or signing authority.
The first positive USDC credit consumes its initial funding claim even below target.
An empty refund address after shielding, or later spending of recovered funds,
does not invalidate historical recovery. Refund receipt and shielding must not
become additional prerequisites for an independently established terminal outcome.

Keep uncertainty attached to the actual unresolved obligation. Preserve current
authority checks, journal-before-disclosure, accepted input ownership, canonical
unfinished-source confirmation/finite-expiry recovery and unresolved payment authorizations. Receipt
or shielding of a refund does not prove non-submission or absence of future USDC
delivery, release another attempt's inputs, or authorize another deposit. Genuinely
conflicting execution/ownership evidence or unrecoverable required facts warrant
scoped investigation; they do not justify halting unrelated work.

Before adding a financial gate or consistency check, identify the concrete
authority, liability or external contract it protects and existing enforcement.
First assess whether correcting derived state or removing duplicate responsibility
resolves the problem. Any necessary check must preserve useful recovery and the
affected operation's ownership without requiring agreement among advisory records.
Regression coverage should show established recovery completing, genuine
liabilities remaining owned, and independent work continuing without redundant I/O.

Ordinary retirement preserves unresolved original delivery obligations. A
never-credited completed job needs each possibly disclosed current/archived swap's
bound NEAR terminal outcome and closed local source work. Proven nondisclosure
has no bridge obligation; any privately held source work must still close.
Completion, abandonment or source expiry/confirmation
alone does not establish bridge settlement. Observed returned coins alone are not
a bound provider refund outcome. No credit or consumed claim is fabricated.
An expired source with an unresolved `PENDING_DEPOSIT` request can therefore defer
signer retirement indefinitely while other funding continues. This is automatic
pending observation, not a manual-repair warning. The [1Click quote contract](https://docs.near-intents.org/api-reference/oneclick/request-a-swap-quote)
defines its deadline as the start of refund handling; the [status contract](https://docs.near-intents.org/api-reference/oneclick/check-swap-execution-status)
provides no terminal EXPIRED result. Neither a local deadline nor additional Zcash
blocks can manufacture that service's final delivery outcome.

## Refunds, state and recovery

Each swap uses a persisted unique external transparent refund address derived
sequentially from treasury account zero. Snapshot recovery retains these addresses;
mnemonic-only discovery can miss later receipts across unused address gaps, so a
seed alone does not replace the complete backup. Refund visibility does not make
funds automatically spendable for another deposit. Serving and bootstrap with
enabled automatic funding attempt per-address shielding
through the existing treasury owner. `auto_fund=false` and qualification restrictions
prevent new shielding; existing prepared bytes remain observable. Positive
qualification funding permits authorize jobs only and do not authorize automatic
jobless shielding. Explicit shielding remains available while serving is stopped,
with its original job recipient network binding. Unattributed automatic shielding
uses a stable per-address/network/treasury identity. Both paths retain observed
inputs, fee caps and exact prepared-byte recovery.
The remaining `funding_refunds` records store previously derived addresses for
quote routing and explicit per-job administration. They store no returned amount
or spendability. Automatic shielding does not consult refund attribution or
provider status: the SDK discovers owned coins and determines when they become
spendable. A return does not reopen an initial funding claim, since it cannot prove
the swap will not also deliver USDC.
The pinned SDK's `WalletWrite::store_transactions_to_be_sent` scans calculated
transactions with the wallet's viewing keys. Validation requires the exact selected
outpoints, no transparent outputs and decrypted account-zero shielded notes equal
to inputs less actual fee. Check all supported shielded pools: the SDK may promote
change from Orchard to Ironwood after an upgrade.
The SDK owns return outputs, spent state, fees and chain corrections. There is no
local refund credit, principal cap or historical refund/source revalidation gate.
Source confirmation remains a distinct audit fact. A refund or stale workflow phase
cannot erase an uncertain input owner or authorize a replacement deposit.

`rotation/store.rs` keeps treasury snapshots, EVM keys, funding/outgoing records,
roles, accepted calculations and payment authorizations in one versioned database.
Sensitive records are authenticated-encrypted with a separate 32-byte key and
record identity bindings; public metadata still links addresses and operations.
The directory, key, database, ownership lock and SQLite sidecars have filesystem
protections. Wrong key, corruption or identity mismatch never initializes an empty
replacement wallet.

Back up while serving is stopped. The consistent database and key must be synced
before atomically publishing `backup.json`; directory sync is also required for
reported success. A failed backup destination cannot be silently overwritten.
Seed-only recovery does not recover EVM keys, pending swap bindings or payment
liabilities. Protect the whole backup as spending material.

`wallet status` reads saved metadata offline. `wallet addresses` displays saved
receive addresses; singular `wallet address` derives an address. Explicit recovery
commands distinguish unstarted preparation, signed/ambiguous source operations,
refund shielding and expiry proof. Use their documented predicates; deleting jobs
or manually clearing reservations is not recovery.

## Qualification boundaries

Default tests use local services and unfunded keys. Synthetic proving and mined
consensus tests separately exercise preparation, refunds, expiry and recovery;
see [testing](testing.md), [LIFECYCLE.md](../tests/LIFECYCLE.md) and
[REGTEST.md](../tests/REGTEST.md).

Public-swap live exercises have observed funding, promotion/refill, service during
pending deposits, cross-listener concurrency and restart. These do not establish
confidential swaps, managed Permit2/upto, all provider routes or a full automated
lifecycle/restart qualification. See [testing](testing.md#qualification-status) for
scope and outstanding acceptance. Current operation requires fresh quotes and chain checks.

Base history enumeration isolates payload conversion failures only after validating
attempt identity, cursor and wallet/pool ownership. It keeps the original liability
and continues independent history and balance work. A one-pass recovery reports
incomplete observation after healthy work drains; background observation revisits
these rows on its normal cadence. Unknown ownership and storage failures remain
shared errors.

Funding inventory similarly isolates phase, provider and scheduling-health conversion failures only
when exact job, wallet, pool and source-operation correspondence remains intact.
The affected operation retains its original journals; independent jobs continue.
One-pass recovery reports the incomplete inventory after draining healthy work.
Unclassified source-operation, ownership and storage failures still stop the shared
owner. Diagnostic text is never substituted for a missing financial fact.

Base background reconciliation retries transient SQLite busy/locked inventory reads
on its existing cadence while retaining the owner, cursor and active observations.
One-pass reconciliation reports an incomplete pass after independent work drains.
This retry applies only to read-only inventory; it never replays a financial commit.

Funding uses the same transient-read classification at inventory boundaries and
pages. Only the affected inventory lane waits for its normal poll cadence; accepted
source work and independent credit/status observations retain their owners. Initial
inventory acquisition also retains its lease while retrying. Financial publication
and source mutations are never replayed by this mechanism.

For ordinary serving without an enabled discovery relay, catalogs and listeners
can start before managed wallets are funded. All listener sockets bind before new
funding starts; an empty pool returns `wallet_not_ready` before provider I/O.
Explicit `wallet bootstrap` still waits for its captured enabled reserve claims.
If shared state authenticates but the Zcash SDK cannot decode its snapshot, ordinary
serving retains that same store for existing Base payments and reconciliation and
pauses new funding. Schema, key, identity, authentication and owner failures still
stop initialization. Funding-only configuration or preflight failures also select this Base-only runtime:
invalid treasury sync/fee settings, source endpoints or NEAR credentials do not
prevent existing Base service. Correcting those settings or SDK snapshot availability
requires restart. Shared identity/path remains required; missing treasury/funding sections still
fail initialization. If Base configuration itself is invalid, static service stays
available while managed API calls and identity-bound probes return `wallet_not_ready`
without provider I/O. No Base reconciliation runs through that invalid configuration;
unsigned catalog/help operations remain independent. Paid relay startup, explicit
bootstrap, config checking and qualification remain strict. The store-only runtime
does not reconstruct the SDK or run ZEC/source/swap observers, including startup
ZEC discovery; their unresolved obligations remain durable and existing Base
startup discovery still runs when its configuration is usable.

## Wallet observation policy

Base transport recovery is a finite sequence inside the existing logical read and
retained acquisition owner. New scopes do not create observations, caches, owners
or faster polling; waiter cancellation does not abandon accepted shared work.
Transport promotion does not change endpoint generations or release liabilities.
See [network policy](network-egress.md) for endpoint-first retries and validation.

The observation scope, cadence and completion rule belong to the work requesting it:

| Trigger / work | Observation and cadence | Completion / stop rule |
| --- | --- | --- |
| Fresh managed serving startup | One finite Base pass over existing eligible managed wallets and initial-funding recipients; one background ZEC scan when the treasury capability is available | Finish or report failure of the captured pass; new allocations do not extend it and startup alone creates no recurring watcher |
| Preparing a new wallet's initial funding | Discover SDK funds as needed, then read that exact Base destination after a valid spendable proposal; reuse applicable same-work observations | Positive credit consumes the unused claim; acceptance rechecks the claim before calculation. An advisory RPC failure does not gate funding, while Store/publication failure defers it |
| Enabled initial-funding demand waiting for capital or confirmations | Shared ZEC scans; each recurring scan starts at least 90 seconds after the previous scan completes | Stop when no enabled unfilled claim can advance and no validated accepted transaction needs observation; a new claim wakes discovery |
| Accepted initial funding without positive destination credit | Shared Base recipient acquisition when due; each repeated read starts at least 60 seconds after the previous read completes | First positive credit permanently finishes that claim's credit lane; source recovery remains independent |
| Recognized signed-payment insufficient funds | Asynchronous scalar Base refresh when due, coalesced per wallet; each repeated read starts at least 60 seconds after the previous read completes | Success, including zero, finishes the captured request. Failure or a notification arriving during the read retains follow-up work |
| Eligible retained-wallet retirement | Candidate-scoped canonical authorization pages and dual balance proof under the existing per-wallet owner, 60-second completion floor and error backoff | Retired or verified latest above tolerance finishes captured demand; confirmed above tolerance with latest low, valid unused authorization or transient failure retains work. Canonical-history changes retain reconstruction work; ineligible or unrecoverably corrupt original work stops locally |
| Validated pending source or shielding transaction | Shared ZEC scans with a 90-second completion-based floor; operation observations retain their own 60-second floor. The funding scheduler owns current operations even after job completion; treasury maintenance owns jobless and archived pending originals | Bound NEAR SUCCESS closes swap source work; otherwise canonical confirmation or signed finite-expiry recovery resolves the source obligation, including after credit, disablement, removal or restart |
| Submitted swap awaiting provider outcome, including archived attempts | Independent exact-operation NEAR status observations; each recurring check starts at least 60 seconds after the previous check completes | Validated SUCCESS/FAILED/REFUNDED stops that operation's status work; SUCCESS closes its own source work and fulfills the initial claim, without creating spendable funds or releasing a different disclosed original |
| Explicit `wallet sync` | One requested ZEC scan and configured managed-USDC pass, including outstanding accepted funding recipients and one eligible-retirement attempt | Report asset results and unresolved retirement work; create no background watcher and require serving to be stopped |
| Completed-result publication or SDK reconstruction | Existing local owner retries after 60 seconds, independently of scan scheduling | Publish the identical result or reconstruct the SDK; this work alone requests no network scan or new calculation |
| Fully funded idle serving without requested refresh or accepted observation work | No recurring Base or ZEC balance discovery | Relevant new work requests observation again |

The interval applies per wallet, operation or serialized treasury scan; it is not
a global RPC rate limit. A scan, provider fallback or canonical proof can issue
several requests. Recurring failures retain independent backoff bounded by
300 seconds and existing jitter where applicable; treasury scan failures wait
90, 180, then 300 seconds after completion; a new event or sibling success
cannot shorten that work's interval or clear its failure. Configured local polling
and healthy handoffs do not change these network observation floors.

Store-only serving retains Base startup discovery and eligible retirement when its configuration is usable.
It has no SDK/source capability and cannot perform startup ZEC discovery or source
observation; the reported capability failure requires correction and restart.
Accepted obligations remain durable. This existing degraded-serving boundary does
not create a hidden SDK owner or relax strict inspection/qualification requirements.

Listeners bind before startup discovery. Ordinary serving does not sweep idle
ACTIVE, READY or RETAINED wallets. The [aggregate startup announcement](configuration.md#multiple-mcp-ports-from-one-configuration)
uses existing saved estimates and capacity notifications to report when all
listening endpoints can cover their finite maximum payment on an eligible payer.
It neither sums separate wallets nor requires reserve inventory to be full, and
rechecks all bound pools together after each wake. It adds no observation or
financial authority and stops after its single announcement or serving shutdown.

In Tor mode, finite serving startup discovery retains its captured work while the
configured local SOCKS port is unreachable. A shared TCP-only check reports the
endpoint and retries every five seconds until reachable; it then ends and the
normal finite passes proceed. This does not verify Tor bootstrap, add recurring
idle discovery or reset an existing wallet/treasury observation cooldown.
Requested refreshes, accepted work and treasury commands remain independent;
a successful demanded treasury scan can satisfy its pending startup pass.
Explicit one-pass sync and qualification do not wait for this startup prerequisite.

Recognized signed-payment insufficient funds requests an asynchronous scalar
refresh. Multiple requests before a read coalesce; a request arriving during a
read remains pending for a later read. Success, including zero, finishes the
captured request. Failed requested reads retain their previous estimate, explicit
failure and pending request across restart. Successful scalar reads consume any
unused initial-funding claim on first positive credit, without reopening consumed
claims or authorizing top-ups. Startup and ordinary requested refresh do not
traverse retired wallets. Accepted recipient work keeps its original identity,
including in explicit sync; observing it never reactivates a retired signer.

A shared Store/wallet owner serializes scalar acquisitions for startup, requested
refresh, preparation and accepted credit checks, retaining them through waiter
cancellation. New requests cannot shorten the cooldown. Startup and explicit
one-pass inspection can reuse the owner's completed result during its cooldown;
reuse does not clear a refresh requested after that acquisition began. Accepted
initial-funding credit checks retain their separate operation scheduler while
using the recipient acquisition's existing due time. Cached failures do not add
another credit retry backoff or count as new recurring observations. Payment
responses and successful payments alone request no observation. Explicit canonical qualification
keeps its separate inspection and authority contracts.

Scalar acquisitions lazily discard unused default metadata and expired successful
results when no acquisition or waiter owns them. Failed observations retain their
consecutive backoff while the wallet remains live; retired-wallet failures can be
discarded after their existing cooldown. Cleanup never drops active ownership or
changes durable claims, estimates or financial journals.

Retirement demand has separate completion from scalar refresh. A Store-lifetime
marker remembers local eligibility and pending demand independently of disposable
scalar results and canonical certificates. Committed retirement discards its marker;
late observation completions cannot recreate it. Committed retirement reports its
lifecycle change without an additional generic delay-recovery message. Finite startup
evaluation, fresh low
scalar observations and transitions into eligibility request work. Local metadata
pages reevaluate retained roles, claims and source blockers; unrelated wakes and
canonical minimum publication do not recreate completed demand. A verified latest
balance above tolerance stops work even when the saved minimum remains low. The
opposite disagreement retains confirmation progress. Wallets with unresolved
original delivery/source obligations request no retirement-driven balance polling.
Evidence-backed closure of the final original obligation is a local eligibility
transition and requests candidate work through the existing owner.
Completion follows the effective evidence committed by publication, including a
newer compatible canonical observation published concurrently by a library caller;
an older acquisition cannot finish or renew that newer observation's work.

The existing reconciliation owner schedules each candidate under shared wallet
serialization. Accepted page acquisition and publication survive disconnected
waiters; no group publication gate spans remote I/O. Demand arriving during an
acquisition remains pending. Retirement uses the existing 60-second floor and
independent error backoff without stacking scalar and proof delays or replacing
a successful scalar outcome with a proof failure. Owner replacement retains
unfinished work; reopening derives a fresh finite evaluation from durable facts.
Canonical contradictions and malformed derived checkpoints retain scoped work
for automatic reconstruction from original authorization identities and fresh
chain evidence. They do not set a permanent manual-repair latch. A same-hash
amount contradiction invalidates the affected evidence and requires fresh proof;
it does not establish a new amount by itself. Stored historical support is audit
data and supplies no independent veto over a fresh canonical observation.
Unrecoverable required records preserve obligations, report the affected operator
requirement and stop proofs that cannot repair them while siblings continue.
Malformed retirement settings, wallet identities or original authorization
bindings, and missing accepted inputs or signed bytes, cannot be reconstructed
from a current balance. These failures need a safe diagnostic identifying the
affected wallet/group and known cause. Transient database failures retain their
existing retry behavior.
An operator-required stop warns once with its safe failure category and affected
wallet/group. Recoverable proof outages report delay transitions at INFO and
individual attempts at DEBUG; retry attempts do not warn as terminal failures.
Finite reconciliation keeps reporting an eligible operator-stopped request as
failed without repeating its proofs or warning. Fresh eligible scalar demand can
request another attempt on the existing schedule; a successfully evaluated change
to an ineligible role, claim or group makes that old retirement stop irrelevant.

Before accepting a new initial-funding calculation, the treasury first obtains an
SDK proposal using actual spendable funds and validates its inputs and fee. It then
requests one scalar observation of that exact Base destination, reusing an active
or recently completed acquisition. Positive credit consumes the unused claim, and
source acceptance rechecks that claim after the read. A failed advisory destination
RPC read preserves previous information and does not prohibit funding. Failure to
publish a positive observation or verify Store ownership defers source acceptance,
including callers sharing that failed acquisition during its cooldown. Waiting
for ZEC or rejecting an invalid proposal does not repeatedly probe Base
destinations; no whole-inventory discovery barrier precedes an individual preparation.

New reserve demand wakes shared treasury discovery. Known shortages keep needed
scans running so later deposits and confirmations can supply spendable SDK funds.
New-funding discovery demand comes from enabled current reserve slots and their
exact pool, wallet and claim-generation bindings, rather than historical funding
jobs. Completed funding jobs do not request new-funding scans or reopen their original
funding authority. With automatic funding enabled, the ordinary vacancy pass
replaces a current `ALLOCATED`, uncredited reserve whose job is complete once all
current and archived source ownership for that job is closed: no live unresolved
calculation or pending transaction. Bound NEAR SUCCESS, canonical confirmation,
verified finite expiry or proven non-disclosure can close the relevant source work. The Store atomically retains the old wallet, clears its exact slot and
increments the claim generation; ordinary allocation then creates a fresh wallet
and operation under current funding authority. A crash between these commits
leaves a vacancy for the next pass. The old financial journal and uncertain bridge
delivery remain unchanged. Late credit stays usable on the retained wallet and
cannot reclaim the replacement slot. Low balances on previously credited wallets
never trigger this recovery or authorize top-ups.

Committed replacement logs at INFO. A completed uncredited slot still waiting
for source ownership is identified once at funding-owner startup at INFO and in
wallet status as degraded inventory; recurring checks remain DEBUG. This condition does not assert insufficient ZEC. Pending
original source work retains its independent observation and recovery. Replaced
or disabled claims do not keep discovery running; validated accepted transactions
retain their independent journal-based observation demand.
Waiting source operations reuse an applicable current-owner scan without extra
tip probes. Local quote, preparation and submission handoffs remain immediate.
Immediate continuation follows an actual successful local result and current
durable preparation facts; a display-phase change or successful no-op cannot
shorten the source observation interval. Quote refresh creates a new operation
identity whose initial work is eligible independently of the obsolete completion.
SDK proposal selection runs locally before canonical validation; insufficient funds do not trigger intervening tip probes. A selected
proposal still requires canonical validation, and is cleared and reselected if a
new scan changes its snapshot. Explicit administration and actual submission/expiry
release proofs keep their separate correctness checks.

Positive destination credit alone cannot finish a pending source transaction.
Validated bound NEAR SUCCESS closes source work as described above. Other pending
originals retain observation demand across stale phases, disabled/removed profiles,
cancellation and restart. Live calculations and completed results retain their
serialized SDK owner; unpublished calculation metadata alone does not request a
network scan or create a global input fence. Invalid retained outgoing bytes keep
their actual pending-source fence, while local publication and reconstruction
handle recoverable failures without network scans. Completed-result publication and SDK reconstruction retry
locally at their own 60-second cadence. A successful needed persisted scan may
recover owned transparent funds through automatic shielding under existing
permission/fee checks; accepted shielding keeps observation even after its original
funding demand vanishes. Once source work closes, destination/provider waiting
alone does not keep ZEC discovery running.

## Best-effort wallet rules

Read the [wallet observation policy](#wallet-observation-policy) when changing
balance estimation, refresh scheduling, funding or recovery, and apply the
[reconciliation policy](#recovery-and-reconciliation-policy).

Managed USDC balances are advisory estimates. Payment selection prefers an affordable
bound payer, then other eligible affordable identities, then unknown or low estimates.
Old or unavailable RPC observations and missing historical funding readiness do not
prevent a permitted signature. Actual pool ownership, role, configured amount cap,
asset/domain and caller authority still apply.
Historical price hints rank estimates without excluding an otherwise authorized
payer, including after a cap change. The current challenge and serialized journal
enforce the actual payment amount against current authority.

Each new authorization is durably journaled before disclosure. That transaction
subtracts its maximum amount once from the current estimate, saturating at zero.
Unknown or malformed estimates stay unknown. Unresolved exposure is not subtracted
a second time. Chain observations may replace estimates even while signatures
remain chargeable; estimates are not a replica of canonical available capacity.
Managed signing constructs the actual version's payment header in the owned
request before journal acceptance. An unrepresentable header map fails without
recording liability or dispatching, including through the low-level Rust payment
API. Low-level callers still own their supplied HTTP client's defaults and
transport policy; use `PaidClient` for factory-managed request construction.

A signed attempt's HTTP 402 can earn another wallet attempt only through the
[pinned x402 seller encodings](../tests/fixtures/x402/insufficient-funds.json):
`PAYMENT-REQUIRED` with exact `error: insufficient_funds`, or a failed
`PAYMENT-RESPONSE` with that exact `errorReason`. An unsigned challenge, arbitrary
prose, a bare 402, allowance failure and malformed protocol headers cannot earn it.
Duplicate protocol headers cannot grant another attempt. When `PAYMENT-RESPONSE`
is present, its settlement record governs classification: a simultaneous challenge
cannot override a successful settlement, another failure or an unreadable receipt.
The unused error body is dropped immediately: truncation, size and read inactivity
cannot delay or prevent fallback after recognized headers. Ordinary returned
responses retain their response-byte and read-inactivity limits.

Each recognized failure marks its wallet low balance and queues an advisory
`balanceOf(..., latest)` read through the existing maintenance owner. Payment
fallback never waits for that read and never allocates or funds a wallet. A failure
during an active read leaves a pending follow-up. A successful read replaces the
estimate; an unsuccessful read retains the previous estimate and reports
`observation_unavailable`. Low balance is a selection classification, not permanent
retirement. It survives restart until an ordinary successful observation clears it.

A nonempty [manual raw header set](agent-sources.md#manual-raw-headers) stays on
its initially selected payer. Price-based handoff stops before another wallet's
unsigned request. Recognized signed insufficient funds still queues advisory
reconciliation and preserves liability, then stops before another payer is selected.
These calls never allocate wallets or wait for funding. Concurrent active/static
payer replacement affects later calls; accepted work retains its captured identity.

Every captured eligible identity is visited at most once per logical call. New
wallets do not extend that call. Exhaustion warns that earlier authorizations may
still charge or execute. No interrupted logical call is automatically replayed on
restart. The amount cap applies to each authorization, so one call can incur
multiple charges. Qualification uses its existing case permit for the sum of all
attempts, retains every receipt/proof, and checks current run authority before
starting another unsigned attempt. A later success does not erase earlier unknowns.

The first positive initial credit permanently consumes that funding claim, even
below target. No later debit, partial return, disablement or restart grants a top-up.
Source observation continues independently using exact original ownership.

Late initial funding or external transfers may update an estimate when observed,
without reopening its consumed funding claim. These possibilities do not require
incoming-transfer surveillance or a new enforcement layer to prove that wallets
are never topped up operationally. A requested low-balance check can observe those
funds through the same ordinary advisory read.

Recurring initial-credit polling begins after durable source acceptance or validated provider success,
not for fresh allocations or unsigned quotes awaiting spendable ZEC. Actual
preparation has the one-pass destination discovery described above. Accepted preparation and
signed or previously submitted work remain observable across restart even when
their current phase is stale or their source is already confirmed. Base credit
reads start immediately when the shared recipient observation is due; a preceding
preparation read retains its cooldown. Repeated reads start no sooner than 60 seconds after
completion, with the shared recipient acquisition's error backoff; credit adds no
separate jitter or backoff. Unrelated source or provider-status progress cannot
shorten this interval. This does not change the first-positive
credit rule or grant authority for another deposit.

Accepted calculations, exact selected inputs, signed bytes and submission intent
survive cancellation and failed local acknowledgment. Source expiry recovery checks
the exact authenticated original, buried finite inclusive consensus expiry and
completed canonical SDK coverage; a refund is never proof
of non-submission. Owned schema-21/22 stores upgrade additively to schema 23, preserving
financial evidence and encrypted formats. Older binaries reject schema 23 because
they cannot enforce archived fulfillment against a replacement deposit. Unsupported state schemas
are rejected before mutation, without private-state conversion.

Failed SDK capabilities reconstruct and exited funding/reconciliation tasks resume
through existing ownership. Automatic shielding recovers owned transparent capital
per address under existing authority. Local tests cover these paths without live-funded,
consensus or Tor qualification; see [testing](testing.md#qualification-status).
Reopening preserves accepted evidence and does not recover unpublished bytes lost
with a process. A pending signed shielding original keeps its source-work fence
until confirmation or verified finite expiry. An unpublished calculation with
neither outgoing nor factual journal recovers through successful offline SDK
reconstruction; later shielding uses a fresh identity and current SDK funds.
Automatic initial submission requires current authority and zero prior attempts;
uncertain broadcasts are never automatically replayed. A factual transaction
record with missing outgoing bytes remains a scoped corruption case.

Multiple charges after the accepted USDC failure category and incomplete external
settlement proof are accepted limitations. The [network guide](network-egress.md#remaining-timers-and-budgets)
records the unresolved 1Click route contract and gRPC upload-inactivity limitation.
