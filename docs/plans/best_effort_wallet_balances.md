# Best-effort wallet balances and simpler treasury rules


## Status and scope

Implement advisory USDC wallet selection and SDK-owned Zcash spending state on
branch `best_effort_wallet_balances`, based on
`49566df4ae9e40c26349dc9ac6f6d3541ef3c02b`. BE02 and its fixup passed independent review. BE03 implementation and focused
acceptance are complete pending exact-commit and payment-transition review.
BE04–BE07 remain pending.

Cross-wallet repetition after an insufficient-funds response is explicitly accepted,
even when the first authorization remains chargeable and both attempts could charge
or execute. Managed wallets receive initial funding and are never topped up.
This exception needs no additional opt-in or alternate admission mode.

No live-funded qualification, private-state conversion or old-binary execution
against current financial state is authorized here.

## Ownership rules

| Information | Owner and use |
| --- | --- |
| Actual USDC balance and token transfers | Blockchain. Saved values are estimates/observations, never permission to sign. |
| Spendable Zcash notes, witnesses and confirmations | Existing wallet SDK and its serialized treasury owner. Do not maintain a competing application balance model. |
| Payment permission | Existing payer/pool/listener binding, supported asset/network/domain, amount cap and caller-specific authority. |
| Signed/payment or source intent | Exact local journal and accepted work owner; the chain cannot reconstruct unpublished signatures or uncertain submission intent. |
| Source-operation correlation | Preserve original quote, recipient/refund address and exact pending transaction identity for recovery. Optional refund attribution is reporting, not a balance credit or spending prerequisite. |
| Status, health, estimates and workflow phases | Scheduling/display information. They cannot independently veto unrelated valid work. |

Local records describe what we authorized, submitted and observed. They need not
remain a synchronously consistent replica of every canonical chain fact. Remove
mechanisms whose only justification is maintaining that stronger guarantee.

## USDC rules

1. **Keep one advisory estimate per wallet.** Reuse the saved balance value rather
   than add an available-balance ledger. Unknown estimates remain unknown; a
   malformed estimate is unavailable advisory data, not missing signing authority.
   Ordinary balance refresh can replace the estimate. Refresh failure retains
   useful previous information and does not prohibit a payment.
2. **Update cheaply.** On successful durable journaling of a new authorization,
   subtract its maximum amount once using saturating subtraction. Do not also
   subtract total unresolved exposure or reapply the debit at response/restart.
   Commit this estimate change with the existing journal write. If a schema change
   is needed, make it a small direct change, not another ledger or projection.
   A later chain observation may overestimate funds available to outstanding
   authorizations; that is accepted best-effort behavior, not a revision race to
   solve with certificates, epochs or mandatory reconciliation.
3. **Choose by hints within existing authority.** Prefer a suitable bound payer,
   then existing eligible wallets whose estimates suggest affordability. Unknown
   or low estimates remain fallbacks. Preserve ownership, configured privacy/
   rotation rules and actual retirement decisions; an estimate does not grant
   permission to use another pool or listener's wallet.
4. **Do not prove affordability before signing.** Remove current balance RPCs,
   historical nonce verification, age/anchor checks, reserve sufficiency and
   revision stabilization as payment prerequisites. Per-authorization amount,
   asset, recipient/domain and actual owner checks still apply. Chain rejection
   and occasional failed payments are acceptable outcomes.
5. **Journal before disclosure.** Retain exact authorization identity, payer,
   payee, amount, signed expiry and possible submission. A journal failure cannot
   dispatch the signature. Preserve accepted work through cancellation and drain.
   Records of earlier attempts remain even when a later attempt succeeds; they
   are not silently released because a seller reported insufficient funds.
6. **Retry recognized insufficient funds on another wallet.** Capture the existing
   eligible inventory boundary for the logical call. Try each wallet at most once;
   use existing wallet IDs/sequence and the call's attempted identities. Do not
   add an arbitrary retry-count ceiling, durable traversal ledger, global epoch or
   wait for wallets created later. When that inventory is exhausted, return the
   failure with the fact that earlier authorizations may still charge.
   Each recognized insufficient-funds failure moves the wallet into the
   low-balance set and schedules an advisory USDC balance check via RPC. Proceed
   to the next wallet without awaiting that check.
7. **Bind each attempt correctly.** Choose an untried payer, obtain its unsigned
   challenge through its own identity-bound transport, validate its actual price
   and permission, create and journal its own authorization, then dispatch. Retry
   the original reviewed arguments/body; do not mutate API arguments or reuse
   another payer's challenge/signature. Uncloneable bodies fail explicitly.
8. **Classify before flattening errors.** Recognize exact structured x402/
   facilitator insufficient-funds codes from the response to that attempt's signed
   payment request. The HTTP response need not itself be cryptographically signed.
   Do not classify
   by arbitrary provider prose, HTTP 402 alone, an initial unsigned challenge,
   insufficient allowance, generic settlement failure or body truncation. Retain
   the existing response-byte bounds. Pin actual supported wire forms in fixtures;
   the baseline SDK names the category `insufficient_funds`, but seller encoding
   must be established rather than invented.
9. **Keep the exception narrow.** Transport loss, malformed replies, cancellation,
   unrelated HTTP failures and successful paid responses do not earn this retry.
   Do not retry a case automatically after process restart. Submission progress
   remains monotonic across attempts; never reset it to make a relay/caller believe
   no signature was disclosed. Existing paid management/relay callers share this
   behavior through the production payment path, subject to their actual authority.
   Static-wallet payment schemes are not expanded by this managed-wallet change.
10. **No top-ups.** Keep one initial funding claim per wallet. After preparation
    or uncertain submission, recover that exact source transaction rather than
    start a second potentially effective deposit. Proven expired/non-included
    initial work may be replaced within the same never-funded claim after all
    original inputs are verified; that is initial funding recovery, not a refill.
    Observed initial credit, including partial credit, must not authorize another
    deposit to make up a target shortfall. A low estimate or an insufficient-funds
    result never reopens a consumed initial funding claim.
    New capacity can use new wallet identities through existing authorized
    background initial-funding controls. The retry loop never allocates, funds
    or waits for one. Proven untouched initial work can use ordinary preparation
    recovery; uncertain work retains its original bytes and inputs.

The low-balance set is a selection classification, not permanent retirement or a
quarantine. Until its scheduled check succeeds, retain the last numeric estimate
as last observed/estimated and rank the wallet as low balance; do not label an
invented zero as a confirmed blockchain balance. On success, replace its estimate
with the returned USDC balance. Use an ordinary current `balanceOf` read, not a
confirmed/latest proof pair, historical nonce scan, certificate or age gate.
Later selection may use that value for an affordable request without treating the
failure as a permanent blacklist. The current call still never tries it again.

Every failure schedules a read starting after that failure. Pending requests may
coalesce before a read starts; a new failure during an in-flight read requires a
follow-up check rather than being considered covered by the older read. Reuse
existing maintenance/RPC resource ownership; this is advisory scheduling, not a
new financial revision or validity system. If RPC fails, keep the wallet in the
low-balance set with its last estimate and an explicit refresh failure/unknown
status. Ordinary backoff/reconciliation may retry; it never blocks the API retry
or creates a whole-call deadline. A failed RPC cannot provide an updated chain
value, so do not report it as successful confirmation.

Late initial funding or an external transfer can still arrive. Do not add incoming-
transfer surveillance or a new enforcement layer to prove the no-top-up operational
assumption. A low-balance check observes those funds without creating a new claim.

The amount cap applies to each authorization; a logical call may incur multiple
charges under the expressly accepted retry policy. Do not add a production daily,
cumulative or aggregate retry spending budget. Separately authorized qualification
runs must reserve/record every attempt against their existing permits before it
starts. Ordinary retry permission does not expand those permits. Reconcile run
assertions and reporting with the changed semantics instead of retaining claims
of freshly proved admission capacity or one authorization per case.

## Zcash rules

The same removal of duplicate balance authority is appropriate for Zcash. The
accepted USDC repeat-payment exception does not authorize duplicate source swaps.

1. **Use the SDK's spendable state.** Displayed totals are hints. Let ordinary SDK
   sync/input selection/proposal establish usable notes, confirmations, witnesses,
   actual amount and fee. Remove parallel application freshness/solvency screening
   that merely repeats that decision. Do not put a full historical accounting pass
   in front of publishing a usable wallet snapshot or preparing unrelated work.
2. **Retain one treasury owner and actual input ownership.** Serialize SDK mutations
   through the existing actor. No second spend may reuse inputs owned by prepared
   or uncertain work. Prefer exact input exclusion when the pinned SDK supports
   it; retain a narrowly justified fence if it does not. Do not delete that fence
   on the strength of a balance estimate, or add a proof framework around it.
3. **Prepare once, persist, then submit.** Preserve atomic wallet snapshot plus
   exact transaction bytes/txid, selected inputs and original destination/quote/
   network binding. Record submission intent before external effects. A lost
   reply, task exit, old workflow phase or expired local timer cannot establish
   that preparation/submission never happened. Recovery owns the same bytes.
4. **Use protocol expiry.** Zcash transaction expiry is block-height based; a
   transaction expiring at height N can be included through N, not N+1
   ([ZIP 203](https://zips.z.cash/zip-0203)). Preserve the actual transaction's
   consensus rules, including special/no-expiry cases. Before releasing ownership
   or creating replacement source work, establish applicable canonical non-inclusion/
   expiry and verify every original input, not just the SDK subset still visible.
   Keep the final wallet/snapshot binding needed for that exact mutation.
5. **Keep actual permissions and costs.** Retain auto-funding controls, exact
   recipient, per-operation amount/fee safeguards, real outstanding source claims
   and qualification permits. Actual spendable treasury funds provide the capital
   limit; do not restore removed daily/cumulative funding budgets. Compare actual
   proposed amount and fee with existing operation limits; use SDK selection and
   exact pending-input ownership without a second monetary reservation.
6. **Scope failures.** Known unsigned insufficient funds waits for ordinary wallet
   progress without invalidating listeners, Base payments or other observations.
   Unknown submission retains its inputs and outcome. Missing refund attribution
   does not constrain SDK-owned funds or stop sync. A historical operation's
   reporting completeness is not whole-treasury readiness.

Funded-wallet identity remains single-use for initial funding under the USDC rule.
The SDK discovers returned funds, handles reorganizations and establishes what can
be spent. Do not credit refunds into a local monetary ledger, reconcile historical
refund credits, or require original-swap attribution before using SDK-owned funds.
Keep existing operation records for exact pending-work recovery and honest status;
a refund observation cannot authorize another potentially effective source deposit.
Recovering owned transparent outputs/shielding may use the existing owner and real
fee/input rules; it does not require a refund-accounting subsystem.

## Implementation sequence

Implement the following commits in order. Keep changes small enough
to delete a responsibility coherently, not small enough to leave two active engines.

| Commit | Scope and acceptance |
| --- | --- |
| BE01 — plan and baseline | This document, exact baseline and explicit retry/no-top-up policy. Independent design review; no runtime acceptance claim. |
| BE02 — advisory USDC selection/admission | Remove balance/history/age permission, including the hidden readiness check in `payment_ownership`; use one estimate/decrement rule and actual payer authority/journal. Prove signing works with stale/unknown/unavailable advisory reads and missing historical readiness; do not enable unsafe intermediate replay. |
| BE03 — typed insufficient-funds traversal | Pin structured response mapping, traverse captured eligible wallets once, preserve every authorization and identity-bound challenge. Move each failed wallet to the low-balance set and schedule its advisory RPC balance confirmation without delaying fallback. Update qualification/relay/caller semantics with the actual behavior. |
| BE04 — initial funding and observations | No same-wallet top-up; new-wallet background initial funding stays under existing authority. Remove balance/readiness repair dependencies, preserve exact initial source owner and late credit. |
| BE05 — treasury/receipt age removal | Remove the enumerated local age/skew/elapsed gates directly, independent cadence and receipt count ceiling. Preserve SDK/input/consensus and signed protocol rules; retire obsolete config clearly. |
| BE06 — narrow source recovery simplification | Implement exact-owner/input recovery fixes, delete duplicate refund/source monetary accounting and historical gates, remove unsupported quote/internal shielding cutoffs, and simplify unfinished-work scheduling. Split only if the real invariant is separately reviewable. |
| BE07 — integration and deletion review | Reconcile all remaining call sites/config/docs, run standard default-build checks, compare full structure and complexity, verify the maintained documentation handoff, and record remaining external/SDK limits and fixup causes. |

### BE01: Plan and baseline

Confirm the rules, baseline and review process before runtime changes.

### BE02: Advisory USDC selection and admission

- Replace `rotation/manager/selection.rs` and Store candidate eligibility with
  advisory selection; remove speculative proof acquisition/publication orchestration
  that exists only to certify affordability.
- Simplify `pay_with_cover_observed`, `prepare_payment`, `journal_authorization`
  and `PreparedPayment` to actual permission, journal ownership and estimate update.
  Remove evidence-recovery/compatibility loops and balance-held group turns.
  Remove
  `store/capacity.rs::payment_ownership → funding::readiness_recorded_valid`
  as a signing prerequisite: retaining that whole helper as "ownership" would
  preserve hidden historical balance authority. Keep only actual owner/pool/cap/
  role checks; the historical readiness data is not permission.
- Delete background nonce/history repair where its only purpose is releasing
  capacity for another signature. Retain immutable attempt records and any explicit
  reporting/qualification consumer without turning it into a new payment gate.
- Demote destination readiness to initial-claim/status information; delete its
  payment-admission and historical repair prerequisites.

Remove `ChainView::admission_valid_until`, balance `valid_until` and
`PreparedPayment` freshness deadlines as signing authority; delete unused
fields/helpers with their consumers. Keep only actual permission, journal ownership
and the advisory estimate update.

BE02/BE03 are an explicit transition: before BE03, an insufficient-funds response
can still fail the call; after BE03 the accepted retry policy applies. No temporary
new signing authority is needed. Update affected AGENTS.md rules in place alongside
their actual behavior; do not silently leave old no-retry/fresh-balance instructions
as the intended final contract. Existing runtime documentation remains descriptive
until its implementation changes.

BE02 review decisions: the independent design review identified qualification
balance validation as another hidden admission gate. New records now carry an
explicit advisory version; historical report validation remains available. Current
role/claim eligibility is rechecked at commit without requiring an unchanged role
snapshot. Schema 17 is the development target; schema 16 cannot be opened for
mutation and no private conversion is included.

The pinned analyzer comparison to `49566df` measured CogC/CycC for payment
orchestration 18/58 → 10/35, candidate acquisition 15/28 → 0/2, preparation
1/23 → 0/7 (plus the temporary test adapter 1/4), and journaling 1/25 → 2/19.
Certificate merging, speculative acquisition lanes, cache-stabilization loops and
balance-held group turns were deleted. Saved balance, unresolved exposure,
historical readiness, block age and revision equality no longer govern signing.
The reporting consumer gained scoped malformed-advisory handling (CycC 46 → 52);
BE07 must examine that complete reporting chain rather than hide the increase.
Exact-commit review found the obsolete exposure decrement still active in canonical
resolution, an unsatisfiable historical repair lane, and malformed-to-zero refresh
failure. The BE02 fixup removes those responsibilities rather than restoring a
reservation. It also preserves old qualification proof records, updates the renamed
estimate consumers, and represents an unobserved estimate as unavailable. New
focused coverage follows a real signed authorization through canonical resolution.
Focused acceptance and independent review of `798b211` and fixup `0951464` passed.
The BE02/BE03 combined transition remains subject to its independent gate.

### BE03: Insufficient-funds traversal and refresh

Implement the USDC traversal, typed response classification and post-failure
balance refresh rules together in the production payment path. Update qualification,
relay and management callers for every attempted authorization. Simplify the
reconciliation owner to advisory refresh without restoring historical payment gates.

- Isolate identifiable bad jobs/rows and advance to independent work
  instead of repeatedly restarting at the same bad page. Retain any affected accepted-work
  ownership. Distinguish shared Store corruption/lost exclusive ownership from a
  scoped advisory read failure; do not open a second Store to bypass either.
- Preserve accepted HTTP/cover work through disconnect, cache eviction
  and shutdown drain. Cache retention cannot cancel live handles, orphan cleanup or
  reset existing traffic accounting. Waiter lifetime does not own operation lifetime.

BE03 review decisions: structured verification/settlement fixtures are pinned to
x402 seller commit `faa1c694c66a3732aaabbce93bdae140e521a67d` (source SHA-256
`9b4f144143a7c7b8f514e835ac04e83d01f41e876c445eaa5b843561ee5d22cb`).
Classification requires the signed attempt's actual 402 response and exact code;
2xx error headers, allowance errors, prose and malformed/truncated responses earn
no retry. Every captured identity is visited at most once, each with its own
challenge/transport, original request and journal. No retry allocation or balance
RPC is added to the foreground path.

Schema 18 adds only durable low/pending/error advisory scheduling fields; schema
17 is rejected for mutation without conversion. Status schema 3 names the value
`balance_estimate_atomic`. Clearing pending before a read and retaining a later
failure schedules a follow-up without a financial revision. Physical indexed pages
advance malformed rows, and the retained owner survives cancelled waiters. The
ordinary history/nonce repair lane is deleted. Explicit qualification alone still
observes canonical authorization outcomes, including retired payer attempts; those
observations never restore affordability admission.

Precommit review caught lost qualification outcome production and report assumptions
that every attempt had a debit. The correction keeps USED debits, proven non-use
and unknown outcomes distinct. Qualification charges every attempt to its original
case permit and rechecks that same start window before another unsigned request;
this protects actual run authority because an unsigned POST can execute free work.
A fallback identity can leave a particular promotion/service assertion unproved
without invalidating its payment permission.

Exact BE03 review found an obsolete two-challenge ceiling and report consumers
that closed observations at the first admission or assumed one admitted price.
The fixup removes those policies, retains per-attempt prices and canonical totals,
and exercises real qualified A→B→C routing under the original case permit.
Canonical authorization observation now captures only its journal/chain binding;
malformed historical readiness or retirement support cannot starve another attempt.
Its actual production loop is tested with bad historical readiness followed by a
healthy wallet. These fixes delete inherited single-attempt and balance-proof
responsibilities; they add no replacement spending ceiling or repair framework.

Analyzer output `20261018T012356Z-1940d1b1` compares against `49566df`:
payment orchestration 18/58 → 6/29; routed dispatch 17/30 → 28/44;
maintenance 29/37 → 41/50 with read-one 14/29 replaced by read-balance 1/9.
The whole historical repair/acquisition engine was removed rather than relocated.
The surviving maintenance loop carries bounded physical paging, active reads,
follow-up scheduling and retained ownership; its increased score and the full
qualification reporting chain require transition review. Selection is 22/29
(including its actual-authority lookup closure 11/15), replacing evidence selection
9/14 plus removed proof orchestration. BE07 must inspect these complete chains.

### BE04: Initial funding and observation

Enforce one initial funding claim per wallet and the no-top-up rules. Background
initial funding may create new wallet identities under existing authority; the
payment retry loop never allocates or waits for them.

- Find unfinished source transactions and accepted shielding work from original
  operation records despite stale terminal phases, removed pools/listeners or wallet
  retirement. Disablement forbids new funding, not observation of accepted work.
  Provider flags remain observations; resolving pending work never grants another
  deposit.
- Keep funding/reconciliation applicability with the existing owner and actual
  unfinished operation. Provider status and phase labels cannot conceal prepared
  bytes or independently overrule established transaction/output facts.
- Keep wallet bootstrap/administration, listener-before-funding startup, dynamic
  source ownership and Tor identity boundaries. Advisory balance failure cannot
  stop independent catalog or payment service.
- Check existing serving/admin/bootstrap/recovery/qualification
  permission before new financial effects, including disabled or declared-restored
  contexts. Credentials and spendable funds alone grant no new action permission.

### BE05: Treasury and receipt age removal

Update each consumer, configuration loader, error/status output and maintained
guide with its owning removal. Preserve source facts when dropping local deadlines.

| Baseline restriction | Planned disposition |
| --- | --- |
| `base_max_block_age_seconds`, `BaseRpc::max_age`, `validate_timestamp` in `rotation/base.rs` | Remove block-age and 30-second host-future gates, including history-only acquisition. |
| Initial/final receipt age checks in `rotation/base/receipt.rs` | Remove; retain actual transaction/event identity and chosen canonical inclusion/confirmations for decisions that need the receipt. |
| `SyncObservation::fresh`, `max_sync_age_seconds`, elapsed rejection in `treasury/freshness.rs` | Remove age permission directly. Keep necessary SDK owner/revision and scanned coverage checks without a new capability layer. |
| Treasury age-derived polling cadence | Give existing maintenance an independent cadence; it never supplies spend permission. Retain completed scan progress and incremental catch-up. |
| Three-block lag/reuse/catch-up checks | Review at their actual SDK consumer. Retain only demonstrated scan/selection requirements, not a second numerical freshness policy. Exact expiry/non-inclusion recovery still needs sufficient chain coverage. |
| Receipt `4096` log ceiling | Remove; the existing response-byte bound protects memory. Inspect all relevant logs rather than silently truncate. |
| `swap_timeout_seconds` and saved polling times | Health/backoff only, never abandonment/replacement permission. Future saved poll timestamps cannot strand restarted work. |
| SDK/gRPC duration parameters | Audit the actual transport before changing: baseline compatibility arguments are not necessarily active total deadlines because transport strips `grpc-timeout`. Keep connection and per-response inactivity ownership. |

Keep signed x402 validity, Zcash consensus expiry and the returned 1Click deadline.
These belong to the protocol/accepted operation, not balance age. Use wall time only
where such an external field requires Unix time, or for optional diagnostics; clock
availability must not gate an otherwise valid balance hint or SDK spend.

The unresolved 1Click deposit-arrival event/route contract and upload-progress
limitation are retained in [the evidence section below](#external-contract-evidence-and-transport-limitation). Neither
justifies an invented margin or another timer. Removing the margin does not prove
timely bridge settlement. Expired untouched quotes can refresh; expiry never
discards prepared bytes or authorizes a second deposit. Keep connection/read
inactivity limits, byte/storage bounds, actual work concurrency bounds and useful
polling. Simplify resource machinery when its protected work disappears; add no
replacement whole-work deadlines or enormous sentinel durations.

### BE06: Source recovery and accounting removal

- Derive recovery from typed operation/input ownership facts, not `last_error`
  wording or a separate phase interpreter. Diagnostics identify affected scope,
  waiting versus operator action and the missing fact; preserve sanitization and
  warning suppression. Changing prose must not change recovery behavior.
- Retain exact accepted commands, committed snapshots/address bindings, prepared
  bytes and input claims through cancellation, interrupted sync/address persistence
  and failed acknowledgement. Cleanup belongs to the durable operation owner.
  Saved bytes remain owned despite stale workflow phases. A query helper must not
  reclaim another operation's claim or perform hidden financial cleanup.
- Publish usable SDK state without full historical source/refund validation.
  Expired historical bytes and missing return attribution cannot veto ordinary sync
  or independent work. Constrain only the actual inputs owned by unresolved work.
- Verify every original input before expiry recovery, not just the SDK subset still
  visible. Check the pinned SDK's actual selection, prepared transaction and committed
  snapshot contracts before narrowing any necessary input fence.
- Preserve completed results through local validation, commit, acknowledgement and
  publication failure. Retry only demonstrated idempotent local boundaries; do not
  repeat an external effect merely because local publication failed. Keep the
  explicit USDC insufficient-funds exception confined to payment retries.
- Avoid side effects in selection/validation. If actual permission or claim ownership
  changes across an asynchronous boundary, recheck it at commit using the existing
  transaction/claim mechanism. Advisory estimate changes need no revision protocol.

**Delete duplicate monetary accounting:**
`store/refunds.rs::record_refund` caps and sums local `refund_outputs.credited`,
while `treasury/refunds.rs::collect_refunds` revalidates old credited outputs and
can fail sync. These credits do not supply current spending authority. Remove
that credit/revalidation pipeline and its mandatory historical traversal, rather
than build a smaller return-accounting engine. The SDK owns incoming funds,
spent outputs, fees and chain corrections. Partial, duplicate, late, already-spent
or unattributed returns require no application credit/debit repair. Existing
address/quote correlations may remain as useful audit facts; missing correlation
does not require a durable repair obligation or block use of owned funds.

Audit the other monetary projections too: `budget_entries.consumed`/`reserved`,
`funding.rs::source_accounted`, gross-principal-fee equality and closure/readiness
consumers. Delete historical monetary equality or cost rebasing as prerequisites
for sync, payment, source completion or new independent work. Retain actual proposed
amount/fee checks, exact prepared/submission journals, input claims and the durable
initial-funding claim; these prevent unauthorized or duplicate external effects.
Where a monetary row currently also encodes that ownership, migrate its concrete
ownership consumers before deleting it. Do not preserve a second capital ledger
under the name of a reservation, or erase unknown submitted work with its ledger.
Reported transaction costs can come from saved transaction facts/SDK observations;
they do not need a mutable cumulative debit/credit model. Qualification-run charges
remain separately authoritative and account every accepted attempt.

A refund is neither proof that an uncertain source transaction never happened nor
permission to fund that wallet again. Source replacement retains the exact
non-inclusion/expiry and original-input rules above. If source recovery attempt
limits remain, use their durable history across restart; unsigned quote/RPC outages
do not consume signed-transaction recovery attempts.

**1Click contract investigation:** obtain authoritative 1Click service/deposit-
watcher code, integration contract or tests naming the route, arrival event,
compared timestamp, confirmation policy and late-deposit/refund behavior. Change
only requirements that evidence actually establishes. Cover quote validation,
preparation and durable submission together, including short returned deadlines,
slow progress and cancellation. Preserve original prepared bytes/inputs; quote
refresh never authorizes another potentially effective source deposit. A UI pending
heuristic or local broadcast acceptance cannot close this question. No live spending
is authorized by the investigation, and no timely-settlement guarantee is claimed.

Remove these local source-operation cutoffs with their quote/preparation/submission
consumers and document the remaining external contract limits.

| Restriction | Change |
| --- | --- |
| Internal shielding `now()+86400` in `treasury/refunds.rs` | Remove artificial wall expiry; use transaction consensus expiry and actual operation ownership. |
| `min(returned_deadline, requested_deadline)` and shared 300-second quote margin | Remove the extra local ceilings. The requested window is a preference; honor the accepted returned external deadline. Do not require five minutes merely because a reference UI uses that heuristic. |

- Isolate identifiable bad jobs/rows and advance to independent work
  instead of repeatedly restarting at the same bad page. Retain affected source
  ownership. Distinguish shared Store corruption/lost exclusive ownership from a
  scoped advisory read failure; do not open a second Store to bypass either.
- Delete needless historical work before tuning resource machinery.
  Check actual remaining query scans, locks and shared RPC/SDK queues. Use physical
  paging/cursor progress through existing database/RPC bounds; a small result page
  must not hide a full scan or long-held mutation lock. Add no history-count deadline
  or resource ceiling. Use event barriers to show independent work can progress and
  deferred work resumes through the actual shared resources.

Recheck existing action permissions at source preparation/submission boundaries.
The BE04 authority check must survive recovery and restart; do not create a second
policy or capability framework.

### BE07: Integration and documentation

Reconcile call sites, configuration and status output; run the standard default-build
checks and complete the deletion/complexity review below. Verify that funding
scheduling and reconciliation now contain only advisory refresh and actual
unfinished work, without accounting-repair lanes.

- Preserve narrow identity reads, retained inventory-read retries,
  scoped funding-only failures, listener-before-funding startup and deferred
  catalog/relay publication. Missing funding capability must not stop independent
  static or usable Base service. Shared key/database/ownership failures affect their
  actual dependants.
- Verify listener binding, exclusive Store ownership, policy installation,
  publication and owned-drain ordering after removing gates. Partial setup failure
  must not start unauthorized financial work, lose accepted work or start a new
  owner during shutdown. Reuse existing lifecycle and publication mechanisms.
- Document which actual treasury, funding and reconciliation task exits
  require restart. Busy/Locked retries inside a retained owner do not prove task
  replacement. Audit and fix small in-owner resumption defects where needed;
  unhealthy SDK mutation and an exited reconciliation owner remain distinct from
  independent listener/Base service availability.

**Schema and restored state:** target one supported development schema,
reject incompatible versions before mutation and do not add a permanent historical
compatibility matrix. Any private-state conversion is separately authorized,
backed up and preserves exact owned work; a code rollback is not a state downgrade.
Keep declared-restoration restrictions where source intent or initial-funding
history is missing. A chain rescan cannot reconstruct unpublished authorizations,
uncertain source submission or consumed initial claims; never claim automatic
detection of an undeclared filesystem rollback. The accepted USDC retry risk does
not authorize recreating lost source intent or topping up an already funded wallet.

**Maintained documentation:**

The plan is a review artifact, not the permanent home of runtime rules. Publish a
concise **Best-effort wallet rules** section in `docs/wallet-rotation.md` alongside
the behavior changes, and link to it from `AGENTS.md` and `docs/README.md`. It owns
the final simple rules: advisory estimates, one decrement per authorization,
low-balance/post-failure RPC refresh, each eligible wallet once, accepted possible
multiple charges, no top-ups, SDK-owned Zcash funds, and exact pending-work ownership
without refund or duplicate balance accounting. Include the accepted limitations;
do not leave these decisions discoverable only through Git or this plan.

Use the existing maintained guides for other durable contracts, linking rather
than reproducing competing normative versions:

| Destination | Required content and owning delivery |
| --- | --- |
| `AGENTS.md` | **BE02–BE06:** replace obsolete no-retry/freshness/accounting instructions as behavior changes and point to the wallet rules. Retain concise guidance on real ownership, authority, cancellation, idempotent publication and scoped failures; no duplicate long financial rulebook. |
| `docs/wallet-rotation.md`, `docs/architecture.md` | **BE02–BE06/R01/R02:** explain SDK versus application responsibility, exact accepted-work ownership, initial funding claims, privacy boundaries and real owner/restart limits. Remove claims requiring local refund credits, historical balance readiness or net-cost ledgers. |
| `docs/configuration.md`, `docs/wallet-cli.md`, maintained deployment examples | **BE04–BE06/BE07:** reconcile removed fields/schema, estimate/status meanings, actual fee/permission safeguards, recovery/restoration behavior and no-top-up policy. Preserve user-owned local configurations. |
| `docs/network-egress.md` timer inventory | **BE05/BE06:** record removed local age/margin/elapsed restrictions, actual remaining protocol deadlines and connection/read inactivity. Document the upload limitation and unresolved 1Click route contract where operators encounter them, with investigation provenance. |
| `docs/testing.md`, `tests/live/INTEGRATION.md` | **BE03/BE07:** document per-attempt qualification charges and changed assertions, tested boundaries, remaining external/SDK limitations and what has not been qualified. Preserve separate run authority. |
| `docs/README.md` and focused unfinished plans | **BE07:** index the maintained rules and explicitly unfinished R01/R02 work. Retain this review artifact until final review; later removal requires all enduring policies/limitations and investigation pointers to have maintained homes. |

BE07 must verify links and reconcile contradictions across these destinations,
not just add a rules section beside obsolete instructions. Keep documents
descriptive of actual delivered behavior until their owning runtime commit lands.
The exact-commit reviews and final hazard analysis must check this handoff too.

## Reviews and complexity checks

Use independent subagents to review every commit and meaningful architecture gate.
Review each exact committed SHA and relevant consumers; address findings with
`git commit --fixup=<introducing-SHA>` and independently review the fixup. Preserve
history without squash/autosquash for the final hazard analysis. Record concise
decisions under their owning section, not a running journal. Reviews must question
whether a check is needed before proposing another guard for it.

Measure baseline/changed function CogC/CycC and the complete resulting call chain.
Compare against `49566df`. Scores are advisory; count removed blocking predicates,
derived state and independent publication paths as well. Do not add assessment/certificate helpers
merely to move scores. Use the baseline's pinned complexity tooling and standard
default Zcash checks; no no-default-feature or live-funded runs are authorized.
Freeze sources/executables during validation and use focused tests after local fixes.

Review these concrete refactoring targets with their owning commits. Remove
responsibilities before extracting helpers; do not add a workflow framework to
redistribute complexity.

| Target | Work and review gate |
| --- | --- |
| `collect_refunds` and shielding | **BE06/R02:** delete credit assessment, mandatory attribution and historical revalidation; review the surviving SDK shielding responsibility. |
| Funding scheduling, `source_step`, `collect_inventory`, `dispatch_pending` | **BE04/BE06:** remove accounting/repair work before separating surviving scheduling, retained completion and health rendering. Keep the coordinating work owner. |
| Reconciliation `maintenance` and `read_one` | **BE03/BE06:** simplify to advisory refresh and actual unfinished transactions; verify physical work/fairness and cancellation. |
| Deployment initialization, deferred runtime, serving and CLI dispatch | **BE07/R01:** preserve usable capabilities, actual context permissions, one cleanup owner and bind/drain order; refactor demonstrated mixed responsibilities. |
| Payment orchestration and bound preparation | **BE02/BE03/BE06:** delete solvency/readiness proof branches; retain journal-before-disclosure and actual ownership/permission checks at race boundaries. |

Measure each surviving target before and after its owning change with the same
analyzer; BE07 records deletion, simplification or a concrete reason for retention.
Review complete call chains and new hotspots, not only the parent function score.
Count removed state, mandatory reads and blocking predicates too. Any new table,
repair obligation, certificate, revision layer or monetary reservation must protect
an actual off-chain action/ownership requirement the existing SDK/journal cannot
supply. Agreement with chain balances alone is not justification.

In addition to per-commit reviews, independently review the BE02/BE03 payment
transition, BE04/BE06 source ownership and accounting removal, and BE07 integration.
Include complexity measurements and the relevant acceptance below at each gate.
The final fixup analysis must distinguish deleted responsibilities from complexity
merely moved into other helpers.

## Required focused acceptance

- Estimates may be old, absent, concurrently overwritten or unavailable. Permitted
  payments proceed; no fresh balance/nonce proof, age deadline or historical pass
  becomes a prerequisite. Debit an estimate once per journaled attempt.
- A valid permitted payer remains usable with missing/malformed historical
  readiness support; its actual identity/owner/cap data must still be valid.
- Real production HTTP flow A insufficient → B success retains both authorizations.
  A is not retried; every captured wallet failing terminates naturally. GET and
  supported cloneable POST preserve original arguments and each payer's isolation.
- Every recognized insufficient-funds failure moves its wallet to the low-balance
  set and schedules a post-failure RPC balance read. Success replaces the estimate
  with the returned chain value; failure retains low-balance status and identifies
  the unsuccessful refresh. A slow/failed check does not delay A→B fallback.
  Failure during an in-flight read earns a follow-up check; coalescing cannot lose
  the newer failure. A later call can use a refreshed low-balance wallet when its
  estimate suggests affordability, without retrying it within the original call.
- Initial unsigned challenges, arbitrary prose, generic 402, allowance failure,
  malformed/error bodies, response truncation and transport uncertainty do not
  masquerade as the typed retry category. A later successful attempt does not erase
  earlier possible charges or reset submission progress.
- Journal failure sends no signature. Concurrent calls use unique authorization
  identities; cancellation/restart retains actual accepted work and never silently
  replays an interrupted logical call. Exhaustion returns one terminal result.
- Qualification accounts every attempted authorization and cannot exceed existing
  run authority; ordinary retries add no new production budget configuration.
- Low estimates/insufficiency never top up a wallet or allocate in the retry path.
  Late initial credit can update an estimate without recreating a funding claim.
  Background initial funding of a new wallet retains ordinary auto-funding controls.
  Proved-never-funded initial work can recover within its original claim; observed
  partial/complete credit or an uncertain deposit cannot earn a second deposit.
- Zcash proposals use SDK-owned inputs/actual fees. Uncertain submissions preserve
  exact bytes and claims; expiry recovery proves all original inputs at the correct
  consensus boundary. Returned funds are usable through the SDK without local refund
  credits, original-swap attribution or historical monetary equality. An absent/
  reorged historical refund does not fail sync; the SDK updates its spendable state.
- Slow scan/proving/receipt work exceeding old local ages still completes; genuine
  inactivity/resource failures retain their scoped diagnostics and ownership.
  Host-clock jumps do not control balance eligibility; genuine protocol expiry does.
- No local refund-credit repair is required for duplicate/partial/late returns or
  already shielded/spent outputs. Exact pending source/shielding work remains owned
  across cancellation/restart after its monetary projections are removed.
- Exercise normal application library/executable paths, including SQLite rollback,
  lost acknowledgement, process exit, interrupted sync/address persistence, malformed
  rows and disabled/removed pools with pending work. SDK inventory projections and
  fake monetary credits do not prove actual transaction inputs/outputs.
- Retain old audit reports without current signing credentials or live admission
  prerequisites. Cleanup/eviction/reopen cannot erase pending transaction ownership.
- Demonstrate independent service and later resumption under held shared I/O,
  cancellation and local publication contention. Distinguish containment, in-owner
  retry, actual task replacement, local fixtures and live qualification; record
  missing evidence as incomplete. Freeze relevant source/executable during checks.
- Documentation and errors describe estimates/observations honestly. No saved value,
  provider success message or `COMPLETE` label mints new funding authority.

## Deferred owner recovery and shielding

Plan two follow-ons after the core BE01–BE07 simplification:

- **R01 — eligible SDK/task reconstruction:** justify each recoverable exit before
  replacing an owner. Fence the old SDK owner, resolve accepted commands, preserve
  one exclusive Store, and test reconstruction while healthy Base handles remain
  open. Race shutdown so a replacement neither inherits a failed worker's stop
  token nor survives process shutdown. Reuse existing supervision; no parallel
  lifecycle framework. Until implemented, state the affected restart requirement.
- **R02 — automatic recovery of owned transparent capital:** investigate the
  smallest SDK shielding path for confirmed spendable owned transparent funds,
  whether or not they can be attributed to a swap. Use the existing treasury owner,
  auto-funding/run authority, actual proposed fee safeguard and selected-input
  ownership; do not reserve/charge returned principal or add a refund ledger.
  Preserve address/network privacy boundaries and exact prepared shielding bytes
  across restart. Check the pinned SDK's actual selected inputs/output ownership,
  not an assumed projection. Cover policy disablement, insufficient capital and
  interrupted submission without preparing a duplicate spend. Returned funds never
  reopen the initial-funding claim.

R01/R02 do not block core BE01–BE07 delivery. BE07 must explicitly report them
unfinished and must not claim their automatic recovery behavior is complete.
Update their status explicitly when implemented; retain only genuinely unfinished
plans afterward.

## External contract evidence and transport limitation

The pinned-source evidence below establishes the starting point for the remaining
contract investigation; it does not verify current deployed behavior. This
investigation does not block removal of unsupported local margins or justify
recreating balance-age authority.

**1Click Zcash route:** the returned deadline remains binding. The inspected code
does not establish whether deposit arrival means mempool detection, inclusion,
required confirmations or completed light-client relay, or which timestamp is
compared. The route for `nep141:zec.omft.near`, connector identity, late-deposit
handling and refund behavior remain unproven.

| Pinned source evidence | Finding and investigation pointer |
| --- | --- |
| Zodl iOS `6a7dc6b39cb8`; Android `9f4e719e3eee` | Both request two hours. Their five-minute heuristic synthesizes expiry only for `PENDING_DEPOSIT`; traced sends have no equivalent pre-broadcast margin. See iOS `secant/Sources/Dependencies/SwapAndPay/sources/Near1Click.swift` and swap coordinator; Android `NearSwapDataSource.kt`, `NearSwapQuoteStatus.kt`, `GetSwapStatusUseCase` and `ProcessSwapTransactionUseCase` under `ui-lib/src/main/java/co/electriccoin/zcash/ui/`. |
| Swift SDK `cf07eb228c1b`; Android SDK `ae538fc5b39f` | `Transaction/EndpointSubmitter.swift` and `internal/transaction/SubmitTransaction.kt` establish broadcast/lookup results, not bridge settlement. App deposit notification follows submission without a confirmation wait; notification success is not settlement, and SDK resubmission is not our replay permission. |
| Near-One `bridge-sdk-rs` `d1361ac1c68f`, `omni-bridge` `3f0349044938`, `omni-bridge-services` `00ccb5d1dd89` | Inclusion/confirmation/light-client requirements can vary by amount, route and contract settings; relay adds delay after local confirmations. See `omni-relayer/src/startup/event_handlers.rs`, `utils/utxo.rs`, `startup/utxo_lc_poller.rs` and `bridge-sdk/bridge-clients/near-bridge-client/src/btc.rs`. No traced path establishes the 1Click deadline event. |
| Public `btc-bridge` `7740fae0ed8d` | [Deposit verification](https://github.com/Near-One/btc-bridge/blob/7740fae0ed8dc56571adfe2aaf36bd4edee0d441/contracts/satoshi-bridge/src/btc_light_client/deposit.rs) requires a light-client inclusion proof with computed confirmations. Default names `zcash-connector.bridge.near`, `zcash-client.bridge.near`, `nzec.bridge.near` do not prove our quoted route; repository heads do not prove deployed settings. |

**Accepted gRPC upload limitation / BE05 documentation:** header inactivity includes
uploading the request because the transport lacks a per-request upload-consumption
signal. A progressing upload can therefore time out. Keep the limitation explicit
until actual supported progress is observable; sibling traffic/keepalives cannot
renew that response's timer. Do not add another timer or speculative vendor rewrite.
An uncertain source submission retains exact bytes/inputs, and an upload timeout
is not the accepted USDC insufficient-funds retry category.
