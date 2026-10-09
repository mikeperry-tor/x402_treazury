# Remove avoidable failure policies

## Objective and scope

Let valid work finish while it is making progress. Remove arbitrary elapsed-time
limits, duplicated guards and unnecessarily broad failure propagation. Preserve
checks that establish a correct result, prevent duplicate spending, enforce an
external expiry, or bound resource consumption.

This is unfinished implementation work, not authorization for live spending,
replaying uncertain requests or relaxing qualification requirements. Use offline
fixtures and temporary unfunded wallets. The earlier sync `Cancelled` log does
not establish which timeout fired; reproduce and identify the responsible layer.

Already implemented: ordinary HTTP downloads use renewable read inactivity;
Base RPC/view/receipt and managed admission total deadlines are removed; catalogs
use configurable local retention. Do not reintroduce those deadlines under new
names. Caller cancellation remains supported.

## Disposition of the audited policies

| Policy | Disposition |
| --- | --- |
| Treasury outer 15–30-second operation deadlines | Remove redundant wrappers once stalled gRPC calls have reliable inactivity handling. |
| Library unary RPC total deadlines, including 10/20-second defaults | Replace on the actual application paths with connection and read-progress handling. |
| 15-second wait for a complete sync-stream message | Replace with progress tracking that permits a slowly arriving large message. |
| Ten-second HTTP shutdown drain | Replace automatic abandonment with progress-reporting graceful drain and explicit force. |
| MCP library two/five-second drain allowances | Trace closure/cancellation ownership; prevent these from implicitly abandoning application-owned paid work. |
| Run-wide discovery-relay shutdown after any failure | Narrow according to proven submission state and failure scope; preserve uncertain-payment protection. |
| Whole-sync observation-age rejection and fixed tip-lag acceptance | Rework catch-up/freshness acquisition; never relabel old evidence as fresh or simply remove final evidence checks. |
| Fixed 300-second quote safety margin | Determine the actual settlement requirement; remove any unjustified blanket margin, preserving upstream expiry and required delivery time. |
| All-or-nothing startup for every source | Add explicit optional-source failure isolation without silently losing required inventory. |
| `swap_timeout_seconds` health threshold | Clarify its name/logging: it marks delayed funding and continues reconciliation; it is not an abort deadline. |
| Exact live-tip equality during treasury preparation | Reconcile with accepted sync lag; perform safe readiness checks before poisoning the owner. |
| Base freshness checked after a serial reconciliation sweep | Avoid making wallet/authorization count an implicit elapsed-time limit; preserve coherent canonical evidence. |
| Managed offer metadata allowlists and funding-target payment ceiling | Separate payment authority from annotations and funding preferences; retain explicit spend caps. |
| Quote transport errors consuming funding recovery attempts | Separate bounded read retries from durable transaction/recovery attempts. |
| Maximum-fee headroom required before actual fee calculation | Investigate rejecting affordable transactions because their configured fee ceiling is larger than their actual fee. |
| Source-import failure caching and eviction of unfinished fetches | Separate active coalescing from completed-result retention and transient failure backoff. |

The findings below are source-level audits, not reproductions of the reported
connectivity incident. Paths and symbols identify the observed guards; proposed
relaxations still need the specified offline fixtures. No runtime behavior or
spending authority changes merely because it appears in this plan.

## 1. One progress policy for treasury/indexer traffic

Relevant code: `src/network.rs`, `src/treasury/{mod,send,submission,birthday,server}.rs`,
`vendor/zingo-netutils/`, and `vendor/pepper-sync/`.

1. Inventory the actual unary, streaming, sync-start, submission and recovery call
   paths. Distinguish transport connection waits, byte/message progress, library
   total deadlines, local worker acknowledgements and cleanup timers. Do not
   assume every timeout constant in a dependency is exercised here.
2. Extend the network-owned injected gRPC transport to use the configured
   `connect_timeout_seconds` and `read_timeout_seconds` in direct and Tor modes.
   A silent request must fail; an actively arriving response must continue.
   Streaming progress must not require a complete protobuf message within the
   inactivity period. Keep message-size and buffering limits.
3. Make inactivity request/stream-specific. Traffic on another HTTP/2 stream,
   keepalive frames or an unrelated wallet must not keep a stalled RPC alive.
   Waiting for initial response data must also be covered.
4. Remove application outer deadlines and library total-request deadlines on
   those paths, including `grpc-timeout` settings that would cancel a progressing
   response. Do not simulate removal using enormous durations. Preserve actual
   protocol expiry and caller cancellation independently of network progress.
5. Consolidate duplicated connection guards. Remove the direct/Tor operation-time
   distinction where it no longer has a consumer; retain intentional differences
   in connection establishment. Network construction stays in `src/network.rs`.
6. Make failures identify the responsible layer and operation using bounded fixed
   labels. A submission transport failure retains an unknown outcome and durable
   liabilities; neither inactivity nor cancellation authorizes resubmission.

Patch only reviewed vendored sources, update provenance, and run the existing
vendor verification tools. Do not edit Cargo caches or reference checkouts.

Acceptance: slow unary responses and large fragmented streaming messages survive
the old deadlines; silent headers and stalled bodies fail on inactivity; unrelated
streams cannot renew the timer; cancellation closes owned work without retries;
direct/Tor identities and remote-DNS policy remain intact. Verify unknown
submission outcomes retain exact saved bytes and recovery obligations.

### Call-path audit findings

- `NetworkContext::grpc` injects a cached tonic channel with a connection timeout,
  but no application read-progress wrapper. Treasury sync, birthday lookup,
  capability checks, preparation and submission use that factory. Changing the
  HTTP client alone cannot fix these paths.
- `vendor/pepper-sync/src/client/fetch.rs` passes `UNARY_RPC_TIMEOUT` (10 seconds)
  and `HEAVY_UNARY_TIMEOUT` (20 seconds) into the injected indexer. The latter
  also applies to **streaming** block ranges, nullifier ranges and subtree roots.
  `vendor/zingo-netutils/src/lib.rs` writes these into `grpc-timeout` through
  `Request::set_timeout`. Removing only `stream.message()` timers leaves a
  server-visible total deadline that can still cancel a progressing stream.
  These library constants do not pass through the application's 240-second Tor
  operation allowance.
- `vendor/zingolib/src/lightclient/sync.rs::sync` polls for a mode change every
  50 ms, then aborts the spawned sync after three seconds if it is still pending.
  Pepper Sync sets the mode at engine entry, before network reads. This is a local
  scheduling/acknowledgement guard, not a network inactivity timeout. Replace the
  polling assumption with explicit started/completed ownership and cancellation;
  test a delayed task start without treating it as failed network traffic.
- `vendor/pepper-sync/src/scan/task.rs` aborts loader and scan-worker handles after
  ten seconds during shutdown. Loader timeout returns an error; worker timeout
  returns `Ok(())` after abort. Trace normal retirement, sync cancellation and
  error cleanup separately. Drain useful scan work with observable ownership,
  and never report forced abandonment as successful completion. A network read
  timer cannot establish that CPU scanning is stuck.
- Retain the one-second mempool drain ceiling as a separate policy for now:
  `sync.rs::drain_verdict` re-enters the main loop when scan workers remain and
  permits shutdown only with no workers. It is not a one-second sync deadline.
  Likewise, Nym/bootstrap/probe and quick-send constants found in the vendor are
  not evidence that this application's injected transport or durable send path
  exercises them. Do not patch those merely because a search found a timer.

## 2. Graceful shutdown that finishes accepted work

Relevant code: `src/server.rs`, deployment shutdown and the pinned MCP service.

- First shutdown request stops new admission and optional cover work, then drains
  already accepted application work without a fixed ten-second cutoff. Report
  counts, stages and elapsed time without credentials or provider content.
- Provide an explicit force action, such as a second signal, with a clear message
  that unfinished paid operations may have unknown outcomes. Force never releases
  reservations or marks operations successful.
- Trace MCP transport closure and its two/five-second response-drain allowances.
  Separate an unavailable response recipient from ownership of accepted financial
  work. Do not keep trying to deliver indefinitely to a closed client, or let
  library cleanup implicitly erase application recovery obligations.
- Preserve serialized treasury ownership and safe worker teardown. A service
  manager's external kill deadline remains outside application control.

Acceptance: a paid response arriving after ten seconds can finish during graceful
drain; no new calls enter; force and client disconnect retain liabilities; shutdown
reports why it is waiting; restart reconciles original operations without replay.

The pinned `rmcp = 3.5.0` audit distinguishes two mechanisms: `src/service.rs`
has five-second EOF and two-second cancellation **response-drain** timers;
`src/transport/streamable_http_server/tower.rs` also has stateless request drop
guards that cancel handlers on client disconnect before the first response.
The application's JSON/stateless transport exercises the latter family of paths.
Simply enlarging the response-drain allowances does not establish ownership of
accepted paid calls. Add application-owned tracking and tests for disconnect
before signing, after journaling, and while receiving a paid response, including
stdio EOF. Inspect dependencies read-only; any necessary patch must be pinned
and reviewed in the repository.

Deployment shutdown additionally cancels a shared stop token before draining
listeners (`src/deployment.rs`), then joins reconciliation, funding and treasury
tasks. Audit whether prerequisites for accepted work stop too early. Removing
the HTTP cutoff must not leave a call waiting forever on a funding worker already
stopped by the same shutdown signal. A failed listener currently stops sibling
listeners too; keep that explicit lifecycle policy unless independent failure
domains are designed, rather than conflating it with an optional source's failure.

## 3. Restrict relay failures to the affected operation

Relevant code: `src/discovery_relay.rs` and the typed payment result/error boundary.

- Replace the undifferentiated failure switch with typed evidence distinguishing
  failure before any potentially paid submission, target-specific failure, and
  possibly submitted/unknown paid outcome.
- A proven pre-submission failure must not disable unrelated targets for the
  entire run. Use finite attempts and backoff where needed to avoid storms.
- Preserve coalescing, wallet scoping and target-specific results. Do not infer
  submission state from error prose, absent receipts or HTTP status alone.
- Keep the conservative stop for uncertain paid relay work unless durable
  reconciliation establishes what further action is authorized. No automatic
  signed replay or repeated charge to repair an output/parse failure.

Acceptance: a pre-submission outage does not permanently poison unrelated work;
concurrent aliases do not multiply charges; post-submission cancellation remains
reserved; target failures do not disable healthy targets; logs explain the scope.

Confirmed boundary: relay `fetch` sets shared `state.stopped = true` before
`PaidClient::execute_response`, and maps every execution error to a generic relay
error. Only a successfully decoded response or typed `TargetFailure` clears it.
Thus even a failure before obtaining a payment challenge can disable other relay
wallets/targets. Carry typed submission facts across this boundary; do not try to
recover them from the generic error later. The outer Curl response is separately
bounded by the bootstrap provider's `max_response_bytes`, while the decoded
target uses `max_spec_bytes` or the pricing bound. Document both limits and allow
for JSON envelope/escaping overhead; raising only the target limit cannot make a
larger response fit the outer limit. Neither size failure authorizes a paid retry.

## 4. Freshness without throwing away useful synchronization

Relevant code: `src/treasury/freshness.rs`, sync orchestration and durable sync facts.

- Preserve completed scanning/checkpoints even when spending readiness is not yet
  established. Continue incremental catch-up rather than treating a long initial
  scan as work that must be discarded or repeated from scratch.
- Separate historical scan duration from acquisition of a fresh, validated final
  observation. Establish a new observation only through the required new reads
  and catch-up work; never reset the old observation's timestamp at completion.
- Establish and test the evidence needed for normal spending before changing the
  three-block allowance. Report precise remaining lag/readiness instead of a
  generic sync failure. Continued catch-up must remain cancellable and observable.
- Preserve stricter expiry/non-inclusion recovery requirements, scanned versus
  observed tip heights, and canonical reconciliation before releasing exposure.

Acceptance: a slow initial sync can become ready through incremental catch-up;
moving tips do not cause repeated full work; stale/regressed observations never
authorize spending; recovery cannot release funds using incomplete scan evidence.

### Preparation contradicts the accepted sync-lag policy

`src/treasury/send.rs::prepare` requires the saved scanned height to equal a
**newly fetched** `get_lightd_info().block_height`. Sync's three-block allowance
therefore does not translate into preparation readiness: even a single new block
between sync and this read can reject the deposit. Worse, the method has already
reserved capacity, set phase `Preparing` and marked `self.healthy = false` before
the network/height checks. A routine readiness race is turned into teardown and
durable recovery, before `propose_send` or `calculate_stored_proposal` runs.

Move demonstrably non-mutating readiness checks before the uncertain-preparation
boundary and use one documented ordinary-spending freshness policy. Where
catch-up is required, perform it before acquiring preparation authority. Issue
`PreparationDeferred` only through a proven pre-preparation path with no outgoing
or budget record; an error after the current boundary is not retroactive evidence
that retrying is safe. Do not loosen exact-tip expiry recovery. Test one-to-three
blocks of tip advancement, regression/reorg, slow network validation, and a crash
on each side of the reservation/preparation boundary.

### Base reconciliation retains an implicit elapsed-time limit

`src/rotation/base.rs::view_inner` validates latest-block time both before and
after fetching balances and pending authorization states serially. The default
`base_max_block_age_seconds` is 120 seconds (`src/rotation/config.rs`). A progressing
sweep can age its initial block out solely because the pool has more wallets or
pending authorizations. Removing the former outer timeout does not resolve this.

Separate historical canonical resolution from the fresh admission view where
the accounting model permits it. Reduce repeated reads with bounded concurrency
or supported batching while preserving per-wallet network identities, then
acquire/revalidate the necessary fresh coherent view. Never stamp old balances
with a new tip or combine fallback providers into one view. Test large pending
sets and slow individual responses, including a moving tip and reorg: valid
historical work should remain useful, but stale balance evidence must never admit
a new payment. Treat this as an evidence-acquisition redesign, not deletion of
the final freshness check.

## 5. Replace the blanket quote margin with an actual delivery requirement

Relevant code: `src/rotation/{near,funding,transaction}.rs`, treasury preparation,
submission and the durable store's final submission check.

- Identify what the provider deadline means: broadcast, inclusion, confirmation
  or another settlement event. Record the contract evidence before changing the
  300-second margin. A transaction being unexpired at local submission is not
  necessarily sufficient for successful delivery.
- Remove the fixed margin only to the extent it is unsupported by that contract.
  Consolidate repeated checks into one named policy used consistently before
  preparation and at submission. If a delivery allowance is genuinely required,
  document/configure that allowance rather than inventing a total RPC deadline.
- Refresh an expiring quote before preparation where safe. Once preparation has
  begun or bytes exist, retain original identity, costs, deadlines and liabilities;
  do not obtain a replacement quote as permission to repeat the transfer.

Acceptance: a usable quote is not rejected solely by an unjustified constant;
expired or undeliverable quotes never authorize submission; slow preparation and
cancelled submission cannot allocate or spend twice.

The margin is duplicated in `near.rs::validate_quote`, funding's quoted/prepared
steps, `treasury/send.rs`, submission, store submission admission, and the
configuration minimum for `quote_deadline_seconds`. Update all consumers together;
otherwise removing one check just moves the failure later, possibly after signing.
Funding's tick also carries an `instant` captured before awaited work: take a new
clock reading at each authority decision rather than depending on that tick time.

Contract research found that NEAR's [quote documentation](https://github.com/near/agent-skills/blob/main/skills/near-intents/rules/api-quote.md)
describes the response deadline as when the deposit must arrive. This supports
retaining a delivery requirement, but does not establish a universal 300-second
margin or the exact Zcash confirmation/detection boundary. That chain-specific
boundary remains unresolved; obtain it from the provider contract before changing
submission policy. A local broadcast timestamp alone does not prove timely arrival.

### Zodl implementation evidence and its limits

The local reference apps provide a narrower interpretation of the five-minute
constant. Audited paths are relative to `reference_repos/zodl-ios` at
`6a7dc6b39cb8`, `zodl-android` at `9f4e719e3eee`, `zodl-swift-wallet-sdk` at
`cf07eb228c1b`, and `zodl-android-wallet-sdk` at `ae538fc5b39f`. The inspected
application and SDK transaction paths had no working-tree changes.

- **Both apps request two hours**, not five minutes: iOS
  `secant/Sources/Dependencies/SwapAndPay/sources/Near1Click.swift` constructs
  `deadline = now + 120 * 60`; Android
  `ui-lib/src/main/java/co/electriccoin/zcash/ui/common/datasource/NearSwapDataSource.kt`
  uses `Clock.System.now() + 2.hours`. This is the requested deadline, not proof
  that every returned quote grants that entire window. Compare our 1,800-second
  default separately from the submission margin; a longer requested window may
  accommodate preparation without changing any authority after signing.
- **Five minutes is a client-side pending-status rule.** iOS `Near1Click.swift`
  and Android `common/model/near/NearSwapQuoteStatus.kt` synthesize `expired` only
  when the server still says `PENDING_DEPOSIT` and local time passes returned
  deadline minus five minutes. Other server statuses are not expired by this
  check. Android's `NearSwapQuoteStatusTest` explicitly pins this behavior.
  This is also a polling policy: Android `GetSwapStatusUseCase` stops on terminal
  status; iOS `TransactionDetailsStore` schedules another check only for pending
  statuses, which exclude `expired`. Do not copy this local status inference into
  treasury reconciliation or release financial exposure because of it.
- **No equivalent five-minute pre-broadcast guard was found in the traced send
  paths.** iOS's `SwapQuote` model does not even retain the response deadline;
  Android retains it, but the inspected proposal/submission flow does not check
  it. This distinguishes their actual use of 300 seconds from our repeated
  quote/preparation/submission veto. It does not prove sending near expiry is safe.
- **Deposit notification follows submission without a confirmation wait.**
  iOS `Features/CoordFlows/SwapAndPayCoordFlowCoordinator.swift` calls
  `submitDepositTxId` after the SDK reports submission success, with a comment
  that it speeds processing, and ignores notification failure. Android
  `common/usecase/ProcessSwapTransactionUseCase.kt` sends each nonempty result
  transaction ID and catches notification errors; it does not require success
  or confirmations at that point. Neither notification is settlement evidence.
- **SDK success is broadcast evidence.** Swift's
  `Transaction/EndpointSubmitter.swift` checks the lightwalletd submission
  response; Android's `internal/transaction/SubmitTransaction.kt` accepts a
  successful submission response or, for certain failures, a successful lookup.
  These paths do not establish a NEAR-specific Zcash confirmation depth. The
  SDKs also have ordinary transaction resubmission machinery independent of the
  quote; do not adopt that behavior as permission to replay treasury operations.

Disposition: the Zodl sources corroborate the existence of a five-minute local
heuristic, but do **not** justify a five-minute financial submission requirement.
Keep the provider deadline and investigate a larger pre-preparation quote window;
do not claim that either change guarantees delivery. To close the remaining
contract question, obtain the 1Click service/Zcash deposit-watcher implementation
or authoritative integration tests/specification defining the event compared
with the deadline (mempool detection, inclusion, confirmations), required depth,
and late-deposit/refund behavior. Another wallet UI checkout is unlikely to
answer that server-side question.

### Omni Bridge confirmation audit

The additional clean reference checkouts under `reference_repos/Near-One` were
inspected at `bridge-sdk-rs` revision `d1361ac1c68f`, `omni-bridge` revision
`3f0349044938`, and `omni-bridge-services` revision `00ccb5d1dd89`.

- `omni-relayer/src/startup/event_handlers.rs` consumes `TransferUtxoToNear`
  events and computes a light-client target through `utils/utxo.rs`. That path
  obtains the transaction's containing block, then targets inclusion height plus
  required confirmations minus one. It is not merely mempool detection. The
  `startup/utxo_lc_poller.rs` queue waits for the NEAR-hosted light client to reach
  the target and republishes the event; the default 30 seconds is polling cadence,
  not a deposit expiry. Slow light-client relay can delay delivery even after the
  Zcash node itself has enough blocks.
- Confirmation depth is not a universal time margin or fixed block count.
  `bridge-sdk/bridge-clients/near-bridge-client/src/btc.rs` reads an amount-tier
  strategy and adds a delta according to the deposit-message path and relayer
  whitelist. Its live `get_required_confirmations` query also supports contracts
  with block-cumulative amount rules. Only method-not-found enables the older
  local formula; other query failures do not silently lower required depth.
- The initial event deferral uses the amount-tier estimate. The worker calls the
  unchecked SDK finalizer, leaving enforcement to the connector contract, and
  handles insufficient-confirmation receipts by asking for the exact target and
  re-queuing. Do not mistake the optional SDK `_checked` helper for the only
  enforcement point, or import its approximate precheck as settlement evidence.
- The actual deposit-verification contract is separate from `omni-bridge`:
  [Near-One/btc-bridge](https://github.com/Near-One/btc-bridge), `satoshi-bridge`.
  Direct inspection of its public revision `7740fae0ed8d` confirms that
  [deposit verification](https://github.com/Near-One/btc-bridge/blob/7740fae0ed8dc56571adfe2aaf36bd4edee0d441/contracts/satoshi-bridge/src/btc_light_client/deposit.rs)
  passes the computed confirmation count into a light-client inclusion proof and
  requires a successful proof before minting. The inspected public view file
  lacks the newer `get_required_confirmations` method supported by the SDK;
  do not assume matching repository heads, deployed bytecode, or live settings.
- No 1Click quote-deadline handling was found in the traced paths. In particular,
  chain confirmation is a requirement for this bridge's mint/finalization, but
  the code does not establish whether 1Click compares its deadline to initial
  detection, inclusion, completed confirmation, or another event. Opaque deposit
  messages and externally supplied indexer events do not resolve that gap.
- Route identity remains unproven: the SDK's defaults identify
  `zcash-connector.bridge.near`, `zcash-client.bridge.near`, and
  `nzec.bridge.near`; our adapter requests `nep141:zec.omft.near`. Migration or
  routing may connect these, but the names alone are not proof. Establish the
  actual quote route and connector before applying these confirmation rules to
  our deposits. No live configuration or private financial evidence was queried.

Disposition: preserve canonical confirmations, account for light-client lag when
reasoning about delivery, and do not derive a 300-second cutoff from block depth.
The missing evidence is still the 1Click deadline/route contract. A local checkout
of `btc-bridge` would support deeper connector work, but its inspected public
deposit code does not itself close that remaining question.

The worker currently moves a never-submitted prepared operation to
`RecoveryRequired` when fewer than 300 seconds remain; its diagnostic explicitly
says recovery waits for transaction expiry. Avoiding an unjustified margin before
preparation is especially valuable here. Do not turn this into automatic replay
or deletion of already prepared bytes.

## 6. Isolate optional provider availability

Add a provider/source optional-startup policy, with required sources retaining
their complete-inventory guarantee. A failed optional source must have an explicit
operator and agent-visible unavailable status, not appear intentionally empty.
Never publish a partially parsed catalog or omit wallet initialization implicitly.

Coordinate with [independent provider startup](deferred/independent_provider_startup.md).
Use that plan for lifecycle/publication details rather than maintaining competing
implementations. Qualification/frozen inventories remain exact and fail on missing
required sources. Pricing failures remain visibly unknown, not fabricated prices.

Acceptance: unavailable optional sources do not prevent usable required sources
from serving; required-source failures still fail; aliases, listener filters,
management snapshots and qualification retain their documented semantics.

## 7. Clarify health thresholds and finish the audit

Rename/document the swap delay threshold as a health warning with a compatibility
alias if a configuration rename is made. It must continue reconciliation, never
trigger replacement deposits or silently reset funding history.

Retain resource-size/storage limits, authentication, financial caps, canonical
evidence, signed-authorization/transaction expiry and qualification authority.
Cover-episode limits bound optional traffic; they must not abort the real call.
HTTP/2 and TLS 1.3 remain mandatory by default. Providers that do not support
them require explicit `allow_http1` and/or `allow_tls12` compatibility flags.
This deterministic security and functionality policy is intentional and outside
this cleanup; it does not depend on network speed or concurrent application work.
Finite retry counts prevent storms or duplicate effects. Short database lock waits
should be reviewed only with evidence of avoidable contention, not deleted as if
they were download deadlines.

Audit tests separately from production: test harness deadlines are required to
make failures terminate. Keep diagnostic probes and explicit operator cancellation
bounded where their documented purpose requires it.

### Additional policy findings and bounded changes

1. **Transient quote outages exhaust durable attempts.** In
   `src/rotation/funding.rs::finish_step`, every non-wait error in `Allocated`
   increments the quote-attempt count; reaching profile `max_attempts` (default
   three) changes the job to `RecoveryRequired`. This includes token/quote HTTP
   failures before preparation. Quote refresh also consumes this attempt policy.
   Separate bounded per-pass read retries/backoff and outage health from durable
   preparation/recovery attempts. Keep explicit operator/qualification limits,
   cumulative allocation budgets and original history; never reset an uncertain
   transaction's attempts. Test repeated pre-quote outages followed by recovery
   with zero preparation/submission and no extra allocation. Also retain the
   actual behavior of `swap_timeout_seconds`: it continues reconciliation but
   raises the polling delay floor to 60 seconds, so a rename must document that
   scheduling effect as well as the warning.

2. **Funding size is also a second API payment ceiling.**
   `manager.rs::select_offer` rejects `amount > target`, independently of
   `max_api_payment_usdc`; `store.rs::admit_for` repeats that condition. A wallet can
   contain sufficient confirmed USDC (including an approved higher funding
   amount) yet reject a payment that fits the explicit API cap. Separate initial
   funding preference from payment authority; assess admission against active
   wallet capacity minus liabilities and the explicit API cap. Do not auto-fund
   more, revive retired funds, or silently broaden existing deployments' effective
   spending authority. Specify compatibility/migration behavior for operators
   that relied on the target ceiling. Test a payment above the initial target
   but below both the explicit API cap and confirmed available balance, plus
   cap violations and concurrent reservations.

3. **Informational metadata can veto valid managed offers.**
   `manager.rs::select_offer` uses closed key allowlists for the offer and `extra`,
   and validates numeric forms of descriptive `totalUsd`/`breakdown` fields.
   Unknown annotations or differently represented descriptive prices can cause
   a generic “no compatible EIP-3009 offer” even though atomic amount, Base USDC,
   payee and domain are valid. Classify authority-bearing fields separately from
   bounded opaque annotations; preserve accepted JSON through the signing SDK
   and verify the resulting payload rather than rewriting terms. Unknown transfer
   mechanisms, meaningful extensions, conflicting terms and wrong domains still
   require explicit support. Add fixtures for extra annotations and string-valued
   display prices, alongside malicious mechanism/amount/domain changes. Do not
   generalize this into accepting arbitrary unknown configuration fields.

4. **A fee ceiling is reserved as though it were the actual fee.**
   Funding `Backend::ready` and `Backend::prepare` use quote input plus
   `max_network_fee` (clamped to the input cap); treasury preparation requires
   that headroom and reserves it before the proposal reveals its actual fee.
   This can reject a transaction whose real input-plus-fee fits the balance or
   remaining daily budget. Preserve reservation-before-prepared-bytes, serialized
   ownership and final exact-cost enforcement. Investigate a calculate-only
   proposal stage that safely determines cost before committing the final
   reservation, with durable handling of cancellation and library mutation.
   Do not simply under-reserve or weaken configured fee limits. Test actual cost
   below capacity while maximum permitted fee exceeds it, plus cancellation and
   concurrent budget use. Apply the same review to refund shielding's maximum-fee
   reservation (`src/treasury/refunds.rs`), without double-counting returned funds.

5. **Source-import caching confuses active work, failures and retention.**
   `src/discovery/mod.rs::document` uses an insertion-time 300-second TTL for
   `OnceCell<Result<...>>` entries and clears the cache on its capacity threshold.
   Both mechanisms can evict an unfinished fetch; a subsequent same-URL caller
   can start duplicate unsigned I/O while the original continues. Completed
   transient failures remain cached for the same window, making recovery look
   like a continuing outage. Separate an active coalescing map from bounded
   completed results and typed negative backoff; measure successful retention
   from completion. Keep import concurrency/resource caps and registration commit
   deduplication. Test a progressing fetch exceeding 300 seconds, capacity churn,
   and a transient failure followed by a healthy origin. The owner/global import
   permits acquired before cache lookup also need a fixture proving that aliases
   do not consume all capacity while merely waiting on one fetch.

6. **Submission preflight failures become unknown broadcasts.**
   `treasury/send.rs::submit_prepared` commits `BroadcastTransaction::request`
   before entering `treasury/submission.rs::submit`. The adapter then connects,
   checks network/tip/expiry, and only afterwards calls `send_transaction`; its
   enclosing timeout and result mapping turn any of those failures into
   `SubmissionOutcome::Unknown`. That conservative outcome is necessary with the
   current durable boundary, but avoidable failures can be moved earlier.
   Investigate a typed preflight stage before minting the single-use submission
   capability, followed by a final expiry/authority check at the send boundary.
   A disconnect after send begins or a crash after durable intent must still be
   unknown; never infer non-submission from timeout prose or absence on chain.
   Do not change historical unknown records or automatically replay them. Test
   connection failure, wrong network, expiry before send, and crash/cancellation
   on both sides of durable intent, verifying exactly when an attempt is charged.

### Resource-budget disposition

`src/limits.rs::read` bounds actual buffered bytes and declared content length;
it does not impose an elapsed-time download budget. Preserve these bounds and
their explicit errors. Ordinary defaults are 16 MiB API, 4 MiB help and 32 MiB
catalog; API/help/catalog limits are configurable. The 4 MiB MCP request cap and
NEAR's fixed 2,000,000-byte JSON cap are separate resource constraints. No observed
oversized valid fixture was established in this audit, so these are not justified
removal candidates. Image count/decoded-byte limits constrain another allocation
layer and must not be mistaken for duplicate HTTP deadlines.

Cache retention, pricing endpoint/concurrency budgets, optional cover episodes,
auth-warning suppression, database busy waits and qualification deadlines have
different purposes. Keep their scopes explicit. The HTTP cache already falls
back to using a complete response without persistence when an entry exceeds
storage capacity (`src/http_cache.rs`); do not make optional cache storage a new
condition for API/catalog success. Prioritize the confirmed relay envelope-limit
mismatch and import in-flight eviction over speculative removal of all bounds.

## Delivery and completion

Implement in reviewable stages: gRPC progress, redundant deadline removal,
shutdown ownership, relay scoping, freshness acquisition, quote policy, optional
startup, then health-threshold terminology. For each stage, document
the removed failure condition and preserved correctness invariant before editing.

Include the preparation live-tip race in freshness work and Base view acquisition
as its own evidence stage. Then address pre-submission outage recovery, metadata
compatibility, fee reservation sizing, funding-target/payment-cap separation and
import cache lifecycle. Each needs independent review because their transaction
or resource invariants differ. Confirmed source behavior above is sufficient to
schedule fixtures; it is not proof of the original incident's cause. The unresolved
provider delivery contract and shutdown ownership tests are explicit gates, not
reasons to weaken expiry or abandon durable liabilities.

Submission preflight ownership belongs with the redundant-deadline stage; do not
merge its attempt-state changes into an otherwise mechanical timer removal.

Run focused default-feature tests and applicable Clippy/network/vendor checks.
Use fixtures that exceed the former deadlines while making progress, alongside
true stalls, cancellation, restart and uncertain-outcome tests. Do not run funded
qualification or modify private deployment files as part of this plan.

Completion requires an inventory of remaining production timers, each tied to
inactivity, an actual external expiry, explicit cancellation, bounded optional
work or a documented resource/correctness constraint. No unexplained whole-work
deadline should remain on the paths above. Move implemented behavior into the
maintained guides and AGENTS.md, retain genuinely unfinished scope, and remove
this plan when complete.
