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
max_retirement_remainder_usdc = "0"
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
| `max_retirement_remainder_usdc` | Nonnegative automatic-retirement tolerance, default zero; it never authorizes transfers or replacement allocation. |
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
the accepted source operation. Partial credited allocations retain their claims.
Already accepted work retains its bytes, reservations and original identities
across disablement. A valid successful operation may receive excess input later;
canonical returns update the original source accounting without reopening funding.

Per-operation safeguards remain:

```toml
[treasury]
max_funding_transaction_fee_zec = "0.0003"
max_refund_shielding_fee_zec = "0.0003"

# Within an existing managed profile:
# max_funding_spend_zec = "0.006" # One transfer including its network fee.
```

Gross source expenditure, canonically confirmed returned principal and derived
net cost remain distinct. Returned principal cannot erase source fees or credit
another operation. Owned-output shielding charges only its fee. Managed serving
can shield confirmed owned transparent returns under the current enabled profile
and fee policy; wallet administration requires an explicit shielding command. Reported
or expected returns are not spendable capacity, and successful funding never waits
for a speculative surplus return.

Return attribution uses the original operation/address binding, including archived
recovery records. Known credited outpoints are rechecked by transaction ID and
output index even after spending or shielding. Missing or ambiguous attribution
preserves the affected accounting obligation while independent returns and otherwise
valid treasury readiness can progress. Exact confirmed owned transparent outputs
may be shielded despite incomplete attribution; this grants no guessed refund
credit, reservation release, swap completion or replacement attempt. Canonical contradictions affecting inputs or shared
financial state, and storage failures, still block the affected treasury work.

The pinned Zingolib exposes an in-memory transaction map. Discovering unknown late
returns therefore retains one linear output scan under the treasury owner, with
keyed binding matching and no second full history copy. Its work and actor occupancy
grow with wallet history. Bounded SQLite reads do not make that dependency scan
incremental or bound its resident history; an incremental discovery API remains
separate work.

Zingolib proposes funding transactions and calculates their standard network fees.
Refund shielding uses the backend's standard ZIP-317 fee rule. The application
accepts or rejects the resulting fee; it does not force a lower fee into the
transaction. Calculate-only proposals determine the actual fee before the serialized
store reserves principal plus that fee (shielding reserves only its fee), and before
signed bytes are created. A configured maximum is a rejection ceiling, not a
reservation of money that will not be spent. Proposal mutation or cancellation
poisons the owner until reopen; exact costs and prepared bytes commit atomically.
Larger transactions can exceed a fee ceiling and require an explicit
configuration adjustment. All money values are decimal strings; accounting uses
integer atomic units, never floating-point currency conversion.

The bridge's USD valuation is not an independent exchange-rate oracle. Reserve output
and conversion-overhead ceilings do not replace a per-operation ZEC safeguard when
one is required. All financial limits still apply alongside qualification permits;
no configuration value authorizes replay or bypasses a zero-new-funding restriction.
Positive funded qualification still requires an explicit `max_funding_spend_zec`
to bound its native-asset permits.

A stopped treasury owner or funding worker pauses funding until process restart.
It does not stop MCP listeners or Base reconciliation. Existing confirmed assets
remain usable under ordinary admission checks, with all reservations and unresolved
liabilities retained. A poisoned treasury owner is closed, never reused in place.

## Treasury sync freshness

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


The Zcash treasury is exclusively spent by this application. Direct and Tor sync
use the same bounded policy: a successful scan must reach the tip observed at the
start, and may trail the final indexer observation by at most **three blocks**.
A regressing indexer tip fails the observation. Greater lag or an aged starting
observation retains the completed scan/checkpoint and starts a fresh observation
with incremental catch-up. It never relabels old evidence as fresh. Confirmation counts
and spendability use the **scanned** height, never the newer unscanned tip.

`treasury.max_sync_age_seconds` (default 300) limits the age of the starting tip
observation, including sync and validation time; completing a slow sync does not
restart this clock. Status records `target_height` (starting tip), `height` (scan),
`observed_tip_height` (final query), and `checked_at` (starting observation time).
Historical snapshots have no observed tip. Accepted lag is logged at debug level
and appears in wallet-sync output. Both serving and one-shot sync can acquire a
fresh observation through incremental catch-up. Ordinary preparation uses the
same three-block allowance and rejects a regressing observed tip. Read-only
readiness checks precede proposal mutation and reservation; only a proven untouched
operation can return `PreparationDeferred`.

This allowance does not change reservations, outgoing transaction tracking,
confirmation depth, reorg checks or expiry. Recovery that releases an expired
transaction's reservation still requires zero observed tip lag on both syncs,
along with buried expiry and confirmed unspent inputs. Unknown historical tip
evidence cannot satisfy that recovery check. Do not spend the treasury from an
external wallet while relying on this policy.

Base reconciliation resolves historical authorizations at their original confirmed
anchor, revalidates that anchor, and then acquires a separate fresh balance view.
Wallet reads run with bounded concurrency and preserve each wallet’s network
identity. Balance acquisition runs once; completed canonical evidence is retained
even if it ages out. The serialized store checks its original block-derived
admission expiry before journaling a payment or consuming a reserve. Stale admission
returns a readiness error and releases the pool gate; it does not restart the
sweep indefinitely. Payment acquisition excludes retired-wallet balances while
still resolving their historical authorizations. Old balances are never assigned
a new block. Failover restarts the entire coherent view.

For payment admission, an unavailable nonce read leaves its full liability
reserved without requiring another endpoint to release it. Other valid canonical nonce
proofs can be committed, and fresh balances minus all remaining liabilities may
authorize a payment. Expiry without a successful nonce observation releases
nothing. Invalid data, TLS failures, wrong chains, stale balances and changed
anchors still fail closed. Required evidence failures retain the ordinary
configured failover policy; views never mix endpoints. Background reconciliation remains strict and reports
unavailable observations as failures with its normal backoff.

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

An ALLOCATED wallet with its exact enabled reserve claim can serve a payment from
fresh proven capacity below the funding target. For example, $2 in a $10 allocation
can cover a $0.10 payment under the ordinary cap and liability checks. Its first
journaled payment promotes it to ACTIVE while retaining the original claim, funding
job, operation and recovery history. No partial credit creates a reserve vacancy
or replacement allocation. Funding observations and already accepted source work
continue independently. Later proven destination readiness detaches the retained
claim once without changing the serving role. Spending before that readiness was
observed can leave destination accounting incomplete; exact delivery-based closure
remains separate work, and this does not block independently proven payment capacity.
Disabling and re-enabling preserves the original claim and obligations.

`rotation/manager.rs` admits supported Base USDC v2 exact EIP-3009 offers. It
validates the envelope, domain, amount and explicit API cap before acquisition or
signing. Advertised extensions outside the managed payment contract are omitted
with DEBUG diagnostics for `bazaar` discovery metadata and WARN diagnostics for
other extensions; seller rejection remains possible.

Fresh compatible balances minus unresolved signed exposure govern each wallet's
available capacity. A recent actual-price hint can select the smallest sufficient
cached wallet without RPC or a pricing probe. Without a usable hint, the existing
active identity remains the default. Existing role and exact reserve ownership bind
the payer; archival funding readiness does not grant or veto current payment
capacity. Explicit bootstrap inventory and funding accounting still validate their
historical support. Malformed current liabilities remain wallet-local failures;
choosing a fallback does not consume a reserve or replace an existing active role.
Once its actual challenge is known, a
sufficient bound payer stays selected. Otherwise a bounded rolling pass considers
eligible retained and ready wallets. Holdings are never summed to authorize one
payment; unavailable evidence differs from a completed insufficient-capacity pass.
The pass checks the completed cache initially, after acquisition completions, after
each page-sized batch of evaluated wallets, before waiting on outstanding reads,
and once before returning a negative result. Batch rechecks include independently
published evidence and changed liabilities; each selection revalidates current
capacity under the group gate. Candidate selection compares cached and caller-owned
certificates by chain observation, using one captured time. It performs no writes
or evidence acquisition; contradictory observations and database errors propagate
instead of becoming a successful fallback.

Each pass captures its inventory boundary before traversal; new wallets become
eligible for a later pass. Priority identities are deduplicated against keyset
pages. Cancelling the caller drops that pass's unsigned acquisitions while
independent maintenance retains ownership and can publish evidence. Cancellation
does not retract requests already received by an endpoint or undo accepted store
jobs. The pass itself neither allocates funding nor changes wallet roles.

Runtime hints expire 300 seconds after the unsigned response was received and
retain at most 512 fixed-size entries per process. Their trusted key includes
source/revision, method/OpenAPI template, wallet group and transport scope; request
arguments cannot choose that scope. Receipt order controls updates, including lower
prices. Restart, expiry and eviction lose only a selection estimate. They cause
no probe, cached-challenge reuse or new payment authority.

The invocation may make one unsigned handoff to an explicitly selected replacement,
including for POST and other supported methods. It preserves the original request
and obtains the replacement identity's own challenge. A changed second price is
checked only against that fixed payer; it cannot trigger another search or handoff.
Free, malformed, unsupported and redirect responses do not authorize a handoff.
A signed response or output-conversion failure never authorizes replay.

Signing produces private bytes. One transaction rechecks current role, exact
reserve claim, cap, evidence, capacity and signed validity, then commits the
possibly-submitted authorization, exposure and immutable admission support. An
ALLOCATED payer retains its exact claim when promoted. A READY payer consumes its exact claim in this transaction: the former active
becomes RETAINED, the payer becomes ACTIVE and the reserve generation advances
with a vacant slot. No replacement allocation or retirement occurs in admission.
Ordinary authorized funding fills the vacancy independently, including while old
remainders remain. Pausing funding does not prevent spending existing capacity.

One evidence-race refresh and one definitely rolled-back unpublished signature
replacement are separate invocation allowances. They remain on the same payer
and survive the unsigned handoff. Contradictory evidence, an unknown commit
outcome or a missing acknowledgement grants no favorable retry. Fresh schema 26
derives payment exposure from authoritative journals; no independently writable
exposure total remains. It preserves original payer role, consumed slot/generation
and exact balance support;
it also binds calculate-only treasury preparation to its operation and snapshot.
Older state requires separately reviewed stopped-service conversion and is never
upgraded on open. Canonical checkpoint conflicts restore conservative exposure for
previously resolved authorizations in the affected wallet without rewriting their
original proofs. A complete canonical replacement pass reads bounded nonce pages
at one confirmed anchor, then acquires a fresh balance that covers that anchor and
the retained numeric debit floor. Atomic adoption appends replacement proofs and
retains original resolutions and checkpoint provenance. It clears only the payment
chain dispute, leaving funding and retirement disputes independent. Changed
membership or generations reject adoption; ordinary duplicate nonce observations
cannot replace a disputed proof. Under chain repair, a separate
payment-only proof can use remaining current capacity when every resolved
authorization is reserved at its full amount and the debit-height floor is met.
It cannot establish funding readiness, retire a wallet or release a liability.
Amount contradictions and incomplete exposure still block affected admission.
Typed endpoint distrust reserves liabilities that depend on effective payment
proofs or the adopted checkpoint, even after restart; ordinary outages do not.

All handles for a store/group share one gate. Accepted store jobs keep that gate
after waiter cancellation, and funding-credit publication uses the same owner.
The pool gate releases before awaiting the signed HTTP response, allowing other
admissible calls to progress without hiding the in-flight liability.

## Automatic retirement

Ordinary reconciliation may retire only a RETAINED wallet in an enabled group,
using fresh compatible confirmed and latest balances. Both must be at or below
`max_retirement_remainder_usdc`: the recorded bound is their maximum, not their
minimum. Default zero therefore requires both observations to be zero. Pending
payment exposure, a current enabled reserve claim, incomplete applicable destination
credit or financial repair prevents retirement.

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

A logical paid request has at most one signed submission. A final 402, upstream error,
connection timeout or response-body loss does not authorize replay on another
wallet. A missing response is not proof that settlement failed. Retired addresses
remain under reconciliation and accept no new payments.

`rotation/base.rs` verifies Base chain identity, confirmed anchors, freshness,
block-pinned USDC balances and EIP-3009 authorization use. A used authorization
or an expired authorization proven unused can release its reservation through the
store's canonical reconciliation transaction. An unused but still-valid signature
remains a liability. Reorg contradictions cannot silently reset accounting.

Balance-only acquisition reads one fresh latest/confirmed header pair, pins both
balance calls to those hashes, and rechecks their canonical anchors. A view that
also scans historical authorizations acquires a new header pair after that scan;
finishing the scan never renews the earlier observation's timestamp. Identical
canonical header checks are shared across captured wallet dependencies within one
verification phase on one endpoint. Initial reads never replace post-state checks,
and reuse neither renews evidence nor skips comparison against each expected hash.

Payment exposure is derived with exact arithmetic from the wallet's outstanding
authorization journal, including during transactional admission and status reporting.
The legacy stored total is unused and no longer maintained; it cannot override
that journal. Malformed identifiable liabilities block the affected wallet; existing
stops for disputed resolution evidence remain in force.

Historical liability release and fresh spending capacity have separate anchors.
A confirmed USED proof requires a new balance observation at or beyond its debit
block before that released amount can contribute to capacity. EXPIRED_UNUSED can
recertify a compatible balance without renewing its original freshness. For example,
a $10 observation with $4 unresolved exposure cannot admit $8: settlement needs a
fresh post-debit balance, while canonical expiry without use can restore the
unspent capacity only while the original observation remains fresh and compatible.

Evidence is wallet- and endpoint-bound, with exact confirmed/latest amounts and
anchors. Malformed acquisition alone does not invalidate earlier independent
evidence. A demonstrated endpoint contradiction invalidates its affected support
across groups. Historical readiness or retirement discrepancies remain separate
from invalidated liability-release/accounting support: fresh observations cannot
silently clear a durable financial repair block or reactivate a retiree. Once a
historical balance is recorded, its archival anchors do not gate independent fresh
capacity; historical revalidation belongs to accounting, and original support and
disputes remain durable. Ordinary payment admission verifies its financial
checkpoint and subtracts all unresolved liabilities. While resolution support is
disputed, the separate capacity-only path reserves every resolved authorization
at its full amount and retains the debit-height floor; it need not reverify the
disputed checkpoint before using independently proven remaining capacity.

Maintenance repairs disputed payment resolutions using exact original authorization
bindings and current canonical used/expired-unused proofs. It retains every disputed
amount as exposure until a complete replacement set is activated. Historical RPC
work and database validation yield between bounded pages; fresh balance expiry or
new pending admissions repeat current validation without discarding historical work.
Inactive replacement pages grant no authority. Activation checks the exact wallet,
endpoint, revisions and original freshness lease in one short transaction, preserving
original proofs and pending liabilities.

Retired wallets remain eligible for liability observation and repair, without
restoring payment eligibility or funding claims. Historical funding status and
readiness balances do not gate this payment repair. Missing authoritative payment
support remains a scoped failure. Cancellation retains accepted work ownership;
abandoned inactive pages are deleted in bounded turns. This repair does not resolve
uncertain swaps or authorize new funding.

Chain-view publication applies canonical resolutions, exposure release and balance,
readiness and retirement updates in the caller's single transaction. A late failure
rolls back those updates together. Scoped publication leaves unobserved wallets
unchanged; complete publication requires balances for every non-retired wallet.

The configured RPC policy defaults to PublicNode, dRPC and Base. Availability
failure can restart an entire bounded view at another configured endpoint; it
cannot combine one endpoint's header with another's balance/nonce evidence. Each
new view begins at the primary, with the same wallet isolation identity. This is
availability failover, not a quorum or protection from internally consistent
fabricated data. Endpoint policy, timeouts and overrides are in the README.

Full seller-receipt capture/accounting is not implemented. The current payment
journal and chain reconciliation must not be described as per-request receipt
verification. HTTP/MCP success also does not establish useful provider data:
AgentFund, for example, has returned nested errors inside successful responses.
Remaining work is scoped in [managed wallet follow-ups](plans/deferred/managed_wallet_followups.md).

## Funding and source safety

Allocations and their exact operation journals commit atomically. Inventory reads
never create missing journals or substitute operation IDs; missing provenance
requires repair. The runtime funding owner schedules work through captured,
paged inventory and independent observation lanes.

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

Before preparation, the shared treasury verifies fresh sync, spendable funds,
actual reservations, per-operation safeguards and its single unresolved outgoing-operation gate.
Zingolib calculates/proves without automatic broadcast. The post-calculation wallet
snapshot, prepared bytes and accounting facts must commit before submission.
A versioned preparation record binds accepted calculation to the exact operation,
funding job when present, and starting snapshot. Calculation or persistence failure
retires the affected SDK without saving its possibly mutated memory. The retained
sync owner must finish and accepted Store jobs must settle before cleanup can prove
that this exact preparation never committed. Missing outgoing records alone do not
supply that proof. Unknown preparation contracts remain unresolved.

The command owner reconstructs the SDK from the committed snapshot while retaining
the same live Store and its Base readers. A cancelled sync waiter cannot detach its
scan or permit a second SDK owner. If preparation committed before its acknowledgement
was lost, reconstruction preserves its exact bytes, input ownership and reservation;
it cannot discard the operation or authorize a replacement. Ordinary exclusive reopen
applies the same known-preparation handoff. It does not convert incompatible state or
clear a declared restored-backup provenance restriction.

A quote crossing its delivery margin before the concrete backend dispatches a
treasury preparation command is a typed pre-preparation deferral. Returning it to
`Quoted` requires an untouched preparation boundary and absence of source effects;
accepted preparation ownership cannot be hidden by a stale phase or missing result.
Ordinary untouched-quote refresh does not consume transaction recovery attempts.

Recovery eligibility comes from the original operation and its bound closure
archive, with liabilities rechecked before reuse. Saved error messages grant no
recovery permission. Startup obstructions use current typed observations and current
funding restrictions; reopening does not turn historical diagnostic prose into a
new policy obstruction.

An uncertain broadcast retains the saved bytes and transaction identity. Recovery
reconciles that operation; explicit rebroadcast uses the same bytes. A timeout,
indexer omission or provider status is insufficient proof to release source inputs.
Expiry recovery additionally requires canonical chain/input evidence.

A live source transaction that loses its original confirmation retains an
operation-specific input dispute across burial and restart. Independent sync work
can finish, but generic shielded sends remain blocked while their input selector
cannot exclude those original inputs. Internal shielding uses its exact transparent
outpoint claims independently. Missing buried archival history alone creates no new
input dispute. Exact re-confirmation clears a dispute only with the matching final
sync publication; an incomplete scan cannot clear it.

A previously confirmed transaction can be closed by the same two-sync, zero-lag
expiry proof as a pending transaction. Recovery checks the original signed expiry
(zero means no finite expiry), indexer inclusion, SDK invalidation and every owned
input. It preserves the original bytes, facts and confirmation record, records the
reversed debit in the expiry proof, and sets the current consumed amount to zero
once. Shielding recovery reverses its fee only. A later contradiction reopens input
ownership; the old expiry proof cannot release inputs again without a new exact
proof. Recovery does not authorize a replacement or rebroadcast.

Source
confirmation and destination credit are independent: a NEAR success response alone
cannot mark a candidate spendable, and Base credit alone cannot prove source
confirmation. Concurrent credit observers cannot overwrite a completed job with
an obsolete persistence warning; genuine source-reconciliation errors remain visible.

Successful financial completion requires the original destination-readiness proof,
current canonical source support, exact canonical destination delivery, and complete
accounting for the exact operation-bound finite return claim. Missing return
amounts remain unknown. An explicit normalized zero must correlate to the original
source, accepted quote and delivered transaction; a positive claim requires enough
canonical owned return outputs and no unresolved accounting issues. Spent or shielded
outputs still count when their original canonical evidence remains available.

Explicit zero is the provider's finite claim, not proof that future returns are
impossible. Return evidence retains its original historical anchor while a completed
pass catches up for fresh spending evidence. Ordinary sync continues observing the
original addresses after completion. A new output, changed claim or canonical
contradiction reopens affected accounting without erasing the historical completion
outcome or crediting an output twice. Destination readiness may permit payments
before financial completion; it never substitutes for source, delivery or return
proof. Historical provider labels remain audit evidence; unresolved or conflicting
financial proofs remain review-required.

One retained funding owner schedules independent status, credit and prerequisite-
complete unsigned quote observations in bounded lanes, alongside serialized treasury
commands. Per-operation/kind backoff cannot stall another group's observations.
Malformed destination-readiness support retains an operation-local integrity
diagnostic and cannot stop independent funding. Completion storage failures still
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
They do not release reservations or authorize another deposit. Sync, RPC, validation
and storage failures remain warnings with error backoff. Swap timeout, failed/refunded
status and incomplete deposits emit explicit warnings describing continued
reconciliation or required recovery.

Bootstrap and the serving funding worker recover eligible jobs automatically.
The operator can run `wallet recover --config FILE` with serving stopped for one
pass through the same classifier, without starting funding or submitting transactions.
Reports distinguish recovered, waiting, failed checks and operator-required cases.
Unknown preparation outcomes, refunds and unresolved confirmed swaps require review.

A prepared deposit is not submitted when fewer than 300 seconds remain before its
swap quote deadline. Its signed bytes and reservation remain retained while recovery
waits for transaction expiry, which is a separate Zcash block height. Existing
canonical absence, fresh-tip and unspent-input proofs authorize recovery; elapsed
time alone does not. Submitted/ambiguous operations are reconciled before expiry
recovery, never automatically rebroadcast. A successful recovery archives the old
operation and returns the job to allocation only under separately revalidated current funding authority.
The profile's `max_attempts` bounds transaction recovery resets using durable
history; neither process restart nor operation replacement erases this count.
New quote refreshes proven to have no outgoing or budget record are explicitly
marked in durable history and excluded from that count; historical
unclassified records still count. Earlier binaries reject the newer state schema.
Unsigned quote outages use bounded per-pass work and backoff without consuming
transaction recovery attempts.


The treasury receives/synchronizes Ironwood with capability checks on the injected
lightwalletd transport. Network integration uses reviewed vendored channel-injection
patches; it does not enable SDK-owned direct networking. The
[egress inventory](network-egress.md) explains treasury versus immutable recipient
identities and the remaining upstream timeout boundaries.

## Refunds, state and recovery

Each swap uses a persisted unique high-index transparent refund address derived
from the treasury. Refund visibility does not make funds automatically spendable
for another deposit. Managed serving maintenance observes historical shielding
operations before selecting confirmed owned inputs. Only serving authority captured
after listener binding can create automatic shielding or submit its previously
unattempted bytes; current policy is rechecked before durable intent. Manual bytes
are never automatically submitted, attempted bytes are never automatically retried,
and qualification permits grant no automatic shielding authority. All paths retain
exact input claims, fee caps, prepared-byte recovery and recipient network binding.
Disabling a profile stops new work while existing operations remain observable.
Deep finalized-chain rollback requires operator investigation rather than an
automatic balance/budget reset.

`rotation/store.rs` keeps treasury snapshots, EVM keys, funding/outgoing records,
roles, source budgets and payment authorizations in one versioned database.
Sensitive records are authenticated-encrypted with a separate 32-byte key and
record identity bindings; public metadata still links addresses and operations.
The directory, key, database, ownership lock and SQLite sidecars have filesystem
protections. Wrong key, corruption or identity mismatch never initializes an empty
replacement wallet.

Back up while serving is stopped. The consistent database and key must be synced
before atomically publishing `backup.json`; directory sync is also required for
reported success. A failed backup destination cannot be silently overwritten.
Seed-only recovery does not recover EVM keys, pending swap bindings or payment
liabilities. Protect the whole backup as spending material. Declare a manually restored
working copy with `wallet mark-restored` before using it. This retains a durable
missing-provenance restriction on new financial work while allowing known-operation
observations. It cannot detect an undeclared rollback or reconstruct later off-chain
authorizations; see [the restoration workflow](wallet-cli.md).

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
Unclassified source-accounting, ownership and storage failures still stop the shared
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
does not run source/swap observers; their unresolved obligations remain durable.
