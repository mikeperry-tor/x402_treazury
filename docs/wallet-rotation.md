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
availability. Uneconomic/empty addresses do not block independent addresses. No
principal reservation, attribution requirement or funding-claim reopening is added.
The explicit per-address command remains available.

Zingolib selects inputs and calculates standard network fees. The application
checks the actual proposal against configured per-operation fee/spend safeguards,
then persists the exact input owner before calculation. A configured maximum is a
rejection ceiling, not a capital reservation. Unknown outcomes after calculation
acceptance retain that input fence; known pre-calculation failures can defer. The
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
queued; an interrupted command is never replayed. Accepted input claims, funding
restrictions and Base handles remain unchanged. Completed bytes take precedence:
only identical local publication repeats on the existing 60-second cadence, including
shutdown. SDK reconstruction also retries unavailable snapshots on that cadence.
Exited funding and reconciliation tasks reacquire their existing Store/group gates
on their ordinary polling cadence; retained source work keeps its existing lease
and backoff state. Explicit shutdown never starts a replacement. An unavailable
shared Store or authentication state can still require restart; listeners and usable
Base payers remain independent. Forced process death or irrecoverable storage can
lose unpublished bytes; the durable input claim still prevents duplicate calculation.

## Treasury sync coverage

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
retain completed work without a local age ceiling or ordinary three-block lag
policy. SDK proposal selection enforces actual confirmations, witnesses, spend
links and scan coverage. Snapshot revision and the serialized actor bind mutation
to its owning SDK state. Cancellation cannot turn an interrupted scan into a
completed observation. An unchanged completed scan can be reused within its owner
after checking its exact canonical tip height/hash; restart or chain changes scan
again. Independent maintenance runs every 60 seconds.

Status keeps `target_height`, scanned `height`, final `observed_tip_height` and
the original `checked_at` for diagnostics. Completion never relabels observation
time. Confirmation and expiry decisions use scanned coverage. Exact expiry recovery
requires both scans to reach their observed tips and to verify all original owned
inputs. No host clock age or ordinary lag allowance supplies that proof.
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

`rotation/manager.rs` admits supported Base USDC v2 exact EIP-3009 offers. It
validates the envelope, domain, amount and explicit API cap before acquisition or
signing. Advertised extensions outside the managed payment contract are omitted
with DEBUG diagnostics for `bazaar` discovery metadata and WARN diagnostics for
other extensions; seller rejection remains possible.

Selection uses the saved wallet estimate as described in the
[best-effort wallet rules](#best-effort-wallet-rules). It performs no balance RPC,
nonce scan, historical readiness validation or certificate publication. Price hints
can choose an existing identity before its own unsigned challenge is fetched.

Runtime hints expire 300 seconds after the unsigned response was received and
retain at most 512 fixed-size entries per process. Their trusted key includes
source/revision, method/OpenAPI template, wallet group and transport scope; request
arguments cannot choose that scope. Receipt order controls updates, including lower
prices. Restart, expiry and eviction lose only a selection estimate. They cause
no probe, cached-challenge reuse or new payment authority.

The invocation captures its existing inventory boundary. It may hand off an unsigned
challenge to an eligible payer suggested by the actual price, and may traverse on
the narrow insufficient-funds exception below. Each identity is visited at most
once. Each handoff creates that payer's own isolated transport and unsigned
challenge while preserving the original request bytes.

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

The retained canonical-publication path may retire only a RETAINED wallet in an
enabled group using compatible confirmed and latest balances. Ordinary advisory
refresh does not collect these proofs or invoke permanent retirement. Both must be at or below
`max_retirement_remainder_usdc`: the recorded bound is their maximum, not their
minimum. Default zero therefore requires both observations to be zero. Pending
payment exposure, a current enabled reserve claim, an actual pending source operation or conflicting canonical retirement evidence
prevents retirement.

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

`rotation/base.rs` verifies Base chain identity, confirmed anchors,
block-pinned USDC balances and EIP-3009 authorization use. A used authorization
or an expired authorization proven unused can release its reservation through the
store's canonical reconciliation transaction. An unused but still-valid signature
remains a liability. Reorg contradictions cannot silently reset accounting.

Ordinary maintenance performs scalar unsigned latest `balanceOf` reads. It does
not acquire confirmed/latest proof pairs or scan historical authorizations.
Canonical historical proofs remain available for separately authorized qualification
and reports; their balance, exposure and repair fields do not gate current signing.
Retired wallets remain retired. Original attempts and canonical proof records stay
readable without a historical capacity-repair pass.

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

The SDK selects available confirmed inputs and calculates standard fees. The
application checks exact output/fee and current operation/run permission, then
journals the exact selected-input owner before calculation. One unresolved input
owner prevents overlapping SDK calculations. Completed snapshots and exact signed
bytes commit atomically; failed publication does not grant another calculation.
An accepted calculation with no bytes remains uncertain across restart.

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
reconciles that operation; explicit rebroadcast uses the same bytes. A timeout,
indexer omission or provider status is insufficient proof to release source inputs.
Expiry recovery additionally requires canonical chain/input evidence. Source
confirmation and destination credit are independent: a NEAR success response alone
cannot mark a candidate spendable, and Base credit alone cannot prove source
confirmation. Concurrent credit observers cannot overwrite a completed job with
an obsolete persistence warning; genuine source-reconciliation errors remain visible.

Successful financial completion combines three independent facts: the wallet's first positive credit, canonical source confirmation,
and validated provider fulfillment. Initial credit alone publishes the funded
wallet role and may permit payments before the other facts arrive. Consumption does
not erase that consumed claim or require rebuilding the original funding balance.
Conflicting terminal outcomes preserve review-required work; provider fulfillment
is not canonical source or return evidence.

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
They do not release reservations or authorize another deposit. Sync, RPC, validation
and storage failures remain warnings with error backoff. Swap timeout, failed/refunded
status and incomplete deposits emit explicit warnings describing continued
reconciliation or required recovery.

Bootstrap and the serving funding worker recover eligible jobs automatically.
The operator can run `wallet recover --config FILE` with serving stopped for one
pass through the same classifier, without starting funding or submitting transactions.
Reports distinguish recovered, waiting, failed checks and operator-required cases.
Unknown preparation outcomes, refunds and unresolved confirmed swaps require review.

A prepared deposit retains its exact bytes and input owner when its external quote
deadline expires. Recovery requires finite consensus expiry, complete coverage on
both scans and every originally selected input confirmed and unspent. Expiry height
N allows inclusion through N; zero never permits expiry release. Dummy shielded
actions are not owned inputs. Submitted/uncertain operations are observed, never
automatically replayed. A new attempt separately requires current funding authority.
The profile's `max_attempts` bounds transaction recovery resets using durable
history; neither process restart nor operation replacement erases this count.
New quote refreshes proven to have no accepted calculation or outgoing record are explicitly
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
for another deposit. Serving and bootstrap with enabled automatic funding attempt per-address shielding
through the existing treasury owner. `auto_fund=false` and qualification restrictions
prevent new shielding; existing prepared bytes remain observable. Positive
qualification funding permits authorize jobs only and do not authorize automatic
jobless shielding. Explicit shielding remains available while serving is stopped,
with its original job recipient network binding. Unattributed automatic shielding
uses a stable per-address/network/treasury identity. Both paths retain observed
inputs, fee caps and exact prepared-byte recovery.
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
does not run source/swap observers; their unresolved obligations remain durable.

## Best-effort wallet rules

Managed USDC balances are advisory estimates. Payment selection prefers an affordable
bound payer, then other eligible affordable identities, then unknown or low estimates.
Old or unavailable RPC observations and missing historical funding readiness do not
prevent a permitted signature. Actual pool ownership, role, configured amount cap,
asset/domain and caller authority still apply.

Each new authorization is durably journaled before disclosure. That transaction
subtracts its maximum amount once from the current estimate, saturating at zero.
Unknown or malformed estimates stay unknown. Unresolved exposure is not subtracted
a second time. Chain observations may replace estimates even while signatures
remain chargeable; estimates are not a replica of canonical available capacity.

A signed attempt's HTTP 402 can earn another wallet attempt only through the
[pinned x402 seller encodings](../tests/fixtures/x402/insufficient-funds.json):
`PAYMENT-REQUIRED` with exact `error: insufficient_funds`, or a failed
`PAYMENT-RESPONSE` with that exact `errorReason`. An unsigned challenge, arbitrary
prose, a bare 402, allowance failure and malformed/truncated replies cannot earn it.
The existing response-byte and read-inactivity limits still apply.

Each recognized failure marks its wallet low balance and queues an advisory
`balanceOf(..., latest)` read through the existing maintenance owner. Payment
fallback never waits for that read and never allocates or funds a wallet. A failure
during an active read leaves a pending follow-up. A successful read replaces the
estimate; an unsuccessful read retains the previous estimate and reports
`observation_unavailable`. Low balance is a selection classification, not permanent
retirement. It survives restart until an ordinary successful observation clears it.

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

Accepted calculations, exact selected inputs, signed bytes and submission intent
survive cancellation and failed local acknowledgment. Source expiry recovery checks
every selected input and finite inclusive consensus expiry; a refund is never proof
of non-submission. Earlier state schemas are rejected before mutation, without
private-state conversion.

R01 reconstructs failed SDK capabilities and replaces exited funding/reconciliation
tasks through existing ownership. R02 automatically shields owned transparent capital per address under existing
authority. The [follow-on review](plans/treasury_owner_followups.md) records both
implementations and qualification limits.
Reopening never erases uncertain accepted work or recovers unpublished bytes lost
with a process. An accepted shielding calculation blocks new calculation until its
original transaction confirms or existing consensus-expiry recovery proves all
original inputs remain unspent. Automatic initial submission requires current
authority and zero prior attempts; uncertain broadcasts are never automatically
replayed. Missing prepared bytes retain their uncertain accepted input claim.

Multiple charges after the accepted USDC failure category and incomplete external
settlement proof are accepted limitations. The [network guide](network-egress.md#remaining-timers-and-budgets)
records the unresolved 1Click route contract and gRPC upload-inactivity limitation.
