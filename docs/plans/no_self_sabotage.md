# Remaining progress-policy limitations

The transport, shutdown, relay scoping, freshness/catch-up, optional-source failure
isolation, fee reservation, payment-cap migration, quote-outage and import-cache
changes are implemented, with the accepted upload limitation below. Maintained behavior and remaining timer scopes live in
[network egress](../network-egress.md#remaining-timers-and-budgets),
[configuration](../configuration.md), [wallet rotation](../wallet-rotation.md),
[wallet commands](../wallet-cli.md), [agent sources](../agent-sources.md) and AGENTS.md.
Background catalog publication remains in its existing
[independent startup plan](deferred/independent_provider_startup.md).
These changes do not establish the cause of the original connectivity incident.

## Accepted limitation: gRPC upload progress

The response-header inactivity timer includes uploading the request. A slow
HTTP/2 upload can time out while making progress because the current transport
does not expose per-request upload consumption. We retain this limitation rather
than vendor another library or infer progress from unrelated connection traffic.
The [timer inventory](../network-egress.md#remaining-timers-and-budgets) records
this scope. Timeout remains an uncertain outcome and never grants replay authority.

## Unresolved gate: the 1Click Zcash deadline and route contract

The application now requests 7,200 seconds by default, measures elapsed awaited
work before quote/authority decisions, and uses one shared
`MIN_QUOTE_VALIDITY_SECONDS = 300` policy. The returned deadline remains
binding, including when shorter than requested. The five-minute requirement is
not yet removed: the evidence below does not establish the event 1Click compares
with that deadline for this Zcash route. Local broadcast success is insufficient
proof of timely delivery.

Obtain the authoritative 1Click service/deposit-watcher implementation or
integration contract establishing:

- Whether timely arrival means mempool detection, inclusion, required confirmations
  or another event, and which timestamp is compared with the quote deadline.
- The actual route for `nep141:zec.omft.near`, connector identity and required
  confirmation/relay behavior, including late deposits and refunds.
- Whether any pre-submission delivery allowance is required. Remove the blanket
  margin to the extent unsupported; otherwise name/configure the actual allowance.

Update quote validation, preparation and final durable submission checks together.
Retain external expiry, exact prepared bytes, transaction identity and liabilities.
Refresh only proven untouched quotes; no replacement quote authorizes replay of
prepared or possibly submitted work. Add boundary fixtures for the established
contract and for slow preparation/cancellation without duplicate allocation/send.
No live spending or private-state inspection is authorized by this remaining plan.

The [NEAR quote guide](https://github.com/near/agent-skills/blob/main/skills/near-intents/rules/api-quote.md)
describes the deadline as deposit arrival but does not resolve the Zcash event.
The prior source audits below remain evidence for this open question, not a
substitute for the missing contract. Retain this plan while either limitation remains unresolved.

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
