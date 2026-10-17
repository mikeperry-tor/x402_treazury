# Improved recovery robustness

## Review status and navigation

This plan is retained at the user's explicit request as a **review artifact**.
The C01–C22 / stages 1–9 implementation and local validation evidence are
consolidated below. One original C17 / stage 6 follow-up remains unresolved:
classify actual treasury/funding task exits and either justify their retained
restart boundary or complete eligible worker-generation handoff and shutdown-race
acceptance. Eligible SDK reconstruction and audited read retries recover inside
retained owners; an actual treasury/funding task exit still requires restart for that capability,
while listeners and Base reconciliation remain available. This distinction prevents
a blanket claim that every original acceptance requirement is complete.
The [hazard-reduction follow-up](#hazard-reduction-follow-up) records
proposed design work and candidate AGENTS.md rules; it has not been implemented
or granted new financial authority by restoring this document.

Progress and review notes are consolidated under their owning
[implementation contracts](#implementation-contracts-and-evidence), with final
[gate decisions](#gate-inventory-and-implementation-evidence) and stage completion
evidence. Superseded reboot instructions and incremental pass-count journals are
removed; commit references retain traceability. Earlier schema numbers in
implementation evidence identify intermediate commits, while runtime state now
supports schema26 only. Saved historical reports remain read-only and schema-aware.

Review the [commit sequence](#recommended-commit-sequence),
[acceptance cases](#test-matrix), [completion criteria](#completion-criteria),
[target design](#target-design), and [original findings](#supporting-review-findings)
alongside the consolidated evidence. Maintained behavior and qualification limits
also appear in [wallet rotation](../wallet-rotation.md),
[testing](../testing.md), and [lifecycle fixtures](../../tests/LIFECYCLE.md).
No live-funded qualification, private-state repair, replay, or migration is
authorized or claimed by this artifact.

## Recommendation and scope

Simplify recovery around minimal authoritative state, current canonical evidence,
and durable side-effect boundaries. First remove historical prerequisites that no
longer protect a current obligation; then automate reconciliation and repair the
exceptional cases that remain. More elaborate historical verification is not the
default solution to an unnecessary historical dependency.
Keep the current journals, serialized treasury owner, atomic outgoing transaction
commit, and canonical chain checks. A wholesale rewrite or another independently
maintained workflow engine is not justified by this review.

The recurring weakness is not simply insufficient trust in blockchains. Several
control paths conflate a stale observation, an unresolved side effect, and a
contradiction. Broad phase labels and repair flags then preserve a halt without
preserving enough information to select the smallest safe recovery action.
Diagnostics also participate in recovery authority. The result is safe refusal in
some cases where a more precise journal could establish forward progress.
Every remaining blocking check must name a concrete unresolved obligation or
correctness invariant. Implementation must delete redundant checks and derived
authority, not merely add typed wrappers, incident records, or another assessor.

This plan covers Base payment reconciliation, treasury preparation/submission,
funding and swap disposition, startup assessment, repair scheduling, and their
cross-component tests. It does not authorize live transactions, private-state
repair, replay of qualification runs, a change to funding authority, or a new
chain verification trust model. Findings describe source control flow; they do not
establish the cause of every previous startup failure.

## Authority map

| Fact or decision | Authority | What cannot substitute for it |
| --- | --- | --- |
| Base authorization identity, amount, validity and possible submission | Original payment attempt and admission journal | Current wallet balance or a seller success response |
| Base authorization used or expired unused | Canonical, sufficiently confirmed nonce evidence for the exact payer/nonce, with required expiry and lineage checks | A newer timestamp alone, different observation height, or HTTP failure |
| Current Base payment capacity | Fresh confirmed/latest balance evidence minus unresolved exposure, respecting the debit floor | Historical readiness balance or relabeled cached evidence |
| Wallet ownership and role binding | Durable wallet identity and current pool/claim ownership, with validated role provenance | Balance alone or provider success |
| Current destination capacity | Fresh canonical evidence and outstanding liabilities under the decision-specific rules below | Permanent dependence on reproducing an old readiness balance |
| Funding delivery attribution and completion | Exact accepted operation correlation and source/destination accounting | Arbitrary balance increase, even when those funds are spendable |
| Zcash prepared transaction identity | Atomic committed wallet snapshot, exact outgoing bytes, transaction facts and operation binding | An SDK return value before persistence |
| Permission to submit those bytes | Durable submission intent and the corresponding capability | Absence of an acknowledgement or expiry of a caller's wait |
| Source confirmation or canonical expiry recovery | Synchronized wallet and configured indexer evidence joined to the original transaction/inputs | Transaction absence or unchanged balance alone |
| Swap outcome | Exact accepted quote/operation correlation plus accumulated provider facts | Source confirmation or destination balance alone |
| Refund accounting | Original return binding and canonical output identity, with source accounting | Provider `REFUNDED`, or an uncorrelated incoming output |
| Permission to start new work | Current auto-funding settings, per-operation safeguards, durable reservations and qualification authority | Successful repair, recovered capital, or historical execution authority |

Base RPC observations remain assertions checked under the configured endpoint,
chain, confirmation and canonical-anchor policy. This plan does not silently
upgrade them to independently verified consensus proofs or add an RPC quorum.
Zcash scanning likewise retains its existing indexer/wallet trust and freshness
requirements. More useful recovery comes from joining chain facts to exact local
identity, not from declaring the local journal disposable.

## Scope constraints and single-schema rollout

Refund accounting and usable funding capital are separate obligations. C20 implements
automatic shielding of confirmed, unspent, treasury-owned transparent outputs under
current serving-installed authority, enabled auto-funding, an enabled original
profile and the existing fee safeguard. Exact owned inputs may be shielded while
return attribution remains incomplete; shielding preserves that accounting issue
and does not create refund credit or close a swap. One original transparent address,
immutable selected inputs, exact signed shielded output ownership and fee-only
accounting remain mandatory.

The serialized treasury owner and existing outgoing journal retain calculated bytes
and accepted work. Manual bytes and previously attempted bytes are excluded from
automatic first submission. Existing operations remain observable after disablement
or restart. Qualification grants no automatic shielding authority. C18 canonical
closure and C20 shielding do not authorize replacement funding after a refund;
that remains an operator-review decision. See their consolidated contracts below.
No live shielding or private-state repair is authorized by this review artifact.

- Runtime state has one supported database schema: **26**. Reject incompatible
  schemas before mutation; no on-open migration or legacy financial interpreter
  was added. Remove obsolete runtime compatibility paths while preserving their
  safety regressions. Historical read-only reports retain saved-schema readers;
  this compatibility grants neither migration nor present execution authority.
  Version checks prevent incompatible binaries from silently ignoring financial guards.
- If preserving the operator's existing funded state requires conversion, handle
  it as a separately reviewed one-time operation with a backup, exact liability
  preservation and validation of the result. Do not add permanent runtime migration
  machinery or replace state silently. Missing proof remains missing; a conversion
  cannot infer preparation/submission safety from absent rows or status labels.
  This implementation plan does not authorize running that operation on private state.
- Define stale-backup restoration separately from reopening the latest committed
  state. Rescanning can recover canonical activity but cannot reconstruct every
  still-valid off-chain payment authorization, missing submission intent or original
  operation binding. Specify the evidence required before dependent spending resumes;
  absence from a restored journal or current chain is not proof of no liability.
  Test restoration after later activity with temporary state, retaining explicit
  missing-provenance status and allowing independent observation. Do not claim
  automatic recovery for unrecoverable missing authority, or add a second permanent
  ledger or compatibility framework to address this validation case.
  Distinguish a declared backup-restoration workflow from an undetectable filesystem
  rollback: the restored database alone may contain no evidence of later off-chain
  authorizations. Specify how restoration is declared and what external evidence
  is actually available; do not claim automatic rollback detection from the restored
  state or add another permanent ledger to manufacture that guarantee.
- Initial typed classification may compare old/new decisions read-only in tests.
  Do not run two mutation engines in production or let comparison mode submit work.
- Operator diagnostics should identify the affected scope, fixed reason, whether
  work is waiting/rechecking/operator-only, and the missing proof or next action.
  Deduplicate repeated warnings with explicit suppression accounting; do not make
  unknown cases quieter by calling them recovered.
- Preserve actual spendable funds as the funding-capital limit. Do not reintroduce
  daily/cumulative spending/allocation budgets, a wallet-count ceiling, an overall
  reconciliation deadline, or redundant preparation timers. Keep per-operation
  cost/fee safeguards, reservations, current authority and existing resource bounds.
- Preserve the three-block treasury preparation lag rule and exact-tip expiry
  recovery. Historical work never renews the original tip observation or payment
  evidence age. Update the [timer inventory](../network-egress.md#remaining-timers-and-budgets)
  only if a justified owning-layer policy actually changes.
- The unresolved 1Click Zcash deadline/route contract remains in
  [progress-policy limitations](no_self_sabotage.md). This review does not resolve
  it, justify changing the 300-second margin, or authorize replay of prepared work.
- Deep reorgs outside the specified replacement contract, contradictory accounting,
  unresolved provenance gaps and uncertain historical swaps remain explicit incident
  cases. A general-purpose
  reset, silent balance rebasing, automatic liability deletion, and blanket
  refund retries are outside this plan.

## Gate inventory and implementation evidence

This inventory preserves the original review obligations and proposed dispositions.
The final column records implemented decisions and evidence. “Proposed” and “decide”
in the disposition column describe the design question at review time; they do not
mean that its implementation is still pending. Slice-specific contracts below record
the concrete predicates removed, retained invariants and acceptance boundaries.

| ID | Source predicate or field; finding | Live obligation / authority | Disposition and release rule | Owning stages | Acceptance | Evidence |
| --- | --- | --- | --- | --- | --- | --- |
| G01 | `funding.rs` recovery prefixes, `recovery.rs::evidence_outcome`, `bootstrap.rs::disposition`, scheduling diagnostics; F1/F5 | Exact preparation/submission facts and current policy | Delete prose-based authority; derive consumer-specific assessments from shared typed facts | 2 | T15, T21, T34 | Implemented C02 typed recovery/startup/observation assessments; prose is diagnostic only. See boundary and closure contracts below |
| G02 | `Treasury::prepare`, PREPARING phase, sync/refund-address mutation, owner health, `Treasury::close` and supervisor cancellation; F2/F6 | Snapshot, address binding, reservation, bytes and possible submission; exclusive SDK/store ownership | Replace ambiguous phase inference with minimal boundary provenance; separate SDK retirement from shared-store shutdown and recoverable worker exit from process shutdown; retain exact commit/owner checks | 3, 6 | T08–T12, T23, T35 | Implemented: C16 versioned preparation provenance `d4d5407`, C17 retained-Store SDK reconstruction `929b6ca`, and reviewed read/completion ownership fixups `3639bbb`, `dd64f58`, `1a8dc14`. T08–T12/T23/T35 retain exact bytes, reservations and one owner through crash/cancellation. T12 `f188ecb` uses actual SDK proving with simulated later authority-check times, not measured slow proving |
| G03 | `evidence.rs::check_capture`, wallet repair flag and invalidation generation; F4/F8 | Affected resolution/debit support | Proposed removal of blanket observation veto; retain only decision-specific disputes until sufficient release evidence commits | 2, 4 | T01, T02, T04, T19, T27 | Implemented C12 conservative capacity and C14/C15 sole canonical repair path; T44/T04 actual paid HTTP during held repair, concurrent liability preservation and reopen passed in `b3a9a28` |
| G04 | `capture_evidence`, original readiness anchors and `capacity.rs` role publication; F8/F15 | Current owned capacity, exposure and debit floor | Proposed derivation from current canonical support; retire obsolete readiness dependencies without rewriting audit | 2, 4 | T05, T17, T18, T26, T32 | Implemented: C10 removes archival readiness authority (`ff9db28`); C11 serves initial payment-sized capacity (`8760345`, fixture `4c876b9`); C12/C14–C15 retain bounded exposure and canonical replacement without rewriting audit. T39 `c582bba` exercises actual PaidClient/manager admission in the normal library build |
| G05 | `repair_snapshot` ACTIVE/RETAINED, completed-job and undisputed-readiness exclusions; F10 | Role/claim ownership and unresolved funding obligations | Delete exclusions lacking a live dependency; separate current capacity from swap completion | 2, 4 | T18, T26, T27 | Implemented role-independent physical repair inventory and phase-independent funding observations; distinct source/return obligations are tracked separately |
| G06 | `base.rs::original_endpoint`, historical verifier selection; F9 | Approved chain/transport trust and exact support | Decide sufficient current replacement or approved alternate verification; retain original provenance | 4 | T06, T17, T19 | Implemented explicitly configured replacement endpoint selection before a pass, with no mid-pass switching; original-endpoint-removed reopen acceptance added below |
| G07 | `treasury::sync` source-history checks and `source_recheck_page`; F13 | Affected inputs, reservations and source accounting | Proposed removal of whole-sync historical veto; retain scoped live obligations | 2, 4 | T24, T27, T32 | Implemented: C13 canonical source passes `05b8346` are staged, inventory-bound and activated with the final sync snapshot. Source loss revokes only affected support; schema26 `c3d4d5a` retains exact input ownership through burial/restart and existing two-sync expiry correction. Unknown/deep-history limits remain explicit |
| G08 | `refunds.rs::collect_refunds` credited-output validation; F14 | Exact return credit, expenditure and remaining liability | Proposed removal of unrelated sync veto; reconcile affected accounting without double credit | 2, 4 | T14, T25, T32 | Implemented: scoped return observations `46489cf`/`c172c62`/`2ea6762`, canonical return closure `f8c8194`, and automatic owned shielding `9863d95` preserve independent sync. Returned principal, gross debit and shielding fee count separately. One resident SDK output-map traversal remains linear; SQLite paging does not bound it |
| G09 | `early_job_disposition`, RefundPending/operator-only classification; F7/F11 | Exact closure proof and current new-work authority | Derive temporary waits and supported closure; automatically shield eligible returns under separate current authority; decide whether unresolved attribution must restrict shielding owned outputs | 7 | T14, T16, T20, T23, T27, T30 | Implemented: canonical closure discharges only fully proven obligations (`f8c8194`, T48 `2d434a8`); eligible owned outputs can be shielded under separately captured current serving authority (`9863d95`). Attribution uncertainty stays scoped; no guessed return credit, manual-byte submission, automatic attempted-byte retry or replacement-funding authority |
| G10 | `Deployment::load` bootstrap-before-discovery and `initialize_discovery_relay`; F12 | Listener/source binding, relay payment ownership, per-call admission and explicit bootstrap completeness | Replace ordinary global barriers with scoped readiness, including no initially usable payer and implemented deferred relay publication; establish binding-before-new-funding and rollback; retain qualification completeness | 8 | T15, T22, T41 | Implemented ordinary deferred discovery and canonical-capacity wake in the same serving process; actual listener-before-origin, late-source isolation, accepted relay drain and panic cleanup tests in `deployment/deferred.rs` preserve explicit bootstrap/qualification completeness |
| G11 | `require_spend_ready`, outgoing commit checks and funding refresh `outgoing_pending`; F16 | SDK input/snapshot ownership and exact pending bytes | Decide required scope; remove only proven redundant global waiting, retain commit race checks | 2, 3 | T29 | Retained with concrete SDK invariant: C16 real two-note preparation test shows Pepper Sync reset_spends can clear pending input markers. Exact live-input exclusion is required before relaxing this gate |
| G12 | Repair snapshot/commit scans, maintenance balance lane, treasury wallet read guard and process-wide Base RPC resources; F3 | Coherent final commit and exclusive mutation owner | Remove avoidable actor/lock occupancy; retain CAS and ownership | 5 | T03, T04, T07, T33 | Implemented: separate repair lane `07b5cad`, bounded inactive canonical publication `55d5b3b`, sole repair path `525069f`, and unchanged-aggregate RPC resource partition `6bdc853`. T03/T04 cover cross-pool current work and concurrent liability; T33 `8abcfcc` releases the SDK read guard during residual Store accounting while retaining the serialized treasury owner after waiter cancellation, without concurrent SDK mutation |
| G13 | Disabled/removed pool and listener scheduling, retired-wallet observation selection; ownership audit | Durable unresolved obligations independent of service demand | Audit/derive observation eligibility; retain prohibitions on new funding and retired capacity | 7 | T16, T31 | Implemented: policy-free Base owners `8bdcc3e` and retired-wallet observation `8b7d78b` retain unresolved obligations after profile/listener removal and restart. Existing shielding remains observable without current creation authority. New funding and retired admission stay prohibited |
| G14 | Derived phases/totals/caches, including `wallets.unresolved_exposure`, scheduling predicates and schema readers; F25 | Authoritative journals, signed authority and liabilities | Define exposure fault model and pre-admission detection/derivation contract; rebuild derived state; derive observation inventory from outstanding obligations so completion labels cannot hide work; delete competing authority and obsolete compatibility paths | 2, 6, 9 | T21, T28, T36, T45, T47 | Implemented: journal-derived pending/disputed exposure, endpoint membership `2f43075`, hidden PREPARED observation `1ef6616`, declared-restoration fence `78c3b0c`, projection removal `dc284e2`, and final fresh schema26. No on-open migration; historical reports retain saved-schema readers. Undeclared rollback is not automatically detectable and missing off-chain authority cannot be reconstructed from chain scans |
| G15 | Base maintenance inventory-read error propagation and deployment log-only exit; funding-scheduler exit audit; F17 | Observation ownership and durable unresolved obligations | Resume recoverable observation work under existing ownership/backoff; justify stops for corruption or lost ownership | 6 | T37 | Implemented audited read resumption: Base `753d25a`/`827668b`, funding inventory `201bebd`/`5aee0a4`, completion admission `dd64f58`/`1a8dc14`. Retained owners retry only Busy/Locked reads; corruption, publication and lost ownership remain scoped fatal boundaries. Actual treasury/funding task-exit restart handling remains unresolved under C17 / stage 6 |
| G16 | `history_page` whole-page decoding and failure propagation; F18 | Malformed live authority and identifiable affected scope | Keep affected liability blocked; allow independent wallets in the same pool to progress when scope is provable | 2, 6 | T38 | Implemented `281279f` with physical-page fairness fixup `5f6f852`; malformed identifiable rows retain liability while healthy peers progress. Unknown ownership and shared storage faults remain fatal |
| G17 | `TreasuryConfig::runtime_id`, `validate_static_identities`, full `store::status` reporting; F19 | Treasury identity, supported schema and managed/static ownership binding | Replace operational full-report reads with narrow identity/ownership queries; unrelated report decoding cannot gate opening or recovery | 2, 8 | T32, T38, T40 | C01 implemented in b4a3b48; see implementation contracts and evidence below |
| G18 | `validate_resolved_funding`, `managed_connection_preflight`, `initialize_wallets`, `Treasury::open` SDK reconstruction before store-worker creation; F20 | Exclusive store/key ownership, Base journal integrity, dependency-specific connection policy and SDK mutation ownership | Isolate independent static service; decide safe Base/store initialization without a usable Zcash SDK or funding-only configuration, including declaration validation before connection setup; retain justified shared-state stops | 8 | T22, T41 | Implemented scoped startup slices `51c4aef`/`151369f`, `59f9b83`, `fc7fed1`, `3d3305a`/`68e5ef4`; independent static/Base service survives unavailable SDK or funding configuration. See C06 contracts and final cross-layer acceptance limits |
| G19 | `funding_work_page` whole-page job/facts decoding and `collect_inventory` propagation; F21 | Identifiable operation ownership, reservations and shared treasury liabilities | Isolate malformed jobs where dependency scope is provable; retain affected obligations and traverse later jobs; restarting an undecodable page is insufficient | 2, 6 | T42 | Implemented `c27987c`/`f53b7cf` and health decoding `173657c`; exact ownership/source correspondence precedes scoped decode failures, including post-capture completion and reopen |
| G20 | `record_readiness`, `payment_ownership`, candidate discovery/selection, balance scheduling and `consume_reserve`; F22 | Per-payment capacity, exact claim ownership and pending funding authority | Separate payment-sized capacity from funding-target satisfaction; specify initial role/claim transition without closing or duplicating funding | 2, 4 | T26, T43 | Implemented `8760345` and reviewed fixture correction `4c876b9`. T43 actual PaidClient `initial_partial_credit_pays_after_restart_without_completing_funding` passes with $2 capacity/$10 target, claim-race and insufficient-capacity negatives; original funding remains open |
| G21 | Wallet-wide admission halt for disputed authorization resolutions; F23 | Complete bounded exposure, compatible current balances and debit accounting | Decide whether full conservative exposure permits use of remaining capacity; retain broader stops for unbounded or unidentifiable liabilities | 2, 4 | T19, T27, T44 | C12 conservative remaining-capacity service and C14 canonical release implemented; immutable audit proofs retained. C15 bounded canonical publication `55d5b3b` and review fixups are implemented; the exact-revalidation fallback was removed in `525069f` |
| G22 | `eligible_payment_evidence` exposure-read failure, cached candidate scan, pre-filter certificate merging and `candidate_for` propagation; F24 | Identifiable wallet-local liability versus shared store integrity | Isolate affected candidates without treating their exposure as zero; retain diagnostics and try independent usable wallets | 2 | T38, T46 | Implemented C03/C10 wallet-local exposure, ownership and certificate-merge isolation across cache/hint paths; shared storage failures remain explicit |
| G23 | `FundingFacts::provider_review_required`, `status_needed`, `record_provider_outcome`, phase derivation and closure predicates; F26 | Exact accepted operation correlation and remaining source/destination/refund obligations | Decide which provider observations are terminal, provisional or superseded; retain audit facts without an unjustified permanent veto or premature end to required observation | 2, 7 | T13, T14, T48 | Implemented: exact external-satisfaction assessment `33689ec`, canonical delivery, source `05b8346`, returns `f8c8194`, and refresh dependency `bdc90b5`. Real worker T48 `2d434a8` closes across retained conflicting statuses/provider outage only with exact canonical source, destination and finite return proof. New inventory revokes dependent reuse and schedules reobservation; unchanged unresolved observations reuse the pass. Refund replacement still requires review |
| G24 | Wall-clock evidence/authority checks, elapsed-time scheduling and persisted polling times; F27 | Canonical expiry, original evidence age, signed deadlines and executable observation ownership | Audit clock correction, suspend/resume and restart semantics; remove any durable halt caused only by a temporary clock condition without adding a timer | 2, 6 | T02, T20, T49 | Implemented: Base lease `7c4f0ae`, scheduling/authority separation `06af797`, original treasury lease `d8b4dbc`, persisted future-poll restart `43abf6a`, and T49 `8ffb521`. Clock/suspend/clone/reopen and late-checkpoint gates pass; the real repair owner retains liability while canonical time stalls despite expired host time and releases only after canonical advancement |

## Implementation sequence and acceptance gates

Deliver small reviewable commits with their tests. Stage numbers identify work,
not a mandatory serial delivery order; prerequisites below govern execution.
The sequence below records the reviewed implementation method. After the applicable
stage-1 contract is reviewed, prioritize stage 2 gate removal
and stage 8 scoped startup as early tracks. Stage 8 requires only its applicable
readiness changes from stages 2/4,
not completion of treasury reconstruction, refund shielding or all historical
repair. Recoverable observation-owner supervision in stage 6 can likewise proceed
after its stage-1 contract without waiting for treasury preparation changes.

Stage 1 is incremental per implementation slice. Review that slice's decision
algorithm, exact deletion/retention predicates, dependency scope and focused tests,
then implement it without waiting for the repository-wide inventory to be complete.
Deliver narrow identity reads, foreground/background failure isolation, recoverable
observation-owner resumption and independent startup first where their contracts
permit. Follow with historical dependency removal and payment-sized eligibility.
This is a delivery preference, not permission to bypass a demonstrated prerequisite.
Resolve cross-slice prerequisites explicitly; local review does not waive shared ownership or accounting invariants. All inventory rows
and acceptance cases still require resolution before overall completion. Automatic
shielding and treasury reconstruction must not delay unrelated gate removals.

Stage 1 must deliver concrete source predicates and persisted fields to delete,
derive or retain, with locations and focused acceptance cases; an authority diagram
alone is insufficient. For each implementation commit, record the inventory rows
addressed, the old checks/flags/interpreters actually removed, any retained gate's
invariant, and the safety and progress assertions exercised. Preparation-only
commits may establish a prerequisite without deletion, but must name that dependency
and the later deletion they enable. Do not count typed wrappers, new certificates
or incident records as simplification while the superseded enforcement remains.
Each completed simplification slice must show the obsolete predicate or field
removed, the invariant still enforced, and a production-path test demonstrating
previously blocked independent work progressing. Extend the existing gate and test
cases for these results rather than adding another framework or broad audit phase.
Resolve each inventory row before completion; retained restrictions require a
concrete invariant, not an indefinite promise of later investigation. Stage 1 records
reviewed contracts and fixture findings; later stages record implementation commits
and executed test results. The completion-evidence fields now record the implemented
decisions and executed acceptance, rather than intermediate resume checkpoints.

Replacement machinery is conditional on a surviving dependency. Before adding a
certificate, durable incident field, verifier or repair action, record which retained
obligation needs it and why existing records plus current evidence are insufficient.
Delete the obsolete gate in the same slice once its prerequisite is available; do
not retain a second enforcement path as a precaution. Stale or superseded observations
normally require recapture, not a new durable incident lifecycle. No general incident
framework, second work ledger or permanent compatibility interpreter is a deliverable.

### Recommended commit sequence

The rows below are recommended reviewable commit points, not a requirement to
finish stages in numerical order. Each row includes its applicable stage-1 contract,
implementation, focused safety/progress tests and gate evidence update. Review the
contract before implementing its transition; a substantial unresolved contract may
land as a preceding documentation commit naming the implementation it enables.
Do not require a repository-wide specification commit before C01.

Follow this order by default, but use the listed dependencies to move independent
fixes earlier. Every row also inherits its owning stages' proof prerequisites.
C06–C08 are early availability work; they must not wait for refund shielding or
SDK reconstruction recovery. If their startup contract needs a particular admission
change from C09–C12, land that narrow prerequisite first and record the dependency.
C16–C17 may proceed independently of residual Base historical repair. C21 may land
as soon as its clock audit is resolved.

| Commit point | Recommended change and review boundary | Gates / stages | Prior commit dependencies | Focused acceptance |
| --- | --- | --- | --- | --- |
| C01 | Replace full financial reports in runtime identity and static ownership checks with narrow queries; preserve schema and binding validation | G17 / 2, 8 | Per-slice stage 1 only | T32, T38, T40 |
| C02 | Replace diagnostic-prefix recovery authority with shared typed facts and consumer-specific assessments; delete superseded interpreters | G01 / 2 | Per-slice stage 1 only | T15, T21, T34 |
| C03 | Isolate wallet-local eligibility/exposure failures in cached, hinted and challenge-time payer selection, including pre-filter certificate merging | G22 / 2 | Per-slice stage 1 only | T38, T46, relevant T39 production checks |
| C04 | Isolate identifiable malformed Base history and funding jobs within paginated inventories; continue past bad rows without dropping their obligations | G16, G19 / 2, 6 | Per-slice stage 1 only; split Base and funding paths into separate commits if independently testable | T38, T42 |
| C05 | Resume recoverable Base observation-owner and audited funding-scheduler exits under existing ownership/backoff | G15 / 6 | C04 for malformed-page cases; transient-only resumption can land earlier | T37, T38, T42 |
| C06 | Scope funding-only declaration/preflight failures and publish independent static service despite managed initialization failure | G18 / 8 | C01 and the startup capability contract | T22, T40, T41 from the deployment entry point |
| C07 | Separate usable Base/store ownership from unavailable Zcash SDK initialization where proven safe; remove that initialization dependency | G18 / 8 | C06; decide the single-owner boundary before code changes | T41, including shared-integrity failure controls and partial initialization cleanup |
| C08 | Replace ordinary global bootstrap and relay-discovery barriers with scoped readiness and deferred publication through the existing lifecycle | G10 / 8 | C06–C07 where relevant; deliver or identify landed commits for required provider-publication mechanisms; record binding/funding ordering and any narrow admission prerequisite | T15, T22, T41; no-payer startup, same-process resumption, uncertain relay submission and listener-bind rollback |
| C09 | Derive or provably rebuild exposure from authoritative obligations and correct obligation-driven inventory membership; remove competing totals/phase authority | G14 / 2 | Shared G04/G14/G20/G21 admission contract; C03–C04 supply isolation while repair runs | T21, T45, T47 and production enforcement under T39 |
| C10 | Remove obsolete readiness/history dependencies from current capacity and observation publication; replace blanket repair vetoes with decision-specific checks | G03–G05 / 2, 4 | C09 where exposure derivation is required; shared admission contract | T01, T02, T04, T05, T17–T19, T26, T27, T32 |
| C11 | Permit payment-sized initial eligibility below funding targets with atomic role/claim transitions and continued pending-funding ownership | G04, G05, G20 / 2, 4 | C09–C10 | T10, T18, T26, T43 through actual calls after restart |
| C12 | Resolve bounded disputed-exposure admission: implement remaining-capacity service where proven, or record the concrete invariant requiring a scoped stop | G21 / 2, 4 | C09–C10 and the reviewed balance/debit contract | T19, T27, T44; a blocked fixture alone does not justify retaining the gate |
| C13 | Remove whole-sync historical source/refund vetoes and separate current spendability from residual accounting | G07–G08 / 2, 4 | Per-slice live-obligation contract; use C02's assessment where needed | T14, T24, T25, T27, T32; forbid irrelevant archival reads |
| C14 | Implement only surviving canonical replacement and approved alternate-endpoint verification requirements; preserve original audit provenance | G03–G08 / 4 | C10, C12, C13 establish which dependencies remain | T01, T02, T04–T06, T17–T19, T24–T27 |
| C15 | Make residual repair fair at database, RPC, wallet-lock and owner boundaries, with complete revision checks | G12 / 5 | C10/C13 remove needless work; C14 where residual proof implementation is needed | T03, T04, T07, T33; scheduling containment may land earlier under stage 5 |
| C16 | Establish minimal preparation/commit provenance and proven pre-commit cleanup; resolve the pending-outgoing gate's scope and remove it only where justified | G02, G11 / 3 | Preparation and SDK/input-ownership contracts; C02 where disposition is shared | T08–T12, T29 with real crash/persistence faults |
| C17 | Reconstruct eligible poisoned treasury owners from committed state after fencing old ownership and settling accepted commands; preserve the live shared store and separate worker-failure cancellation from shutdown | G02 / 6 | C16; explicit SDK retirement/store lifetime and supervisor handoff contract | T23, T35 across preparation, sync and refund-address mutation, with serving Base handles retained and shutdown races |
| C18 | Resolve provider-fact authority and implement fully evidenced source/swap/refund closure without permanent historical-status vetoes | G09, G23 / 2, 7 | C02, C13; C14/C17 only where the particular closure requires them | T13, T14, T20, T23, T27, T48 |
| C19 | Preserve observation ownership after pool/listener removal, disablement and retirement; implement remaining halt-release scheduling | G09, G13 / 7 | C05, C09 and applicable closure assessment from C18 | T16, T20, T31; no new funding or retired-capacity reuse |
| C20 | Automatically shield exactly attributed eligible refunds through the existing treasury owner and committed-byte journal; implement attribution-independent shielding where proven | G09 / 7 | C16–C18 and reviewed shielding authority/input/fee contract | T14, T16, T23, T30; crash/restart, disabled policy and confirmed spendability |
| C21 | Resolve clock-domain audit findings and add correction, suspend/resume and persisted-poll restart coverage; remove any unjustified durable clock halt | G24 / 2, 6 | Per-slice stage 1; C05 only if owner-resumption changes are needed | T02, T20, T49; no new timer or renewed financial authority |
| C22 | Close remaining schema/compatibility cleanup, cross-layer regression and operator documentation; reconcile every gate with actual implementation evidence | G14 and all outstanding evidence / 9 | All applicable prior rows and resolved retained-gate decisions | T01–T49 as applicable; standard repository checks and documented qualification limits |

Keep each commit coherent and testable. C04, C13, C16 and C18 may need multiple
commits along the independent boundaries named in their rows; do not force unrelated
changes together to meet this numbering. Conversely, combine a small prerequisite
with its gate deletion when separating them would leave redundant enforcement or
an unsafe intermediate state. A decision that justifies retaining a gate produces
its documented invariant and regression evidence, not a ceremonial code change.
Test-only completion is appropriate for C21 if the audit finds no runtime defect.

Schema changes belong with the first implementation that needs them, including
supported-schema creation/reopen tests and rejection of incompatible versions
before mutation. Do not postpone those checks until C22 or introduce temporary
runtime migrations between commit points. Each landed binary supports its declared
schema only. Preserve any existing funded state through the separately reviewed
one-time conversion described above; this sequence does not authorize that operation.
C22 removes remaining obsolete compatibility paths/fixtures and verifies the final
single-schema result rather than introducing a rollout compatibility layer.

Run the row's affected focused default-feature tests before its commit, including
normal-library/executable coverage where T39 applies. Update relevant architecture
and operator guidance with each behavior change, especially scoped startup; C22
checks consistency rather than deferring all documentation. Record commit IDs and
executed results in the existing gate/stage evidence fields. Run final applicable
repository checks once the integrated changes are ready; repeat broad suites only
when subsequent changes or failures justify them. No row authorizes live-funded
qualification, replay or private-state repair.

### Implementation contracts and evidence

The following sections consolidate the implementation and review record by its owning
commit point, including the unresolved original C17 / stage 6 task-exit requirement.
References to schema16–25 identify intermediate committed formats and their tests; the final supported runtime
schema is **26**, with no runtime or private-state migration. Historical reports retain
explicit saved-schema read-only contracts. Focused test counts below describe the tests
executed at each boundary, not separate present-day qualification claims. Final
repository checks passed; all chain/provider scenarios here use local, unfunded
fixtures. No live-funded qualification, private-state repair, provider settlement
contract or absence of future returns is established.

#### C01 — Narrow runtime identity and static ownership

Implementation provenance: `b4a3b48`. Schema16 was this slice's intermediate format;
final runtime schema is 26.

**Contract:** Runtime omitted-ID resolution reads only the sole instance identity
and the supported runtime schema (schema16 at the original C01 gate; schema26 in
the final implementation). Static service reads that identity and the pool name
bindings in one read transaction. Retain private-path checks, UUID/network/ account/record-version
validation, explicit configured-ID equality and rejection of a static profile sharing
any managed pool name (including disabled pools). The Store owner still authenticates
the encrypted snapshot before managed use. Delete `TreasuryConfig::runtime_id -> status`
and `Deployment::validate_static_identities -> status`: sync observations, payment
exposure, readiness, funding jobs and refunds are not identity or name-ownership inputs.
These reads neither repair malformed liabilities nor authorize spending. No schema
mutation, migration, key access or network observation is required. Implementation:
b4a3b48;
`static_startup_ignores_unrelated_financial_reports_but_retains_identity_guards` passes
through actual `Deployment::load`, binding and MCP discovery (default Zcash build,
localhost fixture). It also checks omitted-ID resolution, conflicting managed names,
explicit ID mismatch, schema15/schema17 rejection without conversion and continued
reporting failure for the untouched malformed observation. This fixture establishes the
startup/report dependency; production admission and reconciliation evidence is recorded
under C03, C12 and C14.

#### C02 — Typed recovery and startup authority

Implementation provenance: typed source errors `6a79697`, closure `3491cc6`, and
startup/observation authority `e27a5bd`. Later CLI fixture correction `26e502d` replaces
diagnostic prose with actual production closure provenance.

**First boundary contract:** Expiry recovery must classify local proof failures by typed
source errors, including errors wrapped in context. Preserve all existing exact-tip,
age, burial, input identity/value/unspent and canonical inclusion predicates. A
freshness/burial failure waits for ordinary observation; contradictory input/transaction
evidence requires review. Diagnostic text, even an exact copy of a local message, cannot
select either recovery disposition. This boundary deletes the `evidence_outcome` string
interpreter. The closure and startup contracts below remove the remaining
diagnostic-prefix authority; C16 establishes durable preparation provenance. No new
persisted facts or schema are needed for this error boundary. Implemented in 6a79697.
Five `treasury::recovery::tests` and
`rotation::store::expiry::tests::expiry_release_is_revision_guarded_and_preserves_job_identity`
pass in the default Zcash build. The latter verifies the typed exact-tip failure,
retained pending operation, revision checks, later release and recovery restrictions.
This boundary is one part of C02/G01; the other typed-authority boundaries follow here,
and preparation provenance belongs to C16.

**Closure contract:** Real unsigned quote failures retain `Allocated`; refreshes archive
their exact operation then return to `Allocated`. A `RecoveryRequired` row plus
quote-error prose is not a production proof of an unstarted preparer. Delete those
exceptions in `safely_unprepared` and transactional closure. Safe closure uses
`Allocated`/`Quoted`, or the exact job/operation archive joined with
`source_attempt_closed`; absence of bytes alone remains insufficient. Remove generic
operator-prefix and historical conflict-prefix vetoes. Current contradictions are
rechecked against original inputs/exact bytes by canonical confirmation or two-sync
expiry recovery; every failed proof retains ownership, bytes and reservations. A fresh
full proof may discharge the original obligation. The sub-agent audit found no distinct
invariant requiring a new sticky conflict field. New-work permission and durable
recovery count remain separate checks. Fixtures must create real archive provenance
rather than simulate it with prose. Startup/health classification is specified below;
C16 owns safe cleanup of interrupted `Preparing` work. Implementation: 3491cc6.
Validation: nine focused `recovery_` tests, the extended archive/liability regression,
and the normal-library
`confirmation_wait_is_not_failure_but_reconciliation_errors_still_warn` test pass. A
sub-agent review found no code defects; its archive-plus-reservation negative control
was added and passes. Signed conflict/canonical-replacement lifecycle evidence belongs
to C14.

**Startup and observation contract:** Replace `funding_shortages` and
`StartupPath::disposition`'s diagnostic category parser with typed, operation-bound
current-process observations. Actual funding restriction supplies current pause and
qualification status. Cost, amount, input-cap, shortage and authentication producers
return typed errors; unsigned outages remain ordinary retryable observations. The
scheduler retains the type beside diagnostics in its existing bounded records and
publishes it under the same operation check. Reopen does not reconstruct a policy
obstruction from saved prose. Delete source-operator diagnostic append/ preservation and
scheduler integrity-prefix priority; retained provider flags and original journals
remain the authority. Ordinary health text can be replaced or cleared without changing
closure eligibility. Remove the credit-wait string branch: its sole producer is the
test-only `record_credit` helper; production funding credit publication already returns
successful observation while credit is insufficient. No additional timer, funding
permission, durable incident field or schema is needed. Validation: all 26 bootstrap
tests, eight startup tests, five scheduling tests and 14 scheduler tests pass;
library/funding-test Clippy passes with warnings denied. The funding integration suite
passed 23 cases; its remaining historical-diagnostic assertion was updated to check
unknown-preparation ownership after diagnostic clearing and a later tick, and that
focused case passes. Read-only sub-agent review found no actionable runtime defect. The
review fixup adds a sibling-lane condition retention/clearing regression and corrects
the later-tick assertion to inspect the later treasury-operation snapshot.
Implementation: e27a5bd plus review fixup.

**Recurring review lesson.** Diagnostic wording had accumulated control-flow authority
at several independent consumers. Typed production provenance had to reach closure,
startup and scheduler decisions together; updating just one interpreter left the hazard
in adjacent paths. A CLI fixture using prose as authority was a test-contract
correction, not evidence to retain the interpreter.

#### C03 — Wallet-local selection failures

**Contract:** A decimal exposure parse failure is wallet-local only when the wallet row
was successfully read by exact ID; SQLite and ownership errors remain shared failures.
Reject that payer without treating exposure as zero, retain its malformed value and
report the fixed category through candidate traversal. Cached, hinted and challenge-time
selection must continue with independent payers. Certificate merging compares complete
proofs before ranking. A conflict invalidates only the affected wallet/anchor through
the existing generation/dispute mechanism, excludes that wallet for the whole merge and
retains independent certificates. Normal selection stays read-only; discovering a
contradiction may invalidate support but cannot reserve, sign, change roles or start
funding. No new persistent incident record is needed. A shared database failure still
propagates. C09 remains responsible for authoritative exposure derivation; this slice
supplies isolation. Implementation: 07529ca plus review fixup a1817b8. The sub-agent
review identified lost merge-failure metadata and a cached-hint fallback regression. The
fixup retains typed failure through traversal exhaustion, scopes merge/invalidation to
the captured group, and keeps unsigned hint fallback separate from admission. All 29
selection tests and the added uncached-hint regression passed. Tests include cache
churn, repeated/reordered conflicting proofs, foreign-scope exclusion, all malformed
exposures, shared SQL errors, cancellation and independent capacity. C01 and the
typed-evidence C02 slice also received a read-only sub-agent review with no actionable
findings.

#### C04 — Scoped malformed inventories

**Base history contract:** Advance the bounded cursor past an identifiable malformed
payload only after decoding the sequence, attempt ID, wallet ID and exact wallet/pool
ownership. Payload conversion failures produce a typed scoped outcome; SQL stepping
failures, unknown ownership and cursor failures remain shared errors. No malformed row
receives RPC, resolution, exposure subtraction or replacement. Existing exposure and
atomic admission remain unchanged; this slice does not infer that an unknown liability
is bounded or repair corrupted liability amounts. An all-malformed page is not an empty
page. Independent balance/history reads continue; a one-pass caller receives the scoped
failure after independent work drains. The continuous owner revisits durable malformed
rows on its existing cadence. Buffered valid records retain their exact pre-I/O
revalidation. No new durable marker, quarantine, retry ceiling, timer or schema is
needed. Funding inventory isolation is a separate C04 boundary with separately reviewed
source-liability dependencies. Validation: the 20 history-focused tests pass, including
cancellation, retained active work, retry saturation, all-malformed-page continuation,
unchanged malformed liabilities, unknown-ownership/storage failures and restart.
Sub-agent review found an acquisition fairness issue for long malformed prefixes.
Implementation 281279f plus a separate fixup yields after PAGE_SIZE scanned rows,
retaining the cursor and resuming immediately while polling independent
reads/cancellation. All ten maintenance tests pass (nine together plus the corrected
multi-page regression separately). The expanded test retains 256 malformed obligations,
resolves seven later records and completes within a three-second harness deadline
(existing cadence is five). The reviewer confirmed the fairness fix with no additional
actionable findings.

**Funding inventory contract:** Only typed decoding failures from scheduler payloads
(phase/provider observation fields) may become local inventory outcomes. Validate exact
job/wallet/pool/current-operation ownership and intact source-journal correspondence
before scoping the error. Unknown ownership, source liabilities and SQL failures remain
shared errors. Existing fresh SDK spendability and prepared-outgoing checks, not funding
inventory totals, guard source preparation. Keep source bytes, reservations, archives,
counters and operation identity unchanged. Advance every lane's bounded ID cursor past
malformed entries, including entirely malformed pages. Retain a first error for one-pass
completion after healthy work drains; continuous observation revisits the original rows
on its existing cadence. Apply the same classification to post-capture completion
decoding failures, without swallowing store writes or granting new work. No additional
quarantine ledger, funding budget, retry mechanism or schema is required. Validation: 16
scheduler tests pass, including post-capture phase/provider corruption,
same-pool/other-pool observation progress, source-byte retention, no new recovery,
restart, cancellation, saturation and fatal store-error ownership controls. Five initial
scheduling tests and the added all-malformed-page/reopen/ownership regression pass.
Completion's provider decoder uses the same typed conversion as inventory; its SQL and
transactional writes remain shared failures. Sub-agent review found no blocking defect.
Implementation c27987c plus a separate review fixup binds late completion diagnostics to
their captured operation; the rollover/reopen regression passes and the reviewer
confirmed the correction. The health-field extension below applies the same decoder.
Source facts/attempts with unknown liabilities intentionally remain shared failures.

**Health-field extension contract:** Apply the same scoped decoder to funding health
field conversions and negative started-at/next-poll values. These are scheduling
observations, not source spending authority. Preserve malformed values and current
operation identity, retain missing-health-row behavior, and keep SQL execution/stepping
errors shared. The existing ownership/source guard remains mandatory; do not default bad
values to zero or create an incident record. Validation: six malformed health
representations retain exact SQL values across reopen with healthy-peer continuation and
missing-table failure. The runtime pre/post-capture regression also passes for health
streak and next-poll corruption. Library Clippy passes. Read-only sub-agent review found
no actionable issues.

**Recurring review lesson.** Recognizable malformed data may be local only after exact
ownership and intact source liabilities are established. Cursor progress must count
scanned physical rows, including bad or ineligible prefixes; returned-row counts alone
concealed starvation.

#### C05 — Retained observation ownership through transient reads

Implementation provenance: Base inventory `753d25a`, funding inventory `201bebd`,
completed-observation retention `dd64f58`, and independent-review cancellation fixup
`1a8dc14`.

**Base inventory resumption contract:** Retry only typed SQLite Busy/Locked failures at
read-only priority, wallet and history inventory boundaries. Keep the same owner guard,
captured cursor, active futures and history retry metadata. Use the existing five-second
cadence; acquired independent reads continue while the failed inventory waits. A
one-pass caller drains independent work then reports incomplete observation. Preserve
existing handling of per-wallet RPC failures. SQL corruption, unknown ownership,
channel/task failure and financial publication failures still propagate. Do not restart
the whole owner, replay a mutation or add a deadline. Funding inventory resumption
follows separately under the same audited-read restriction; treasury generation
replacement remains C17. Validation: 19 inventory-focused tests and library Clippy pass,
including injected Busy/Locked at all five Base boundaries, independent observations,
retained owner, no duplicate active nonce, cancellation, incomplete one-pass and
corruption controls. Implementation: 753d25a. Review found no implementation defect; its
test fixup records actual balance RPC payer addresses and requires observation of an
added non-priority retained wallet after the failed page resumes. The expanded test
passes.

**Completion admission-read continuation:** Completed source/status/quote observations
retain their exact payload and original lane permit while the initial operation-identity
read returns Busy/Locked. Only that read retries on existing poll cadence; sibling work
continues, and neither I/O nor publication repeats. Operation rollover discards stale
observations. Source-owned completion drains through cancellation; unsigned work retains
its prior shutdown behavior. Corruption, mutation and later diagnostic errors keep their
original classification. All 21 scheduler tests passed. Independent review cleared
production and requested preserving both cancellation-during-I/O and retained-completion
tests; the parameterized fixup passed separately. One-pass work retains accepted
completion through read contention rather than abandoning it or imposing a new deadline.

**Funding inventory resumption contract:** Share Base's typed Busy/Locked read
classifier, restricted to initial/rolling funding boundaries and page reads. Preserve
the funding lease, lane cursor, active source/read futures and per-operation backoff.
Defer only the affected lane under its existing poll cadence; other lanes keep
observing. A transient initial boundary failure waits with the same lease; one-pass
requests report it. Never retry `publish_observation`, phase changes, timeout writes or
accepted source work. Permanent store faults retain existing fatal owner behavior and
any separately owned source drain. No new worker generation, SDK reopen, spending
authority, retry ceiling or durable scheduling record is added. Validation: all 17
funding scheduler tests and library Clippy pass. New fault cases cover both inventory
read kinds in continuous and one-pass mode, independent credit/status progress, retained
source lease through cancellation, unchanged signed bytes/attempt counts after reopen,
and corruption propagation. Sub-agent review found no code defect. Implementation:
201bebd plus a review fixup that injects the rolling boundary failure after the source
task is held, confirms the fault is consumed, then requires new credit/status
observations without a duplicate source. The expanded regression passes.

**Recurring review lesson.** Keeping an I/O future alive was insufficient if the
following local identity read could abandon its accepted result. Owner, exact payload
and original permit must survive through publication, with read-only contention
distinguished from writes and corruption.

#### C06 — Capability-local configuration and Base failures

**Shared-state prerequisite contract:** Serving fallback must not mask shared identity,
schema, key, snapshot authentication or exclusive-owner failures. Split shared Store
opening from SDK reconstruction, retaining the exact exclusively owned store only for
the typed SDK authenticated-snapshot decode failure. SQL, decryption, offline
configuration and unexpected network binding errors remain fatal. Strict wallet commands
still require the SDK and report the same fixed failure. No new state, schema, owner
generation or financial authority is introduced. This narrow prerequisite supplies C06's
preflight ordering and C07's live Base store; C06/C07 serving evidence below exercises
the resulting degraded capability paths.

**Funding-only configuration contract:** Ordinary no-relay serving validates shared
treasury identity/paths and Base environment, fallback, confirmation/freshness policy
before publishing service. All wallet, payment-cap, assignment, registry, listener and
network validation remains strict. Strict config inspection/check, explicit bootstrap,
relay and qualification compose all validators. Missing treasury/funding sections remain
strict dependencies where required. The Base-unavailable path below isolates Base
capability failures. After authenticating the single shared Store, source-only
validation/construction can select the store-only runtime: treasury
sync/confirmation/fee/endpoints and NEAR credentials/quote/slippage/poll policy. No
alternate values are substituted. Only this source constructor is caught, never shared
opening, Base setup or profile reconciliation. Funding remains paused with no source
observer or preparation worker; liabilities and exact committed snapshots remain
durable. Corrections take effect on restart. Entry-point tests exercise valid SDK
snapshots with malformed credentials, invalid quote window and fee, successful Base
signing and static MCP calls, retained exposure, and a declaration matrix with
shared-key failure controls. This is local synthetic evidence, not live settlement.

**Base-unavailable static-service contract:** Ordinary no-relay serving defers Base
dependency validation to its constructor after shared Store authentication. A Base
declaration/connection failure retains that owner with funding paused and installs an
explicit unavailable PaidClient for each managed binding. Profile/identity
reconciliation remains authoritative; no Base manager, reconciliation or funding actor
starts through invalid Base policy. API execution and wallet-identity documentation
probes return typed WalletNotReady before transport/provider I/O. Static API service,
catalog discovery and independent unsigned help/import capabilities remain available.
Strict validation, explicit bootstrap, enabled relay and qualification are unchanged.
Missing treasury/funding sections and shared-state failures still stop dependent
deployment initialization. Corrected Base configuration takes effect on restart.
Entry-point fixtures cover invalid Base limits, missing explicit environment values and
insecure endpoints, actual managed MCP rejection versus successful static MCP
invocation, cloned clients/probe identity refusal, no allocation/funding jobs and owner
cleanup.

#### C07 — Usable Store/Base ownership without an initialized SDK

**Authenticated SDK failure contract:** Ordinary non-qualification binding can retain
the exact authenticated Store when SDK decoding alone fails. Shared schema, key,
ciphertext authentication, identity and owner errors remain fatal; explicit bootstrap,
relay setup and qualification remain strict. Pause new funding before configured
profile/claim transitions. Existing Base keys, journals, admission and reconciliation
use the same serialized StoreHandle; no SDK, funding actor, source submission or
recovery reset is started. Configuring claims can apply ordinary cap/target/removal
transitions but allocates no wallets. Setup errors, binding rollback and normal shutdown
drop dependent handles and await the owner. Cancellation/drop preserves accepted store
work and releases ownership when that queue drains. SDK restoration in this slice needs
restart; supervised SDK reconstruction remains C17. Funding-only configuration/preflight
isolation remains C06, and enabled relay deferred publication remains C08. Synthetic
entry-point fixtures cover signed Base payment, unresolved-liability rejection, mixed
static discovery, unchanged snapshot/funding inventory, strict/shared-failure controls,
rollback and cancelled/dropped deployment cleanup. These do not constitute live funded
qualification.

#### C08 — Deferred discovery and listener-first lifecycle

**No-relay startup contract:** With no enabled serving discovery relay, retain
authored-provider and listener-credential preflight, load catalogs, initialize dormant
managed owners and bind all listeners before starting funding. Pool declaration
establishes configured claims without allocating wallets. Existing managed admission
returns typed WalletNotReady before provider I/O for empty pools; no new readiness gate
or wait is required. Explicit bootstrap retains its separate captured-claim authority.
The relay publication and capability-local initialization contracts below complete these
paths. Entry-point fixtures cover enabled auto-funding with an empty treasury, no
bootstrap wait, typed no-payer failure, zero provider requests/allocations/jobs and
second-listener rollback.

**Shared-publication prerequisite contract:** A late static source or pricing result
must compose with current dynamic registrations under the existing Manager mutation
lock. Store the static base there, expose an off-lock immutable capture and compare its
revision when replacing complete static views. Validate listener keys, static bindings,
reserved names and collisions, then construct the combined snapshot and check all
generation overflows before changing anything. Dynamic registry commits retain
durability-before-publication and use the latest static base; catalog generations
advance across both writers. Unchanged dynamic record revisions retain their help cells,
and captured snapshots remain immutable. An accepted blocking publication survives a
cancelled waiter; stale retry/revision failure cannot undo a later removal. Static
changes are process-local and add no registry schema or durable work ledger. Deferred
scheduling, source status, pricing integration, standalone publication and relay drain
use this publication contract as described below. Tests cover overlapping
registration/publication, stale pricing after removal, view/binding/name/overflow
failures without partial publication, and accepted-work cancellation.

**Relay-readiness and drain prerequisite:** Check existing unsigned identity
availability using the same read-only default-identity calculation, without payment's
retained-role promotion. Only an absent identity becomes typed dependency waiting;
unavailable configuration, malformed/shared evidence, exhausted target attempts and a
stopped breaker remain errors. Waiting consumes no relay attempt or provider I/O. Every
actual request still resolves identity and validates fresh payment admission; readiness
is not authority. Typed retry dispositions retain the existing one-second backoff and
three actual pre-submission attempts. Paid uncertainty still closes the shared run-wide
breaker. Relay drain delegates to the existing shared PaidClient owner and is for final
deployment shutdown only, never an individual target completion. Tests cover repeated
empty-payer waits, same-owner resumption after canonical fixture evidence, no readiness
role mutation, attempt exhaustion/backoff, terminal immutable/malformed state, and
cancelled paid-waiter drain. Deferred scheduling and shutdown wiring below retain this
readiness/drain contract.

**Relay listener-ordering prerequisite:** Ordinary enabled-relay serving validates
listener credentials and authored provider composition, then reserves every configured
socket before opening relay wallets, bootstrapping or performing paid discovery. Static
payers and `DenyNewFunding` use the same order. Binding later consumes the exact
sockets, including ephemeral ports, and rejects an incomplete listener map. A failed
bind releases earlier sockets; later setup failure releases sockets and accepted
financial owners. Bound sockets are not yet serving MCP. Inspection and explicit wallet
bootstrap retain their existing scope. Tests cover second-socket failure before any
wallet can open, both funding policies and payer modes, ephemeral socket reuse, later
catalog failure, and post-initialization credential failure with immediate treasury
reopen. This removes the spending-before-bind ordering hazard; the deferred-publication
contract below establishes independent listener service.

**Retained-catalog continuation prerequisite:** Split catalog loading into an unsigned
cache/origin phase and an owned relay continuation. The unsigned phase retains fresh
relay-cache precedence and performs the original origin load once. Only remote failures
with a configured relay become continuations; local and no-relay failures stay
immediate. A continuation captures the original typed failure, immutable request
configuration, same shared relay and exact cache slot. Resumption performs relay fetch,
parse and cache publication without origin I/O or validator refresh. The strict loader
composes both phases once, preserving inspection and qualification behavior. The
HTTP-status/parse-failure fixture matrix verifies zero relay calls during preparation,
concurrent resumptions sharing one payment, no repeat origin requests, immutable caller
configuration and relay-only cache provenance. Deferred scheduling and dependency
notifications below use this continuation. A follow-up review found historical readiness
restoration omitted the existing pool notification. It now notifies that pool after
commit; canonical restoration fixtures assert the wakeup, unrelated-pool isolation and
unchanged balance/provenance.

**Event-driven relay wait prerequisite:** The opt-in deferred-fetch API enables existing
pool-capacity and shared relay-outcome notifications before calling the same serialized
fetch/readiness check. Empty identity waits consume no attempts or I/O; there is no
added polling cadence or deadline. Pre-submission failures use the existing finite
attempts and returned backoff. A completion guard wakes sibling wallet scopes on every
accepted relay outcome, including unwind, so an empty payer observes a shared terminal
failure. Cancellation drops only the waiter; accepted paid work remains drainable. Tests
exercise cancelled unsigned waiting, production reconciliation wakeup, sibling
paid-failure disablement and cancelled accepted payment drain. Strict fetch retains its
behavior; deployment-owned publication below uses this API.

**Deferred-publication integration contract:** Enabled-relay ordinary serving retains
eligible origin failures as shared, immutable continuations and binds available tools
before invoking those continuations. One serving-owned publisher schedules relay
attempts under the configured catalog concurrency, releasing lanes during identity
waits; accepted owners retain permits. Startup pricing uses its existing source/request
limits after binding. Complete listener views and instructions publish together through
the shared Manager or a standalone revision comparison. Late source validation failures
remain source-local; existing priced definitions, captured calls and help cells survive
other publications. Pending, unavailable and stopped-publication status is explicit.
Shutdown cancels unsigned waits, finishes accepted publication, drains foreground calls,
then drains retained relay owners before closing financial owners.
Inspection/qualification and warm commands remain strict. Review found and corrected
retained-wallet bind cleanup, unbounded accepted-work scheduling, reserved-name
rejection scope and early return on publisher panic. Independent drain handles survive
worker failure; its error is reported only after financial ownership closes. Tests cover
140 waiting requests with one runnable lane and an independent ready wallet, alias
coalescing, invalid/valid late-source ordering, healthy calls during accepted relay
work, price/help retention, and shutdown without replay or late publication, including
an injected publisher panic followed by immediate store reopen. The serving-bootstrap
removal and deferred unsigned-origin contracts below remove those remaining startup
barriers and complete the integrated T41 scope.

**Serving-bootstrap removal contract:** Delete the serving-only FundingSupervisor,
bootstrap wait and handoff rather than preserving a dormant second startup policy. All
ordinary serving, including enabled relay, prepares dormant owners and starts
funding/reconciliation only after listener binding. Extend the same authenticated
store/SDK/funding/Base capability isolation to relay initialization; shared
identity/key/schema/ownership faults remain fatal. Explicit `wallet bootstrap` retains
its independent captured-claim monitor, accepted-command drain and completeness.
Entry-point tests cover enabled auto-funding with an empty managed payer, zero
pre-serving allocation/provider I/O, immediate managed refusal, SDK/funding/Base fault
isolation with real static MCP calls, and paused observation after serving starts.
Obsolete supervisor-handoff tests are replaced by real serving ownership coverage. The
deferred unsigned-origin and observation evidence below complete this scope.

**Deferred unsigned-origin slice:** Enabled-relay ordinary serving retains remote
catalog preparation before origin I/O. Aliases share one immutable preparation result;
catalog permits cover unsigned requests and are released before relay dependency
waiting. Cancellation retains the existing unsigned OnceCell semantics; accepted relay
work retains its independent owner. Local authored catalogs and strict
inspection/qualification/warm loading are unchanged. Focused tests cover deferred I/O,
alias coalescing, late publication and shutdown. The ordinary deferred-discovery and
recovery/status evidence below includes no-relay startup.

**Observation and recovery evidence:** A combined managed-relay fixture now waits
deterministically for discovery's dependency wait, reconciles canonical Base capacity
through production code, and observes late publication plus callable independent/late
tools in the same serving process. It preserves the initial allocation count, wallet
snapshot and one signed relay request; it is not a full entry-point initialization
proof. `tools/list` now carries listener-scoped current catalog instructions, generation
and publication-stopped status in namespaced metadata, so status is readable without
reinitialization. The response captures one snapshot for definitions and instructions.
Operator-dependent discovery below reads allocation policy separately instead of
inferring operator action from a false identity lookup.

**Ordinary deferred-discovery scope:** The same serving publisher now owns remote
unsigned discovery and pricing for ordinary deployments with or without a relay. It
retains local authored validation and strict inspection/qualification completeness. A
real load/bind/serve fixture holds an unsigned origin response open while independent
tools remain callable, then publishes the late catalog. All ordinary listener binding
still precedes funding-worker start. This removes the initial remote-catalog barrier
without introducing another scheduler or changing signed relay retry policy.

**Operator-dependent discovery:** No-wallet evidence is read separately from identity
availability. When this serving owner cannot allocate, that fact reports
operator-required discovery while retaining the existing notification waiter. Committed
allocation notifies its pool and clears the diagnostic before Base readiness; existing
allocated wallets stay recoverable pending. Tests cover both paths with no relay request
before canonical evidence, then one signed request and same-process publication.
Diagnostics never grant spending or replacement authority.

**Recurring review lesson.** Removing a startup barrier exposed lifecycle seams:
publication must compose with concurrent registry changes, listener reservation must
precede financial actions, and shutdown must retain independent drain handles even after
publisher failure. Readiness notifications are progress signals, never spending
authority.

#### C09 — Journal-derived exposure and obligation-driven inventory

Implementation provenance: journal exposure `2229598`, projection-write removal
`8384e4e`, disputed membership `a0f5f2b`, endpoint dependency `2f43075`, obligation
traversal `29bc0d2` with `08a5c5d`, and hidden prepared obligations `1ef6616`. C22
removes the obsolete projection from the final fresh layout.

**Payment projection-removal contract:** Pending payment-attempt membership is the
durable journal state constrained to POSSIBLY_SUBMITTED/RESOLVED; sum exact outstanding
amounts with checked U256 arithmetic using the existing wallet/state index, validating
each row's pool ownership. Selection, admission, transactional commit and reporting
share this derivation. Before C22 removed it from the fresh layout, the legacy wallet
total was only a compatibility projection: malformed/stale totals and projection
underflow could not control payment/repair eligibility. Preserve serialized admission
exposure comparisons and all existing resolution-support invalidation gates. The
disputed-release membership and obligation inventory contracts below complete those
separate C09/G14 boundaries. The fault model covers malformed identifiable rows and
stale projections, not undetectable deletion or internally consistent rewriting of the
authoritative journal.

C09 review removed production writes to the obsolete exposure column entirely: a
compatibility refresh must not veto a valid resolution because another liability is
malformed. A focused canonical-proof test resolves the valid attempt while leaving the
malformed attempt outstanding and admission-blocking. Reports derive from journals; the
old column remained solely for intermediate schema compatibility until C22 removed it
from the fresh layout.

**Disputed-resolution membership and schema17 provenance:** The initial schema17 implementation added
`payment_attempts.resolution_disputed` with a partial `(wallet_id,id)` index. C15
replaces per-row dispute updates with one wallet-level payment-resolution dispute state
and paged revision-checked activation; the conservative membership and exact liability
invariants remain. In the schema17 gate, runtime open and identity checks rejected
schema16 before mutation;
historical read-only reports remain available, with schema16 no longer claiming current
pool capacity fields. No private conversion or runtime migration is included.
Chain/checkpoint invalidation conservatively includes every RESOLVED attempt for the
affected wallet in disputed exposure in the same transaction as generation invalidation.
The final implementation does so through `wallets.payment_resolutions_disputed`, not
individual per-attempt writes; indexed RESOLVED membership supplies the exact amounts. Amount-only conflicts leave nonce
membership unchanged. Exposure is an exact checked sum of disjoint pending and
disputed-resolved sets; missing resolution rows, ownership mismatches, malformed amounts
and overflow remain failures. No full history scan is added to admission. A duplicate
same-outcome nonce observation cannot clear membership. Exact historical repair includes
the membership state in its digest/exposure invariant and clears it only with the verified repair
commit, with rollback on any failed guard. Original proof bytes remain immutable.
Endpoint-derived membership, canonical release and remaining-capacity admission use the
C09 dependency contract, C14 repair and C12 admission below. Validation: 259 rotation
tests passed (four ignored subprocess helpers), including 31 evidence tests; the focused
deployment schema/identity rejection regression also passed. Review found an old-schema
retirement-report regression, addressed in the following fixup.

**Conservative membership invariant (consolidated C09/C12 design review):** A checkpoint
mismatch does not identify the fork's starting height. Invalidating only its hash or
later heights would leave possibly invalid releases unreserved. Affected resolved
authorizations are conservatively included in disputed exposure; ordinary pending and
disputed-resolved sets are disjoint, checked and exact. Membership invalidation and
generation change commit atomically. The final wallet-level dispute state retains this
invariant without unbounded per-attempt updates. Repair activation checks
membership/revision and rolls back completely on failure. Endpoint provenance includes
original and effective replacement supports. C12's separate current-evidence path
preserves debit constraints while omitting the disproven historical checkpoint; deleting
a repair flag alone could not authorize it. The original schema17 per-attempt proposal
is implementation history, not a second current membership authority. Supported-schema
rejection precedes mutation; no implicit private-state conversion is introduced.

**Qualification exposure followup:** Explicit qualification snapshots now use the same
pending/disputed exposure authority as payment admission, enumerating wallets by actual
pool ownership and emitting only positive totals. Corrupted journal pool bindings cannot
hide a sole obligation. Missing original resolution proofs and malformed amounts fail
for the affected pool; unrelated wallet amounts are not decoded. Focused regression
passed; independent review found no findings.

**Durable payment endpoint dependency prerequisite:** Typed endpoint distrust now finds
original resolution dependencies through guarded JSON authority fields. It atomically
reserves every resolved liability of each affected wallet, preserves original proofs and
financial/debit anchors, bumps wallet and endpoint generations, and evicts affected
wallet caches even when acquired through another endpoint. Malformed or missing
provenance blocks its identifiable owner without treating its liability as zero;
existing stronger amount/evidence repair is never downgraded. Ordinary RPC outages do
not invalidate facts. No schema change is needed because current compact payment
checkpoints originate only from original resolutions. C14 extends this dependency lookup
to adopted replacement proofs/checkpoints before enabling replacement release. The
focused regression passed rollback, restart, sibling isolation, malformed provenance and
repeated invalidation checks; independent review found no actionable findings.

**Funding inventory traversal contract:** Remove the `state != COMPLETE OR
readiness_disputed` SQL filter. Traverse the existing funding job primary key in bounded
64-row pages under the captured insertion boundary; coherent operation facts select
actual credit, provider and submitted-source lanes. A stale completion projection must
not hide those observations, including after restart. No new ledger, financial
authority, preparation or submission is introduced. The hidden-prepared-obligation
contract below covers prepared-but-unsubmitted bytes hidden by a stale phase;
C13/C18/C20 traverse historical operations. The review fixup skips timeout bookkeeping
only for inert completed history, retaining timeout-triggered recovery for other phases.
Shared obligation facts keep active observation errors, streaks and polling diagnostics
visible despite stale completion labels. Historical schema reports retain their existing
read-only projection without querying newer fact columns. `29bc0d2` implements
traversal; the accompanying fixup addresses both review findings. Validation: 18
scheduler tests and 129 store tests passed (two ignored subprocess helpers), including
stale completion across reopen, preserved signed bytes, provider-outage diagnostics,
multi-page traversal, and historical completed schema reports. Final focused
health/late-result regressions, formatting checks and all-target Clippy passed.

**Schema17 historical-report review fixup:** Independent review found that schema16
retirement totals lost their per-wallet records when current admission reporting moved
to schema17. Historical reporting now reads exact retained retirement records
independently of current admission fields. The regression uses an actual schema16 layout
and query-only connection, retaining nonzero totals without fabricating current
capacity. Focused regression and all-target Clippy passed; independent re-review found
no remaining actionable findings.

**Hidden prepared-obligation followup:** Exact PREPARED treasury bytes with zero
submission attempts remain observable when a stale derived phase says ALLOCATED, QUOTED
or COMPLETE. Inventory and health use that durable obligation, and route it through the
source lane independently of quote slots. A captured observer-only fact fences the
entire pass against new quote, preparation or submission even if canonical observation
changes the phase. Ordinary PREPARING/PREPARED work keeps its existing path without an
extra lookup; explicit recovery dispositions retain their separate authority.

The restart regression passed all three phases with enabled and disabled pools,
preserving exact bytes, operation, reservation, zero attempts and recovery count. It
verifies actual inventory routing and forbids quote/preparation/submission or provider
polling. All 27 funding tests passed, including the concrete backend reconciliation
check with the current wallet profile removed and auto-funding disabled. Independent
review cleared the core behavior; root review cleared the lane-routing followup. C19
retains removed-pool Base observation ownership.

**Recurring review lesson.** Phase labels and independently writable totals repeatedly
drifted from exact durable obligations. Observation membership and exposure must derive
from authoritative journals; scoped reporting cannot make an unrelated malformed
liability veto a valid proof publication.

#### C10 — Decision-specific current capacity and historical reporting

Implementation provenance: archival independence `ff9db28`, resolution-only separation
`3904c78`, and wallet-local ownership decode `e559f28`. Bootstrap/status inventory uses
shared explicit ownership: malformed archival support reports incomplete inventory with
a fixed warning, current serving identity remains independent, and shared SQL failures
propagate. Startup/status parity and shared-error regressions passed.

**Current-capacity historical-dependency deletion contract:** Current balance captures
retain financial checkpoint/debit support and namespace, wallet/endpoint generations,
freshness and cache ordering. They no longer decode or require archival
funding/retirement balance anchors. Payment identity uses the enabled pool, configured
cap, existing eligible role and exact READY claim binding; historical funding support
remains validated by explicit bootstrap inventory and accounting. Same-anchor
contradictions and disputed resolution repair restrictions remain enforced. The
resolution-only contract below separates their decision-specific consumers. C11 defines
ALLOCATED eligibility separately; RETIRED wallets remain excluded from serving capacity.

**Resolution-only observation contract:** Namespace and wallet/endpoint generation
validity remain universal certificate checks. The wallet repair halt belongs to current
balance/capacity decisions, not an independently bound canonical nonce result.
Resolution-only maintenance may commit USED or EXPIRED_UNUSED under the original exact
authorization, expiry and checkpoint compatibility checks while leaving repair flags and
previous resolution support intact. Mixed balance/resolution views still fail atomically
when capacity is blocked. Historical funding restoration explicitly retains the repair
halt because it can publish roles/close accounting. This slice neither reopens admission
nor repairs disputed resolved membership; those C09/C12 obligations still prevent
indiscriminate repair-gate deletion. Validation: all 28 evidence/repair tests and
all-target Clippy passed; independent review found no actionable findings.

**Wallet-local ownership decode followup:** Ownership row-conversion failures for the
bound wallet's role/claim fields now produce the existing typed malformed support
result. Pool authority decoding and shared SQL failures retain their wider scope.
Candidate selection retains that failure for an all-malformed result while still
selecting an independent healthy payer, including cached, hinted and challenge-time
selection. No ownership checks, liabilities or payment limits are removed. All 36
selection tests passed, including both candidate orderings, malformed joined claim
generations, all-malformed diagnostics, shared failures and absence of journal side
effects. Independent commit review found no issues.

**Recurring review lesson.** Current capacity and archival accounting shared unnecessary
readers. Separating those contracts required historical-report tests so admission
simplification did not erase old read-only records. Reporting compatibility is distinct
from runtime state compatibility.

#### C11 — Initial payment capacity and atomic claim transitions

Implementation provenance: `8760345`; the readiness-publication ownership fixup
recaptures the post-publication claim in the same preparation transaction.

**Initial-capacity contract:** ALLOCATED is an owned identity with an exact enabled
slot/wallet/generation claim, not a financial minimum. Fresh payment-sized capacity and
ordinary liability/cap checks may admit it below the allocation target. First signed
admission atomically promotes it to ACTIVE and demotes the prior active wallet,
retaining its original claim, funding job, operation and reservations. An occupied claim
suppresses replacement allocation. Quote/recovery continuation follows that exact claim
through ACTIVE/RETAINED roles; role projection cannot strand accepted source work.
Independent destination readiness later detaches the original claim once, without
changing the serving role or fabricating source/provider completion. Bootstrap inventory
still requires original destination readiness. `serving_ready` continues to mean a
serving identity is available, not that a balance or payment is guaranteed.

Readiness balance observations and exact operation-correlated destination delivery are
separate facts. C18 establishes historical delivery even after spending; claims and
observations remain owned while independently proven payment service continues. It never
lowers readiness requirements, treats signed exposure as delivery, or treats USED nonce
state as an exact debit proof. Schema18 records honest ALLOCATED admission and paired
claim metadata for early ACTIVE/RETAINED use; older runtime schemas are rejected before
mutation, with no private conversion. The real restarted paid-call regression passed
($0.10 from $2 against a $10 target), including zero-signature controls for
disabled/mismatched claims and insufficient capacity. Promotion rollback/claim retention
and ACTIVE/RETAINED quote-refresh/recovery tests passed. Store validation passed 133
tests with two ignored helpers; obsolete bootstrap/schema expectations and one fixture
job-count assertion were corrected; all six bootstrap tests and the focused funding
continuation rerun passed. Version2 qualification ownership and four saved-payment
evidence tests passed while preserving version1 semantics. The deployment identity/
schema rejection regression passed. Independent review found a preparation race when
strict balance publication establishes readiness and changes the captured role/claim.
The reviewed fixup binds preparation to post-publication ownership inside the same
transaction. Both paid-call regressions passed: target-reaching ALLOCATED and
already-ACTIVE claim holders each dispatch exactly one signature with correct journal
ownership and a single claim-generation advance. All-target Clippy, formatting and diff
checks passed for the core slice.

**Recurring review lesson.** A correct freshness/readiness publication can itself change
the role or claim captured before it. Final preparation must bind the resulting
ownership inside the same transaction; caller-side validation alone cannot close this
race.

#### C12 — Conservative service during disputed-release repair

Implementation provenance: `f7ec2b0`; its separate additive lookup-index fixup covers
reopen without changing financial rows or authority.

**Conservative remaining-capacity contract:** A separate payment capture may omit
disputed historical checkpoint RPC dependencies only for exact `chain_repair_required`
state when every RESOLVED authorization is again reserved. An indexed absence check for
undisputed resolutions and checked pending/disputed exposure establish that conservative
inventory; unknown amounts or missing original resolution rows fail. The capture binds
original financial/debit tuples as an exact commit-time snapshot, retains the numeric
debit-height floor, and uses ordinary fresh canonical confirmed/latest balances and
namespace/wallet/endpoint generations. Any new undisputed resolution or changed snapshot
invalidates admission. Cache rechecks preserve purpose and original validity, never
converting payment-only proofs into historical proof. This does not release disputed
liabilities or claim individual endpoint-membership completeness.

Payment acquisition, candidate publication/selection, preparation and final journal
commit share that validator. Conservative publication updates only current wallet
balance projections; it cannot record funding readiness, publish funded roles, retire
wallets or release authorizations. Strict funding/history consumers reject the purpose
before publishing observations. Existing payment caps, ownership and atomic exposure
comparisons remain authoritative. Amount-only repair remains blocked because restored
authorization maxima do not explain contradictory balances. Validation: the rotation
suite passed 260 tests with four ignored helpers and two fixture failures; the invalid
ACTIVE/claim fixture and obsolete blanket-chain-repair expectation were corrected, and
both focused regressions passed. The real PaidClient restart test also passed: $20
balance with $1 disputed exposure refuses $20 before signing and sends exactly one
journaled $19 payment, without historical checkpoint RPC reads. Review identified the
missing lookup index on existing schema17 reopen; the reviewed fixup adds it through
existing additive lookup setup, without changing financial rows or format. Its
dropped-index reopen regression passed and independent re-review cleared the finding.
Final all-target Clippy, formatting and diff checks passed.

**T44 / T04 paid HTTP acceptance (implemented and reviewed):** The existing
remaining-capacity test now holds the production canonical repair on its nonce read
while an actual PaidClient call proceeds. With $20 current balance and a $1 disputed
release, a $20 challenge fails unsigned and a $19 payment succeeds. Repair then adopts
EXPIRED_UNUSED while retaining the concurrent $19 pending authorization and immutable
original USED proof. After Store/manager reopen, a separate $1 payment succeeds with a
distinct nonce; both new pending authorizations total $20, and no funding job is added.
The fixture uses an owned ACTIVE payer without synthetic readiness support. The focused
default-build test passed and independent review found no issues.

**T39 production-library disputed capacity (`c582bba`; fixture correction `7f834f3`):**
The normal library build uses the real PaidClient and manager. A real released $1 USED
authorization becomes disputed after a fork while the production repair nonce RPC is
held. A $5 challenge is refused unsigned; a $4 payment succeeds with admission balance
$5 and reserved exposure $1. The same manager recaptures after concurrent liability;
original proof is immutable and effective USED support has the new canonical hash. After
restart, a distinct $1 payment brings pending exposure to $5, with no extra funding jobs
or retained dispute flag. Fixture reconciliation advances bounded public passes because
an older balance conflict can precede the payment checkpoint. The focused gate and
nonce-outage/expired-unused siblings passed; the existing reorg regression now binds an
actual released payment checkpoint.

#### C13 — Current spendability, source history and retained input ownership

Implementation provenance: source expiry `de6d93e`, scoped refund observations `46489cf`
and `c172c62`, unidentifiable attribution `2ea6762`, and retained input ownership/exact
expiry correction `c3d4d5a`.

**Source-history expiry contract:** A fresh scanned height at or beyond the original
nonzero expiry plus required confirmations ends that transaction's ability to spend on
this chain. Before omitting the historical wallet-transaction lookup, decode original
retained signed bytes and require exact transaction-ID/expiry binding to the journal.
Metadata alone never establishes expired authority. Zero expiry and still-live signed
transactions retain the source reorg guard. Current SDK spendability, outgoing PREPARED
restrictions, freshness, reservation ownership and the separate two-sync
reservation-release proof remain unchanged. Original bytes, attempts, consumed expenses
and historical dispositions remain intact; bypass makes no new canonical confirmation
claim.

The local synthetic-chain regression passed pre-expiry rejection, exact burial progress
without source reappearance, metadata/bytes mismatch rejection, restored independent
spendability, reopen, height-regression protection and a subsequent prepared operation
with its reservation retained. The zero-expiry/boundary test also passed. Independent
review found no blocking findings; its requested provenance negative cases were added
and passed. The live-input gate is retained under C16's SDK invariant. Refund
attribution, source ownership and bounded traversal are specified in C13/C18 below.

**Refund-observation contract:** Known credited outpoints enter deduplication before
canonicality, amount or ownership checks. Disputes become typed operation-scoped
observations without deleting original credits, rewriting costs or preventing unrelated
SDK spendability. Positive ownership contradictions affect both original and asserted
owners. Malformed identity can be isolated only with an unambiguous original address;
otherwise the existing wider unresolved-attribution stop remains explicit. New issue
categories are visible through fixed warning categories/counts. Explicit shielding
repeats fresh refund evidence checks for its operation before proposal or reservation.
This observation result authorizes no replacement credit, liability closure or
automatic shielding. C20 supplies separate current serving and exact-input authority
for automatic shielding.

Three synthetic refund-observation tests passed. Independent review identified that
missing original outpoints and contradictory owners must also suppress fresh credit
attribution for their affected operations, not only shielding. The immediate review
fixup adds per-pass affected-owner suppression and independent-owner controls. All four
synthetic refund-observation tests and all-target/all-feature Clippy passed; independent
review cleared the fixup. The attribution contract below resolves unidentifiable
historical ownership without claiming input ownership or completed accounting.

**Unidentifiable refund attribution followup:** The consumer audit found that refund
accounting does not authorize source retries, source reservation release or current SDK
spendability. Unidentifiable historical refund identity/ownership now returns explicit
`Unidentifiable` attribution with a fixed warning, suppressing new attribution-derived
refund credits and manual attribution-dependent shielding while synchronization can
finish. C20 separately requires exact owned inputs and current serving authority for
automatic shielding; incomplete attribution grants no accounting closure. Original credits and
costs remain unchanged. Store failures and unusable owner records still propagate.
`SyncObservation::Ready` describes current SDK freshness/spendability, never completed
refund accounting; C18 closure acquires complete fresh accounting independently. All
five synthetic refund tests passed, including the actual asynchronous reconciliation
stage preserving exact old rows and suppressing an independent fresh credit. Independent
review found no blocking issues. The source-support and ownership lifecycle evidence
below supplies integrated synthetic-chain acceptance.

**Retained source ownership and exact expiry correction (C13/C18):** Fresh schema26
retains operation-specific source input disputes independently of historical scan
retention. Live confirmation loss or concrete contradictory history revokes accounting
support immediately without failing independent sync work. Burial/restart cannot release
that ownership; buried archival absence alone creates no new dispute. Staged exact
re-confirmation clears ownership only with final sync publication. Cleanup advances over
physical 64-row pages and retains only needed pass headers, invalidating abandoned
inventories before reclaiming their rows. Generic shielded preparation and submission
recheck input ownership; exact internal shielding retains independent immutable
transparent claims.

Previously confirmed operations use the existing two-sync zero-lag expiry/input proof.
Original consensus expiry zero is non-expiring. Every owned input must remain confirmed,
matching and unspent; shielding additionally checks every immutable exact outpoint. The
expiry proof records the reversed original debit while preserving signed bytes, facts,
attempts and original confirmation label. Current consumed cost is corrected once;
repeated/reopened recovery cannot recharge or duplicate it. A contradiction of an old
expiry proof retains ownership until a new exact proof. Ordinary source observation and
recovery invoke this proof without preparing, submitting or authorizing a replacement.
Existing current-policy recovery limits remain separate.

Validation: five focused ownership tests, five source-support tests, and three real
synthetic-chain lifecycle regressions passed, including ordinary recovery with
new-attempt authority disabled and restart/idempotence. Independent review found
physical-page cleanup, zero-expiry and pre-existing generic submission guard issues;
these were addressed before the gate passed. C20 activation, remaining production-build
acceptance and final repository checks subsequently passed under their own gates.

**Recurring review lesson.** Current spendability, historical accounting and retained
signed-input ownership are separate decisions. Removing blanket history vetoes was safe
only with operation-specific ownership that survives burial, reorg and restart;
canonical expiry must prove the exact original inputs, not merely enough value.

#### C14 — Complete-wallet canonical replacement

**Complete-wallet canonical replacement contract:** For an exact payment chain dispute
with all resolved liabilities reserved, acquire current nonce facts in bounded pages at
one pinned confirmed anchor. A pass uses one configured endpoint; only initial selection
may use ordinary configured failover. No original admission, readiness or resolution
hash is fetched. Original proof bytes remain in the audit/digest, not a historical truth
prerequisite. A fresh balance is acquired last, covering the pinned anchor and retained
numeric debit floor. Temporary SQLite staging is owned by the invocation and grants no
release; cancellation and restart discard it.

Atomic adoption verifies the complete resolved set, original/effective raw proofs,
authorization ownership/amounts, endpoint/wallet generations and original
financial/debit tuple. New pending liabilities remain reserved; changed resolved
membership rejects adoption. New immutable replacement rows become effective proof
authority, while original rows remain unchanged. The adopted checkpoint carries its
final endpoint provenance and the audit retains the previous anchors. An existing or
newly proven debit is reanchored without inventing a debit for an all-unused history.
Only the payment chain flag/membership clears; funding and retirement disputes remain
independent. Effective proofs and the adopted checkpoint participate in later conflict
invalidation and qualification export. Fresh schema19 rejects older runtime state before
mutation; no automatic or private state conversion is performed. Original historical
read-only reports remain valid.

Independent review corrected initial-anchor fallback, unnecessary old-support
validation, creation of a nonexistent debit, an empty-inventory/retained-anchor case and
missing indexed conflict checks for the adopted balance support after restart. Malformed
historical endpoint provenance alone now restores conservative liabilities and requests
canonical repair; it does not invent an unknown amount. Same-hash amount conflicts still
reject the affected observation without adding a blanket historical stop. Four real RPC
tests and all 42 integrated evidence tests passed with localhost permission, including
the production repair owner after restart, both outcome reversals, paging, duplicate/new
conflicting provenance, rollback and immutable audit checks. Another 108 store tests
passed (two ignored subprocess helpers), including historical reports and qualification
exports. The deployment identity/schema test and all-target/all-feature Clippy passed.
The deployment fixture now disables irrelevant external pricing probes; its first
external-dependent run was stopped and the local-only rerun passed after firewall
permission. Formatting and diff checks passed. The older exact-revalidation path for
unmatched legacy/manual flags remains; selected canonical repair never falls back to it
after failure or an unresolved nonce. C15 supplies fairness; C12/C14 production
acceptance is recorded below.

**Recurring review lesson.** Replacement evidence needed complete consumer coverage:
endpoint provenance, effective proof pointers, debit anchors and qualification exports
all participate in invalidation. Original immutable audit rows cannot silently become
current proof authority.

#### C15 — Bounded repair, fair shared resources and retained accounting

Implementation provenance: independent repair owner `07b5cad`; bounded publication
`55d5b3b`; cleanup ownership `0f36e2f`; historical report fixup `110f2e0`; sole bounded
repair path `525069f`; process-wide RPC fairness `6bdc853`.

**Separate repair-owner contract:** A residual canonical repair owns one dedicated pass
per pool, independently of the four current-balance readers and four
authorization-history readers. The pass retains its staging and cursor while useful work
continues. Existing process-wide query/RPC bounds still apply; the extra owner bound
protects retained repair staging, not spending capacity. Physical wallet pages advance
even through ineligible rows. Failure backoff is per wallet; shutdown cancels unsigned
reads while accepted serialized store jobs and staging cleanup retain ownership.

Current maintenance uses C12's all-liabilities-reserved capacity path during a known
payment dispute. It cannot release liabilities, publish readiness or retire a wallet.
This also avoids repeatedly invalidating a progressing repair by rechecking its
already-disputed old checkpoint. A one-pass caller receives the final repair disposition
rather than an intermediate balance deferral.

The separate owner and capacity path passed independent review. All 15 maintenance tests
passed, followed by two new one-pass success/unresolved-nonce tests and the
retired/healthy physical-page regression. The final one-pass rerun also passed. Tests
retain a repair beyond the normal cadence, verify current capacity prefetch and
independent history progress, then require adoption; shutdown/restart retains
liabilities and cleans staging. The bounded adoption contract below completes G12 by
moving paged validation outside the short activation transaction with complete revision
coverage; no validation is weakened and no work deadline replaces it.

**Bounded adoption followup:** Publish bounded replacement-proof pages into an inactive
set, then activate an exact wallet-bound set with a short atomic pointer update.
Original proofs remain audit; inactive pages cannot affect effective resolution, balance
support, endpoint membership or qualification. Replace bulk disputed-row clearing with
one wallet payment-resolution dispute state, separate from stronger malformed-evidence
stops. Use complete database mutation-revision coverage for resolved membership and
pending liabilities. Historical RPC work survives new pending admissions; pending
validation repeats separately, while changed resolved membership invalidates the
historical set. Checked liability validation and original-proof hashing are paged before
fresh balance acquisition. Activation validates owner, revisions, capture, endpoint,
debit, completeness and original freshness without rescanning/copying history. The
existing same-hash journal-support check also requires paged physical validation: its
indexed LIMIT does not bound successful scans. Recheck its mutation revision and balance
freshness at activation; expiry reacquires balance/support validation without discarding
historical proof pages. Cleanup is paged; cancelled or restarted inactive work never
releases liability. Follow the single supported schema policy, without adding runtime
migration machinery.

Independent design audit identified every active-proof consumer and the existing
unbounded acquisition-time exposure scan; both changed with publication. The acceptance
gates cover interleaved store work, membership mutation, new pending admission, stale
final balance, cancellation/reopen, later repair supersession and immutable audit. The
implemented adoption contract and qualification limits follow.

**Bounded canonical adoption:** Introduced in schema21 and retained in final schema26,
`wallets.payment_resolutions_disputed` supplies one wallet-wide dispute state, alongside
separate resolved-inventory and liability/support revisions. The old per-attempt
`resolution_disputed` decoder is limited to historical saved-schema exposure reporting. SQL
triggers cover authoritative membership/proof and support changes with overflow
rejection. Replacement proofs publish in physical pages into inactive sets; effective
resolutions, endpoint membership and balance support exclude those sets until the short
activation transaction commits. Original proofs remain immutable. Pending admission and
final-balance expiry repeat only current liability/support validation, retaining
completed historical RPC work. Same-hash support validation and abandoned-set deletion
also yield between pages. No full-history scan, bulk copy or per-attempt dispute update
remains in canonical activation. The single-repair-path contract below removes the older
exact-revalidation fallback.

Independent financial review and root review cleared the implementation. All 17
replacement tests passed, including inactive visibility, concurrent membership and
pending changes, overflow rollback, stale final evidence, restart and bounded cleanup.
The store suite passed 178 tests with three ignored helpers and one obsolete fixture
reset; the corrected fixture passed its focused rerun. All-target/all-feature Clippy
passed. Review identified two followups: transient cleanup must retain its owner and
retry only idempotent deletion on the existing five-second cadence; archived schema19/20
reports must retain their read-only row-dispute decoder. Both fixes and their new
regressions passed review and focused tests; no runtime migration or private state
conversion is introduced.

**Single repair path followup:** Removed the old exact-revalidation fallback and its
unused historical-resolution verifier. Supported production invalidation marks payment
resolutions disputed atomically; canonical replacement is now the sole repair path.
Undisputed history with a chain block, or an empty resolution journal retaining
financial/debit anchors, fails before RPC. A genuinely empty journal without such
anchors uses the existing bounded owner, pending/support validation and fresh balance
activation without inventing a financial checkpoint or releasing pending liability.
Ordinary cached-balance invalidation with no historical resolution creates no financial
block and refreshes normally. Historical balance accounting and read-only audit records
remain available. Root review found no further correctness issue. All six repair tests
and 17 replacement tests passed. This removes the previously documented unbounded
fallback; Cross-layer production acceptance is recorded with C12/C14 below.

**Cross-pool resource fairness fixup:** Per-group repair lanes alone did not prevent
enough historical queries from occupying every process-wide RPC socket. Historical
query/caller/socket subsets now leave one current evidence slot at each existing
boundary: 63 of 64 queries, 127 of 128 callers and 31 of 32 sockets. Aggregate bounds
are unchanged. Classification follows isolated endpoint and fallback clones; waiting
historical calls cannot take the reserved current socket. Query and caller admission
stay bounded, and cancellation releases both subset and aggregate ownership without
abandoning accepted financial work.

Independent review is clear. The Base filter passed 41 tests with one ignored helper;
resource accounting, 36-pool historical saturation, four-pool aggregate saturation and
actual managed-payment saturation tests passed. With 31 historical RPCs held, every
pool's current balance reads and independent Store work progressed. Original liabilities
and immutable proofs survive shutdown. A production repair/reopen test also passed after
the original RPC was stopped and an explicitly configured alternate was selected,
retaining original endpoint provenance and funding disputes (T06).

**T33 retained accounting ownership (`8abcfcc`):** A one-shot test gate pauses the
actual source-accounting continuation after its SDK wallet read guard drops and before
Store staging. Exact Store reads and a real Base RPC complete while it is paused; SDK
`try_write` succeeds. A refund-address command enters the bounded actor queue and waits.
Cancelling the sync waiter does not cancel the owned continuation: release lets
accounting finish and the queued SDK command complete, preserving exact gross debit,
original bytes, attempts and reservations. The corrected fixture drains its owned HTTP
server and drops dependent Store handles before awaiting actor close; the passing local
lifecycle gate took 22.55s. This proves retention across a deliberately held accounting
boundary, not a general latency bound.

**Recurring review lesson.** Separate tasks and per-pool lanes did not guarantee
independence when every path shared serialized Store jobs or process-wide RPC sockets.
Physical paging, short revision-checked activation and reserved current-evidence
capacity were necessary at the actual shared bottlenecks.

#### C16 — Preparation provenance and safe cleanup

Implementation provenance: calculate-only ownership `d4d5407`.

**T12 proof-completion/expiry acceptance (`f188ecb`):** The actual SDK calculation and
verified Orchard proof retain an immutable accepted deadline fixed before proof. The
test supplies simulated late serialized-command times at the 300-second margin, at
expiry and after expiry, with both retry flags. It does not measure a wall-clock proving
overrun. No broadcast capability or attempt is minted; exact facts, bytes and cost
reservation survive reopen. The serialized authority checks add no network calls after
reopen; read-only observations retain their separate permission. The focused
actual-proof gate passed (22.23s).

**Preparation provenance contract:** Before SDK proposal mutation, atomically bind the
exact operation/job and starting committed snapshot revision to version 1 of the
calculate-only contract. Its private owner capability must accompany reservation and
atomic publication. This contract cannot submit or expose signed bytes before the
existing snapshot/outgoing/facts commit. Generic unstarted/closure paths must respect
accepted provenance even if a derived phase says QUOTED or ALLOCATED. Unknown contracts
and legacy PREPARING records do not acquire cleanup authority.

After a synchronous preparation error, first take the SDK offline and drop it without
saving. Then a serialized cleanup job may close only the exact owner with the unchanged
starting snapshot, no outgoing bytes or unexplained durable effects, and only its own
unconsumed reservation. Archive managed closure through existing `funding_recovery`; a
terminal provenance marker also fences unmanaged operations. An already accepted
outgoing commit wins over a missing acknowledgement and keeps its exact bytes and
liability. Closure cannot restore SDK health. C17 reconstructs on the existing shared
Store; process death retains open provenance for that explicit ownership handoff.

Retain the global pending-outgoing preparation gate: the pinned SDK can clear
input-spend markers when its transaction status becomes Failed while the application
still owns PREPARED bytes until independent expiry recovery. Relaxing the gate requires
exact exclusion of every application-live input, including after restart and reorg,
rather than trusting SDK spendability alone. Synthetic multi-note evidence confirms this
invariant. Two real-SDK preparation tests, nine provenance and crash tests, and 26
funding/classifier regressions passed. The subprocess helper exercised three
process-death boundaries. Independent review cleared the qualification-retirement and
generic-abandonment guards. All-target/all-feature Clippy, formatting and diff checks
passed. Fresh schema20 adds the provenance boundary; there is no automatic state
conversion.

**Recurring review lesson.** Absence of outgoing bytes is weaker than proof that
mutation never began. Lost acknowledgements and synchronous SDK failure require exact
versioned provenance and retired-owner cleanup; accepted commits win over missing
replies. The pending-input gate remains justified by observed SDK behavior, not generic
caution.

#### C17 — SDK retirement, reconstruction and live Store continuity

Implementation provenance: live reconstruction `929b6ca` and snapshot-read contention
continuation `3639bbb`.

**Live SDK ownership recovery:** Treasury retains and drains the sync worker handle
across cancelled waiters before retiring the SDK and fencing accepted store jobs.
Reconstruction reads the committed snapshot, checks its revision before publication, and
preserves the same shared store and worker while Base consumers retain handles. Failed
commands are not replayed; queued commands retain normal admission checks. Only typed
snapshot reconstruction failures retry at the existing maintenance cadence; shared
SQL/ownership failures remain scoped stops.

The retired-owner capability binds the store namespace. Both live reconstruction and
initial exclusive reopen close only eligible calculate-only preparation markers,
traversing at most 64 physical rows per store job. Unknown contracts, changed revisions,
committed bytes and unexplained effects retain their obligations. Nine ownership tests
and two real SDK preparation/proving tests passed, including cancelled drain, accepted
late commits, lost acknowledgement, refund mutation, retained Base handles, shutdown,
reopen and physical paging. Independent review cleared the implementation. This
reconstructs poisoned SDK state without exiting its financial owner.

**Unresolved original C17 follow-up:** Actual deployment task-exit cancellation and
funding-scheduler restart handling were explicitly left separate in the original
implementation journal. Current `RunningDeployment::serve` still shares the treasury/
funding stop token and reports restart required after it is cancelled
([task ownership](../../src/deployment.rs#L1751),
[stop handling](../../src/deployment.rs#L1838)). Listeners and Base reconciliation
remain available. The recorded in-owner reconstruction and read-retry tests do not
establish automatic replacement after actual task termination. This is an original
stage 6 acceptance gap, not a newly proposed H follow-up or authority to blindly
restart corrupted or ambiguously owned work. Close it by auditing actual exits and
justifying each retained restart boundary, or implementing only proven eligible
handoff with accepted-work ownership and shutdown-race acceptance. Containment
after injected cancellation alone does not prove recovery; no new supervisor is
implied by this gap.

**Reconstruction read-contention fixup:** Only Busy/Locked errors from the two read-only
snapshot/revision checks receive a retryable reconstruction marker. The retired SDK,
shared store and queued commands retain ownership under the existing maintenance
cadence. Retirement fences and preparation handoff mutations remain outside this retry
classification. Root review and the focused regression passed for both boundaries, with
corruption and untagged SQLite errors remaining fatal; successful retry preserves exact
snapshot bytes and revision. All-target, all-feature Clippy passed before this narrow
followup.

**Recurring review lesson.** Waiter cancellation, task failure, SDK lifetime and Store
lifetime are different ownership boundaries. Reconstruction can reuse the live shared
Store only after the old SDK worker and accepted jobs are fenced; only specifically
audited read contention is retryable.

#### C18 — Canonical source, destination and finite return closure

Implementation provenance: external closure `33689ec` with fixture fixup `9e7396f`;
provider observations `891f848`; canonical transfer verification `63354df`; immutable
delivery support `4585782`; canonical source support `05b8346`; return closure `f8c8194`
with refresh fixup `bdc90b5`; production worker acceptance `2d434a8`. Source-input
ownership and correction is cross-referenced in C13 (`c3d4d5a`).

**External-satisfaction closure:** An exact unstarted source closure or already proved
signed expiry, together with undisputed independent destination readiness, may finish
EXTERNAL/RECOVERED despite historical provider status flags. This does not claim
provider delivery. Keep every status flag and original operation, and prevent later
status ingestion from turning this proven disposition into a provider-review phase. The
unsigned closure proof also excludes orphan source-operation or expiry records, not just
bytes and reservations. Open/unknown preparation provenance, unexplained source effects
and disputed readiness still prohibit closure. SUCCESS requires the exact
operation-correlated canonical delivery and source/return evidence specified below. This
narrow contract and implementation passed independent review. All 21 completion tests
passed, including reopen, retained status flags, mismatched or missing archives,
malformed accepted terms, unexplained source effects and disputed readiness negatives.
External readiness validates exact archived operation identity without treating retained
accepted terms as a new delivery claim.

**Provider-proof assessment:** The official [1Click status
contract](https://docs.near-intents.org/api-reference/oneclick/check-swap-execution-status)
was inspected on 2026-10-16. It correlates status by the original deposit address and
echoed quote, and exposes origin/destination transaction hashes and actual swap/refund
amounts. The inspected contract does not establish that a later terminal label
supersedes an earlier contradictory label. Do not invent that rule. The pre-change
adapter discarded transaction correlations and destination readiness used a balance
checkpoint. Neither a balance change nor a provider label alone proves exact delivery
after funds have already been spent.

The implemented contracts below retain validated original-operation settlement
correlations and independently acquire canonical transfer evidence for the accepted Base
USDC recipient/amount. Existing source accounting and any return obligations remain
separate. Persisted provider observations remain audit facts; closure may ignore a
historical status veto only after every relevant obligation is actually proved
discharged. Missing hashes, ambiguous attribution and unavailable proofs remain scoped
incomplete outcomes, without replay or replacement authority.

**Provider observation prerequisite:** The adapter now exposes a private-constructor,
exact-quote-digest-bound status observation with explicitly unverified
origin/destination transaction candidates and optional atomic amounts. Missing values
remain unknown. Malformed known fields, duplicate/invalid hashes or financial quote-echo
mismatch suppress the complete candidate set without losing the ordinary provider
status. Original deposit/request mismatch still fails. No explorer URLs, provider prose
or tracing correlation IDs are retained. Existing status polling delegates to this
parser and retains its behavior; persistence, canonical verification and closure use the
distinct contracts below. Root and independent review cleared the slice; all 17
near-adapter tests passed with one intentional child helper ignored. The official live
`/v0/status` documentation linked above supplied field shapes; it declares no standalone
schema revision.

**Canonical transfer prerequisite:** An opaque expectation joins the exact saved quote
digest, original source-transaction hint, recipient and accepted amount to one
destination candidate. The Base verifier checks successful receipt, pinned USDC
contract, unique exact recipient transfer/log index, configured confirmations and
canonical block rechecks. The proof carries original capture, endpoint generation and
the pre-request monotonic freshness lease. Provider amounts can contradict a candidate
but cannot prove delivery. Wrong chain is typed; only a changed anchor already bound by
the capture becomes a durable evidence conflict. Unknown candidate changes remain local
verification failures. The payment receipt's payer/nonce validator is unchanged.

Root and independent review cleared this destination-proof-only slice. Three focused
tests passed, including a receipt mutation matrix, 13 local RPC scenarios and original
quote/source/amount binding. Durable publication below CASes wallet/address and original
operation/source/quote, enforces unique event ownership, and discharges source and
return accounting separately. Multiple source/destination candidates remain
unsupported/incomplete. This proof discharges destination delivery only, with no
independent closure authority.

**Canonical delivery publication:** Retain validated original quote/deposit and
source-transaction correlation from the provider observation independently of its status
label. That mapping supplies attribution; a canonical transfer alone cannot identify a
swap. Validate a unique Base USDC transfer event for the exact accepted recipient/output
amount, with successful canonical receipt/header, configured confirmations and opaque
runtime freshness/generation binding. Preserve the stricter payer/nonce validation in
the existing payment receipt path. Do not invent a bridge sender/nonce contract.
Missing, multiple or split candidates remain incomplete until separately supported.

Publish immutable delivery support under exact operation/quote/source CAS and unique
chain/token/transaction/log identity, preventing one event from closing two operations.
No present-balance threshold is needed to prove historical delivery. Provider outage
after retained correlation need not block canonical verification; conflicting
correlations do not supersede one another. Delivery discharges only the destination
obligation. Complete financial closure separately requires original source
debit/reservation/outgoing accounting and explicit return obligations; status flags
remain audit and never erase ambiguous refunds. Typed endpoint/hash conflicts reopen
only dependent support; unsigned outages grant no replay or replacement.

Independent audit identified the status adapter/scheduler's discarded correlation, the
payment-only receipt validator, and destination/completion consumers as the
implementation seams. Required tests include zero-current-balance delivery, wrong
correlation/amount/token/recipient, receipt/reorg negatives, unique-event ownership,
operation rollover, retained correlation through outage, and unresolved source or return
obligations despite valid destination proof.

Schema22 retains one active and one queued encrypted hint per operation, permanent
unique event ownership, immutable proof versions and one current proof pointer.
Canonical verification uses a fifth independent scheduler lane (four active reads);
blocked delivery reads do not block source, status or credit observations. A current
event takes priority over unrelated hints, while a chain-disputed event fairly
alternates with a corrected candidate after completed reads. Cancellation retains
ownership. Old proofs never become fallback authority after the current proof is
disputed. Publication rechecks freshness and original source/quote bindings.

Positive or malformed return claims survive missing candidates and subsequent status
labels. A late claim withdraws effective completion while preserving the historical
outcome and gross debit. Exact source-closed external/recovered cases remain exempt by
proof. Historical schema reports preserve their saved diagnostics. Eleven delivery-store
tests and 22 funding tests passed. The scheduler gate passed 20 tests, then its
corrected terminal-status restart fixture passed separately; independent review is
clear. Canonical source/return reconciliation below completes the financial proof; it
does not authorize automatic refund replacement.

**Canonical source support:** Schema23 adds bounded inactive source observations,
activated with the final wallet snapshot only after exact inventory and fresh
observation checks. Saved CONFIRMED labels and immutable gross debit entries alone no
longer grant source accounting support. Known canonical loss revokes support
immediately; completed jobs remain observable with `source_accounting_pending`, without
new submission. The existing sync reuse path shares current unavailable observations
across jobs.

A sealed historical pass retains its original scanned block anchor across an expired
observation window. Incremental catch-up rechecks that hash and unchanged inventory
before binding publication to the fresh snapshot; original balance freshness is never
renewed. Restart cannot adopt an inactive pass.

Independent review passed. Focused validation: source Store5, completed-job source
scheduler1, accounting1, canonical-delivery/source-loss1, and real synthetic-chain
lifecycle1. The latter checks one historical staging pass across expiry/catch-up, eight
unavailable observations without another scan, canonical reorg/reappearance, and
unchanged gross debit, original bytes and submission attempts. Earlier completion20 and
delivery10 siblings passed before localized fixture corrections. C22 documents the
separately implemented restoration declaration. Canonical return closure follows below;
C13 supplies operation-scoped live orphan recovery. The C16 generic live-input guard
retains its separate SDK invariant.

**SDK enumeration qualification limit.** Store inventory, staging, publication and cleanup
use physical pages, but SDK output accounting retains one linear traversal of the
resident output map. The local progress gates do not establish a bounded SDK-internal
traversal or a live-provider settlement contract.

**Canonical return closure:** Schema25 binds finite return accounting to the same
bounded historical source pass. Production loads original return bindings and immutable
credits in pages, publishes one discovered output per Store job, and carries exact
credited totals under the shared inventory revision. Duplicate provider observations do
not invalidate useful history. Canonical original outputs remain accounting evidence
after shielding or spending; pending positive outputs are scoped incomplete, and
invalidated never-credited outputs do not create permanent obligations.

SUCCESS now requires exact canonical destination delivery and current source/return
proofs. Unknown amounts are not zero. Explicit zero or a positive amount must bind to
the original source, accepted quote and delivered transaction; positive amounts need
complete canonical owned-output coverage. Historical provider terminal labels remain
audit rather than a permanent veto after all proofs discharge the obligations. Changed
or malformed claims remain incomplete. New outputs or canonical contradictions reopen
accounting without erasing historical completion or duplicating return credit. Explicit
zero describes the current finite claim, never absence of future returns.

Independent review covered financial correlation, revision coverage, bounded publication
and historical readers. Focused gates: delivery/return13, completion21, source Store5
and synthetic SDK refund6 passed; the latter includes spent origins, ambiguity
isolation, pending positives and the failed-output negative control. The follow-up
storage-error propagation regression also passed (1 test). Production funding-worker
acceptance follows below. Full returned-principal accounting does not authorize a
replacement attempt: refunded retry still requires review under the explicit repository
instructions. C13 records the separate schema26 operation-scoped live orphan recovery
contract.

**T48 production worker acceptance (`2d434a8`; return-refresh fixup `bdc90b5`):** The
real funding worker, treasury actor and synthetic chains now exercise persisted FAILED,
REFUNDED and INCOMPLETE observations followed by exactly correlated delivery hints, a
missing-receipt control, and provider outage. Completion occurs only after canonical
destination, original source and finite return proofs are present. Original bytes,
operation facts, gross debit and one submission remain unchanged; no SUCCESS label or
replacement attempt supplies authority.

This acceptance exposed a missing refresh dependency: semantic return-inventory changes
revoked return proof but did not schedule its canonical reobservation. Fixup `bdc90b5`
routes confirmed sources with pending returns through the existing source lane. Reuse
requires the original pass's inventory revision; an unchanged observed issue remains
reusable and does not trigger repeated historical scans.

Independent review and focused validation passed: T48 worker (1), source support (6),
and existing concurrent bootstrap/rotation worker (1). The latter retains actual
lookup-progress and unchanged-snapshot assertions while allowing existing poll jitter.
These local fixtures do not establish an external provider contract or prove absence of
future returns. Refund replacement still requires review.

**Recurring review lesson.** SUCCESS/FAILED/REFUNDED labels repeatedly stood in for
discharging financial obligations. Exact correlated source, destination and finite
return proofs are separate authorities. Invalidating return support without scheduling
its owner caused a real liveness defect; unchanged unresolved evidence must also remain
reusable to avoid repeat scans. Worker fixture upgrades supply canonical proof rather
than weakening completion to match status prose.

#### C19 — Observation after removal, disablement and retirement

Implementation provenance: durable observation ownership `8bdcc3e`; retired liability
repair `8b7d78b`.

**Durable Base observation ownership:** Ordinary serving retains policy-free
reconciliation owners for durable pool identities, including removed and disabled
profiles. Existing explicit treasury/funding configuration and state are required;
scoped inspection and qualification do not inherit unrelated journals. Observation
owners cannot sign, allocate or fund, and retired wallets remain excluded from payment
capacity. Shutdown drops observer handles before joining the shared store. Independent
review cleared this slice; all 55 deployment tests and 17 maintenance tests passed. C11
fixture corrections also passed independent review, preserving captured-claim
obstruction checks while allowing initial-capacity serving. The reviewed followup
removes the retired-role exclusion only for already-disputed payment liabilities.
Canonical replacement releases exact liability while preserving retired role, unassigned
claims and funding readiness; fresh positive balances still cannot admit that payer. The
production repair regression and physical paging past a healthy retired prefix passed,
along with existing retired-wallet regressions.

#### C20 — Owned automatic shielding under serving authority

Implementation provenance: inactive substrate `4c2f79d`, physical inventory fixup
`3f5df1f`, serving activation `9863d95`, and actual ciphertext ownership fixup
`7e82183`. These are distinct calculation, authority, input-proof and scheduling
boundaries, not additional spending ceilings.

**Shielding implementation contract:** Automatic shielding uses the serialized treasury
owner and existing maintenance cadence. Bind each accepted calculation to C16
preparation provenance, one original transparent address, exact selected
outpoints/amounts, snapshot revision and actual fee cap. Commit immutable input
correlation with the outgoing bytes, transaction facts, snapshot and fee-only
reservation. Principal is internal wallet movement; never charge it again or require
shielded funds to pay a transparent-input fee. Use checked amounts, exact raw-input
equality, no transparent outputs, and verified internal shielded output/value balance.
Preserve the demonstrated pending-outgoing SDK guard. Precommit cleanup requires drained
owner provenance; uncertain committed work retains its exact bytes for observation and explicitly
authorized recovery, never fresh selection. Canonical expiry may release only the original operation under existing
recovery proof rules.

New automatic work requires enabled auto-funding, effective configured profile policy
and the existing refund-shielding fee safeguard. Removed/disabled profiles wait;
existing signed work still receives observation. Qualification permits cannot be
borrowed from source funding: the current permit contract does not authorize a new
shielding operation. Deny it before mutation unless explicit run authority is
implemented separately. No new cumulative budget or whole-work deadline is added.

Current confirmed, unspent SDK-owned transparent inputs may be shielded despite partial
or unresolved return attribution when exact input ownership, script and reservation
exclusion are proved. Keep one original address per transaction. An attribution warning
alone is not an input-ownership veto; unknown selected inputs or reservations are.
Preserve unresolved source/refund accounting and immutable outpoint-to-shielding
correlation across reorg, spending and restart; shielding is never provider settlement,
refund credit or new funding authority.

Acceptance requires restored confirmed spendability from zero shielded capital,
partial/unattributed owned inputs with unresolved accounting preserved, exact fee and
single-address bounds, disabled-policy resumption, qualification denial before mutation,
cancellation/lost acknowledgement/restart with no duplicate selection, and canonical
confirmation/expiry with fee consumed once. The owned calculation, submission,
maintenance and activation contracts below implement the durable input correlation and
automatic command path.

Independent C20 review identified additional required boundaries: attach automatic work
only to explicitly authorized serving/funding maintenance, never ordinary wallet sync;
observe standalone shielding operation IDs independently of funding-job phase. Reject
all qualification permits that lack explicit shielding authority before even creating a
preparation marker, including positive source-funding permits. Internal shielding
submission follows transaction expiry, not the existing invented one-day external-style
deadline; source funding retains its accepted quote deadline/margin. Expiry must prove
the exact selected input set, not merely inputs sufficient to cover the fee-only
accounting amount. A confirmed shielding transaction reorg must not free its original
inputs for reselection while its old signed bytes could still be valid: immutable input
ownership survives outgoing RESOLVED until canonical closure or expiry proves
reassignment safe. These are implementation requirements, not new spending ceilings or
reasons to require completed provider attribution.

**Owned shielding calculation:** Schema22 stores immutable operation/policy/address and
exact outpoint/value/script bindings beside the existing outgoing journal. Manual
shielding now uses C16 calculate-only ownership and atomic publication of these
bindings, signed bytes, facts, snapshot and fee-only reservation. It checks SDK
ownership, confirmation, unspent status, one original address, exact raw inputs, no
transparent outputs and owned shielded output/value balance. Original transparent
principal is not charged again. Input claims survive confirmation and reopen; only
canonical expiry can release them. Qualification permits, including positive
source-funding permits, cannot authorize shielding. Failure retires the SDK before
proven owner cleanup; accepted commits remain intact.

Independent review is clear. Four store tests passed, and a real synthetic-chain proving
test passed both injected commit rollback and successful fee-only publication with
exact-byte reopen. Manual calculation retains its attribution check. The expiry-only,
owned-input maintenance and activation contracts below establish the automatic T30 path.

**Internal submission constraint:** Transaction facts now distinguish an external
deadline from no external deadline. Historical numeric deadlines remain readable. Only
an opaque constraint reconstructed from the immutable shielding header, exact inputs,
decrypted original address, signed bytes and fee permits expiry-only submission. A
missing deadline or zero amount alone grants no such authority. New manual shielding
omits the invented one-day timer. Source funding still requires its accepted deadline
and shared first-submission margin. The Store, gRPC preflight and final local check
share that calculation; consensus block expiry continues to reject every expired
transaction. Automatic activation below uses this same constraint.

Six Store tests passed, and the real synthetic SDK proof/reopen test passed with
expiry-only submission, legacy header-bound numeric facts and block-expiry rejection.
The fixture now awaits production owner close before reopening. Independent review is
clear; no live-funded work was run.

**Serving capability:** After every listener bind succeeds, ordinary enabled managed
serving may capture the current enabled profile/pool fee policy in an opaque
Store-owner-bound capability. The command owner retains it across SDK reconstruction;
generic sync, bootstrap, administration and reopened state cannot create or inherit it.
Positive and zero qualification limits, paused policy and declared restoration prevent
capture or new preparation. The accepted policy source determines the immutable
automatic flag; callers supply no flag. Current policy is checked before the C16 marker,
fee reservation and new publication, while an exact already-accepted commit remains
readable after policy changes. Invalid optional shielding policy retains the scoped
listener startup path.

Automatic scheduling was activated only after the canonical input reorg/expiry
accounting gate described under C13/C18 was complete. Focused tests cover
removed/disabled profiles, policy changes, namespace mismatch after reopen,
qualification/restoration denial and unchanged pre-mutation journals. The real SDK
regression repeats an accepted exact commit after policy is paused to qualify
lost-acknowledgement handling. Ten focused Store cases and the real SDK shielding
regression passed; independent review is clear. This prerequisite was tested before
scheduling activation; no live-funded qualification was run.

**Bounded maintenance substrate:** Bounded original journal pages retain every shielding
operation for observation and current/historical refund addresses independent of
funding-job completion or provider attribution. An opaque address candidate rechecks its
exact journal binding and current serving policy before C16 ownership. The staged
preparation helper selects confirmed, unspent SDK inputs, excludes retained input
claims, and shares the existing proposal minimum before any mutation; it uses the same
calculate-only atomic publication. The command owner retains the address cursor across
SDK reconstruction, with actor scheduling and automatic submission enabled by the
activation gate below. First-submit candidate inspection excludes manual bytes and all
attempted bytes; actual intent uses the atomic serving-policy gate specified under
activation. Validation:13 Store tests, both real SDK manual/automatic proving tests, and
repeated dust-only passes with no preparation, reservation, snapshot mutation or SDK
retirement. Activation followed the separately reviewed scoped canonical orphan/expiry
recovery gate.

**Inventory review fixup:** Independent review found that the combined current/archive
UNION could scan and sort complete history before its LIMIT. Separate indexed keyset
queries now bound physical rows before filtering absent historical addresses. The
pending-slot query uses the existing unique `one_outgoing` partial index. Query-plan
assertions and a sparse 130-row history regression pass with the 14-test shielding Store
suite; the activation contract below uses these bounds.

**Automatic maintenance activation:** After a successful persisted scan, the serialized
serving actor observes bounded pages of original shielding operations before selecting
exact confirmed owned transparent inputs. Only captured serving authority can prepare
automatic bytes; first submission atomically rechecks the current enabled policy, actual
fee cap, automatic header and zero attempts. Manual bytes and already-attempted bytes
never receive an automatic submission. Disabled profiles and absent capability retain
observation/recovery of original IDs. Typed per-operation indexer outages preserve
independent history progress without advancing a prefix from the prioritized pending
slot; storage/ownership failures and retired SDK capability remain fatal. Actual
local-chain gates pass for fresh actor preparation/submission/confirmation, exact
fee-only accounting and spendable increase, policy change before intent, restart without
replay, and disabled-profile observation (two tests, 53.57s). The injected
pending-outage/prefix regression also passes. A separate review fixup validates original
UFVK ownership directly from exact Orchard/Ironwood ciphertext, including
consensus-promoted change, with wrong-key/wrong-value controls passing in the actual
proving regression (22.97s). These are unfunded local fixtures, not live qualification.
Final repository validation passed; this does not add live qualification.

**Exact shielding expiry inputs:** The ordinary two-sync expiry path additionally
reloads the validated immutable shielding binding and checks every journaled outpoint
against the current SDK wallet. Value, original address, locking script, confirmation
depth and unspent status must all match. A subset covering the fee-only reservation is
insufficient. The original snapshot proof, zero-lag scans, buried block expiry, indexer
observation and final snapshot revision check remain required. The separate
source-support contract under C13/C18 supplies the release correction for an orphaned
previously confirmed transaction; this check alone cannot release it.

The focused synthetic-wallet test passed exact success and missing, changed, duplicate,
insufficiently confirmed and spent-input controls, including a remaining input worth
more than the fee. The real SDK shielding proof/reopen test also passed after the shared
immutable-binding loader refactor. Independent review is clear.

**Recurring review lesson.** The exact selected input set and owned shielded outputs
must be proved from actual serialized transactions, not assumptions about SDK note
population or fee-only value coverage. Scheduling also needed physical SQL bounds and
priority-prefix independence; activation tests exercise actual actor preparation and
first intent, with no automatic retry of attempted bytes.

#### C21 — Separate clocks and financial authority

Implementation provenance: Base lease `7c4f0ae`, financial versus scheduler time
`06af797`, treasury lease `d8b4dbc`, and canonical-time acceptance `8ffb521`.

**Base observation clock contract:** Measure the existing validity window from a clock
capture before the latest-header request. The elapsed clock includes suspend on macOS
and Linux; no new timeout or durable clock halt is introduced. Wall-clock expiry,
elapsed expiry and elapsed regression retire that observation's shared lease. Clones,
rechecks and equal-observation cache merges cannot renew it. Equal merges retire
detached incoming handles so eviction cannot restore their discarded authority;
supersession alone must not expire an independent retained lease. Unsupported
elapsed-clock platforms fail the affected acquisition explicitly.

Payment selection, admission, repair activation and retirement consume the same lease.
Admission repeats the check after journal writes immediately before commit; failure
rolls back the payment and reservation. A qualified payment retains its existing
nonretryable classification after external qualification recording. Historical
resolution evidence does not acquire a balance-age prerequisite. Independent review
identified detached-clone/cache-eviction and final-commit gaps; their regression fixes
passed independent re-review. Final focused validation: seven freshness-filter tests,
all 42 evidence tests and all 32 selection tests passed; the earlier 38 Base tests also
passed. The final selection run waited in the macOS loader before the harness started,
then completed normally; no timeout or security workaround was added.
All-target/all-feature Clippy passed with warnings denied. Funding quote deadlines,
treasury freshness and persisted polling have distinct C21 contracts and acceptance
below.

**Funding deadline clock followup:** Polling/backoff and health metadata keep their
elapsed scheduling clock. Quote-window decisions after awaited readiness and operation
reads now use actual wall time. The supplied `tick` epoch controls scheduling only; it
cannot grant or revoke financial authority. Production treasury preparation and
submission retain their own checks at the mutation boundary. The existing 300-second
submission margin is unchanged. Independent review found no issues. The exact-boundary
unit test, all 24 funding integration tests and seven concrete-backend unit tests
passed. Integrations preserve prepared bytes and attempts after actual expiry, and
artificial future scheduling ticks cannot expire a valid quote. All-target/all-feature
Clippy also passed with these changes present.

**Treasury observation lease:** Capture the suspend-inclusive elapsed origin before the
starting tip RPC and carry the same lease through scan, teardown and final snapshot
publication. Only the exact committed observation/revision receives process-local
freshness. Plain persisted JSON, reopen and phase restoration cannot mint it; snapshot
and phase mutations revoke even held preflight clones. Saved `sync_fresh` remains an
explicitly labeled wall-age diagnostic. Ordinary three-block lag and exact-tip expiry
checks remain unchanged. A final checkpoint that outlives the original window retains
the scan and continues incremental catch-up before reporting successful fresh
completion.

Validation passed: eight store freshness tests, one expiry regression, 18 rotation
integration tests, 24 funding tests and 18 treasury-sync tests (one public-network test
ignored). The actual synthetic-chain final-checkpoint expiry regression also passed,
proving a second observation/final checkpoint is required before fresh capacity and
reuse. Independent review found and cleared missing shared-lease revocation on the
atomic preparation commit; its held-clone regression passed.

**T49 canonical-time progress acceptance (implemented and reviewed):** A local RPC
fixture runs the production payment repair owner twice while canonical time is before
the immutable signed expiry, although host wall time is already past it. Both passes
retain the exact disputed liability and repair reason without any activated release.
Advancing the canonical head and timestamp permits the same StoreHandle/Base owner to
adopt EXPIRED_UNUSED and clear the payment repair reason; the original USED audit row
and single authorization remain intact. The focused default-build test passed. Together
with shared sticky clock/suspend freshness, treasury late-checkpoint catch-up and
future-poll restart tests, this covers the separate time domains without replacing
canonical expiry with local elapsed time.

**Recurring review lesson.** One numerical time value had been used for unrelated
purposes. Observation age, scheduling cadence, immutable quote authority and canonical
chain expiry now retain separate origins; clock/suspend tests prevent freshness renewal
through clones, cache merges or artificial scheduler ticks.

#### C22 — Restoration fencing, final schema and evidence compatibility

Implementation provenance: restoration declaration `78c3b0c`; fresh exposure cleanup
`dc284e2`. Final runtime supports schema26 only, with no runtime/private-state
migration. Frozen historical reporting retains schema-specific readers, including
schema16 retirement records (`6bfdf75`), schema19/20 row-dispute records (`110f2e0`) and
saved source-report dependencies (`f0a249c`). Final fixture corrections (`121ed57`)
preserve canonical-pending observations and actual readiness assertions; fixture and
lint corrections are not additional financial-policy changes.

**Declared restoration contract:** Ordinary reopen cannot detect an undeclared
filesystem rollback. The explicit offline `wallet mark-restored` command declares a
manually restored working copy before serving or submission. Its durable, one-way
missing-provenance marker fences new managed payments, source/qualification allocation,
treasury preparation, shielding, all submission including saved bytes, and funding
reauthorization. Known-operation observations, scans, accounting and read-only
inspection remain available. Current balances, reconstructed projections and canonical
expiry of known operations cannot clear unknown later authorizations or missing
journals. Original bytes, gross debit, known liabilities and attempt counts remain
intact; backups retain the declaration.

This is not an atomic file-restoration tool or automatic rollback detection. Operators
must stop the original process and declare the restored working copy before use.
Unrelated static credentials remain independent; reusing restored keys as a static
wallet bypasses this state's ownership and is not protected. Complete externally
retained authorization/submission journals and a separately reviewed reconciliation
would be required to recover dependent spending; there is no generic reset switch or
claim that scanning known chains recovers unseen off-chain liabilities.

The Store backup/reopen test preserves exact known bytes and attempts while blocking new
authority and startup readiness. A current Base balance cannot recover a missing signed
authorization from an older backup. The actual CLI test verifies exclusive ownership,
offline declaration and persistent status without opening the SDK. All three focused
gates passed; independent review is clear. No private backup was used.

**Fresh-schema exposure cleanup:** Fresh schema 24 removes the independently writable
`wallets.unresolved_exposure` projection. Runtime admission continues deriving exact
pending and disputed liabilities from authoritative journals; no financial limit
changes. Test-only incremental projection helpers are removed. Frozen historical schemas
remain readable, and explicit legacy-shaped corrupt projections remain in fault tests to
prove they cannot override journal authority. Runtime identity guards require the new
layout without an on-open migration. Independent review cleared the cleanup. Default
Store and manager coverage passed after separately committed reporting/fixture
corrections, including historical read-only reports and the actual process-loss
regression; the deployment schema/identity gate and treasury recovery tests also passed.
This does not claim automatic rollback detection; C18 and C20 record canonical return
and shielding acceptance independently.

**Recurring review lesson.** Schema evolution exposed historical readers that
accidentally inherited current admission requirements. Explicit saved-layout readers
preserve evidence without migration or new authority. Restoration declarations protect
missing provenance only when the operator declares the backup; chain scans cannot
recreate unknown off-chain liabilities.

### Stage 1 — Minimal authority and dependency specification

**Prerequisites:** Reviewed source findings; no runtime transition is authorized by this specification alone.

**Deliverables:** Produce these artifacts in order for each implementation slice,
before designing replacement certificates, incident records or repair machinery for
that slice. Extend the shared inventory as slices land; do not require a separate
repository-wide specification phase before the first implementation:

1. Minimal decision algorithms for startup identity/ownership, payment admission,
   usable-role publication, treasury preparation, source closure and refund closure.
   Express inputs as authoritative identity/ownership, current canonical evidence,
   unresolved liabilities, exact side-effect boundaries and current permission.
   State each allowed action, missing proof and commit-time race check without
   reproducing the existing phase branches or repair flags. The payment algorithm
   is the primary acceptance artifact for G04/G14/G20/G21: resolve their shared
   contract before building historical repair for admission. Base eligibility on
   owned wallet identity, current permission, fresh compatible chain balances and
   outstanding authorization liabilities, with debit protection and atomic checks.
   Classify each role/claim field as actual ownership or a derived eligibility
   conclusion. Removing historical RPC reads while retaining an unjustified
   structural historical-readiness veto does not satisfy this contract.
2. A predicate/field deletion table expanding the existing G01–G24 inventory with
   exact source locations, persisted-state classification, delete/derive/retain decisions and focused acceptance cases. For every retained
   historical read, name the outstanding obligation and explain why fresh canonical
   evidence plus unresolved liabilities cannot resolve it. Audit retention alone
   is insufficient. Remove unjustified dependencies before specifying replacement
   proofs; a new certificate or typed wrapper is not a deletion.
   For each historical record type, specify the remaining canonical recheck, the
   event ending that dependency, the audit-only state afterward, and how a reorg
   within the supported recovery model restores affected obligations. Cover resolved
   payments and completed source/refund operations as well as readiness. Do not
   replace obsolete readiness checks with permanent resolution-checkpoint or
   accounting verification lacking a live dependency.
   Resolve this retirement contract alongside the admission algorithm, before
   removing historical checks or building replacement verifiers. Define ordinary
   and unsupported deep reorgs operationally under the configured chain policy;
   name the evidence that detects a supported change, the affected records it
   selects, and how obligations reactivate without rescanning all audit history.
   Dependency retirement is not audit deletion. Do not introduce an arbitrary
   history ceiling or deadline to manufacture an end to a live obligation.
3. A halt/release table and expected startup, observation, closure and new-work
   outputs derived from those algorithms. Every wait names an executable owner,
   trigger and release action that can run while the halt exists. Specify residual
   dependency lifetimes and canonical replacement proofs only for retained gates.

Include F13–F27 in these artifacts. Decide the pending-outgoing gate's required
scope and specify authority/accounting for automatic eligible refund shielding.
Resolve whether shielding owned outputs needs completed refund attribution; carry
unresolved accounting through any supported internal transfer, preserving exact
inputs, reservations, fees and address-isolation policy. Partial receipt alone
neither proves closure nor proves that the received outputs cannot be shielded.
Specify recovery ownership after consumer removal, recoverable owner exits,
same-pool isolation of malformed live records, shared funding-scheduler isolation
for malformed jobs, and treasury lock/actor fairness. Audit foreground cached and
challenge-time payer selection as well as background inventory isolation. Include
certificate merging before candidate filtering: a wallet-local comparison conflict
must not prevent consideration of an independent healthy payer. Distinguish wallet-local
malformed data from shared database failures; neither may become zero exposure. Specify authoritative exposure derivation from complete payment attempts
and resolution support, including invalidated support. G14 resolved the original
projection-versus-reconstruction alternative in favor of indexed journal derivation:
the independently writable `wallets.unresolved_exposure` projection is removed.
Reconstruction after a test-injected repair signal would have been insufficient;
the implemented admission path reads the authoritative obligations directly. Resolve the shared
G04/G14/G20/G21 admission algorithm and exact predicate deletions before building
residual repair for admission. Prefer indexed authoritative derivation where practical;
retaining a projection must not introduce another independently enforced consistency
subsystem. A failure to release bounded exposure does not itself justify a whole-wallet
halt; demonstrate the remaining balance/debit invariant if that halt is retained.
Distinguish interrupted atomic writes, application projection bugs and arbitrary database edits;
do not promise detection of arbitrary internally consistent corruption.
Do not add a full-history sweep before every startup or payment. Audit inventory
selection and cursor advancement alongside admission gates: derived terminal labels must not hide
outstanding obligations, and malformed rows must not starve later pages.
Specify payment eligibility below the original funding target, including candidate
discovery, balance scheduling, initial and challenge-time selection, reserve claim
consumption and pending-operation ownership. Evaluate conservative retention
of fully bounded disputed authorization exposure as an alternative to whole-wallet
admission blocking; define its balance/debit contract before enabling it.
Trace prerequisites from identity lookup and store opening through SDK reconstruction
and bootstrap; full financial reporting must not be an identity prerequisite.
Distinguish independent static service, funded Base service with usable journals/keys
but unavailable Zcash SDK state, and justified shared-state integrity/ownership stops.
Decide whether the store owner can start independently of the SDK while preserving
one exclusive owner and accepted mutation ownership; do not infer this from separate
chains alone. Specify stale-backup restoration separately from ordinary restart,
including missing off-chain authority and undetectable undeclared rollback.
Include declaration validation (`validate_resolved_funding`), funding-only connection
preflight and paid discovery initialization in the startup dependency map. Distinguish
parse failures or ambiguous identity/security/ownership from a validly parsed
funding-only configuration error; scope the latter to its dependent capability.
Start acceptance fixtures at the actual deployment entry point, before validation.
Specify independent publication, deferred discovery ownership
and the callable/unavailable behavior when no payer is initially usable; use the
existing lifecycle, without paid retries after uncertain relay submission.

Specify G23's provider-observation authority and closure rules before implementing
new automatic closure. Establish terminal/provisional semantics from the applicable
provider contract; do not assume a corrected status is valid or a balance increase
proves delivery. Define exact canonical correlations that can discharge each
financial obligation despite missing or inconsistent provider status, and name any
remaining obligation requiring a stop. Define G24's clock domains and production
reevaluation paths without changing expiry, freshness or retry authority.

Add startup/operator-disposition and proposal/fee-failure fixtures, plus both
four-repair and cross-pool shared-RPC saturation fixtures. Define startup faults
before SDK reconstruction and before bootstrap supervision. Resolve removals before
designing alternate historical verifiers or incident machinery.

**Required deletions/decisions:** G01–G24 (individual predicates and release conditions).

**Acceptance:** Review each slice's minimal algorithms and deletion table before
its implementation or residual repair design. Demonstrate each limitation or
document why it is unreachable;
every retained gate has a concrete invariant and every Waiting outcome has an
owner and trigger. Do not refactor from an unverified incident hypothesis.

**Tests:** T03, T08, T15, T40–T49 as applicable to each slice; define expected
outcomes for its cases before implementation.

**Completion evidence:** Reviewed slice contracts and gate decisions are recorded
above, with the original C17 / stage 6 task-exit follow-up explicitly unresolved.
The shared admission contract derives payment-sized capacity from current canonical balances and complete bounded liabilities, with atomic ownership/exposure
checks; historical readiness and diagnostic prose are not authority. The pending-input
gate is deliberately retained because the real SDK `reset_spends` contract can clear
pending markers (T29). Versioned preparation provenance, exact canonical
source/delivery/return obligations, independent observation ownership, clock domains and
scoped startup prerequisites are implemented. Unknown preparation, shared-state
integrity failures and declared-restoration fences remain explicit restrictions, not
claims of automatic repair.

### Stage 2 — Remove redundant gates; share typed assessment

**Prerequisites:** Stage 1 proof contracts and resolved dispositions for affected gates.

**Deliverables:** Delete obsolete dependencies
and competing derived authority using stage 1's proof contracts. Separate current
capacity from funding completion for ALLOCATED, READY, ACTIVE and RETAINED wallets,
including initial usable-role publication, incomplete funding jobs and disputed
readiness. Implement payment-sized eligibility independently of funding-target
satisfaction under G20, preserving exact pending operation/claim ownership. Resolve
G21's bounded-exposure alternative before building historical repair solely to
unblock admission; implement it only where the reviewed accounting proof permits.
Separate treasury scan/current spendability from historical source
and refund accounting under the stage-1 contracts. Add typed durable recovery causes
and typed errors; update startup, worker and wallet recovery to use the same
facts assessment with distinct outputs. Keep last_error as presentation only.
Scope malformed live-record failures to identifiable obligations without treating
them as resolved or absent. Include foreground candidate scans: a wallet-local
exposure/eligibility failure or certificate-merge comparison conflict must not abort
selection of another provably usable payer. Preserve the affected dispute; shared
store/ownership failures are not absent candidates. Derive exposure from authoritative
journals instead of rebuilding a competing stored total; missing authoritative
records remain a scoped unresolved liability.
Preserve independent same-pool observation and admission
where no shared authority is affected; justify any wider stop when scope is unknowable.
Apply the same scope analysis to funding job/fact decoding across the shared
scheduler. Implement G23's reviewed provider-fact assessment without converting
retained observations into independent permanent admission authority. Apply G24's
clock-domain decision to temporary waits; local elapsed time never proves canonical
expiry and a clock correction never renews evidence. Derive actionable inventories
from authoritative outstanding obligations,
removing terminal-label exclusions that lack a live justification.
Replace operational full-status reads with narrow identity/ownership queries,
including omitted treasury IDs and static-profile validation. Keep schema, identity
and binding checks; report malformed financial history separately.

**Required deletions/decisions:** G01, G03–G05, G07–G08, G11, G14, G16–G17, G19–G24.

**Acceptance:** Changing diagnostic wording cannot change eligibility; cross-consumer
fixtures agree on missing evidence while preserving their different purposes;
sufficient current evidence permits progress without irrelevant historical
reads, and rebuilding derived state preserves all liabilities and authority.
Prove historical independence with forbidden-call assertions and deliberately
unavailable irrelevant history, not merely successful historical repair.

**Tests:** T15, T17, T18, T21, T24–T27, T29, T32, T34, T38, T40, T42–T49.

**Completion evidence:** Completed: typed consumer-specific assessments replace prose gates; journal-derived
pending/disputed exposure replaces the writable projection; payment-sized initial
capacity and conservative remaining capacity operate independently of the original
funding target. Narrow identity reads and scoped malformed-row/job handling preserve
independent service without treating failed exposure reads as zero. C13/C18 source and
return proof replace whole-sync historical vetoes. Inventory membership follows
outstanding obligations, including completed labels; `bdc90b5` couples changed return
evidence to refresh scheduling and tests unchanged unresolved reuse. T39 `c582bba`
verifies actual PaidClient/manager enforcement in the normal library build; T43/T44 and
scoped-decode gates cover adjacent paths.

### Stage 3 — Preparation provenance and crash recovery

**Prerequisites:** Stage 1 preparation and pending-outgoing scope contracts.

**Deliverables:** Add the minimum versioned
boundary records and transactional bindings, then implement only proven
proposal/pre-commit cleanup.

**Required deletions/decisions:** G02, G11.

**Acceptance:** Process death and persistence faults at every
boundary yield exact resumable committed work, a proven closed attempt, or a
scoped unresolved obligation when evidence is unavailable or contradictory;
no missing bytes, reservation or ambiguous commit grants replay authority.

**Tests:** T08–T12, T29.

**Completion evidence:** Completed by versioned boundary provenance `d4d5407` and reviewed crash/reopen
acceptance. Committed facts, original identities, bytes, reservations and uncertain
attempts survive loss of acknowledgement; only proven pre-commit cleanup can close an
attempt. Real two-note SDK acceptance justifies retaining the pending-input exclusion
(T29). T12 `f188ecb` calculates and proves actual transaction bytes, then applies
simulated later authority-check times around the unchanged accepted deadline: no late
capability or submission is created, and reopen retains the exact work. This tests
expiry enforcement after proving, not measured slow proving beyond a wall-clock
duration. Unknown/legacy preparation provenance remains unresolved.

### Stage 4 — Canonical replacement and scoped residual disputes

**Prerequisites:** Stage 1 replacement contracts; stage 2 removes irrelevant dependencies first.

**Deliverables:** Implement the specified
current-evidence and approved cross-endpoint paths without rewriting audit history.
Preserve the exact invalidation cause and dependent records; centralize repeat/supersession/
contradiction classification.

**Required deletions/decisions:** G03–G08, G20–G21.

**Acceptance:** The listed cases pass for the retained dependencies. Persist only incident provenance needed by live dependencies; original audit support remains immutable.

**Tests:** T01, T02, T04–T06, T14, T17–T19, T24–T27, T43–T44.

**Completion evidence:** Completed: C10–C15 remove obsolete historical admission dependencies and provide the
sole bounded canonical replacement path while retaining immutable audit proofs.
Explicitly configured replacement endpoint selection occurs before a pass, never mid-
pass. T39 `c582bba` and T44/T04 use real paid calls during held repair, preserve
concurrent liabilities, and reopen with distinct authorizations. C13 `05b8346`, C18
`f8c8194` and schema26 `c3d4d5a` extend scoped replacement to original source/return
accounting and exact input ownership. Concrete live contradictions revoke dependent
support; burial alone does not create a new dispute or release existing ownership. Exact
existing two-sync expiry correction is idempotent and grants no replacement authority.

### Stage 5 — Incremental repair with fair scheduling

**Prerequisites:** Stage 1 for scheduling containment; stages 2 and 4 determine residual repair dependencies.

**Deliverables:** Separate historical repair from
fresh balance progress, yield between database pages, and implement complete
revision checking for short final commits. Exercise process-wide query, queued-caller
and active-RPC bounds across pools,
not only per-pool balance slots. Remove unnecessary historical calls first; if
residual repair starves current evidence, adjust scheduling within existing bounds
rather than automatically increasing concurrency or adding financial ceilings.
Scheduling containment can follow stage 1 independently of preparation work; build targeted historical repair only
for dependencies retained after stages 2 and 4.

**Required deletions/decisions:** G12.

**Acceptance:** T03, T04, T07 and T33 demonstrate progress at database, RPC, wallet-lock and owner boundaries. Residual repair follows the target design without a whole-work timer or count limit.

**Tests:** T03, T04, T07, T33.

**Completion evidence:** Completed by separate repair lanes `07b5cad`, bounded inactive publication `55d5b3b`,
the sole repair path `525069f`, and process-wide resource partition `6bdc853` at
unchanged aggregate limits. T03/T04/T07 cover physical paging, current RPC/socket
progress across saturated pools, coherent activation and concurrent liabilities. T33
`8abcfcc` pauses real residual source-accounting work after the SDK read guard is
dropped: independent Store/Base work progresses and a cancelled sync waiter does not
abandon the serialized treasury owner; queued SDK work resumes after accounting. It does
not assert concurrent SDK mutation. Shielding inventory fixup `3f5df1f` verifies indexed
keyset work on sparse physical history. The SDK's single resident output-map traversal
remains linear and is documented separately from bounded SQLite work.

### Stage 6 — Supervised owner recovery and treasury reconstruction

**Prerequisites:** Stage 1 observation-owner failure/ownership contract; stage 3
proven reopen contract and accepted-command ownership for treasury reconstruction.

**Deliverables:** Once stage 3 is established, reconstruct
from committed state after eligible owner poisoning. Explicitly settle accepted
command ownership and fence the old owner before reopening. Include proposal,
sync and refund-address derivation/persistence failures, not only preparation.
Separate SDK-owner retirement from final shared-store shutdown. Currently
`Treasury::close` releases the SDK client but awaits the store worker, which healthy
Base service can keep alive through retained handles. Replacement must not wait
for that final shutdown: preserve one exclusive store owner, fence and retire the
old SDK mutation owner, then reconstruct against committed state. Specify the
handoff of the worker join handle and accepted commands without overlapping SDK
owners or abandoning store work. `RunningDeployment::serve` also currently shares a
cancellation token whose task-exit guards stop both treasury and funding work.
Distinguish process shutdown from a recoverable worker-generation failure; a
replacement must neither inherit an already-cancelled worker token nor escape
process shutdown. Reuse the existing supervisor rather than adding another one.
Independently implement recovery of Base reconciliation after recoverable inventory
read/owner failures and funding-scheduler exits. Isolate identifiable malformed
funding jobs within inventory traversal, preserving their obligations while later
jobs and independent observation lanes progress. Retrying a permanently malformed
page through owner restarts does not satisfy recovery. Verify scheduling membership
against outstanding obligations, including inconsistent derived completion labels.
Reuse existing deployment supervision, ownership gates and backoff; do not create a second workflow engine.
Exercise scheduling after clock correction, suspend/resume and restart with persisted
polling times under G24; temporary clock conditions must not strand observation
owners or reset signed authority/recovery counts.
Keep failures scoped when ownership is known; do not blindly restart on database
corruption or lost exclusive ownership. Preserve or settle accepted store jobs and
funding work before replacement ownership; task exit alone does not prove their end.

**Required deletions/decisions:** G02, G14–G16, G19, G24.

**Acceptance:** T23/T35 establish the reopen contract through the production owner,
including exact committed address bindings, snapshots, queued commands and uncertain
commits. T35 must complete reconstruction while healthy Base listeners retain
store handles and continue serving; recovery cannot require their shutdown or
final store-worker termination. Race replacement with process shutdown and assert
no replacement owner survives shutdown. T37 proves resumed observation after a transient inventory failure without
service restart or duplicate ownership. T38 proves same-pool isolation while retaining
malformed live obligations. Nonrecoverable stops name the invariant and affected scope.

**Tests:** T23, T35, T37, T38, T42, T45, T49.

**Completion evidence:** Partially complete: C17 `929b6ca` fences/drains the old SDK owner and reconstructs against the
same live Store without waiting for healthy Base holders to disappear; accepted commands
and shared Store jobs retain ownership. Read/completion fixups `3639bbb`, `dd64f58` and
`1a8dc14` preserve completed work and original permits through eligible local
contention/cancellation. Base and funding owners resume eligible Busy/Locked reads under
existing backoff, while identifiable malformed rows/jobs remain scoped and shared
corruption/lost ownership remain explicit stops. Clock correction, suspended freshness
and persisted future-poll restart retain original authority anchors; T49 `8ffb521`
requires canonical time advancement. T23/T35/T37/T38/T42/T45 and retained-owner T33 pass
without a second store owner or workflow engine. Actual treasury/funding task-exit
cancellation and scheduler restart handling remain the original unresolved C17
follow-up. The current restart-required boundary is recorded above; the passing
in-owner tests do not establish the separate worker-generation acceptance.

### Stage 7 — Automatic closure and resumption

**Prerequisites:** Stages 2–4 assessment/closure contracts; stage 6 where owner reconstruction is needed.

**Deliverables:** Specify exact terminal refund/source/destination
proof requirements and implement automatic closure for fully evidenced supported
cases. Resolve G23's provider-status dependency explicitly: original observations
remain auditable, while exact canonical settlement can close only the obligations
it actually proves under the reviewed contract. Missing provider availability or a
historical status alone must not preserve a veto after every relevant obligation
has been discharged. Unproven attribution or additional unresolved obligations
remain scoped; no closure grants replay or replacement funding authority. Complete
the halt/release table with actual scheduling for restored RPC,
newly spendable funds and restored effective policy, without manual reset or a
restart solely to clear a halt. Implement automatic shielding of eligible, exactly
attributed returns through the existing treasury owner and transaction journal.

**Required deletions/decisions:** G09, G13, G23.

**Acceptance:** Source debit, return credit and shielding
fee each count once;
closure neither allocates nor submits a replacement. Unresolved cases remain
visibly incomplete. Any new funding uses ordinary current admission separately.
Demonstrate acquisition and commit of release evidence while each supported halt
is active; no recovery action may depend circularly on clearing its own halt.
Verify observation ownership after pool/listener removal, disablement and wallet
retirement across restart. Eligible refund shielding restores confirmed spendable
capital automatically when current policy permits it, with exact input/output and
fee accounting. Policy-disabled new shielding remains visibly pending and resumes
when authority becomes effective. Under the reviewed input/accounting contract,
partial or unattributed returns may remain scoped accounting obligations while their
proven owned outputs are shielded; missing input ownership or unresolved reservations
still block dependent spending. Any retained attribution gate needs its concrete
invariant. No manual shielding step may substitute for the supported T30 automatic-recovery outcome.

**Tests:** T13, T14, T16, T20, T23, T27, T30, T31, T48.

**Completion evidence:** Completed: exact canonical source support (`05b8346`), destination delivery and finite
return accounting (`f8c8194`) discharge only the financial obligations they prove. T48
`2d434a8` runs the real funding worker/treasury actor across conflicting
FAILED/REFUNDED/INCOMPLETE observations and provider outage: exact bytes, facts, debit
and one submission remain unchanged, and completion waits for canonical source,
destination and bound return proof. Semantic inventory changes trigger one required
refresh (`bdc90b5`), while unchanged unresolved observations reuse it. Durable policy-
free observation survives disablement/removal/retirement. Automatic shielding
(`9863d95`) creates/submits only under current captured serving authority, exact owned
inputs and fee safeguards; manual bytes and previously attempted bytes are excluded from
automatic submission. Existing operations remain observable after authority removal.
Returned principal and shielding fee count once. Partial/unattributed owned outputs may
be shielded without guessed accounting credit or closure. Refund replacement, unknown
preparation and unresolved confirmed swaps still require review.

### Stage 8 — Scoped serving startup

**Prerequisites:** Stage 1 lifecycle/binding contract and coordinated provider-startup design; applicable readiness work from stages 2 and 4.

**Deliverables:** Implement the stage 1 isolation contract with the
existing deployment lifecycle and coordinated provider-startup design.
Background source publication in
[independent provider startup](deferred/independent_provider_startup.md) is deferred,
not an existing implementation prerequisite that can be assumed available.
Stage 8 must deliver or explicitly depend on landed commits for the portions it
uses: visible pending/unavailable source state, bounded deferred discovery ownership,
atomic publication of affected listener views through the existing catalog snapshots,
and callable independent sources while a relay wallet recovers. Record those
dependencies in the affected commit rows. Broader provider-startup work may remain
separate only where T22/T41 do not depend on it; referencing the deferred plan alone
does not satisfy acceptance. Do not create a parallel publication mechanism.
Reconcile startup ordering with that plan's all-listeners-bind-before-serving-or-
funding rule. For ordinary serving, complete listener binding before starting new
funding, with atomic binding rollback; explicit wallet bootstrap remains a separate
command. Specify where initialization and read-only recovery occur in that order,
and retain ownership/drain obligations for already accepted financial work if
startup fails. Background recovery must not silently change this side-effect boundary.
Deliver this as an early independent track once those prerequisites are met;
stages 3, 5, 6 and 7 are not blanket prerequisites.
Include failures before bootstrap supervision: narrow identity reads, managed
initialization and SDK snapshot reconstruction. Preserve independent static service
when managed initialization fails. Implement the stage-1 decision for funded Base
service with valid journals/keys but unavailable Zcash SDK state, including any
required separation of store-owner startup from SDK reconstruction. Shared database,
decryption or ownership failures must retain explicit dependent-service stops;
never open a second store owner to bypass them.

Separate Base payment prerequisites from funding-only declaration validation and
connection setup. Include `validate_resolved_funding` and its treasury/funding
validators, which run before initialization, in the same capability contract. A
later scoped initializer cannot fix an earlier global validation veto. Include
NEAR credentials and treasury sync/submission configuration. Missing or malformed
explicit configuration remains an error for its dependent work, not permission
to substitute defaults, endpoints or credentials. Existing Base service must not
require usable funding-only configuration when its own ownership, keys, journal,
RPC policy and liability checks suffice. Specify funding and swap-observation
prerequisites separately, including how corrected configuration becomes effective;
this does not imply hot reload.

Include `initialize_discovery_relay` in the same startup dependency contract.
Specify publication of independently loadable sources/listeners while a relay
wallet recovers, the visible status of dependent sources, and the owner/trigger
for deferred discovery through the existing provider publication mechanism.
Retain relay serialization, submission markers and run-wide disablement after
possibly submitted failure; wallet recovery cannot authorize another paid attempt.

Define ordinary serving when no managed payer is initially usable: with valid
listener configuration and independently available catalogs, listeners/discovery
start, affected calls return a typed unavailable result without waiting for the
entire recovery, and the same process becomes callable after valid evidence arrives.
Do not move the global bootstrap barrier into the first tool call or require
listener reconstruction to clear wallet unavailability. Preserve accepted financial
work ownership and explicit bootstrap/qualification completeness.

**Required deletions/decisions:** G10, G17–G18.

**Acceptance:** T15, T22 and T40–T41 establish scoped startup before and after bootstrap supervision, including funding-only declaration-validation and connection faults, relay dependencies and no initially usable payer, with per-call admission and explicit bootstrap/qualification completeness. No second publication, store owner or supervision mechanism.
T22/T41 also inject listener-bind failure during partial initialization: no new
funding starts before all binds succeed, acquired resources are cleaned up, and
previously accepted financial work retains its owner and durable obligations.

**Tests:** T15, T22, T40–T41.

**Completion evidence:** Completed by scoped startup slices `51c4aef`/`151369f`, `59f9b83`, `fc7fed1`,
`3d3305a`/`68e5ef4`, narrow identity reads and the implemented deferred-discovery
lifecycle. T15/T22/T40/T41 start at deployment entry and distinguish independent
static/Base service from unavailable SDK/funding prerequisites, no initially usable
payer and required/qualification completeness. Listener binding precedes new funding;
partial bind failure rolls back resources while accepted financial work drains under its
existing owner. Dependent discovery uses existing immutable catalog publication and
bounded retained ownership; recovery does not grant another paid relay attempt after
possibly submitted failure. Restored canonical capacity wakes the same serving process
without listener reconstruction. Broader deferred-provider work remains outside this
implementation where not required by these gates.

### Stage 9 — Cross-layer regression, single-schema rollout and documentation

**Prerequisites:** All preceding stages and their recorded implementation evidence.

**Deliverables:** Complete the
matrix below; document new typed reasons and supported automatic transitions.

**Required deletions/decisions:** G14; close all inventory rows.

**Acceptance:** The supported schema reopens correctly, incompatible schemas are rejected,
restart does not reset retries or signed authority, and qualification restrictions
remain intact. Remove obsolete compatibility paths and update architecture,
startup policy and operator guidance to describe the resulting behavior.
Validate stale-backup restoration without assuming that rescanning reconstructs
unredeemed signed authorizations or missing side-effect journals, or that the restored
database can detect an undeclared rollback. Validate admission/recovery with production
certificate enforcement enabled. Reconcile the final implementation with the gate
deletion inventory.

**Tests:** T01–T49 and applicable repository checks.

**Completion evidence:** Final schema and validation evidence: fresh schema26,
incompatible-schema rejection and no on-open migration;
read-only historical reports use saved-schema contracts. The declared
`wallet mark-restored` fence blocks new financial actions and saved-byte submission while preserving
observations/accounting, and explicitly cannot detect undeclared rollback or reconstruct
missing off-chain journals. T01–T49 evidence is recorded in the matrix and maintained
lifecycle/testing guides, with T12's simulated-time, T33's serialized-owner and retained
linear SDK scan limitations stated. All required repository validation passed in
segmented runs after focused fixture corrections: default all-target tests,
synthetic-chain treasury/refund lifecycle gates, vendor/toolchain/format checks, all-feature
Clippy, dependency compatibility and complexity tooling (hotspots advisory). Maintained
guides reflect current boundaries. Passing repository checks do not resolve the
original C17 / stage 6 actual-task-exit acceptance gap. These checks use local unfunded fixtures and do not
establish external provider settlement contracts, Docker consensus acceptance,
live-funded qualification or automatic arbitrary-corruption recovery.

All stages are required for this plan's completion. Proof contracts precede new
automatic transitions, but stages 6–8 are not optional follow-up work. Unsupported
or genuinely ambiguous cases retain scoped obligations and explicit missing-proof
status; they do not justify marking supported automatic recovery complete early.

## Test matrix

For implementation, run focused default-feature suites per change, then the
applicable standard checks in [AGENTS.md](../../AGENTS.md) and
[testing](../testing.md). Run synthetic proving/local consensus suites only under
their documented prerequisites; live-funded qualification remains a separate
operator-authorized step.

Use the existing production paths with temporary unfunded state and deterministic
local RPC/indexer/provider fixtures. Extend the current crash and SQLite fault
harnesses rather than build another independent simulator of implementation details.
A small reference model should express obligations and allowed actions, not mirror
the production phase branches.

| ID | Boundary or schedule | Required safety assertion | Required progress assertion |
| --- | --- | --- | --- |
| T01 | Duplicate resolutions, both completion orders, newer heads | Exactly one release; original support retained; genuine conflicting outcomes refused | Superseded work is discarded/rechecked without durable quarantine |
| T02 | Cache eviction, process restart and endpoint invalidation | Freshness never renewed; dependency/generation changes invalidate stale captures | Reacquisition resumes once necessary evidence is available |
| T03 | Four blocked repairs plus healthy wallet; multiple pools holding historical requests against process-wide query, RPC caller and active-RPC bounds | No relaxed admission, duplicate repair owner or increase in aggregate resource bounds | Healthy current-balance acquisition, liability reconciliation and unrelated store jobs progress while historical work is held, including across pools |
| T04 | Repair races ordinary pending-history publication | No double release or stale repair commit; admission remains blocked until justified | Concurrent resolution can finish or safely force recapture; neither owner is silently orphaned |
| T05 | Latest-only historical readiness reorg vs confirmed-anchor reorg | Different dependencies are identified; original support retained | Unrelated valid observations continue; supersession follows an explicit rule |
| T06 | Original RPC removed/unavailable; approved alternative matches or contradicts | No implicit endpoint/transport adoption; exact proof and trust-generation checks | Supported additional verification can finish; unavailable archival evidence has a distinct status |
| T07 | Multiple history pages, changing journal during scan | Complete revision check rejects stale repair; no partial commit | Stable histories finish without size-dependent timeout; unaffected work retained where safe |
| T08 | PREPARING commit before proposal; proposal before reservation | Exact recorded boundary; no unjournaled submission | Proven proposal-only failures can close/reconstruct without operator intervention |
| T09 | Reservation before calculation; calculation before snapshot/outgoing commit | Reservations and authoritative snapshot preserved; uncertain commit reopened | Exact committed operation resumes, or proven discard completes |
| T10 | Destination readiness arrives during owned proposal/calculation | Source work remains owned; replacement claimant cannot inherit its authority | Destination service and source-accounting completion progress independently when evidence permits |
| T11 | Commit before capability return; broadcast intent before network response | Same bytes/identity; no new authorization from lost acknowledgement | Canonical reconciliation continues after caller cancellation/restart |
| T12 | Quote expiry during slow proving | Returned deadline and signed bytes remain authoritative | Observations and eventual canonical expiry recovery continue without new submission |
| T13 | Provider status, source confirmation and destination credit in all orders | No attribution from balance alone; contradictory facts retained | Independent source/status/destination work does not wait on an unrelated lane |
| T14 | Canonical refund, duplicate output, partial refund, missing binding | Exact credit once; no double-counted shielding input/fee; no inferred closure | Fully evidenced accounting can finish while ambiguous operations remain scoped |
| T15 | Startup, wallet recovery and serving over the same facts | No hidden funding authority or weakened captured bootstrap completeness | No indefinite Waiting when the only remaining disposition requires an operator |
| T16 | Auto-funding disabled, exhausted recovery allowance, qualification reopen | No new preparation/allocation/submission | Existing observation/accounting work continues where already authorized |
| T17 | Archival balances unavailable, sufficient current canonical evidence available | Exact outstanding authorizations and debit obligations remain covered; no fabricated historical proof | Current capacity recovers without irrelevant archival reads |
| T18 | READY reserve, incomplete funding job, disputed historical readiness | Role/claim ownership and unresolved source/swap obligations retained; no credit misattribution | Independently proven capacity and observation progress without requiring funding completion |
| T19 | Ordinary reorg with previously released exposure, then stable canonical evidence | Affected authorizations and debit accounting reconciled before admission; original audit retained | Supported canonical replacement recovers automatically; unsupported deep reorg stays scoped |
| T20 | RPC outage then restoration; insufficient funds then spendable credit; disabled policy then effective enablement | No replay, reset recovery allowance, stale authority or funding while disabled | The scheduled owner reevaluates and resumes eligible work without manual reset or restart solely to clear a halt |
| T21 | Reconstruct derived phases, totals and caches from authoritative records | Liabilities, exact bytes, ownership and signed authority unchanged; no double accounting | Restart reaches the same eligible actions without independent derived-state enforcement |
| T22 | Recovering group alongside healthy service; recovering relay wallet; no initially usable managed payer | No bypass of affected-wallet admission, listener binding rules, explicit bootstrap or qualification | Independent service becomes callable while recovery remains pending; deferred discovery preserves relay submission boundaries; typed unavailable calls recover in the same process |
| T23 | Proposal failure and fully evidenced refund through supervised recovery | One owner, exact accounting, no unauthorized replacement | Supported automatic closure/reconstruction completes without operator intervention |
| T24 | Historical confirmed/expired source operation reorg, unrelated spendable notes and pending refunds | Affected inputs/reservations remain constrained; scanning alone grants no spending authority | Independent observations and proven capacity progress; canonical recovery completes across restart |
| T25 | Credited refund reorg or missing historical output after shielding/spending | Original credit, shielding expenditure and remaining liabilities reconcile exactly once; no inferred swap closure | Unrelated refund discovery and justified current-note capacity proceed; affected accounting recovers when proof returns |
| T26 | ALLOCATED wallet with sufficient canonical funds and stalled/failed/disputed funding | Exact wallet/claim ownership and qualification authority retained; no misattribution or inherited source permit | Supported initial usable-role publication proceeds independently of funding completion |
| T27 | Active Base quarantine or treasury historical-accounting halt during release-evidence acquisition/publication | No liability release or admission before its own proof is valid | Release evidence can be acquired and committed while the halt exists; recovery does not wait on itself |
| T28 | Declared restoration of an older backup after later confirmed activity and a still-valid off-chain authorization; separately characterize undeclared rollback | Rescan does not erase missing exposure, recreate permits or authorize replay; restored state is not assumed to detect later off-chain activity | Declared restoration retains missing-provenance status and permits independent observations; dependent spending requires specified sufficient evidence; undetectable rollback is an explicit limit, not a claimed automatic detection |
| T29 | Pending treasury bytes with unrelated spendable notes | Exact input exclusion, original bytes/reservation ownership and commit checks survive uncertainty/restart | Independent sync and preparation progress where proven safe; any retained global gate has a demonstrated SDK/persistence invariant |
| T30 | Fully accounted eligible transparent refund with insufficient shielded capital; owned partial-return outputs with unresolved attribution under the reviewed contract; policy disablement and crash/restart around shielding | Current authority and fee safeguards enforced; exact inputs/outputs and fee accounted once; committed bytes retained; no duplicate shielding or unauthorized replacement | Automatic shielding restores confirmed spendability without operator intervention when authorized; disabled new work waits and resumes under effective policy |
| T31 | Removed pool/listener, disabled group or retired wallet across restart | No new funding, retired-capacity reuse, erased liability or replay | Remaining obligations retain an observation owner independently of current listener demand, with scoped missing-evidence status where necessary |
| T32 | Forbidden irrelevant archival calls, large completed history, paired states differing only in irrelevant history, and an unrelated malformed historical record | Live obligations retain validation; audit/reporting failures remain visible | Startup, fresh admission and recovery succeed without irrelevant historical RPC reads or full-history decoding |
| T33 | Slow residual treasury accounting with independent observations and eligible accepted commands | Single mutation owner and accepted-work ownership remain intact | Unrelated work progresses under the specified ownership contract; database pages do not conceal lock/actor monopolization |
| T34 | Diagnostic wording changes and cross-consumer assessment fixtures | Diagnostics cannot change eligibility; distinct readiness/completion/new-work outputs preserve authority | Startup, worker and recovery agree on missing evidence without duplicate phase interpreters |
| T35 | Poisoned-owner reconstruction after preparation, sync and refund-address mutation/persistence failures, with accepted/queued commands and dropped waiters | Old owner fenced; committed snapshot and address bindings preserved; uncertain commits resolved; no duplicate owner, lost accepted mutation, poisoned save or unauthorized queued submission | Each supported mutation path reconstructs through bounded failure backoff while independent service remains available |
| T36 | Supported schema reopen and incompatible binary/schema | Reject incompatible schema before mutation; no legacy bypass or reset of retries/signed authority | Supported state reopens with equivalent eligible actions; obsolete compatibility paths removed |
| T37 | Recoverable Base inventory-read failure followed by restoration; funding-scheduler exits identified by audit; cancellation during owner replacement | Accepted work remains owned; no duplicate owner, liability release, reset funding allowance or replay; corruption/lost ownership never blindly retried | Background observation resumes without service restart; independent service stays callable and nonrecoverable stops expose their justified scope |
| T38 | Malformed live authorization for wallet A and valid obligations/capacity for wallet B in the same pool, including the same inventory page | A's liability remains blocked and visible; no fabricated resolution or skipped exposure; unknown ownership justifies an explicit wider stop | Where ownership is provable, B's observations, reconciliation and admission progress despite A; traversal does not repeatedly die or starve later rows |
| T39 | Integration tests with application compiled without `cfg(test)` certificate exemptions, using local unfunded RPC/provider fixtures | Missing, mismatched, stale and invalidated evidence cannot admit payment or release liability; test-only record helpers cannot satisfy acceptance | Valid current evidence and supported recovery succeed through production acquisition, certificate checks and commit boundaries |
| T40 | Actual startup with omitted treasury ID; static service with unrelated malformed managed history | Narrow identity/schema/ownership checks remain mandatory; malformed liabilities remain blocked and reportable | Identity resolution and independent startup do not decode unrelated financial reports; T32/T38 isolation holds before recovery starts |
| T41 | Startup faults before SDK reconstruction and bootstrap supervision, including funding-only declaration validation and credentials/connection configuration; funded Base journal/keys usable; separate shared-state failures | One exclusive store owner, valid identity/keys and per-call liability checks; no funding through unavailable SDK state or bypass of shared corruption/decryption/ownership failures | Independent static service starts; funded Base service starts where the stage-1 ownership contract proves independence; every retained broader stop has a demonstrated shared invariant |
| T42 | Malformed funding job or facts beside healthy jobs in the same and different pools, including later pages and restart | Preserve affected operation/reservation ownership; unknown shared treasury liability blocks dependent spending; never treat malformed facts as no liability | Independent source/status/credit observations and provably safe work continue; cursor advances without dropping the affected obligation; no scheduler restart loop on the same row |
| T43 | ALLOCATED wallet with $2 verified available capacity, $10 funding target and a $0.10 payment; pending funding and concurrent reserve consumption | Current payment cap, exposure, ownership and qualification enforced; exact claim transition; no false swap completion, transferred permit or duplicate replacement funding | Payment succeeds below the funding target when the reviewed role/claim contract permits; original funding accounting remains independently observable |
| T44 | Known disputed authorization bounded at $1, $20 compatible current capacity; later resolution/reorg and concurrent admission; unknown-amount/ownership controls | Full affected exposure retained atomically; no double release, pre-debit balance reuse or stale admission; unbounded/unidentifiable liabilities retain justified stops | Where G21's proof permits, remaining capacity serves payments before historical repair completes; resolution later removes exposure exactly once; otherwise record the concrete failed invariant |
| T45 | Outstanding obligation with stale/inconsistent derived completion or phase label; restart and multi-page inventory | Original bytes, liability, ownership and current permissions remain authoritative; labels grant no new work or replay | Observation inventory finds every outstanding obligation regardless of derived terminal labels; repair of derived state restores the same eligible actions without manual reset |
| T46 | Foreground cached, initial hinted and challenge-time selection with malformed wallet A exposure/eligibility data or conflicting cached/completed certificates and healthy wallet B; both candidate and completion orders | A remains blocked and reportable; no zero exposure or blanket suppression of shared database failures | B remains selectable and callable without waiting for A's repair |
| T47 | Malformed, too-high and too-low derived exposure under an explicit fault model; production detection/derivation before admission; complete versus missing authoritative records; concurrent admission/resolution/invalidation and restart | Complete authoritative liability reconstruction or derivation, atomic revision check and admission fencing; no invented resolution, erased liability or unsupported corruption-detection claim | Production owner automatically restores eligible capacity when provable; independent wallets progress without unconditional startup/payment history sweeps |
| T48 | Duplicate, reordered and contractually corrected provider observations; provider disappearance after exactly correlated canonical settlement; genuine unresolved-obligation controls | Preserve original observations and exact operation/source/destination/refund accounting; no closure from arbitrary balance changes, replay, erased liability or invented provider semantics | Where the reviewed G23 contract proves all relevant obligations discharged, closure and independent capacity progress without a permanent historical-status veto; otherwise identify the precise missing proof |
| T49 | Forward/backward wall-clock correction, suspend/resume, restart with persisted polling times, stalled then advancing chain | Local elapsed time never establishes canonical expiry; old evidence never becomes fresh again; signed deadlines and durable retry counts are not reset | Observation scheduling resumes through its production owner once valid time/evidence is available; temporary clock/chain conditions do not leave permanent repair flags |

### Acceptance evidence and interpretation

The matrix records required invariants; it does not imply 49 separate standalone
executables or live-provider qualification. Consolidated contract evidence supplies
the implementation and controls for each boundary. These frequently conflated
acceptance scopes are explicit:

| Cases | Implemented acceptance and location | Qualification boundary |
| --- | --- | --- |
| T12 | C16 / `f188ecb`: actual SDK calculation and verified proof, followed by simulated later checks at the production serialized submission-authority API, including reopen. | Checks the unchanged submission margin and accepted deadline; it does not measure proving that physically lasts beyond a wall-clock bound. |
| T33 | C15 / `8abcfcc`: hold real residual accounting after its SDK read guard is released; independent Store/Base work progresses, the cancelled waiter retains its owner, and a queued SDK command resumes after release. | Proves serialized-owner resumption, not concurrent SDK mutation or incremental SDK output-map discovery. |
| T39, T43, T44 | C11/C12/C14: actual PaidClient and ordinary-library admission under real certificate enforcement, held repair, concurrent liabilities and restart. | Local deterministic chain/provider fixtures; no mainnet-funded payment qualification. |
| T48 | C18 / `2d434a8`, with `bdc90b5`: real worker and treasury actor close exact source/delivery/finite-return obligations during provider outage while contradictory historical labels remain. | Explicit zero is a finite current claim, not absence of future returns or a refund-replacement permit. |
| T49 | C21 / `8ffb521`: expired host time cannot release a liability while canonical time stalls; canonical advancement permits exact expiry recovery. | Separate original evidence age, canonical expiry and scheduling clocks; no new runtime timer. |

The schema26 source-ownership and C20 actor gates additionally exercise actual
synthetic confirmation/reorg/expiry, automatic preparation, policy changes at
preflight/intent, one submission, confirmation, disabled restart, and fee-only
accounting. Final integrated validation and the retained linear SDK traversal
limit are recorded in stages 5 and 9 and the C13/C18/C20/C22 evidence above.

### Cross-layer fixture requirements

T48 exercises production status acquisition, persisted facts, scheduling selection
and closure, not only a classifier. Record which status transitions the applicable
provider contract supports; unsupported corrections remain explicit controls.
Distinguish closure of source, destination and refund obligations, and independent
payment capacity, from a provider's own success label. A missing provider response
is not proof of settlement; use exact canonical correlation and assert every
remaining liability. Extend T13/T14 rather than creating another status ledger.

T49 distinguishes chain timestamps/heights, wall-clock authority checks and
monotonic elapsed-time scheduling. Exercise fresh acquisition after correction,
not just manually clearing a flag. Restart with polling timestamps saved before
correction; assert eventual observation without resetting financial authority or
adding a runtime deadline. Use deterministic injected clocks through production
paths. Unsupported clock conditions may temporarily block dependent admission,
but must expose their reason and retain a reevaluation owner.

T42 extends T38 to the shared funding scheduler, not just Base history. Exercise
`funding_work_page`, `collect_inventory` and actual owner lifecycle; prove traversal
past the malformed job and progress in independent lanes before repairing that row.
Where shared treasury exposure cannot be bounded, continue safe observations but
assert dependent preparation remains blocked. Preserve a visible affected obligation
rather than silently skipping malformed records.

T46 exercises the actual cached scan, initial hinted selection and challenge-time
selection with valid certificates for A and B, malformed derived exposure for A,
and sufficient capacity for B. Reverse candidate ordering so an early error cannot
hide B. A remains unavailable and reportable; B succeeds without repairing A first.
Exercise relevant wallet-local eligibility decoding failures too, with shared
store/ownership failures as controls that must not be silently skipped.
Also exercise `merge_payment_certificates`, which runs before candidate filtering.
Supply individually valid cached/completed observations for A whose comparison
conflicts, alongside valid B capacity; reverse completion and candidate order.
A's conflict must remain visible and scoped while B succeeds. Cover wallet-local
validation/decoding failures in that merge separately from shared-store failures.

T47 distinguishes malformed, valid-but-too-high and valid-but-too-low derived totals.
Specify the reconstruction trigger and completeness proof, not just a test helper
that overwrites a value. Drive repair through its production owner and race it with
admission, resolution and invalidation; restart around publication. Understated
exposure within the specified fault model must not admit spending before verified
reconstruction or authoritative derivation. Test the production detection path before
the first affected admission, without injecting a repair notification that production
would not receive. Distinguish interrupted atomic transactions from projection bugs
and arbitrary edits; document undetectable internally consistent corruption as a
limit rather than claiming recovery from information the application does not have.
If exposure is derived directly from indexed unresolved obligations, test completeness
and invalidation through that path instead of requiring a redundant stored total.
Overstated exposure must recover automatically when authoritative records suffice. Missing or malformed
authoritative obligations remain blocked; unrelated wallets continue. Assert no
unconditional full-history startup or per-payment sweep is introduced.

T43 makes “sufficient funds” in T26 mean sufficient for the requested payment after
liabilities, not necessarily the original target or accepted swap output. Cover
both unprepared and already prepared funding, a late destination credit, concurrent
claim consumption and restart. Serving the payment cannot itself satisfy delivery
attribution or authorize a new allocation. Exercise an actual tool call after restart
with empty evidence caches and pending funding. Candidate discovery, background
balance acquisition, initial payer selection and challenge-time selection must all
reach the below-target wallet. Assert exact reserve consumption, continued late-credit
observation and no duplicate allocation or transfer of the pending funding permit.
A fixture that directly publishes a usable role does not establish this behavior.

T44 is a proof-gated alternative to wallet-wide quarantine, not permission to weaken
T19/T27. Include previously released resolutions whose support is invalidated,
multiple affected authorizations and commitment races. If the required upper bound
or compatible current evidence is unavailable, assert the scoped stop. Record the
G21 decision explicitly; a passing blocked-case fixture alone does not establish
that a broader admission veto is necessary.

T45 extends T21 to the production scheduling queries, not only rebuilding reports
or phases. Use contradictory derived labels with intact authoritative obligations;
do not fabricate missing submission provenance or infer it from a phase.

For historical-independence fixtures, instrument the production acquisition paths
to reject irrelevant archival requests or assert that they never occur. Inject an
unrelated malformed record only where the stage-1 dependency specification proves
it is unnecessary to the tested decision; this is not permission to ignore malformed
live financial authority. Assert audit/reporting failure separately from operational
availability. Use deterministic scheduling gates for large-history and lock/actor
fairness tests rather than timing benchmarks or new runtime deadlines.

Extend T21/T32/T34 with paired-state tests through production decision paths:
identical ownership, current canonical evidence, outstanding liabilities and effective
permissions must yield identical allowed actions despite differences in irrelevant
completed history. Vary old readiness records, diagnostic text, derived completed-job
labels and availability of historical endpoints. Keep authoritative obligations
identical; do not classify a record as irrelevant merely to make the test pass.
Add usable canonical funds while unrelated historical accounting remains unresolved
and assert that independently justified capacity resumes without repairing that
accounting. Assert forbidden historical calls and reads, not just eventual recovery.
For resolved payments and completed source/refund operations, test the specified
end of operational dependency and reactivation under the supported reorg contract.
T19/T24/T25/T32 must exercise the actual detection and affected-record selection
path after dependency retirement, not supply an artificial repair signal that
production cannot obtain. Assert cessation of irrelevant reads before the reorg
and safe reactivation afterward, without an unconditional audit-history sweep.
Growing audit-only history must not add operational verification reads; each retained
history-dependent read needs its documented live obligation. These are focused
correctness tests, not a new simulator or a quantitative performance claim.

Extend T14/T30 with confirmed owned partial-return outputs and unresolved attribution.
Under the reviewed shielding contract, prove that eligible internal transfers can
finish while swap accounting remains unresolved, or demonstrate the specific invariant
requiring a stop. Include unknown ownership, competing input reservations, fee-policy
failure and address-linking controls. Preserve exact output-to-shielding accounting
across restart and forbid inferred swap closure, duplicate principal credit and
replacement authority. No blanket permission to shield arbitrary incoming funds is
implied by these fixtures.

T03 must exercise the production shared resource paths across enough pools to
reach aggregate contention, not only mocked per-pool slots. Use deterministic
holds/release points and assert current evidence progress before historical holds
are released. This tests scheduling sufficiency, not a claimed reproduced starvation
incident or justification to raise the existing resource limits.

T40 extends T32/T38 through actual identity resolution and startup. Diagnostics may
fail or report malformed records explicitly; operational identity lookup must not
invoke the full report. T41 must cover initialization failure rather than only a
running funding worker's later exit. Preserve safe cleanup and ownership when only
part of initialization succeeds.

T41 starts at deployment configuration validation, including
`validate_resolved_funding`, before connection preflight or SDK initialization.
Cover validly parsed funding-only declaration errors as well as missing/malformed
NEAR credentials and treasury sync/submission configuration with otherwise usable
funded Base state. Parse ambiguity and identity, security or ownership errors remain
controls requiring their justified scope. Assert independent Base
service, explicit scoped configuration errors, no funding through unavailable
dependencies and no silent credential/endpoint fallback. Base RPC, shared ownership,
key and journal failures remain controls requiring their justified dependent stops.
Restoring configuration follows the specified effective-configuration lifecycle.

Extend T22/T41 with a listener-bind failure after partial initialization, including
an already acquired store handle and a recovering managed group. Assert atomic
binding rollback, cleanup of initialized resources, and no new funding before all
listeners bind successfully. Where accepted work predates the failure, assert its
owner drains or preserves its exact durable recovery obligations; cleanup cannot
erase liabilities or detach active work. Exercise the landed deferred-publication
path rather than assuming the separate provider-startup plan is implemented.

Extend T35 through the actual supervisor and `Treasury::close` lifecycle. Keep
healthy Base listeners serving with live store handles while an eligible SDK-owner
failure occurs. Reconstruction must complete without closing those handles or
joining a store worker that service still owns. Assert one store owner, fenced old
SDK ownership, preserved accepted commands, and usable replacement command routing.
Race process shutdown with retirement and reconstruction: a recoverable worker exit
must not permanently cancel its replacement, and shutdown must not leave a new
owner running. Use deterministic lifecycle gates, not a new runtime deadline.

Extend T22 through paid discovery: a recovering managed relay wallet coexists with
an independently loadable source, which becomes callable through actual deployment
startup. Assert dependent-source status and supported deferred discovery publication
through the existing owner. An uncertain paid relay submission must disable further
relay spending for that run even if the wallet later recovers.

T22 also starts with every managed payer temporarily unavailable and independently
available catalogs. Exercise actual MCP listing and a call returning typed
unavailability, then restore evidence and successfully call in the same process with
initially empty caches, no manual reset and no listener reconstruction. Use deterministic
holds to prove the first call does not wait for recovery; add no runtime deadline.
Cancellation of an unrelated call with accepted financial work must preserve its
owner, bytes and liabilities. Explicit bootstrap and qualification still require
their captured inventory completeness.

T38 is separate from historical independence: malformed live authority is still
required evidence for its affected wallet. Preserve that obligation and prove the
scope of isolation; never interpret a decoding failure as an empty page or zero
exposure. T37 must exercise actual owner exit/recovery, not only a failed per-wallet
RPC future that the existing loop already retries.

T39 must include integration coverage linking the normal application library or
running the real executable, so `validate_chain_balances` and other production-only
certificate requirements are enabled. Audit relevant `cfg(test)` exemptions and
record which acceptance paths they could mask. Unit fixtures remain useful but
cannot alone establish production admission/recovery correctness. Use deterministic
unfunded keys and local fixtures; this is not live-funded qualification.

For T28, supply the restoration declaration and available external evidence explicitly
in the fixture. Do not inject knowledge of later missing authorizations into the
application while claiming it discovered that knowledge from the restored database.
Document the unsupported undetectable-rollback case without adding a second ledger.

Test crashes before and after each durable boundary, including actual WAL write/
fsync failure, process kill, dropped waiters and reverse callback completion. Assert
forbidden side effects explicitly. Liveness tests use deterministic release points
and test-harness deadlines; they must not introduce production whole-work deadlines.
Preserve slow valid progress, true stalls, unrelated concurrent progress and
cancellation as separate scenarios. Local fixtures do not prove provider settlement
contracts or live finality.

## Completion criteria

The original implementation criteria are retained here for review. Implemented
dispositions and the unresolved original C17 / stage 6 task-exit follow-up are
recorded in the gate inventory, C01–C22 contracts and stage evidence.
The proposed H01–H09 hazard-reduction follow-up below is a separate review scope.

Per-slice stage-1 review permits incremental delivery; it does not reduce final
scope. Completion includes foreground payer and funding-page isolation,
authoritative journal-derived exposure with the competing projection removed,
obligation-driven scheduling,
end-to-end payment eligibility below funding targets with empty caches after restart,
and a resolved bounded-disputed-exposure decision with either implemented progress
or a demonstrated invariant requiring the
remaining restriction. A generic audit item or owner restart is insufficient evidence.

Completion requires all nine stages, every gate inventory row resolved with actual
removal or a justified retained invariant, and all required acceptance cases passed
with recorded implementation evidence. The halt/release table must name executable
owners and triggers, including after configuration removal. Unsupported cases retain
scoped obligations and explicit missing-proof status.

Completion also requires narrow operational identity reads, tested isolation before
SDK/bootstrap initialization, and cross-pool progress under shared RPC contention.
Treasury reconstruction must complete while healthy Base service retains shared
store handles, with worker-failure cancellation distinct from process shutdown.
Stage 8's required deferred-publication mechanisms must be implemented, with
listener-binding failure tests establishing the agreed funding and cleanup boundary.
Startup evidence must include funding-only configuration isolation, paid discovery
dependencies and recovery from no initially usable payer. The minimal payment
algorithm must demonstrate that retained roles encode actual
ownership rather than obsolete readiness conclusions. Provider-status acceptance
must resolve G23's terminal/provisional semantics and canonical closure alternatives;
clock acceptance must demonstrate G24's automatic reevaluation without renewed
financial authority. Exposure acceptance must establish production detection or
authoritative derivation before admission under
an explicit fault model, not only reconstruction after an artificial repair trigger.
The stage-1 algorithms and deletion table must account for retained restrictions;
broad coverage by a generic audit item is not evidence that a gate was removed.
Completion includes pre-filter certificate-merge isolation, explicit end conditions
for historical operational dependencies, and paired-state acceptance showing that
irrelevant history cannot change allowed actions. G09 must resolve owned-output
shielding with incomplete attribution and implement it where proven, or identify the
concrete retained invariant; a blanket partial-refund exclusion is insufficient.

Architecture, repository guidance and operator documentation must describe the
implemented startup/recovery behavior, one supported schema, automatic eligible refund
shielding under current authority, and restoration-detection limits. Production
certificate enforcement and owner-resumption evidence are required.

## Invariants to preserve

Preserve removal of buffered resolved history before new RPC acquisition,
original resolutions across compatible overlapping observations, validation of
resolution bindings before evidence invalidation, and narrow audited revalidation
of false chain blocks.

Preserve the current atomic snapshot/outgoing commit, exact-byte submission
capabilities, accepted-work ownership after waiter cancellation, immutable financial
support, durable recovery counts, operation-bound callback checks, and separate
historical/fresh evidence clocks. Existing tests already exercise real SQLite
write failures, process crashes, stale callbacks and owner drain. The initial review
identified gaps in how those boundaries composed and how eventual progress was
asserted; the consolidated evidence records their implemented resolutions and
remaining qualification limits. See [testing](../testing.md#qualification-status).

## Target design

These decision contracts retain the design rationale for review. The gate inventory
and consolidated C01–C22 evidence state the implemented choices; alternatives
discussed here are not additional live authority or an instruction to rebuild
removed machinery.

### Minimal authoritative state and dependency removal

Before expanding repair, classify every persisted field and blocking predicate:

| Class | Treatment |
| --- | --- |
| Identity, signed authority, exact bytes, possible submission, ownership and unresolved obligations | Retain durable authoritative records and atomic bindings; chain balance cannot reconstruct an unredeemed signed authorization |
| Current chain observations | Reacquire under configured trust/freshness policy; never relabel historical observations as fresh |
| Historical accounting and audit support | Preserve original facts and disputes, but require them for current decisions only while a named dependency remains live |
| Derived phases, totals, caches and eligibility conclusions | Rebuild from authoritative records where possible; do not maintain independent competing authority |

For every dependency, record the decision it supports, release/supersession rule,
and exact invariant protected. Prior use in a readiness decision is not a reason
to require the same balance or latest anchor forever. Define when fresh canonical
balances and reconciliation of outstanding authorizations suffice for current
capacity. Retain the debit floor, or prove an equivalent rule that prevents reuse
of pre-debit balances after liability release. Preserve race checks at selection
and commit boundaries using shared calculations.

Use the [gate inventory](#gate-inventory-and-implementation-evidence) as the single
checklist for predicate/field classification, release conditions and actual removals.
Do not build a general incident dependency framework to keep unnecessary checks
alive. Rebuilding derived state must preserve liabilities, authority and exactly-once
accounting. Stage 1 expands the initial grouped rows into concrete predicates.
Its decision algorithms must make current canonical evidence the default for present
capacity. Retain a historical read only for a named outstanding obligation that
current evidence cannot resolve. Audit retention alone is never that obligation.
Delete unnecessary dependencies before designing cross-endpoint certificates,
incident records or residual repair; do not build those mechanisms for removed gates.

Exposure must have one authoritative derivation. C09/C22 resolve the original
projection-versus-reconstruction question by deriving pending and disputed amounts
from indexed payment journals and removing `wallets.unresolved_exposure`. Original
resolution audit facts remain immutable; canonical invalidation restores exact
affected liabilities before admission can commit. Missing authority cannot be
reconstructed from a balance or assumed absent.

The fault model distinguishes interrupted atomic writes, application projection
bugs and arbitrary internally consistent edits. Atomic journals and exact
generation/input checks protect supported transitions; they do not promise
detection of every plausible database rewrite. The declared-restoration fence
prevents financial actions on acknowledged incomplete backups without pretending
that rescanning detects undeclared rollback. No replacement aggregate, second
ledger or unconditional full-history admission scan is required.

### Decision contracts and canonical replacement evidence

Assess current payment capacity, source accounting, swap/refund closure, and
permission for new funding separately. A disputed historical readiness record
must not automatically veto independently proven present capacity. An unrelated
credit can contribute capacity only under ownership and liability checks; it must
not be attributed to a swap or used to close its remaining obligations.

Specify two distinct evidence paths: exact historical reverification (including
an explicitly approved alternative endpoint), and a new current decision supported
by canonical replacement evidence. The latter preserves the original observation
and dispute rather than rewriting history or pretending the old anchor survived.
It must not require archival balance reads when they are irrelevant to the current
decision. Both paths retain configured chain, endpoint, transport and trust policy.

Before implementing replacement, define proof obligations for exact original
authorizations, nonce outcomes at sufficiently confirmed canonical anchors,
chain-time expiry, compatible current balances, previously released exposure and
debit accounting. Identify affected authorizations after a reorg and reestablish
their obligations before admission; neither sufficient balance nor a missing
transaction proves safe release. Ordinary reorgs should recover automatically
when those proofs are obtainable. Unavailable necessary evidence or a remaining
contradiction blocks only dependent actions. Deep reorgs outside the supported
proof contract remain explicit incident cases.

### Payment capacity below a funding target

Funding targets and accepted output amounts describe allocation preferences and
delivery/accounting obligations. They must not independently become minimum payment
balances. The intended G20 behavior permits an owned ALLOCATED wallet with $2 of
verified available capacity to serve a $0.10 payment even if its funding target is
$10, once ordinary payment authority and the reviewed role/claim contract are met.
This does not establish that a swap delivered its output or that funding is complete.

Specify the initial usable-role and reserve-claim transition atomically with payment
admission. Preserve the exact operation, source reservations and accepted bytes if
funding is underway. A consumed claim cannot transfer the old operation's authority
to a new wallet or automatically justify duplicate replacement funding. Identify
how pending destination credit remains observed after the role changes. Reuse the
existing claim generations and journals; do not create a second funding authority.
Qualification and explicit bootstrap retain their own completeness requirements.

### Bounded disputed exposure as an admission alternative

Before requiring historical repair for every disputed resolution, evaluate whether
retaining each affected authorization's full maximum liability permits safe use of
the remaining current capacity. C09/C12 implement that G21 contract for complete,
exactly bounded affected membership and compatible fresh balance/debit evidence;
unbounded or unidentifiable liabilities retain their justified dependent stops.

The contract must identify all affected authorizations and their exact maximum
amounts, use compatible fresh canonical balances, preserve debit-floor semantics,
and atomically restore conservative exposure with invalidation of stale admission
captures. Previously released exposure must be covered before another payment can
commit. Retain original audit support and resolve each liability only through its
own valid proof. Do not add a duplicate reservation ledger or subtract the same
obligation twice through competing accounting paths.

If that proof holds, a wallet with $20 current capacity and $1 bounded disputed
exposure may use the remainder while reconciliation continues. Unknown amounts,
incomplete affected-authorization inventory, missing ownership or incompatible
balance/debit evidence still require a justified dependent-action stop. Distinguish
inability to release a liability from inability to admit any payment. Resolve this
option before designing residual historical repair solely to unblock that capacity.

The membership decision is implemented in C09/C12 and extended by C14/C15:
the final `wallets.payment_resolutions_disputed` marker conservatively covers the
affected wallet's resolved authorizations, with indexed journal membership and
revision-checked staged replacement publication. Pending and disputed-resolved
sets remain disjoint, and generation invalidation commits atomically with dispute
changes. Earlier schema17 per-attempt `resolution_disputed` records remain a
historical read-only format, not the final runtime layout. Canonical replacement participates in
later endpoint/chain invalidation while preserving original resolution audit rows.
The detailed review evidence is consolidated in those contracts rather than kept
as an unresolved chronological prerequisite here.

### Facts, assessments and actions

Use existing authoritative journals for facts. Add only missing preparation and
incident provenance. Derive an assessment such as:

- work is already satisfied;
- observe a named missing fact;
- recapture superseded evidence;
- continue an owned operation using its committed identity;
- reconstruct a poisoned owner under a proven snapshot contract;
- require an operator for a specified unresolved contradiction or provenance gap;
- new work is disabled by current policy.

The assessment is not a spending permit and should not become another durable
phase machine. An action rechecks current operation identity, generations,
reservations and authority at its commit boundary. Unsupported schema versions are
rejected; there is no permanent legacy interpretation path.

Maintain a halt/release table covering every supported durable halt. Each row names
its scope, missing fact, scheduled action, owner, trigger/backoff and terminal
outcome. Waiting must correspond to work an existing owner will actually schedule.
Policy-disabled new work is distinct from operator-only contradictions and does
not stop authorized observation/accounting. Reevaluate temporary conditions through
existing maintenance ownership without resetting recovery counts or signed authority.

An owner must remain executable, not merely named in durable state. Recoverable Base
inventory failures must not leave deployment with only a logged dead task. Supervise
eligible observation-owner exits and audit funding-scheduler termination through the
existing lifecycle. Preserve accepted work and ownership gates before replacement;
reuse backoff and distinguish transient reads from corruption or lost ownership.
For malformed live records, identify the affected wallet/obligation before isolating
failure. Keep that liability blocked and visible while independent wallets in the
same pool progress. If scope cannot be established, state the invariant requiring a
wider stop; retrying an undecodable whole page forever is not recovery.

Include removed pools, removed listener bindings, disabled groups and retired wallets
in this ownership audit. Configuration reconciliation disables absent managed pools;
that does not establish who observes their remaining obligations after restart.
Enumerate outstanding durable obligations independently of current service demand
and identify their observation owner under the effective configuration. Missing
required credentials or approved endpoints remain explicit scoped missing evidence.
Removing a consumer must not orphan accepted work, erase liabilities, reactivate
retired capacity or authorize new funding.
Audit the actual SQL inventory predicates and pagination alongside these owners.
For example, `funding_work_page` selects
`state != 'COMPLETE' OR readiness_disputed IS NOT NULL`; derived completion labels
must not exclude authoritative unresolved
obligations. Correct or derive the scheduling projection from those obligations,
without another permanent work ledger. Preserve indexed bounded traversal and
ensure an identifiable malformed row cannot repeatedly hide later work. This is an
audit requirement, not a claim that the present implementation has a reproduced orphaning bug.

Audit recovery's own prerequisites as an action graph: evidence acquisition, sync,
publication, accounting, closure and new admission. For every halt, show that its
release evidence can be acquired and committed while the halt is present. Remove
circular prerequisites that require recovery to have succeeded before it can run,
including treasury sync failures and Base quarantine publication checks. Retain
necessary checks on the specific financial transition. A scheduled loop that
repeatedly fails the same prerequisite does not satisfy automatic recovery.

Ordinary serving should expose independently usable service while affected groups
recover, with explicit unavailable status and admission enforced per call. Validate
configuration, ownership and listener bindings before publishing service; avoid
starting new financial work for a listener whose startup failed. This does not
relax explicit bootstrap completeness, qualification, or source inventory rules;
provider publication follows the coordinated startup design referenced in F12.

Distinguish these evidence relationships explicitly: exact repeat, consistent
later observation, superseded observation, missing/unavailable evidence, and
contradiction. Use one typed comparison at acquisition/publication/application
boundaries, retaining each boundary's necessary race and integrity checks. Define
which decision each support record justified, and distinguish its immutable audit
lifetime from the lifetime of a current admission dependency. In particular, a new height for the same supported nonce outcome
is not itself contradictory; a current balance does not supersede an unresolved
authorization; and a new observation cannot renew an old observation's freshness.

### Preparation provenance

Design versioned provenance around actual boundaries rather than more descriptive
phase names: intent and starting snapshot revision; proposal/calculation ownership;
actual reservation identity; atomic committed snapshot/bytes; durable submission
intent. Reuse existing records where they already establish a boundary. Do not
create a second copy of the transaction or reservation authority, or persist every
SDK substep when it adds no distinction between uncommitted work, committed bytes,
and possible submission.

A proposed safe-discard path must prove exclusive owner termination, no escaped
submission capability, authoritative persisted snapshot identity, exact operation
and reservation correlation, and the result of any uncertain database commit.
Reopen and inspect durable state after uncertain persistence; an error return is
not proof of rollback. Any new attempt then requires current policy and fresh
spending evidence. Never manufacture that provenance for old records from missing
outgoing rows alone.

### Scoped disputes and incremental repair

Only for dependencies retained after the deletion audit, persist the minimal
missing incident provenance: category, affected record IDs, old/new anchors or value
digests, endpoint identity and the decision being disputed as required by that
specific recovery contract. Reuse existing support records instead of duplicating
them. Do not create durable incidents for ordinary staleness or supersession that
recapture resolves, or an independent incident lifecycle that gates admission.
Keep private evidence in protected state; warnings expose fixed categories and counts, never credentials,
signed payloads, arbitrary upstream prose or raw private identifiers.

Drive repair through the existing maintenance ownership model with independent
progress from normal balance refresh. Reuse existing RPC/socket authority rather
than increasing aggregate concurrency or stacking redundant ceilings. Page database
work into separate actor turns. Protect the final commit with a complete relevant
journal revision/CAS scheme; every relevant mutation must participate. A cursor,
partial digest or count alone is not proof that the journal stayed unchanged.

Extend this fairness audit to treasury reconciliation. Current sync traverses
historical source operations while holding the wallet read guard. After deleting
irrelevant historical dependencies, identify wallet-lock duration and serialized
owner occupancy for the remaining work. Database pagination alone does not provide
fairness at either boundary. Test that slow residual accounting permits independent
observations and eligible accepted commands to progress where the ownership contract
allows; distinguish necessary serialization of SDK mutations from avoidable waiting
on unrelated historical I/O. Do not obtain fairness by overlapping treasury owners
or abandoning an accepted mutation.

Retain historical work only while its evidence dependencies remain valid. Acquire
fresh balances after historical verification; restart or recertify affected work
on real changes. Exact original proofs remain immutable and liabilities change only
through justified accounting transitions. Use historical repair only for dependencies
that survive the removal audit; use canonical replacement when its proof contract
is satisfied. Archival unavailability is not itself a permanent current-capacity
veto when sufficient replacement evidence exists. Missing necessary evidence stays
explicit and never authorizes an unconfigured transport or liability deletion.

## Hazard reduction follow-up

This section retains the design lessons from implementation and review. The
recommendations below are proposed design work, with separate H identifiers. They
are distinct from the original unresolved C17 / stage 6 task-exit follow-up recorded
above and grant no permission for live-funded qualification.

At the plan-retirement commit, `main..593e703` contained 125 commits, including 41
explicitly titled `fixup!`. This is a classification aid, not
a defect count. Fixups include correctness repairs, stronger assertions and
fixtures, feature-boundary compatibility, documentation, and lint cleanup. Some
substantive hazard reductions were ordinary implementation commits rather than
fixups. The recurring patterns matter more than the label or count.

### Patterns observed in implementation and review

| Hazard category | Evidence from the branch | Recurring mechanism and implication |
| --- | --- | --- |
| Workflow labels and projections substitute for obligations or authority. | `08a5c5d` retained observation inventory beyond completion labels; `bdc90b5` connected changed return inventory to refresh scheduling; `8384e4e` removed remaining reliance on a redundant exposure projection. | A status can be correct as a summary yet insufficient to decide what work remains. Separately maintained validity, scheduling, and accounting projections can disagree. Derive effective exposure and outstanding work from authoritative journals and evidence dependencies. |
| Ownership ends between external completion and local publication. | `dd64f58` retained completed funding results and their original permit across admission-read contention; `d4d5407` bound preparation cleanup to durable provenance; `0f36e2f` retained repair ownership through transient staging-cleanup contention. | External completion, durable commit, acknowledgement, and publication are different boundaries. A late local failure must not discard an accepted result, release its owner prematurely, or authorize repeated financial I/O. |
| Task separation conceals shared bottlenecks or unbounded physical work. | `6bdc853` separated historical repair and current-balance RPC resources; `3f5df1f` replaced a superficially paged shielding inventory with indexed physical keyset traversal. | Independent tasks still block one another through sockets, semaphores, queues, serialized Store turns, or SQL scans. A small returned page does not prove a small scan or sort. Independence must hold at the resource actually being protected. |
| Historical readers inherit live admission contracts. | `6bfdf75`, `110f2e0`, and `f0a249c` preserved reporting across older journal/evidence schemas and source-accounting dependencies. | Current authority decoders and saved evidence readers have different jobs. Updating live validation must not make committed historical evidence unreadable, normalize away diagnostics, or manufacture present execution authority. |
| SDK stage projections are mistaken for exact transaction evidence. | `7e82183` verified owned shielded outputs against actual Orchard/Ironwood transaction ciphertext instead of expecting newly calculated outputs in the wallet's note inventory; `3da65a7` required every owned shielding input for expiry recovery. | An SDK's in-memory representation is stage-dependent. Preparation, proving, submission, scanning, and canonical recovery need explicit contracts tied to exact serialized bytes and immutable inputs. |
| Test models preserve old shortcuts after runtime authority becomes more precise. | `1932d13`, `6aeb41b`, and `121ed57` updated completion and lifecycle fixtures after canonical return accounting became required. | Provider success, destination readiness, source confirmation, delivery, returns, and submission authority had been conflated. Fixtures that recreate credit on each status poll can also hide duplicate side effects. Establish these distinctions in production-path tests early. |
| Validation orchestration invalidates its own evidence. | Frozen-build runner checks were invalidated by concurrent source edits or executable replacement; an isolated EOF fixture subsequently passed without a runtime change. | A test that pins source or executable identity requires a frozen checkout and executable throughout execution. Such failures must be separated from reproduced product defects. |
| Mechanical integration remains distinct from financial redesign. | `c870b03` corrected feature boundaries between durable journal readers and Zcash-specific delivery construction; `f45d07b` addressed Clippy findings. | Compatibility and lint checks remain necessary completion gates. These fixes are not evidence by themselves for another runtime state machine or wider authority. |

Across these categories, the common danger is disagreement between two adjacent
contracts: evidence validity and scheduling; completed I/O and publication;
logical independence and physical resources; live authority and historical
readability; exact transaction facts and SDK projections. Several failures were
revealed only when tests crossed that boundary using the actual worker, ordinary
library build, real SDK calculation/proving, restart, or retained owner. Review
should explicitly identify those boundaries before treating a focused predicate
or mocked result as sufficient evidence.

Restoration review exposed the same evidence-scope hazard in the plan itself:
passing in-owner reconstruction and task-exit containment tests had been summarized
too broadly as completion of actual worker-generation recovery. The original C17 /
stage 6 follow-up remains explicit above. Acceptance summaries must state which
boundary a passing test crosses, rather than infer adjacent lifecycle guarantees.

### Architectural direction

Separate **durable obligations**, **revocable evidence**, and **current execution
authority** in the existing journals, adapters and decision boundaries:

- Obligations retain exact operation identity, signed bytes, inputs, liabilities
  and accepted-work ownership until their specific discharge proof exists. Cache
  eviction, historical burial, a completion label or a restored balance cannot
  discharge them.
- Evidence states what was observed, its provenance and dependencies, and whether
  it still supports a particular decision. Historical observations remain
  immutable; canonical contradiction or changed inputs can revoke current support.
  Missing, explicit zero, disputed and positively established facts stay distinct.
- Execution authority states what may start now in this process and context.
  Serving, administration, qualification and recovery have different capabilities.
  Evidence of spendability or completion cannot mint a capability or renew an old
  permit. Already accepted work retains its own exact recovery obligations.

Implement this separation incrementally through decision-specific calculations
and opaque capabilities at existing mutation boundaries. Reuse journals and owner
lifecycles; do not introduce a second ledger, generic supervisor, universal
assessment object or whole-treasury readiness gate. A calculation may expose
independent source, delivery and return decisions without requiring every one to
succeed before unrelated service proceeds. Share evidence-dependency logic between
validation and refresh scheduling, while keeping current authorization checks at
the boundaries that own mutation and concurrency.

### Proposed change sequence and review gates

The earlier H01–H06 identifiers are retained and expanded; H07–H09 cover atomic
rebinding, context-specific authority and lifecycle transitions. All are **proposed**.
Existing protections described in C01–C22 are starting points, not instructions to
reimplement them. Each package starts by identifying a concrete remaining duplicate
calculation, missing boundary or missing acceptance case. If the existing code
already meets the contract, record evidence and make only the necessary test or
documentation change.

| Order | Reviewable change | Dependencies and commit boundary |
| --- | --- | --- |
| 1 | H06 acceptance/evidence discipline and H05 production-path fixtures | Establish before using new passing checks to declare a package complete; extend throughout the sequence. Commit focused harness/documentation changes separately from runtime behavior. |
| 2 | H01 one obligation dependency slice, with H04 retention/historical interpretation | Begin with funding source/delivery/return consumers. Commit one shared calculation plus its migrated consumers, obsolete-path deletion and acceptance evidence; repeat by obligation. |
| 3 | H07 atomic rebinding and H08 explicit execution contexts | Review current admission and serving capabilities against H01 decisions. Land separately by mutation boundary; neither requires a repository-wide rewrite. |
| 4 | Original C17 actual-task-exit disposition, then H02 accepted-result publication | Resolve the original C17 requirement by audited justified stops or proven eligible handoff; do not count it as new H work. Local publication fixes with no task-handoff dependency may land earlier. |
| 5 | H03 physical work/independence and H09 lifecycle transitions | Apply to the affected Store/RPC and deployment paths. Independent slices may land earlier once their owner and authority contracts are explicit. |
| 6 | Final integration and proposed AGENTS.md rules | Reconcile every package with evidence and remaining limitations; run applicable standard default-build checks. Make the rules below explicit alongside the changes they govern rather than waiting until all runtime work ends. |

Use subagent review at each package commit boundary and any material authority or
ownership design gate. Address findings as fixup commits. Record the exact invariant,
production consumer, removed competing path, focused test and limitation under the
owning H entry. Include the implementation commit, review result and any fixup
commits; leave the entry proposed until that evidence is recorded. Avoid parallel
source mutation during pinned validation. No package
adds a spending budget, blanket deadline, financial replay authority or automatic
replacement after a refund; live-funded qualification remains separately authorized.

#### H01 — Shared obligation dependencies and revocable support

**Starting evidence:** C09, C14 and C18 (journal membership, replacement support and source/delivery/return consumers).

**Change:** Inventory the source, delivery and return predicates used by accounting,
effective completion and scheduler membership. Consolidate one obligation at a time
into a shared dependency calculation consumed by both validity and refresh
eligibility. Preserve immutable observations and derive current support from their
exact dependencies; remove superseded phase-based predicates and parallel writable
projections in the same slice. Do not merge the independent obligations into one
all-or-nothing success result.

**Acceptance:** Changed inputs revoke dependent support and schedule its owner even
after terminal labels. Repeated unchanged unresolved observations reuse completed
work. Late return claims and canonical contradictions withdraw effective completion
without rewriting history or falling back to an older disputed proof. Explicit zero
is distinct from missing or malformed evidence. Test all migrated consumers,
including restart and independent service while one obligation remains unresolved.

#### H02 — Accepted results through local publication and recovery

**Starting evidence:** C05, C16 and C17 (retained completion, preparation provenance and reconstruction).

**Change:** Trace the existing funding completion, preparation and repair owners
from external completion through local validation, commit, acknowledgement and
publication. Close demonstrated gaps by retaining identity, original permit,
completed result and publication disposition in the existing owner. Retry only
classified idempotent local boundaries. Where process loss is supported, recover
from the existing exact durable journal rather than pretending in-memory results
or concurrency permits survive restart.

**Acceptance:** Inject Busy/Locked contention at the actual local boundary after
external completion, then cancel the waiter. Add restart coverage where durable
recovery applies. Assert no second external request, lost committed bytes, duplicate
credit, premature permit release or overlapping SDK owner. Ambiguous mutation and
lost ownership remain explicit stops. C17 task-generation closure is independently
evidenced; an in-owner retry or listener-survival test cannot satisfy it.

#### H03 — Physical maintenance bounds and actual resource independence

**Starting evidence:** C04, C15 and C20 (physical paging, shared RPC resources and maintenance inventory).

**Change:** Audit affected serialized Store queries and shared RPC/queue resources.
Use indexed physical keyset traversal before filtering, with bounded Store turns,
and eliminate duplicate or unbounded fallback paths. Preserve existing justified
resource partitions and distinguish linear SDK output-map traversal from bounded
SQLite work. Add another partition only for a demonstrated distinct resource need.

**Acceptance:** Sparse bad/ineligible prefixes, pending-priority work plus old
history, and scoped failures all advance cursors. Verify relevant query plans and
rows visited through appropriate instrumentation, not just result counts. Hold the
actual historical RPC/Store boundary with event barriers while independent current
read/admission progresses through production paths. Retain the documented SDK
serialization limit; no benchmark or second ceiling substitutes for this proof.

#### H04 — Obligation retention, historical decoding and restoration limits

**Starting evidence:** C13, C18 and C22 (input ownership, unique financial attribution and historical/restored state).

**Change:** Separate retention of disposable observations from retention of signed
input ownership, liabilities, unique event ownership and exact operation bindings.
Review cleanup and archival consumers against those dependencies. Keep historical
saved-schema readers separate from live admission validation, sharing fixtures
where representations overlap. Preserve the declared-restoration fence and its
explicit inability to discover unknown off-chain authorizations.

**Acceptance:** Eviction, burial, cleanup and reopen cannot erase a live obligation
or permit reuse of a financial event. Committed historical evidence remains readable
without live credentials or current admission prerequisites, preserving diagnostics
and unknowns. A restored balance or scan cannot clear missing provenance. Tests
supply the restoration declaration explicitly and never claim automatic rollback
detection, migration, or reconstruction of missing authorization journals.

#### H05 — SDK stage contracts and faithful financial fixtures

**Starting evidence:** C11–C14, C16 and C20; T12/T33/T39/T48 acceptance limits.

**Change:** Document adapter inputs and outputs at calculation, proving, committed
publication, submission, scan and canonical recovery. Verify transaction identity
and ownership against exact serialized facts at the appropriate stage. Extend real
SDK and ordinary-library fixtures early; make fake financial credit idempotent by
operation identity. Keep provider status, readiness, delivery, returns and financial
closure as separate fixture facts.

**Acceptance:** Cover exact inputs/outputs, real ownership verification, lost
acknowledgement, restart, status conflict, every-input expiry recovery and zero
versus missing returns. Preserve T12's actual-proof/simulated-later-time distinction;
claim a real wall-clock proving overrun only if measured. Repeated polls cannot
mint credit. Do not treat pending-input SDK guards as removable without new proof.

#### H06 — Precise acceptance evidence and reproducible validation

**Starting evidence:** the acceptance-evidence interpretation table, the C17 gap and recorded final validation limits.

**Change:** Maintain an acceptance-to-evidence map under each package: requirement,
actual production boundary exercised, fixture/build context, result and limitation.
Distinguish design review, focused unit predicates, ordinary-library paths, real
SDK work, containment, recovery and external qualification. Freeze source and
executable identity for suites that pin either; coordinate builds and CLI tests
sharing the binary.

**Acceptance:** An intentionally incomplete requirement remains open despite related
passing checks. Record source/executable pins and invalidate results if they change
before owned drain completes. Report segmented repository runs accurately; never
infer an uninterrupted wrapper success. Run focused checks per change and applicable
standard checks at integration, without adding unrequested feature configurations.
Fixture timing failures do not justify runtime policy changes without a reproduced
cause. C17's task-exit requirement remains open until its own proof is recorded.

#### H07 — Atomic rebinding after validation changes state

**Starting evidence:** C11 readiness/claim rebinding and C21 final-commit freshness revocation.

**Change:** Audit preparation/admission boundaries where freshness, readiness or
repair publication can alter role, claim, generation or evidence revision after a
caller captures them. Bind the final decision to post-publication ownership in the
same transaction, or use the existing revision/CAS protocol across necessary
asynchronous boundaries. Reuse the shared authority calculation while retaining
checks at separate race boundaries; do not add a second spending ceiling.

**Acceptance:** Exercise readiness promotion, claim transition, concurrent policy or
ownership change, and held stale captures. At most one operation commits for a
uniquely claimed action; when authority remains valid and dependencies permit
progress, one succeeds. Reject stale pre-acceptance decisions without a signature,
broadcast or surviving new reservation/intent. Preserve accepted or uncertain
intent and its exact work rather than rolling it back or reclassifying it as an
unauthorized new start.

#### H08 — Context-specific execution capabilities

**Starting evidence:** C20 serving capability and C22 declared-restoration fencing.

**Change:** Enumerate the commands that can create preparation, reservation,
submission intent or new automatic maintenance. Check their serving, administration,
bootstrap, qualification and recovery contexts against existing opaque capabilities.
Close any demonstrated implicit-authority path; credentials, successful sync,
spendability, historical permits and positive funding limits cannot substitute for
permission for a different action. Recheck effective policy at its owning boundary.

**Acceptance:** A context/action matrix includes paused/removed profiles, declared
restoration, namespace mismatch, qualification limits and SDK reconstruction/reopen.
Forbidden starts leave preparation, reservation and intent unchanged. Independently
permitted observations and exact accepted-work recovery continue. Manual or already
attempted bytes cannot become automatic first-submit candidates; shielding cannot
authorize replacement funding or close unresolved attribution.

#### H09 — Lifecycle ordering when removing barriers

**Starting evidence:** C06–C08 scoped startup/deferred publication and C17 task ownership.

**Change:** For each moved or deferred startup/maintenance step, trace listener
reservation, shared Store ownership, capability installation, registry publication,
financial work and shutdown draining. Reuse existing snapshot publication and owned
drain mechanisms. Define which resources and accepted operations remain owned after
partial initialization, concurrent registry changes or publisher failure. Apply
this review to changed paths, not a new universal startup coordinator.

**Acceptance:** Bind failure and partial initialization create no unauthorized
financial work. Concurrent static/dynamic publication retains both changes. Failure
or cancellation preserves accepted-work drain and exact durable recovery obligations;
shutdown does not start a replacement owner. Notifications trigger fresh admission,
never grant authority. Keep task-exit containment distinct from the original C17
recovery acceptance and test the applicable actual deployment path.

### Complexity-guided refactoring targets

The completed 2026-10-17 source analysis compared with merge base
`638eca4e108f41b380deb143ed430e9a0f2c716c` on `main`: 18 production callables
received advisory warnings (25 metric warnings), with 23 production callables above
either threshold and no excluded files. At inspection, all analyzed source hashes
still matched the checkout. Saved evidence is under
`target/complexity-runs/20261017T133731Z-675114d9/`; retain this measured baseline
when recording follow-up changes rather than depending on the mutable latest link.

CogC > 20 and CycC > 40 are review triggers, not acceptance failures. Scores include
nested callable spaces and parse source feature branches rather than one compiled
configuration. “Unmatched” includes ambiguous identities and renamed/reorganized
functions; it does not mean all measured complexity was introduced by this branch.
These findings locate responsibilities to separate, not new demonstrated defects.
Do not extract helpers merely to lower a score or add a general workflow framework.

| Target and measured CogC / CycC | Concrete refactoring scope | Owning packages and behavioral gate |
| --- | --- | --- |
| [Deployment::initialize_wallets_mode](../../src/deployment.rs#L817), 58 / 64, unmatched | Separate establishment of Store/Base/SDK capabilities from pool and worker assembly. Replace ambiguous capability combinations with explicit usable outcomes while preserving independent static/Base service and one cleanup owner. | H08/H09: capability/context negatives, partial setup failure, bind-before-financial-work, exclusive Store ownership and owned drain. |
| [FundingWorker::schedule_owned](../../src/rotation/funding/scheduler.rs#L224), 40 / 55 (was 22 / 36), and [publish_current_observation](../../src/rotation/funding/scheduler.rs#L759), 16 / 48, unmatched | Separate lane scheduling, retained completion handling and financial publication decisions within the current coordinating owner. Make result/permit handoff explicit; separate health rendering from financial applicability. Do not turn each responsibility into an independently cancelling task. | H01/H02/H03: terminal-label refresh, local publication contention, cancellation, one-pass completeness, shutdown drain and independent lanes. Original C17 task-exit closure has its own gate. |
| [collect_refunds](../../src/treasury/refunds.rs#L160), 53 / 44 (was 20 / 21) | Produce an explicit historical-credit assessment before current-output discovery and total/new-credit selection. Preserve deduplication before revalidation and one traversal of the resident SDK history; do not copy the history or scan it once per operation. | H01/H04/H05: scoped and unidentifiable attribution, conflicting owners, missing/noncanonical historical credits, explicit zero versus missing totals, no duplicate credit, restart and retained input claims. |
| [ReconciliationOwner::maintenance](../../src/rotation/manager/maintenance.rs#L172), 37 / 47, unmatched, and [read_one](../../src/rotation/manager/maintenance.rs#L548), 25 / 34, unmatched | Group each lane's cursor, active identities, retry state and completion state under its existing owner. Make invalid combinations harder to construct without merging balance/history/repair resources or hiding different retry semantics behind a universal lane abstraction. | H03/H06: sparse traversal, priority fairness, scoped failures, one-pass completion, held historical I/O with current admission progress and cancellation. |
| [Store::replacement_support_page](../../src/rotation/store/evidence/replacement.rs#L457), 27 / 53, unmatched | Separate typed support-source traversal and support validation while retaining the bounded physical cursor, original/effective provenance coverage and final snapshot/liability revision checks. | H01/H03/H07: every support consumer participates in invalidation; stale activation rolls back; inactive/sparse rows remain bounded; disputed old proofs cannot regain authority. |
| [Store::capture_funding_delivery](../../src/rotation/store/funding_delivery.rs#L177), 11 / 55, unmatched | Separate candidate selection, authenticated saved-hint decoding, validated delivery expectation construction and read-ticket capture. Replace the internal provider-shaped JSON round trip through the common typed construction path described below. | H01/H05/H07: current-event/queued-correction fairness, exact correlation, malformed hints, saved-schema compatibility, revocation and publication races. |
| Remaining warnings: `source_step`, `collect_inventory`, `dispatch_pending`, `ManagedPool::pay_with_cover_observed`, `Store::prepare_bound`, `Store::return_inventory_page`, `Treasury::calculate_owned_refund`, deferred `Runtime::run`, `RunningDeployment::serve`, and wallet CLI `run_args` | Review their affected call chains in the packages above: source/quote progression and inventory under H01–H03; payment/preparation under H07/H08; returns and SDK calculation under H04/H05; deferred startup/serving under H09 and C17; CLI context dispatch under H08. Record a concrete mixed responsibility or missing gate before changing a function. | H06 records disposition for each warning: refactor with owning package and acceptance, or retain with its concrete cohesive responsibility and applicable evidence. A score alone does not require a code change. |

Prioritize funding scheduling/publication, then refund assessment, then deployment
capability establishment. This prioritizes slices within the dependency sequence
above; it does not defer H06/H05 acceptance setup or move implementation ahead of
its authority/ownership contract. Reconciliation and Store support/delivery slices
may proceed independently when their contracts are established. Never move the
final authority check outside its transaction or introduce a new asynchronous gap
merely to separate responsibilities.

For each changed target, record before/after CogC and CycC under the same analyzer
and comparison settings, review the resulting call chain, and report whether
complexity was reduced, redistributed or intentionally retained. Preserve the
behavioral gate as the completion criterion. New high-scoring helpers require the
same responsibility review; reducing the parent score alone is insufficient.

### Typed construction of internal delivery evidence

`capture_funding_delivery` currently builds provider-shaped JSON, including a
synthetic `UNKNOWN` status, then calls `restore_status_observation`. This reuses
validation but couples durable internal evidence to an external response format.
Treat removal of that representation dependency as an H01/H05 implementation slice,
with H04 historical compatibility and H07 publication checks retained.

**Change:** Identify the common validated facts and introduce a narrow checked typed
constructor used by both the external response adapter and authenticated durable
hint restoration. External parsing remains responsible for untrusted provider
syntax; saved hints retain their decryption/AAD, version and operation binding
checks. Both paths use the same semantic validation for accepted quote/source,
recipient, asset, amount, transaction identity and optional return claims. Remove
the synthetic provider JSON round trip after migrating its consumers. A typed
constructor must not be an unchecked bypass or a new form of spending authority.

**Acceptance:** Equivalent external and saved inputs produce equivalent validated
facts and the same rejection of conflicting/malformed correlation. Preserve absent,
explicit zero, positive and malformed return distinctions and historical diagnostics.
Cover wrong operation/source/quote/recipient/asset/amount, changed destinations,
conflicting current/queued candidates, unsupported saved layouts and restart. Keep
exact unique-event ownership, current revocation checks and final publication CAS.
Neither successful construction nor old saved evidence grants freshness, canonical
confirmation, new execution permission or closure of another financial obligation.
Use existing local fixtures; no extra provider probes or financial requests.

### Coverage and completion of the addendum

| Observed hazard | Owning proposed changes |
| --- | --- |
| Phases/projections substitute for obligations; evidence invalidation and scheduling disagree | H01 |
| Accepted work loses ownership before publication | H02; original C17 for actual task exits |
| Logical lanes conceal shared bottlenecks or unbounded scans | H03 |
| Historical readers inherit live contracts; retention/restoration loses financial knowledge | H04, with H01 obligation dependencies |
| SDK projections and fixtures substitute for financial facts | H05 |
| Validation changes its own source/executable; passing evidence is overgeneralized | H06 |
| Readiness/validation changes captured ownership | H07 |
| Authority crosses execution contexts | H08 |
| Barrier removal exposes publication, startup and shutdown ordering | H09 |
| Reorgs or late claims invalidate prior conclusions | H01, H04 |
| Concentrated scheduler/refund/deployment responsibilities and remaining complexity warnings | Complexity-guided targets under H01–H09; H06 records every disposition |
| Internal evidence depends on provider JSON representation | Typed delivery construction under H01/H05, with H04/H07 compatibility and publication gates |
| Mechanical feature/lint integration | H06 standard checks; no separate financial redesign |

The addendum is ready to guide implementation, not evidence that these changes have
landed. A package closes only when its concrete code/test/documentation delta and
acceptance limits are recorded and reviewed. Preserve any demonstrated unmet
requirement explicitly; do not close it through a broader completion summary.
H01 is the central architectural change, supported by H04's lifetime separation,
H07's atomic binding and H08's explicit capabilities. The remaining packages make
those contracts survive SDK, owner, resource and deployment boundaries. The
complexity-guided target inventory and typed delivery-construction slice are part
of their owning packages, not a separate score-reduction project.

### Rules to make explicit in AGENTS.md

The current instructions already cover retained ownership, fresh canonical
admission, scoped failures, exact preparation provenance, timer ownership, and
forbidden redundant budgets. Keep those rules and add the following narrow
requirements in their relevant sections instead of creating another broad safety
checklist. These are proposed documentation changes; this plan does not itself
modify AGENTS.md.

1. **Evidence invalidation and refresh:** Changes to evidence inputs must invalidate
   dependent observations and make their owner eligible to refresh, including
   after terminal workflow labels. Share the dependency calculation between
   validity and scheduling; test changed inputs and repeated unchanged unresolved
   observations.
2. **Accepted results through publication:** Retain accepted external results and
   their original concurrency permits through transient local validation,
   publication, and cleanup failures. Retry only the failed local boundary when its
   idempotence is established; do not repeat external or financial work.
3. **Physical work bounds:** Bound serialized maintenance by physical work, using
   indexed keyset traversal before filtering. Verify sparse-history behavior and
   query plans; a returned-row limit alone does not establish bounded work.
4. **Historical interpretation:** Read-only historical reporting uses saved-schema
   contracts and must not require current admission credentials, synthesize live
   execution authority, or rewrite committed diagnostics. Extend historical
   fixtures when evidence representation changes.
5. **Independent progress:** Claims of independent progress must account for shared
   sockets, permits, queues, and Store turns. Test the actual shared bottleneck
   with event barriers while unrelated production-path work completes.
6. **SDK and fixture contracts:** Treat SDK representations as stage-specific. Test
   exact serialized transaction facts through real calculation/proving where
   applicable, and keep readiness, provider status, canonical settlement, returns,
   and execution authority distinct. Fixture credits must be idempotent by
   operation identity.
7. **Frozen validation:** Freeze the checkout and executable for tests that pin
   either identity. Do not overlap source mutation or executable-replacing builds
   with those suites; coordinate CLI tests sharing a binary path and invalidate
   results if their declared pins changed.

8. **Atomic rebinding:** Validation or evidence publication that changes ownership,
   role, claim or generation must bind subsequent mutation to the resulting state
   atomically, or through the existing revision/CAS protocol. Test stale captures
   and validation-induced transitions; caller-side checks alone are insufficient.
9. **Context-specific authority:** Reusable financial machinery must require the
   capability for its actual action and execution context. Credentials, sync,
   recovered capital or a permit for a different action cannot supply it. Preserve
   exact accepted-work recovery separately from authority for new starts.
10. **Obligation lifetimes and revocation:** Cache retention and archival cleanup
    cannot discharge durable obligations. Preserve historical facts while revoking
    disproven current support; missing evidence and explicit zero remain distinct.
    Neither old proofs nor restored balances reconstruct missing authority.
11. **Lifecycle transitions:** Moving work across a startup or publication barrier
    requires review of bind, capability, publication and owned-drain ordering under
    partial failure and shutdown. Progress notifications require fresh admission.
12. **Acceptance scope:** Each completion claim must name the requirement and actual
    boundary its evidence demonstrates, plus its limits. Containment, in-owner
    recovery, task replacement and live qualification are distinct. Keep uncovered
    requirements open despite related passing tests.

Apply rules 1 and 10 with H01/H04; rule 2 with H02; rules 3 and 5 with H03;
rule 4 with H04; rule 6 with H05; rules 7 and 12 with H06; rule 8 with H07;
rule 9 with H08; and rule 11 with H09. Extend the relevant existing AGENTS.md
paragraphs instead of repeating them as another independent policy layer.

## Supporting review findings

These findings describe the source baseline that motivated implementation. Source
descriptions and proposed actions below preserve that review context; they are not
claims that the old defects remain in current HEAD. Use the gate inventory and
C01–C22 contracts for each implemented resolution and retained restriction.

Priorities are implementation order, not claims of a demonstrated loss of funds.
“Confirmed” means source control flow establishes the behavior; it does not mean
an incident was reproduced on live state.

### F1 — Diagnostic prose is part of recovery authority (P1, confirmed)

[`safely_unprepared` and `can_recover_facts`](../../src/rotation/store/funding.rs#L7)
inspect `last_error` prefixes. [`evidence_outcome`](../../src/treasury/recovery.rs#L35)
classifies errors by `cause.to_string()`. The
[source disposition writer](../../src/rotation/store/scheduling.rs#L80) preserves
selected prefixes while appending another diagnostic, specifically to retain
recovery eligibility. Startup has another diagnostic classifier in
[`StartupPath::disposition`](../../src/rotation/store/bootstrap.rs#L54).

This is an architectural defect: editing diagnostic wording can change behavior,
and later errors must preserve earlier text to retain authoritative facts. Replace
this with typed causes and provenance; render human-readable diagnostics from
those facts. Do not merely consolidate string comparisons into one helper.

### F2 — PREPARING loses the distinction between proposal and committed bytes (P1, confirmed)

[`Treasury::prepare`](../../src/treasury/send.rs#L99) commits preparation intent
before proposal creation, marks the in-memory owner unhealthy before SDK mutation,
then validates proposal shape/fees, reserves actual input plus fee, calculates,
and finally commits the snapshot and outgoing bytes. A proposal failure, fee
rejection or deadline crossing can therefore leave the same broad phase as an
interrupted calculation. The [funding worker](../../src/rotation/funding.rs#L260)
moves PREPARING without a pending operation to RecoveryRequired;
[recovery](../../src/treasury/recovery.rs#L239) then reports
`preparation_outcome_unknown` unless the old eligibility rules establish otherwise.

That refusal protects unknown legacy work. It also exposes a recoverability gap:
the calculate-only path and committed-byte broadcast capability provide stronger
boundaries than the phase records. Future operations should durably record those
boundaries so a failed proposal need not require manual incident repair. This
review does **not** prove that every historical PREPARING row can safely reset.

### F3 — Full-history repair can delay unrelated work (P1, confirmed mechanism; latency unmeasured)

The new [repair snapshot](../../src/rotation/store/evidence/repair.rs#L188) reads
all attempts in a single serialized store call, despite using keyset pages. The
[commit](../../src/rotation/store/evidence/repair.rs#L287) repeats that scan.
[StoreHandle](../../src/rotation/store.rs#L1142) executes these jobs serially, so
page-bounded memory does not imply bounded actor occupancy.

The [historical RPC scan](../../src/rotation/store/evidence/repair.rs#L342) is
awaited inside an ordinary balance read in
[maintenance](../../src/rotation/manager/maintenance.rs#L471). Four sufficiently
slow repairs in a pool can occupy its four balance-read slots. Existing shared
[RPC resource bounds](../../src/rotation/base/resources.rs) still apply, but do
not guarantee healthy-wallet balance progress through that occupied lane.

Implementation: stage 5 / G12; T03, T04 and T07. The target design specifies
separate repair scheduling and bounded database turns without a history deadline.

### F4 — Broad flags retain the consequence but insufficient incident evidence (P1, confirmed)

[Wallet invalidation](../../src/rotation/store/evidence.rs#L1351) persists broad
repair reasons, disputes dependent support and advances a runtime generation.
It does not retain a structured record of the incoming contradictory observation
and exact comparison that caused the block. The new repair audit records success,
not that original incident. Consequently automatic repair must conservatively
revalidate much more than a precisely recorded dependency might require.

Implementation: stage 4 / G03; T01, T02 and T04. See scoped disputes in the target
design; a stale generation alone is not a chain contradiction.

### F5 — Recovery decisions have multiple partially overlapping interpreters (P2, confirmed)

[`FundingFacts`](../../src/rotation/store/funding.rs#L100),
[`finish_if_proven` / phase derivation](../../src/rotation/store/funding.rs#L659),
[treasury recovery](../../src/treasury/recovery.rs), and
[startup disposition](../../src/rotation/store/bootstrap.rs#L54) each interpret
source, provider, destination and historical phase information. They have different
responsibilities, but duplicated classification can disagree about whether work
can progress, requires observation, or requires an operator.

One concrete review target is startup's early `operation.submission != EXPIRED`
branch, which reports Waiting before inspecting later recovery restrictions.
A non-expired operation can also have a persisted operator-only conflict. Add a
reachable fixture before calling this a reproduced infinite-startup bug; the
review establishes the branch-order mismatch, not its occurrence in the incident.

Share a pure assessment of authoritative facts and missing evidence. Keep serving
readiness, captured bootstrap completeness, source-accounting completion and
permission for a new attempt as separate outputs; do not collapse them into a
single ready/complete flag. Provider terminal facts are deliberately accumulated
monotonically, while phase derivation retains terminal/recovery phases. A shared
assessment must preserve contradictory provider facts rather than select the
latest response. Keep `finish_if_proven` as the authoritative completion gate;
the new assessment cannot bypass its source accounting and destination correlation.

### F6 — Poisoned owner recovery is restart-only (P2, confirmed and partly intentional)

The [treasury actor](../../src/treasury/actor.rs#L124) stops after an interrupted
mutation. [Close](../../src/treasury/mod.rs) discards potentially poisoned SDK
memory. This is the correct current safety behavior. It also makes a recoverable
proposal failure stop funding until restart rather than reconstructing the owner
from its authoritative committed snapshot.

Importantly, [serving](../../src/deployment.rs#L1492) already keeps listeners and
Base reconciliation alive after treasury/funding stops. Preserve that isolation;
there is no basis for claiming that this path currently shuts down all MCP service.
A supervised reconstruction path is appropriate only after F2 supplies a proven
reopen contract and queued/accepted command ownership is accounted for.
The contract must also cover [`sync_once` and `refund_address`](../../src/treasury/mod.rs),
which mark the owner unhealthy around SDK mutation/persistence. T35 must establish
reconstruction for these paths, preserving exact committed derivation bindings and
snapshots after uncertain commits, rather than proving only proposal recovery.

### F7 — Refund closure and uncertain swaps require separate design gates (P2, confirmed policy limitation)

[Recovery](../../src/treasury/recovery.rs#L90) recognizes RefundPending and returns
`OperatorRequired("refund_requires_review")`. [Canonical refund collection](../../src/treasury/refunds.rs#L96) can already
credit exact outputs while retaining operation-scoped binding problems. Receiving
and accounting for money is different from establishing that an original swap's
remaining obligations are closed or that a replacement attempt is authorized.

Specify and implement observation-only closure for an exact, fully accounted
terminal refund when its proof obligations are satisfied. Define accounting first.
Partial refunds, conflicting provider facts, missing bindings, unknown preparation
and unresolved confirmed swaps cannot be reset on that basis; keep observation
scheduled where useful and distinguish missing evidence from a contradiction that
requires an operator. No step should blindly reset all RefundPending or
RecoveryRequired rows.
Stage 7 additionally requires automatic shielding for eligible, exactly attributed
returns under current spending authority and fee safeguards. Reuse committed-byte
recovery and account for the return, shielding inputs/outputs and fee once. Observation
closure remains separate from permission to shield or start replacement funding;
manual shielding is not an alternative completion path for supported automatic cases.
G09 also decides whether current `shield_refund` job/binding prerequisites unnecessarily
couple an owned-output internal transfer to completed swap attribution. Confirmed
partial receipt is not inherently an input-spendability failure. Implement separation
only with proven ownership, input exclusion, current policy, privacy boundaries and
exact accounting carried through shielding; otherwise name the retained invariant.
T14/T30 must test this decision independently of refund closure.


### F8 — Quarantine and historical dependency lifetimes are too coarse to reason about locally (P1/P2, confirmed coupling; relaxation requires proof)

[`check_capture`](../../src/rotation/store/evidence.rs#L854) checks a wallet-wide
repair flag for both balance and resolution certificates. Thus quarantine blocks
publication of newly acquired nonce resolutions as well as spending. This is not
proof that releasing those liabilities would be safe, but it couples independently
useful observation/reconciliation to whole-wallet repair.

[`capture_evidence`](../../src/rotation/store/evidence.rs#L756) also retains both
confirmed and latest original readiness anchors while undisputed; ordinary
[Base acquisition](../../src/rotation/base.rs#L265) checks them again. A latest-only
historical reorg can dispute that readiness even when other confirmed evidence
remains intact. The original readiness decision genuinely used both balances, so
removing the latest anchor mechanically would be unjustified.

Make dependencies per decision and their lifetime the first design task. Separate
permission to
acquire evidence, publish an observation, release one liability, admit a payment,
publish readiness and retire. Specify when stronger canonical evidence can satisfy
an old dependency without rewriting original audit support. Preserve the debit
floor and prohibit release through a genuinely disputed resolution checkpoint.

### F9 — Original-endpoint reverification creates an explicit availability constraint (P2, confirmed)

[`original_endpoint`](../../src/rotation/base.rs#L406) requires the saved endpoint
identity still to be configured. [Historical verification](../../src/rotation/base/historical.rs)
uses that endpoint for original anchors and state. Removed endpoints or unavailable
historical state can therefore prevent repair even if another approved endpoint
can supply equivalent evidence. The current strictness preserves provenance; it
is not proof the original RPC must remain the only possible verifier forever.

First remove historical reads unnecessary for current decisions. For dependencies
that remain and cannot be satisfied by sufficient current evidence, define a
cross-endpoint reverification certificate: obtain the exact original facts from a currently
approved endpoint, retain original provenance, and record the additional verifier
and trust generation. Check chain, canonical anchors, values, nonce outcome and
required times as appropriate to each proof. Contradiction or unsupported history
must stay explicit. Exact historical reverification is distinct from canonical
replacement evidence for a new current decision; neither requires rewriting the
old record. These are proof-design gates, not permission for an
automatic transport bypass, implicit endpoint adoption or a new quorum policy.

### F10 — Existing repair eligibility excludes important wallet states (P1, confirmed)

[`repair_snapshot`](../../src/rotation/store/evidence/repair.rs#L188) accepts only
ACTIVE/RETAINED wallets with chain repair required, a completed funding job,
undisputed readiness, and no specified adverse provider facts. READY reserves,
incomplete funding jobs, and disputed readiness therefore fall outside this repair
path. Making its scan fairer does not supply recovery for those cases.

Implementation: stages 2 and 4 / G05; T18, T26 and T27. Current capacity and
funding attribution remain separate decisions under the target design.

### F11 — Automatic recovery needs an executable owner and wakeup (P1)

A typed Waiting result alone cannot ensure supervised owner reconstruction or
refund closure happens.
[`early_job_disposition`](../../src/treasury/recovery.rs#L63) also groups several
balance/configuration conditions under OperatorRequired. These are distinct from
irreconcilable financial evidence and need explicit reevaluation triggers.

For every durable halt, specify affected scope, next evidence/action, scheduling
owner, wakeup/backoff, and terminal disposition. Restored RPC access, newly
spendable funds, and restored effective policy must resume eligible work without
manual journal edits or a restart solely to clear a halt. Configuration reload is
not implied: document how a changed configuration becomes effective. Keep existing
observation running while new work is disabled or a true contradiction is scoped.

### F12 — Startup isolation differs from surviving a later worker failure (P1, confirmed)

[`Deployment::load`](../../src/deployment.rs#L526) performs managed bootstrap before
source loading. Preserving listeners after a later funding failure does not remove
this startup barrier. Audit whether an unavailable group must block independently
usable groups/listeners, including static-wallet service.

This plan proposes changing ordinary serving's global bootstrap-before-discovery
policy to readiness scoped to the affected wallet/group. Specify the lifecycle and
binding contract before implementation and update the repository guidance with the
change. Explicit wallet bootstrap and qualification keep their complete captured
inventory requirements. Coordinate provider publication with
[independent provider startup](deferred/independent_provider_startup.md); do not
create a second publication or supervision mechanism.

The contract includes `initialize_discovery_relay`, which can initialize wallets
before loading sources and propagate failure through deployment loading. Removing
only the explicit bootstrap wait leaves this path. G10/T22 must cover independent
source publication, deferred discovery ownership and the existing no-paid-retry
boundary after uncertain relay submission. Also specify ordinary serving with no
initially usable payer: discovery remains available where catalogs permit, affected
calls return typed unavailability, and fresh evidence restores calls in the same
process without rebuilding listeners or blocking the first call on global recovery.

### F13 — Treasury source history gates fresh sync results (P1, confirmed)

[Treasury sync](../../src/treasury/mod.rs#L674) rechecks previously confirmed and
expired source operations before publishing fresh balances or reconciling refunds.
The [historical query](../../src/rotation/store/treasury_reads.rs#L48) has no
live-obligation filter. A changed confirmation or expiry conclusion fails the
whole pass, even after scanning has completed.

Implementation: stages 2 and 4 / G07; T24, T27 and T32. Synchronized spendability
may replace historical enforcement only under the stage-1 live-dependency contract;
a scan alone is not spending authority.

### F14 — Historical refund validation can block the whole sync (P1, confirmed)

[Refund collection](../../src/treasury/refunds.rs#L113) revalidates every credited
refund, including already spent or shielded outputs. Missing transactions/outputs,
changed confirmation or value, and contradictory ownership can fail the entire
collection and propagate through sync. This is separate from F7's terminal swap
closure problem.

Implementation: stages 2 and 4 / G08; T14, T25 and T32. Join original credit,
shielding expenditure, current notes and remaining obligations; the test matrix
requires exact accounting without inferring swap closure from present funds.

### F15 — Initial usable roles need the same capacity separation (P1, confirmed coupling)

[Payment ownership](../../src/rotation/store/capacity.rs#L62) excludes ALLOCATED
wallets and requires structurally valid historical readiness. Initial
[role publication](../../src/rotation/store/capacity.rs#L138) also depends on recorded
destination readiness. F10's READY/ACTIVE/RETAINED scope therefore does not cover
every transition into usable capacity.

Implementation: stages 2 and 4 / G04–G05; T26. Preserve exact claim generations,
qualification restrictions and attribution; this is not promotion on balance alone.

### F16 — Pending outgoing work imposes a treasury-wide preparation gate (P1, confirmed coupling; relaxation requires proof)

[`require_spend_ready`](../../src/rotation/store.rs) rejects preparation while any
outgoing transaction is PREPARED; the atomic preparation commit enforces the same
restriction. [Funding refresh](../../src/rotation/funding.rs) also defers scans
while outgoing work exists, relying on its reconciliation owner. Removing historical
checks alone therefore does not remove this treasury-wide dependency.

Implementation: stage 1 resolves G11; stages 2 and 3 implement the decision under
T29. Serialized mutation ownership alone does not prove that unrelated notes must
wait for settlement. The retained SDK/persistence invariant must justify any global
gate; input exclusion, commit race checks and original work ownership remain required.

### F17 — Base reconciliation owner exit has no automatic resumption (P1, confirmed control flow)

[`maintenance`](../../src/rotation/manager/maintenance.rs) propagates inventory-read
errors, including priority, balance-page and history-page acquisition, out of its
owner loop. [`Deployment`](../../src/deployment.rs) logs a failed
`run_reconciliation` task but does not restart it. This differs from per-wallet RPC
failures that the running loop already retries. Durable obligations can therefore
remain without an active background observation owner until restart.

Implementation: stage 6 / G15; T37. Resume recoverable observation work using existing
ownership and backoff, preserving accepted store jobs and independent service.
Audit funding-scheduler exits under the same contract without assuming every failure
is restartable. Corruption or lost ownership requires a justified scope and explicit
stop, not blind reconstruction. This is a static control-flow finding, not a reproduced
private-state incident.

### F18 — Malformed live history can stop unrelated same-pool recovery (P1, confirmed control flow)

[`history_page`](../../src/rotation/store/history.rs) decodes a page into one
`Result`; a malformed pending authorization can fail the entire page. The maintenance
propagation described in F17 then stops that pool's observation owner. T32's malformed
irrelevant historical record does not cover this live-authority case.

Implementation: stages 2 and 6 / G16; T38. When wallet/obligation ownership is provable,
retain the affected liability and diagnostic while independent same-pool wallets
continue acquisition, reconciliation and safe admission. Do not discard malformed
authority or infer no exposure. If corruption prevents scope identification, document
the concrete invariant requiring a wider stop. Test both same-page isolation and
continued traversal so a bad row cannot repeatedly terminate or starve later work.

### F19 — Operational identity lookup depends on full financial reporting (P1, confirmed)

[`TreasuryConfig::runtime_id`](../../src/rotation/config.rs) resolves an omitted ID
through `store::status`. [`read_status`](../../src/rotation/store.rs) loads pools,
operations, jobs, refunds and sync observations; [pool reporting](../../src/rotation/store/reporting.rs)
evaluates exposure, historical readiness and payment ownership. Static-only
[`validate_static_identities`](../../src/deployment.rs) also uses full status.
An unrelated reporting/decoding failure can therefore prevent identity resolution
and startup before recovery begins, although the identity itself remains readable.

Implementation: stages 2 and 8 / G17; T32, T38 and T40. Replace these operational
report dependencies with narrow identity/schema/ownership queries. Preserve actual
binding checks and report malformed financial data; do not interpret it as absent.
This is confirmed control flow, not a reproduced private-state incident.

### F20 — Treasury initialization precedes independent service publication (P1, confirmed coupling)

[`initialize_wallets`](../../src/deployment.rs) constructs static clients but opens
the treasury before returning them. [`Treasury::open`](../../src/treasury/mod.rs)
reconstructs the SDK snapshot before spawning the store worker used by managed pools.
Moving the bootstrap wait into the background alone therefore leaves an earlier
initialization dependency that can still block independent service.

Before connection setup, `validate_resolved_funding` requires and validates treasury
and funding configuration globally. G18/T41 must include this declaration layer:
scoping connection failures alone leaves an earlier startup veto. Preserve globally
necessary identity/security/ownership validation and scope validly parsed
funding-only errors to dependent capabilities; do not silently default missing
settings. This is confirmed control flow, not a reproduced private-state incident.

`managed_connection_preflight` additionally bundles Base RPC construction with
treasury sync/submission configuration and NEAR credential validation before opening
managed state. SDK separation alone does not isolate these dependencies. G18/T41
must separate funding-only setup from existing Base service and scope explicit
configuration errors to dependent work, without silently supplying defaults or
changing endpoint policy. Swap observation retains its own required prerequisites.

Implementation: stage 8 / G18; T22 and T41. Distinguish independent static service,
funded Base service with usable journals/keys but unavailable Zcash SDK state, and
shared integrity/decryption/ownership failures. Stage 1 must decide whether and how
the store owner can start independently of SDK reconstruction without duplicate
ownership or abandoned accepted mutations. Separate chains alone do not establish
storage independence. Broader stops require their actual shared invariant, and
fault tests must begin before bootstrap supervision, not only after worker startup.

### F21 — A malformed funding job can stop the shared scheduler (P1, confirmed control flow)

[`funding_work_page`](../../src/rotation/store/scheduling.rs) decodes each selected
job and its facts into a single `Result<Vec<FundingWork>>`. One malformed job or
facts record fails the page. [`collect_inventory`](../../src/rotation/funding/scheduler.rs)
propagates that error out of the scheduler, affecting its shared observation lanes.
This is distinct from F18's Base history-page failure. Restart alone repeatedly
encounters the same permanently malformed row.

Implementation: stages 2 and 6 / G19; T42. Identify operation and liability scope,
retain the affected obligation and continue independent jobs and later pages.
Unknown shared treasury liabilities can justify blocking dependent preparation,
not silently treating the bad job as absent. Exercise same-pool and cross-pool
progress through the real scheduler. This is a source-control-flow finding, not a
reproduced private-state incident. G14/T45 additionally audit inventory membership
so derived terminal labels cannot hide authoritative outstanding work.

### F22 — Initial payment eligibility inherits a funding-target threshold (P1, confirmed coupling)

[`record_readiness`](../../src/rotation/store/funding.rs) requires observed capacity
to reach the original target or accepted output requirement.
[`payment_ownership`](../../src/rotation/store/capacity.rs) excludes ALLOCATED wallets
and requires historical readiness. This couples initial payment eligibility to a
larger funding threshold even when current capacity could cover a smaller payment.
G04/G05's separation of capacity from completion must explicitly remove this coupling,
rather than merely allow incomplete funding after the full target was observed.

Implementation: stages 2 and 4 / G20; T26/T43. Define the exact initial role/claim
transition and preserve pending funding ownership, payment authority and qualification.
A below-target payment neither proves delivery nor closes funding, transfers permits
or independently authorizes replacement allocation.

### F23 — Bounded unresolved exposure may replace some whole-wallet admission halts (design decision)

The plan's scoped-dispute model should evaluate retaining the full known liability
of affected authorizations while admitting against remaining current capacity.
This may remove the need to finish historical repair before every payment. It is
not a confirmed safe transition in the present implementation or a claim that every
quarantine can be removed.

Implementation: stage 1 resolves G21; stages 2 and 4 implement the proven outcome
under T19/T27/T44. Require complete affected-authority bounds, compatible current
balances, correct debit accounting and atomic stale-capture invalidation. Where
those cannot be established, document the concrete invariant requiring the wider
stop. Prefer this simpler exposure treatment when proven sufficient; do not build
historical verification solely to preserve an unnecessary whole-wallet veto.

### F24 — Foreground candidate selection can propagate a wallet-local exposure failure (P1, confirmed control flow)

[`eligible_payment_evidence`](../../src/rotation/store/selection.rs) propagates
`evidence::exposure` failures while scanning cached candidates. The
[exposure reader](../../src/rotation/store/evidence.rs) parses the persisted decimal
`wallets.unresolved_exposure`; a malformed value can therefore abort the scan before
an independent healthy payer is selected. Initial hinted
[`candidate_for`](../../src/rotation/manager.rs) also propagates that error.
Existing handling of typed malformed historical support does not cover every
wallet-local decoding failure.

`payment_candidate_with_evidence` first calls `merge_payment_certificates`, before
pool/candidate eligibility filtering. That merge propagates `BalanceObservation::compare`
errors for cached/completed observations of one wallet, so a comparison conflict can
abort consideration of another healthy payer. Extend G22/T46 isolation to this earlier
boundary, preserving the affected dispute and distinguishing shared-store failures.
These are source-control-flow findings, not reproduced private-state incidents.

Implementation: stage 2 / G22; T46 extends T38 through foreground selection. Keep
the affected wallet unavailable and diagnostic while considering other candidates.
Distinguish identifiable wallet-local failures from shared database or ownership
failures; do not catch arbitrary errors as absent candidates or zero exposure.

### F25 — Derived exposure reconstruction needs an executable contract (P1, specification gap)

[`exposure`, `add_exposure` and `subtract_exposure`](../../src/rotation/store/evidence.rs)
read and incrementally update `wallets.unresolved_exposure`. G14/T21's general
rebuild requirement must explicitly cover this field: malformed and overstated
totals can prevent service, while understated totals can weaken admission. This
review identifies a required recovery/test contract, not evidence that normal
production updates currently produce inconsistent totals.

Implementation: stages 1 and 2 / G14; T47. Define the authoritative attempt and
resolution-support inputs, treatment of invalidation, completeness proof, trigger,
owner and atomic publication against concurrent mutations. Repair projections
automatically where those inputs suffice; distinguish missing authoritative records.
Do not add a mandatory full-history scan at startup or on each payment, or another
permanent liability ledger. T46 proves isolation before repair; T47 proves actual
reconstruction and resumption rather than indefinite quarantine.

The reader accepts a syntactically valid aggregate without proving it matches its
inputs. T47 therefore needs a production detection/derivation contract before the
first affected admission, not only a rebuild algorithm after a supplied trigger.
Distinguish atomic-write failure, projection bugs and arbitrary database edits;
evaluate deriving exposure from indexed unresolved obligations before retaining
another validated projection. Explicitly bound detection claims where internally
consistent corruption supplies no observable contradiction.

### F26 — Accumulated provider observations can become permanent operational authority (design decision)

[`FundingFacts::provider_review_required` and `status_needed`](../../src/rotation/store/funding.rs)
use accumulated status flags to require review or stop polling.
`record_provider_outcome` retains terminal flags monotonically; phase derivation and
closure consume them. Retaining contradictory observations is useful audit evidence,
but does not by itself establish that each flag must remain a permanent veto after
exact canonical evidence discharges the relevant obligations.

Implementation: stages 1, 2 and 7 / G23; T13/T14/T48. Specify supported provider
terminal/provisional semantics, supersession rules, exact settlement correlations
and the remaining obligation behind every retained stop. Evaluate automatic closure
with unavailable or inconsistent provider status only where canonical evidence and
original bindings suffice. This is a proof decision, not a claim that the provider
permits corrected terminal statuses or that a current balance proves swap delivery.
Preserve observations without creating another durable status authority.

### F27 — Clock correction and resume need explicit progress acceptance (specification gap)

[Base evidence](../../src/rotation/base.rs), [treasury freshness](../../src/treasury/freshness.rs)
and [funding scheduling](../../src/rotation/funding/scheduler.rs) use chain evidence,
wall-clock checks and elapsed-time scheduling for different purposes. Existing
freshness and outage cases do not explicitly specify clock correction,
suspend/resume or restart with persisted polling times.

Implementation: stages 1, 2 and 6 / G24; T02/T20/T49. Audit each clock's owning
invariant and production reevaluation path. Do not infer a present bug or change
expiry/freshness policy merely from the use of different clocks. Prove that elapsed
time cannot substitute for canonical expiry, old evidence cannot become fresh
again, and temporary clock conditions cannot strand recovery after valid evidence
returns. Add no timer, deadline or permanent repair flag to address this test gap.
