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
availability. Selected inputs must exceed the actual proposed fee before accepting
input ownership: the pinned SDK permits fee-only proposals, which cannot recover
capital. Uneconomic/empty addresses do not block independent addresses. No
principal reservation, attribution requirement or funding-claim reopening is added.
The explicit per-address command remains available.

Zingolib selects inputs and calculates standard network fees. The application
checks the actual proposal against configured per-operation fee/spend safeguards,
then persists the exact input owner before calculation. A configured maximum is a
rejection ceiling, not a capital reservation. Unknown outcomes after calculation
acceptance retain that input fence. The preparation phase commits with input
ownership; known proposal shortages and fee rejections leave the quoted operation
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
again for an actual preparation or submission. Scheduled source observations share
the last successful current-owner scan without fresh tip probes between due scans;
see [wallet observation policy](#wallet-observation-policy).

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
with DEBUG diagnostics for `bazaar` discovery metadata and INFO diagnostics for
other extensions during challenge preparation; this does not establish signing
or submission. Seller rejection remains possible. Terminal payment-call failures
are WARN events; see [logging and diagnostics](development.md#logging-and-diagnostics).

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
Settlement receipts take precedence over simultaneous insufficient-funds challenges.
`X-PAYMENT-RESPONSE` (v1, network `base`) and `PAYMENT-RESPONSE` (v2, network
`eip155:8453`) share one ambiguity boundary: duplicates, including both names
together, grant no retry. Qualification normalizes valid legacy receipt networks
to `eip155:8453` for existing canonical-debit recovery; seller claims remain
unverified until the chain checks succeed.

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
After source observation, one serialized Store job publishes any due phase,
settles proven completion and reads the current operation and facts. These steps
retain their existing transactions; they are not one all-or-nothing transaction.
Derived eligibility for new funding is shared by observers and source work;
mutation boundaries still recheck current preparation and submission authority.

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
Unknown preparation outcomes, refunds and unresolved confirmed swaps require review
before restarting or replacing the funding attempt. Ordinary discovery and shielding
of owned outputs can continue under the current funding authority.

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

The observation scope, cadence and completion rule belong to the work requesting it:

| Trigger / work | Observation and cadence | Completion / stop rule |
| --- | --- | --- |
| Fresh managed serving startup | One finite Base pass over existing eligible managed wallets and initial-funding recipients; one background ZEC scan when the treasury capability is available | Finish or report failure of the captured pass; new allocations do not extend it and startup alone creates no recurring watcher |
| Preparing a new wallet's initial funding | Discover SDK funds as needed, then read that exact Base destination after a valid spendable proposal; reuse applicable same-work observations | Positive credit consumes the unused claim; acceptance rechecks the claim before calculation. An advisory RPC failure does not gate funding, while Store/publication failure defers it |
| Enabled initial-funding demand waiting for capital or confirmations | Shared ZEC scans; each recurring scan starts at least 60 seconds after the previous scan completes | Stop when no enabled unfilled claim can advance and no validated accepted transaction needs observation; a new claim wakes discovery |
| Accepted initial funding without positive destination credit | Shared Base recipient acquisition when due; each repeated read starts at least 60 seconds after the previous read completes | First positive credit permanently finishes that claim's credit lane; source recovery remains independent |
| Recognized signed-payment insufficient funds | Asynchronous scalar Base refresh when due, coalesced per wallet; each repeated read starts at least 60 seconds after the previous read completes | Success, including zero, finishes the captured request. Failure or a notification arriving during the read retains follow-up work |
| Validated pending source or shielding transaction | Shared ZEC scans and operation observations with their own completion-based minute floor | Canonical confirmation or exact consensus-expiry/input recovery resolves that obligation, including after destination credit, disablement, removal or restart |
| Submitted swap awaiting provider outcome | NEAR status observations; each recurring check starts at least 60 seconds after the previous check completes | Existing provider outcome/recovery rules stop that lane; status alone cannot release source inputs |
| Explicit `wallet sync` | One requested ZEC scan and configured managed-USDC pass, including outstanding accepted funding recipients | Report each asset's completion, failure or inapplicability; create no background watcher and require serving to be stopped |
| Completed-result publication or SDK reconstruction | Existing local owner retries after 60 seconds, independently of scan scheduling | Publish the identical result or reconstruct the SDK; this work alone requests no network scan or new calculation |
| Fully funded idle serving without requested refresh or accepted observation work | No recurring Base or ZEC balance discovery | Relevant new work requests observation again |

The interval applies per wallet, operation or serialized treasury scan; it is not
one RPC per minute globally. A scan, provider fallback or canonical proof can issue
several requests. Recurring failures retain independent backoff bounded by
300 seconds and existing jitter where applicable; a new event or sibling success
cannot shorten that work's interval or clear its failure. Configured local polling
and healthy handoffs do not change these network observation floors.

Store-only serving retains Base startup discovery when its configuration is usable.
It has no SDK/source capability and cannot perform startup ZEC discovery or source
observation; the reported capability failure requires correction and restart.
Accepted obligations remain durable. This existing degraded-serving boundary does
not create a hidden SDK owner or relax strict inspection/qualification requirements.

Listeners bind before startup discovery. Ordinary serving does not sweep idle
ACTIVE, READY or RETAINED wallets.

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
jobs. Replaced or disabled claims do not keep discovery running; validated accepted
transactions retain their independent journal-based observation demand.
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

Positive destination credit and provider outcomes cannot finish a pending source
transaction. Its original validated journal keeps observation demand across stale
phases, disabled/removed profiles, cancellation and restart. Accepted unpublished
calculations and invalid retained bytes keep their input fence, but do not request
scans that cannot reconstruct or publish them. Such an owner also prevents new
funding-demand scans whose calculations it blocks. Completed-result publication and
SDK reconstruction retry locally at their own 60-second cadence, independently of
scan scheduling. A successful needed persisted scan may recover owned transparent
funds through automatic shielding under existing permission/fee checks; an accepted
shielding transaction continues observation even if its original funding demand
vanishes. Once source confirmation/expiry resolves, destination/provider waiting
alone does not keep ZEC discovery running.

## Best-effort wallet rules

Read the [wallet observation policy](#wallet-observation-policy) when changing
balance estimation, refresh scheduling, funding or recovery.

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

Recurring initial-credit polling begins only after durable source acceptance,
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
every selected input and finite inclusive consensus expiry; a refund is never proof
of non-submission. Earlier state schemas are rejected before mutation, without
private-state conversion.

Failed SDK capabilities reconstruct and exited funding/reconciliation tasks resume
through existing ownership. Automatic shielding recovers owned transparent capital
per address under existing authority. Local tests cover these paths without live-funded,
consensus or Tor qualification; see [testing](testing.md#qualification-status).
Reopening never erases uncertain accepted work or recovers unpublished bytes lost
with a process. An accepted shielding calculation blocks new calculation until its
original transaction confirms or existing consensus-expiry recovery proves all
original inputs remain unspent. Automatic initial submission requires current
authority and zero prior attempts; uncertain broadcasts are never automatically
replayed. Missing prepared bytes retain their uncertain accepted input claim.

Multiple charges after the accepted USDC failure category and incomplete external
settlement proof are accepted limitations. The [network guide](network-egress.md#remaining-timers-and-budgets)
records the unresolved 1Click route contract and gRPC upload-inactivity limitation.
