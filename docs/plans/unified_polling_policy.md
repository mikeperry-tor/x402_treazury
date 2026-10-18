# Unified wallet polling and balance observation policy

## Status and intent

This is an implementation plan, not a description of current runtime behavior.
It replaces blanket balance maintenance with observations requested by actual
work. Both USDC and ZEC continue to support unsolicited funds. Dedicated discovery
occurs on fresh serving startup, when preparing initial funding for a new wallet,
and on explicit `wallet sync`; idle serving does not watch for incoming transfers.
Other necessary observations, such as an insufficient-funds refresh or accepted
swap-credit check, may also discover incoming funds incidentally.

The starting contracts are the maintained
[best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[wallet administration](../wallet-cli.md),
[resource inventory](../local-resource-limits.md) and
[timer inventory](../network-egress.md#remaining-timers-and-budgets).
The completed `best_effort_wallet_balances.md` plan is recoverable from the parent
of `b737955bfc579c24b6b060236a46f90d23e70172`; do not restore it as another rulebook.
Commit `a990025d3ee1b982b845852ad7637e5dcf10eaaa` already gates initial USDC
credit observation on accepted source work and supplies its minute cadence. That
does not cover ordinary USDC maintenance or command-triggered ZEC observations.

## Shared principles and separate responsibilities

Keep one coherent policy for why observations are requested and how their results
are used. Keep estimate updates, observation scheduling and financial ownership
as distinct responsibilities with their existing owners.

- The chain owns actual USDC balances. A saved USDC estimate only ranks existing
  authorized payers; stale, zero, unknown or malformed estimates do not prohibit
  an otherwise permitted signature. Do not refresh merely because an estimate
  is old or absent.
- Durable authorization journaling subtracts its maximum amount once, saturating
  at zero. Unknown estimates stay unknown. Do not debit again on response,
  restart or canonical resolution, subtract unresolved exposure again, or introduce
  a local refund-credit ledger. A later scalar chain read may replace the estimate
  while older authorizations remain chargeable.
- The SDK owns ZEC spendability, notes, witnesses, confirmations and input selection.
  Do not apply USDC estimate arithmetic to ZEC or add a competing monetary model.
  Exact transaction/input journals remain necessary for accepted-work ownership;
  they are not a second balance or spending ledger.
- Ownership, amount/fee safeguards, signed validity and consensus expiry retain
  their current financial meaning. An observation schedule neither grants new
  spending authority nor proves that accepted work never happened.
- Every recurring background wallet/swap observation waits at least 60 seconds after
  the previous observation for that work completes. Preserve independent error
  backoff up to 300 seconds and existing jitter where applicable; repeated events
  and unrelated progress cannot shorten that interval or clear another failure.
  First requested observations may run immediately when no relevant observation
  cooldown already applies. A fresh process startup or
  explicit sync is a one-pass trigger, not a new recurring loop.
- The interval is per observed wallet/operation or serialized treasury sync,
  not a claim of one RPC per minute globally. A scan, provider fallback or canonical
  proof can contain several requests. Keep existing transport policy and isolation.
- Mandatory validation for an actual newly authorized financial action remains
  distinct from polling: retain network/tip binding, SDK proposal validity and
  exact input/expiry proof. Coalesce duplicate work and reuse completed scans under
  existing canonical checks; do not skip correctness checks or invent a freshness
  permission gate to meet the cadence. Do not pace local phase transitions or
  preparation-to-submission handoffs as though they were recurring observations.

## Observation decision table

| Trigger / work | Observation and cadence | Completion / stop rule |
| --- | --- | --- |
| Fresh managed serving startup | One background ZEC sync and one finite Base observation pass over existing eligible managed wallets and outstanding initial-funding recipients | Finish the pass; startup alone creates no recurring balance watcher |
| Preparing initial funding for a new wallet | Discover ZEC through the SDK owner and observe that exact destination's USDC before accepting a new source calculation; reuse applicable observations from the same work | Positive destination credit consumes the initial claim; otherwise current authority and actual SDK spendability govern preparation |
| Waiting for capital/confirmations to fill enabled initial-funding claims | ZEC sync at least 60 seconds apart, with error backoff | Stop recurring discovery when no enabled funding demand or accepted treasury work still needs it |
| Accepted initial funding without observed positive destination credit | Existing accepted-work Base credit check, immediate if the recipient observation is due, then at least 60 seconds apart | First positive credit permanently ends initial-credit polling for that wallet's claim |
| Recognized signed-payment insufficient-funds failure | Queue the existing asynchronous scalar USDC refresh; immediate if due, otherwise coalesce until its wallet's minute interval permits | A successful requested read completes the refresh even if it returns zero; failed reads retain the request and retry |
| Accepted source/shielding transaction needs confirmation or recovery | Necessary ZEC sync and transaction lookup at least 60 seconds apart while waiting | Existing canonical confirmation or consensus-expiry/input recovery resolves that particular obligation |
| Submitted swap needs provider outcome | NEAR status observation at least 60 seconds apart, independent of Base credit and source confirmation | Existing provider outcome/recovery applicability determines completion; status alone cannot release source inputs |
| Explicit `wallet sync` | One requested ZEC sync and configured managed-USDC refresh pass | Return honest per-asset completion/failure; no background watcher is created |
| Fully funded idle serving, no requested refresh or accepted work needing observation | No recurring USDC or ZEC balance discovery | Relevant new work requests observation again |

Unsolicited funds discovered at these points may be used only under existing
permission. They never reopen a consumed funding claim, authorize a top-up,
reactivate a retired signer or grant access to another pool's funds. A positive
observation of a never-funded destination consumes its initial claim even when no
swap was started and even below target. Recheck the current claim at the existing
source-acceptance boundary so concurrent discovery cannot authorize an extra deposit.

The pre-funding destination read is a deliberate one-pass exception to today's
accepted-source-only Base checks; it is not pre-funding polling. Do not rescan all
existing USDC wallets for each new wallet or repeatedly probe its destination while
the treasury still lacks spendable ZEC. Arrange prerequisite checks so capital
waiting uses the shared ZEC cadence, and destination discovery is part of actual
preparation. A failed advisory Base read retains unknown/previous information and
does not become a new balance-based funding or signing readiness gate.

## USDC estimation and requested refresh

Delete the ordinary ACTIVE/READY/RETAINED periodic sweep. Saved estimates plus
the journal decrement already support payment selection; a successful payment
alone does not request another RPC. Low-balance refreshes are not scans that run
until a wallet becomes positive. A successful zero read finishes the requested
observation, and future eligible calls may still try that identity under the
existing captured-inventory traversal rules.

Reuse the durable low-balance/pending/error flags and retained maintenance owner.
Pending failures may coalesce before a read starts. A recognized failure occurring
during an in-flight read requires a later read that starts after that failure;
retain it across the minute cooldown, cancellation and restart. A failed read keeps
the old estimate and explicit observation failure, leaves requested work pending,
and cannot clear low-balance classification as though it confirmed a new value.
Apply the same wallet cooldown after successful reads so notification bursts
cannot turn maintenance into a fast poller. Coordinate same-wallet background
observers so startup, funding and refresh events do not duplicate active I/O.
If a destination was just read during preparation, reuse that observation and
honor its minute cooldown instead of immediately repeating the same Base read
when accepted-credit monitoring starts.

Do not make scalar reads repair historical nonces, release outstanding
authorizations or collect retirement proof. Explicit canonical qualification and
reporting retain their separate contracts and authority. Startup and normal
refresh passes do not reactivate or sweep retired wallets.

## ZEC demand, discovery and accepted work

Replace unconditional periodic sync with demand from current enabled unfilled
funding claims or actual accepted treasury work. Creating new authorized reserve
demand resumes discovery; known insufficient spendable funds keep minute-cadence
sync active so `wallet init` followed by `serve` still detects deposits and funds
the configured reserves automatically. Disabled auto-funding or removed pools do
not create new funding demand, but never hide their already accepted obligations.

Startup and explicit sync discover arbitrary owned incoming funds, including
unattributed transparent returns. Before new wallet funding, discover SDK funds
again as needed for actual preparation. Automatic owned-transparent shielding runs
after needed successful persisted scans under existing permission/fee rules. A
discovered shielding operation retains its owner until completion even if the
funding demand that caused discovery has disappeared. Idle arrival does not itself
cause a scan; it is discovered on the next relevant trigger.

First positive Base credit does not finish a pending source transaction. Provider
success/refund flags do not establish canonical source or return evidence. Preserve
exact accepted inputs, bytes, identities and original liabilities across timeout,
disconnect, cancellation, removed configuration and restart. Classify observation
needs from existing journals/claims, not a second durable work ledger or one global
"funding complete" flag. Do not poll an already resolved source just because its
historical job exists; do not abandon an unresolved one because its phase is stale.

Completed-result publication and SDK reconstruction retain the existing owner's
60-second retry behavior independently of balance discovery. A pending publication
does not justify another calculation, send or whole-treasury scan. Share successful
scans across waiting source work through the treasury owner where valid; source
polling must not drive extra full scans or tip probes between minute ticks merely
to discover that nothing changed. Keep actual mutation/recovery correctness checks.

## Startup and CLI integration

Bind serving listeners before startup observations. Run discovery with existing
background ownership so old usable payers and tools remain available during slow
or failed scans. A startup pass is finite, covers its existing inventory boundary,
and is not extended indefinitely by new allocations. New allocations follow the
preparation and accepted-credit rules separately. Avoid a global all-wallet scan
barrier in front of one wallet's valid funding work.

A failed startup or explicit sync pass reports its failure without creating a
perpetual discovery watcher. Any already requested insufficient-funds refresh,
enabled funding demand or unresolved accepted work retains its own observation
and retry rules; finishing the one-pass discovery does not cancel those needs.

`wallet sync` currently syncs the Zcash treasury. Extend its explicit deployment
sync path to refresh configured managed-USDC wallets and outstanding funding
recipients as well, using the same Store and existing exclusive administration
ownership. Keep treasury-only configurations valid without Base configuration.
Resolve only the relevant treasury/managed-Base settings; do not load provider
specs or require NEAR or unrelated API credentials. Report attempted, completed,
failed and inapplicable asset scopes honestly; failure of one asset must not
silently suppress an independent useful observation. Preserve current CLI JSON
contracts or explicitly version affected output and fixtures if needed.

Retain the current requirement to stop `serve` before `wallet sync`. Concurrent
`wallet status` remains a cached read with shielded receive address and saved
observations; it must not trigger hidden network traffic. Clearly describe that
idle balances/status may omit incoming funds until a discovery trigger.

## Implementation sequence

1. Establish a source-matched baseline and independent design review of complete
   estimate, maintenance, funding-source and treasury command consumers. Read the
   owning guides; compare CogC/CycC under the
   [complexity review discipline](../../tests/COMPLEXITY.md#review-discipline).
2. Replace blanket USDC sweeps with requested refresh ownership/cooldowns. Delete
   unnecessary regular-sweep cursors, pages and due state; retain explicit one-pass
   inspection where its API contract requires it. Preserve post-failure races and
   truthful error status before relying on removal of the old sweep's retries.
3. Make treasury discovery demand-driven and pace recurring source/provider
   observations at a minute or longer. Preserve immediate valid phase progress,
   SDK spendability and exact accepted-work recovery; do not create duplicate sync
   loops, monetary tracking or readiness frameworks.
4. Wire startup, exact-destination pre-funding discovery and explicit deployment
   sync through those owners. Verify fresh unsolicited USDC consumes only an unused
   claim and fresh unsolicited ZEC/transparent capital is available through the SDK.
5. Update permanent documentation with each implemented behavior, perform combined
   independent financial/concurrency/network review and focused validation, then
   remove this completed plan after its documentation handoff is verified.

Use independently reviewed commits and fixups under AGENTS.md. Keep sources and
executables frozen during validation. This plan authorizes no live-funded tests,
private wallet inspection, consensus invocation or state conversion.

## Acceptance evidence

- Idle funded wallets issue zero recurring Base reads; idle treasury issues zero
  recurring indexer requests. Explicit scopes do not trigger other hidden watchers.
- Signing works with stale, zero, unknown and unavailable estimates; journaling
  decrements once. Payment responses and restart never reapply that debit;
  successful payment alone requests no probe. Restart still triggers the finite
  startup discovery pass specified above.
- Multiple insufficient-funds notifications coalesce. A notification during a read
  survives until a follow-up read starts after it; failures preserve old estimates
  and pending work. No recurring same-wallet reads start before the minute floor.
- A successful zero refresh stops requested work; siblings need no observation.
  Positive initial credit stops that credit lane without stopping source recovery.
- Startup and explicit sync discover unsolicited USDC and ZEC; destination discovery
  prevents a new initial deposit when a first positive credit is already observed.
  No later transfer reopens a consumed claim or reactivates retired wallets.
- New funding demand resumes ZEC discovery; an empty treasury continues waiting at
  minute cadence and later detects a deposit. Funding prerequisites do not cause
  repeated pre-funding Base probes or full-inventory passes.
- Exercise source-confirmation waits and unchanged-tip reuse without five-second
  indexer probes. A changing chain still receives required SDK/canonical checks;
  valid prepare/submit transitions do not acquire artificial minute-long delays.
- Slow useful work may exceed a minute without cancellation or duplicate I/O.
  Unrelated progress cannot reset another observation's backoff or postpone its
  due accepted work. Cover cancellation/restart, disabled pools, stale phases,
  accepted shielding, publication failure and true transport inactivity.
- Verify actual endpoint call counts with local fixture servers and real production
  owners. Use paused-time unit coverage for scheduling plus focused integration
  tests; ensure filters execute nonzero tests. Do not use real secrets or claim
  local fixtures qualify live settlement/Tor. Run appropriate wrapper tests,
  all-target Clippy, formatting and source-matched complexity checks.

## Permanent documentation handoff

Keep maintained documentation descriptive of delivered behavior until its owning
implementation lands. The plan must not become the permanent policy archive.

| Document | Required update |
| --- | --- |
| `docs/wallet-rotation.md` | Own a clearly named **Wallet observation policy** section, linked from **Best-effort wallet rules**. Publish the final trigger/cadence/stop table alongside estimate updates, no top-ups, SDK spendability, unsolicited funds and accepted-work exceptions. Reconcile unconditional maintenance language in existing sections. |
| `AGENTS.md` | Link directly to both permanent wallet sections in financial boundaries and the polling/payment read-first entries. Require agents changing estimation, polling, funding or recovery to read them. During implementation also link this unfinished plan; remove only the plan link at completion. |
| `docs/network-egress.md` | Replace five-second ordinary advisory/source descriptions and unconditional ZEC scans in the timer inventory. Describe the minute floor, discovery triggers, separate correctness reads and publication/reconstruction retries. |
| `docs/local-resource-limits.md` | Keep its inventory consistent with the same implemented cadence and scope; link to the owning wallet policy instead of defining competing applicability rules. |
| `docs/wallet-cli.md`, `docs/configuration.md`, README and deployment examples | Explain startup discovery, explicit sync scope/configuration, cached status, idle incoming funds and resumed capital waiting. Reconcile `poll_seconds` and swap health-threshold descriptions with actual network scheduling; keep existing environment contracts. |
| `docs/architecture.md` | Link to wallet observation rules and explain existing owner boundaries, estimate updates and listener-before-discovery startup; no second normative scheduling table. |
| `docs/README.md` | Index the permanent observation policy beside the best-effort wallet rules and remove the completed plan reference. |
| Relevant `tests/` guides | Describe focused scheduling/side-effect coverage and its qualification limits; keep generated metrics/logs under ignored `target/`. |

Completion requires checking all these consumers for conflicting old claims,
verifying AGENTS.md's permanent links resolve after plan deletion, and independent
review of the accumulated implementation and documentation. Do not preserve this
completed plan or duplicate its policy in a dated handoff.
