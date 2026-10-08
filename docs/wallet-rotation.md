# Zcash treasury and managed wallet rotation

A single embedded Zcash treasury funds named virtual EVM wallets on Base. Each
virtual wallet is a durable pool with its own keys, balance reservations, payment
cap and active/standby roles. Names determine sharing across sources and MCP
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

Ordinary funding settings describe USDC allocations, not the exchange rate of ZEC:

```toml
[funding]
daily_funding_limit_usdc = "20.00"
total_funding_limit_usdc = "50.00"

[wallets.research]
mode = "zcash_rotation"
funding_amount_usdc = "2.00"
max_funding_amount_usdc = "3.00"
max_api_payment_usdc = "0.05"
max_conversion_overhead_percent = 5
```

| Setting | Meaning |
| --- | --- |
| `funding_amount_usdc` | Target for each newly allocated wallet; defaults to 2 USDC. Initial bootstrap needs two wallets per pool. |
| `max_funding_amount_usdc` | Maximum accepted output when the bridge minimum increases the target. Defaults to the configured funding amount: no automatic increase. |
| `max_api_payment_usdc` | Cap on one API payment, independently of funding budgets. Defaults to 1 USDC. |
| `daily_funding_limit_usdc` | Treasury-wide gross USDC allocation allowance per UTC day. Omission leaves this allowance uncapped. |
| `total_funding_limit_usdc` | Treasury-wide gross allocation allowance across retained history, including retired pools. Omission leaves this allowance uncapped. Raising it explicitly authorizes more future funding. |
| `max_conversion_overhead_percent` | Integer percentage (0–100) by which the bridge's quoted USD input valuation may exceed the USDC output. This includes quoted conversion overhead, not the separately calculated Zcash transaction fee. |

A managed deployment must configure at least one aggregate budget: daily USDC,
total USDC, or daily ZEC. Omitting all three is rejected.

The daily and total allowances cover initial wallets and replacements, shared
across every pool in this treasury state. They reserve the accepted quote's exact
USDC output before transaction preparation, in the same serialized store operation
as the ZEC reservation. Merely quoting or allocating an unfunded address does not
consume an allowance. A typed deferred preparation creates no reservation.

Accounting reuses the durable ZEC journal and authenticated original quotes,
including archived operation bindings; changing pool names, configuration, or
restarting cannot reset it. Missing or malformed relevant history blocks new
funding. The daily bucket follows the source journal's reservation/confirmation UTC
day. Unresolved source transfers or destination allocations remain counted across
midnight until canonical source and destination credit/refund evidence resolves
them. A partial refund retains the pending allocation; fees are not assumed to
explain missing principal. A provider's success/failure message alone releases nothing.

Refunds do not replenish the total USDC allowance: it measures gross funding,
not net expenditure. Verified expiry without a spend and guarded recovery of
unprepared work can release an unused reservation. New budgets never prevent
reconciling already accepted work. When an allowance is exhausted, funding pauses;
existing funded wallets can still pay for API calls. This is not a daily or lifetime
API-consumption budget and excludes transaction fees from its USDC totals.

Advanced safeguards use ZEC because they bound actual native-asset withdrawal:

```toml
[treasury]
# Optional across funding transfers and refund-shielding fees:
daily_treasury_spend_limit_zec = "0.012"
# Defaults; both are acceptance ceilings, not fee overrides:
max_funding_transaction_fee_zec = "0.0003"
max_refund_shielding_fee_zec = "0.0003"

# Within an existing managed wallet profile:
# max_funding_spend_zec = "0.006" # optional: one transfer plus its network fee
```

The ZEC daily limit retains unresolved reservations across UTC rollover and credits
canonically confirmed returned principal. Refund shielding charges only its fee.
The USDC gross allowance deliberately does not use those refund credits.

Zingolib proposes funding transactions and calculates their standard network fees.
Refund shielding uses the backend's standard ZIP-317 fee rule. The application
accepts or rejects the resulting fee; it does not force a lower fee into the
transaction. Larger transactions can exceed a fee ceiling and require an explicit
configuration adjustment. All money values are decimal strings; accounting uses
integer atomic units, never floating-point currency conversion.

The bridge's USD valuation is not an independent exchange-rate oracle. USDC
allocation and overhead limits do not replace a hard ZEC withdrawal limit when
one is required. All financial limits still apply alongside qualification permits;
no configuration value authorizes replay or bypasses a zero-new-funding restriction.
Positive funded qualification still requires an explicit `max_funding_spend_zec`
to bound its native-asset permits.

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
wallet corruption or authorize a reset/rescan. Tree addresses and note positions
are omitted. CLI errors and saved sync
`last_error` retain the same sanitized details. Upstream messages, metadata and
wallet identifiers are omitted. These diagnostics do not change retry or funding
policy; a successful sync is still required before new treasury preparation.


The Zcash treasury is exclusively spent by this application. Direct and Tor sync
use the same bounded policy: a successful scan must reach the tip observed at the
start, and may trail the final indexer observation by at most **three blocks**.
A regressing indexer tip or greater lag fails the observation. Confirmation counts
and spendability use the **scanned** height, never the newer unscanned tip.

`treasury.max_sync_age_seconds` (default 300) limits the age of the starting tip
observation, including sync and validation time; completing a slow sync does not
restart this clock. Status records `target_height` (starting tip), `height` (scan),
`observed_tip_height` (final query), and `checked_at` (starting observation time).
Historical snapshots have no observed tip. Accepted lag emits a warning in serving
logs and wallet-sync stderr. Background serving continues periodic catch-up; the
one-shot wallet command reports its observation and exits.

This allowance does not change reservations, outgoing transaction tracking,
confirmation depth, reorg checks or expiry. Recovery that releases an expired
transaction's reservation still requires zero observed tip lag on both syncs,
along with buried expiry and confirmed unspent inputs. Unknown historical tip
evidence cannot satisfy that recovery check. Do not spend the treasury from an
external wallet while relying on this policy.

## Double buffering and admission

Healthy steady state has one `ACTIVE` address and one funded `READY` standby.
During refill, the active continues serving while a new candidate progresses
through allocation/funding. Bootstrap allocates the initial pair and establishes
readiness using independently verified credit.

The default future allocation floor is **2.00 USDC**. The funding adapter validates
an exact-output quote and may raise a new allocation's target to the reported
bridge minimum only up to `max_funding_amount_usdc`, subject to funding budgets
and fee caps. Omitting that option permits no increase above `funding_amount_usdc`. This is a floating route
constraint, not a hard-coded permanent minimum. A quote-bound target cannot be
resized after acceptance. Changing configuration affects future allocations.
A double-buffered pool allocates roughly twice its target; allocation is not API
consumption and is not a lifetime spending cap.

`rotation/manager.rs` admits only supported Base USDC v2 exact EIP-3009 offers for
managed wallets. It validates offer/domain/amount and schema metadata before
signing or promoting. Unsupported schemes/assets and over-cap or over-target
offers cannot cause wallet churn. Advertised extensions outside the managed
payment contract are omitted with visible diagnostics; seller rejection remains
possible. See the README for the supported extension policy.

Canonical balance and nonce observations, less unresolved reservations, govern
spendability. Funds tied up in pending authorizations cause bounded waiting or a
`payment_pending` result, not automatic promotion. A depleted active can promote
only a verified standby. The store compares the expected generation and commits
retirement, activation, the replacement key and its funding outbox together.
Concurrent callers cannot independently allocate replacement wallets for the same
promotion. Existing pending operations retain their original identities.

The signer/transport candidate captures the actual EVM address and generation.
If promotion occurs before signing, GET/HEAD can obtain one new challenge under
the new identity; other methods return an actionable payer-change error. Once a
payment is signed, its authorization is durably journaled before network submission.
The pool gate releases before awaiting the signed HTTP response, allowing other
admissible calls to progress without hiding the in-flight liability.

## Payment reconciliation

A logical paid request has one signed attempt. A final 402, upstream error,
connection timeout or response-body loss does not authorize replay on another
wallet. A missing response is not proof that settlement failed. Retired addresses
remain under reconciliation and accept no new payments.

`rotation/base.rs` verifies Base chain identity, confirmed anchors, freshness,
block-pinned USDC balances and EIP-3009 authorization use. A used authorization
or an expired authorization proven unused can release its reservation through the
store's canonical reconciliation transaction. An unused but still-valid signature
remains a liability. Reorg contradictions cannot silently reset accounting.

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

`rotation/funding.rs` drives the durable outbox through the states defined in
`rotation/store/funding.rs`:

```text
ALLOCATED → QUOTED → PREPARING → PREPARED → DEPOSIT_PENDING
                                                 ↓
                                              SWAPPING
                                                 ↓
                                         VERIFYING_CREDIT → COMPLETE
```

A confirmed credit path can skip the separate swapping observation. Refund and
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
per-operation/daily exposure and its single unresolved outgoing-operation gate.
Zingolib calculates/proves without automatic broadcast. The post-calculation wallet
snapshot, prepared bytes and accounting facts must commit before submission.
Calculation followed by persistence failure invalidates that in-memory owner for
further preparation; reopening restores the last durable state rather than making
a conflicting send.

An uncertain broadcast retains the saved bytes and transaction identity. Recovery
reconciles that operation; explicit rebroadcast uses the same bytes. A timeout,
indexer omission or provider status is insufficient proof to release source inputs.
Expiry recovery additionally requires canonical chain/input evidence. Source
confirmation and destination credit are independent: a NEAR success response alone
cannot mark a candidate spendable, and Base credit alone cannot prove source
confirmation. Concurrent credit observers cannot overwrite a completed job with
an obsolete persistence warning; genuine source-reconciliation errors remain visible.

The treasury receives/synchronizes Ironwood with capability checks on the injected
lightwalletd transport. Network integration uses reviewed vendored channel-injection
patches; it does not enable SDK-owned direct networking. The
[egress inventory](network-egress.md) explains treasury versus immutable recipient
identities and the remaining upstream timeout boundaries.

## Refunds, state and recovery

Each swap uses a persisted unique high-index transparent refund address derived
from the treasury. Refund visibility does not make funds automatically spendable
for another deposit. Refund shielding is an explicit operation with observed
inputs, fee caps, prepared-byte recovery and its original recipient network binding.
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
