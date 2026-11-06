# NU7 release readiness and bridge qualification

## Intent and scope

Own near-term consensus readiness, general Rust dependency updates, dependency
security, bridge evidence and retirement of
wallet-owned transparent address support under the
[SDK integration contract](../zingo-integration.md) and
[refund/recovery policy](../wallet-rotation.md#refunds-state-and-recovery). The operator reports NU7
activation on November 5, 2026; verify current network activation height, consensus
parameters, deployed indexer/submission capabilities and exact upstream cohort
before changing production dependencies. Do not hardcode that date or assume all
NU7 work is missing: current preparation already calls the pinned SDK's
upgrade-aware expiry rule and current sync has Ironwood-related integration.

This is unfinished release work, not a completion claim or authority for funded
execution, paid API calls, production key access, upstream submission or deployment.
Capture a fresh exact baseline when the track starts. Preserve original accepted
bytes, identities, input ownership, liabilities and stable storage/environment
contracts. Follow [SDK integration](../zingo-integration.md),
[wallet reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy),
[network policy](../network-egress.md), [development](../development.md),
[testing](../testing.md), [release builds](../reproducible-builds.md) and
[vendor provenance](../../vendor/README.md).

## Execution baseline and commit gates

Working branch: `nu7_release_readiness`. Starting baseline:
`571eba2f8a8f4df9d3f3e89d5992ccee93f81154`. The unrelated untracked `TODO`
is outside this work. Production keys, funded calls, submissions and deployment
remain outside current authority. Public source retrieval and unfunded diagnostics
are permitted; private evidence inspection requires the reviewed recipe first.

The N1–N6 assessment boundaries below remain acceptance requirements. Execute
these coherent commits in order, refining candidate details from exact sources:

| Commit | Boundary and permanent documentation | Acceptance and review gate |
| --- | --- | --- |
| C0 | Retain baseline, implementation boundaries and review evidence in this plan. | Independent read-only design review before substantive implementation. |
| C1 | N1: migrate the complete SDK/backend cohort to verified v8 tag, carry necessary Pepper patches and provenance, adapt offline/indexer/send/sync consumers; assess general workspace and protoc updates. Update SDK integration and vendor ledger. | Exact source comparison, vendor integrity, production compiler and all-target Clippy, compatibility and affected lifecycle/optimized checks. Independent exact-SHA and CogC/CycC review. |
| C2 | N2: reproducible local advisory assessment of active lockfiles and path sources, resolve signing advisory with compatible migration or evidence-based exposure/release disposition; update development/vendor/testing/tooling guidance. | Exact source and signing-call-chain assessment, independent vectors and payment compatibility; exact-SHA review. Dependency gate combines C1/C2 and all fixups. |
| C3 | N3/N4: verify deployed refund format/receiver and published service semantics; fix demonstrated gaps only, with network/wallet/live-runbook handoff. | Public unfunded route checks, faithful contract regressions where code changes, independent exact-SHA review. Bridge gate combines all preceding work before dependent retirement. |
| C4 | N5: reviewable inspection and qualification recipe, preferring retained evidence and simplest supported controlled-refund trigger. | No production key access or spending until separate operator authority. Record precise missing evidence and execution path; review exact recipe. |
| C5 | N6: retire ordinary wallet-owned transparent behavior only when receiver and funded evidence support it; preserve explicit legacy recovery and accepted obligations. | Fresh/imported/reopened/seed-recovery negative RPC/address controls, shielded discovery/spendability, legacy/restart regressions; exact-SHA and applicable complexity review. Blockers remain explicit if prerequisites are unavailable. |
| C6 | Final integration and documentation handoff; delete completed plan only when all release conditions and reviews pass. | Independent full-chain combined review of all commits/fixups, required ordered checks, exact cleanup-SHA/final-branch review. |

Keep fixups unsquashed and review each exact SHA. Run cheap affected contracts and
`check-static`, complete applicable complexity review, then `check-clippy` and
`check-compat` before remaining named runtime stages. Carry forward unchanged
passing evidence. Optional native consensus, real Tor, funded and reproducibility
qualification require their actual runbooks; unavailable evidence stays missing.

### Evidence and remaining work

- C0: exact `411ecdce2eaaa340b0a9a7a89ce5afc72d53e91b` independently
  reviewed; no material finding. Design review selected upstream's coordinated
  1000-block retention and previous-session-height fix while retaining local
  conservative recovery; preserving 100 would add an unnecessary divergence.
  Exact mainnet activation/branch/expiry regression selected to cover the
  prerelease schedule gap. Implementation validation remains outstanding.
- Refund design: assess documented EXACT_OUTPUT excess-input refund as a simpler
  candidate before a duplicate-deposit harness; applicability and fees still need
  route evidence. Dry quotes establish format acceptance only, not returned
  deadlines or terminal behavior. No funded execution authorized.
- Verified public v8 tag resolves to `81d799775f11e882896e8710e8af12d6d4d29a41`
  through Git remote metadata and a read-only checkout under ignored `target/`.
  Release notes and source cohort must agree before migration.
- N5 authority and actual refund evidence remain outstanding; N6 is conditional.

### Added timing scope

The operator additionally requests review of Zcash polling, expiry, confirmations,
reorg/rescan timing and faster swap resolution, including a requested sub-hour
window. Independent design review selected a separate timing commit after C1/C2:
healthy demanded scans follow SDK target spacing at the next established height;
source/status success follows configured polling without the universal minute
floor; Base credit cadence and independent failure backoff remain separate.
The quote preference default becomes 1800 seconds, retaining overrides, returned
deadlines and accepted bytes. Confirmation counts and SDK finite expiry remain
unchanged: 40 pre-NU7 / 120 NU7 blocks each target about 50 minutes. None of these
changes manufactures bridge terminal outcomes or guarantees sub-hour abandonment.
Update wallet/configuration guides and both timing/resource inventories; test
activation, completion-based cadence, independent backoff and retained ownership.

### C1 review and validation

C1 `69d7d7cd85b7d212b6751bc0c25d498c827e2d9d` independently reviewed.
Material findings: removed mempool timeout argument, removed SDK diagnostic
variants/new panic payload, and changed shard-tree checkpoint fixture contract.
Fixup adapts the adapter/test signature, sanitizes `SyncPanicked`, and uses a real
one-block merge fixture with prior empty checkpoints in all pools and no RPC.
Also corrects copied-code origin wording and upstream clear-test expectation.
Vendor reconstruction passes for all 35 exact upstream files. Full compiler,
Clippy, runtime and numerical complexity evidence remain pending; the independent
manual review examined Pepper scan ordering/drain, checkpoints, conservative
recovery, reorg retention, offline/send/expiry and original-work ownership.

## Ordered assessment boundaries

| ID | Work and required exit evidence |
| --- | --- |
| N1 — activation/cohort and general Rust updates | Independently assess actual NU7 activation and update the Rust dependency graph generally, including application, build, development and workspace/tooling dependencies and standalone tool manifests/lockfiles. Assess released Zingolib v8.0.0 and its compatible Pepper, Zcash backend/crypto/proofs and indexer cohort as the upgrade candidate. Resolve related direct/transitive constraints together rather than change only the Zingolib revision. Review signing, receive/decrypt, sync/reorg, migration, transaction serialization, fee, expiry, source confirmation and recovery consumers across activation. Record selected versions, retained pins and concrete incompatibility/blocker decisions; blanket unpinning or forcing every crate to its newest incompatible release is not acceptance. Preserve every existing vendor blocker and provenance check. No fixed legacy block interval or application-maintained expiry algorithm. |
| N2 — B1 security updates | Reassess the whole locked production dependency graph, including path-vendored sources. Explicitly resolve the documented `crypto-bigint` signing-cohort advisory by a compatible migration, separately reviewed backport, or concrete exposure analysis and supported release disposition. A Zcash/NU7 upgrade does not automatically fix an EVM signing advisory. Preserve exact EIP-712/address/signature and transaction/state contracts; no incompatible forced manifest override or new vendoring without its required discussion. Establish reproducible local advisory evaluation for exact source versions even while Linux CI automation remains deferred. |
| N3 — refund format and receiver selection | Determine the deployed native-ZEC/Base-USDC route's support for shielded refunds and how it selects the actual receiver. Recheck dry quotes with public unfunded shielded-only refund UAs, including the exact receiver set generated by the selected SDK. Distinguish shielded-only UAs from mixed UAs that can receive transparently. SDK tests, bridge-source/Zodl precedent and historical dry acceptance are useful distinct evidence, not proof of deployed funded refund delivery. Preserve transparent source-deposit validation and existing quote/refund bindings; do not silently fall back to transparent refunds. |
| N4 — R7 service contract | Obtain current Zcash-specific 1Click/bridge semantics for deposit first observation, inclusion/confirmations, returned deadline versus requested preference, late deposits, refund fees/minimums, and unresolved unused `PENDING_DEPOSIT` requests. Identify terminal outcomes and transaction evidence; absence of an EXPIRED status cannot be replaced by a local timer. Map the contract to current source/credit/status/retirement ownership and repair only demonstrated gaps. |
| N5 — live qualification | Prepare an exact reviewable unfunded qualification recipe, then obtain the existing separate operator authority before spending. First inspect any suitable retained evidence; if it does not establish actual shielded refund delivery and SDK spendability for the selected route/cohort, qualify a controlled refund. Assess deliberate second funding of the same exact-output quote as the candidate trigger described below, without assuming it guarantees a refund. Retain exact UA receiver set, request/returned quote, every source identity/txid, provider outcome, refund transaction and SDK discovery/spendability. Test selected near-deadline/late-arrival cases only within separately reviewed authority. Keep private evidence out of Git. |
| N6 — retire wallet-owned transparent support | Use N3's deployed receiver contract and N5's actual refund evidence to establish whether this upgrade can fully disable wallet-owned transparent address generation/publication, refund selection, discovery/scanning and shielding. Audit fresh wallets, imports, reopened snapshots, SDK defaults, CLI paths and recovery consumers. Select the smallest supported SDK configuration or code removal that achieves shielded-only operation; improving isolation of transparent queries is not the objective. Preserve accepted refund bindings, original signed work and recoverable owned funds through an explicit migration/recovery path for legacy transparent obligations. Exit with implemented retirement and focused acceptance evidence, or concrete blockers and remaining work; accepting shared-indexer clustering does not complete this track. |

N1 and N2 inform any coherent dependency update; N3 and N4 inform bridge behavior
and N5's test cases. The cleanup plan's one-hour requested quote preference never
rewrites old quotes or signed bytes and is not a guaranteed refund completion time.
SDK block expiry and the provider's returned wall deadline remain separate.

The published [Zingolib v8.0.0 release](https://github.com/zingolabs/zingolib/releases/tag/zingolib_v8.0.0)
is a candidate, not an already-qualified replacement. Its release notes describe
the NU7 Zcash cohort, Pepper 0.7.0, changed reorg retention and one-session indexer
APIs. Inspect the exact tagged sources and resolved revisions against every
retained adapter/patch and consumer; a larger upstream rollback window alone does
not establish that the local reorg correction or entire Pepper checkout can retire.
Include dependency migration changes and source-matched compatibility, network,
SDK, financial lifecycle and vendor-integrity evidence in the implementation
boundaries. N2's advisory disposition covers the resulting complete graph; a
Zcash update does not automatically fix the separate EVM signing cohort.
Include `tools/complexity` in the root workspace graph and `tools/protoc`'s
independently locked graph. Assess path-vendored versions and patch consumers
explicitly; provenance-only vendor lockfiles are not the application's active
dependency resolution and must not be regenerated as though they were.

N1's selected SDK/indexer cohort informs N6's retirement design; N3 and N5 establish
whether deployed refunds can avoid transparent receivers. Current new swaps already
derive Sapling+Orchard unified refund addresses without a transparent receiver,
while existing bindings are reused unchanged. Fresh wallets disable transparent
discovery, but imported/reopened settings and legacy recovery still support it.
These facts do not establish actual deployed refund delivery or complete retirement.

N6 supersedes the earlier per-address isolation assessment. Its goal is removal of
wallet-owned transparent use and scanning, not a different circuit for each query.
Paying a bridge-owned transparent source-deposit address from shielded funds is a
separate outbound contract: it does not require a treasury-owned transparent refund
receiver or address scan. Identify any actual SDK dependency before removing
transparent primitives needed to construct or validate those outbound payments.

Before removing legacy handling, identify disclosed transparent/mixed refund
bindings, possible late receipts, existing owned transparent funds and pending
shielding transactions using established state. Define a supported migration or
explicit recovery path that preserves their original identities and accepted
bytes without rewriting old quotes, deleting keys/evidence or abandoning funds.
Do not turn legacy compatibility into permanent ordinary transparent scanning by
default. If the deployed route or unresolved legacy obligations prevent complete
retirement, record the exact blocker and next action rather than claim completion
or substitute a query-isolation improvement.

N6 acceptance must demonstrate shielded refund discovery and spendability without
transparent scans on fresh, imported and reopened supported states, with negative
controls proving no transparent discovery RPCs or new transparent receive/refund
addresses are produced. Inspect SDK transparent key/address derivation separately
from published addresses, and known-address queries separately from gap probing;
disabling one does not demonstrate that the others are absent. Check recognition
of owned transparent outputs in otherwise wallet-relevant shielded transactions
as well; disabled discovery alone does not prove transparent support is retired.
Exercise restart, snapshot/seed recovery and the selected
legacy migration/recovery path, preserving accepted pending work and late-receipt
coverage. Distinguish any remaining offline legacy recovery from ordinary serving.
N5 must retain evidence of the actual refund receiver/pool and SDK-observed funds;
a UA string, accepted quote or successful forward swap alone does not qualify
refund delivery. Funded evidence retains its separate operator authorization.

## Controlled refund qualification candidate

First inspect existing retained swap/refund evidence: the operator reports that
one of the existing wallets may already hold a shielded refund from the earlier
duplicate-deposit experiment. Identify the actual wallet and bind its saved quote,
refund receiver, source transactions and returned funds before proposing another
funded test. Prefer existing evidence when it establishes the actual deployed
receiver/pool, attribution and SDK spendability. A historical refund need not be
re-created solely because dependencies change: separately qualify discovery and
recovery of that receipt with the selected upgraded SDK and transparent scanning
disabled. Document missing evidence or relevant service/receiver changes before
deciding that a new funded refund is needed. Preserve original state and private
evidence; any production-key access still requires the reviewed inspection recipe.

The operator reports that a second deposit using one exact-output quote previously
caused a refund. Treat this as a useful experiment to reproduce, not a guaranteed
service contract. N4 must assess whether that route accepts quote reuse,
how it accounts for additional deposits, which refund binding it uses and how
status/transaction evidence distinguishes the two deposits. Record unresolved
semantics as experimental questions in the reviewed recipe rather than claim a
guaranteed refund. The experiment is not
an automatic retry, nor may it be implemented by changing ordinary funding recovery
to send another deposit after an uncertain outcome.

Prepare a concrete recipe using `research_desk` or another operator-selected
treasury wallet with sufficient SDK-confirmed spendable ZEC. Confirm the actual
wallet/state mapping before selecting it; do not infer a ZEC treasury from an EVM
wallet label. Record the exact executable/cohort, state owner, route, shielded-only
refund UA and decoded receivers, USDC recipient, quote mode/identity, returned
deadline, amounts, network/refund fees and the intended deposit count. Review the
two proposed transfers and their financial authority before loading production
keys or spending. No separate permanent ZEC cap or ordinary-funding budget is
introduced by this qualification recipe.

Identify an execution path that can represent two intentionally authorized source
transactions against one quote without violating existing job/operation bindings.
Current allocation permits do not automatically authorize an extra deposit to a
completed quote. If a focused production-owner harness or CLI change is necessary,
design, implement and independently review it before execution; do not add a
parallel financial ledger or weaken normal recovery to make the test possible.

Use one exact-output quote and, if the reviewed recipe supports the method,
observe the first deposit's swap outcome before deliberately making the separately
authorized second deposit to its deposit address. Recheck whether the saved quote
is still usable at that boundary; do not invent an extension or replace it with
a fresh quote while claiming to have tested reuse. Journal each transfer's intent,
inputs, exact signed bytes and identity before disclosure through the existing
treasury owner. Do not rebroadcast the first transaction to simulate a second
deposit or bypass exclusive Store/SDK ownership with an ad hoc signing script.
If either outcome is unknown, retain its liability and observe/reconcile it;
absence of a result grants no repeat transfer or additional refund experiment.

Retain both source transactions, the first swap's actual USDC outcome and all
provider status/refund evidence, including any unexpected second swap or pending
deposit. A provider `REFUNDED` label alone is insufficient: establish the returned
ZEC transaction and the SDK-decrypted note's actual pool, amount, confirmation and
spendability at the bound shielded refund receiver, with no transparent receipt
or shielding transaction used as a substitute. Correlate the received funds with
the experiment without exposing keys or private financial evidence in Git.
Demonstrate the selected upgraded SDK can discover and recover the refund without
transparent address scans, including restart/recovery evidence required by N6.
Report a pending, failed or unsupported trigger as incomplete qualification rather
than silently buying another attempt. Use the existing live runbooks and record
any purpose-built recipe gaps before execution; the general provider runner is
not presumed to already support duplicate Zcash deposits.

Before implementation, revise these boundaries into coherent ordered commits for
the actual selected cohort/contract, with owning documentation and tests in each.
Have an independent read-only design reviewer challenge necessity and simpler
alternatives before affected code. Require non-author exact-SHA/fixup reviews,
unsquashed history and source-matched CogC/CycC review under
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline). At the dependency/
bridge gate and final integration, inspect all preceding commits and fixups through
full signing, network, financial and recovery chains; analyze repeated assumptions
and testing gaps and resolve material findings before dependent work.

Use ordered production/compiler/all-target Clippy, compatibility, synthetic lifecycle
and affected optimized-profile checks before broad/optional qualification. Native
consensus, live-funded, real-Tor and reproducible-build claims need their actual
runbooks and evidence, not fixture watchdogs or a newer version number.

## Completion and handoff

Keep actual consensus/SDK behavior in SDK and wallet guides, exact advisory status
and update constraints in development/vendor guidance, qualification scope in testing
and its runbooks, and current provider deadline semantics and the N6 shielded-only
support/migration contract in network/wallet guidance.
AGENTS.md already links these owners. Linux CI infrastructure remains in the
[deferred release plan](deferred/release_followups.md), not a hidden prerequisite
for resolving current dependency/security findings locally.

Unmet release conditions stay explicit in this plan with a concrete next action;
guides retain truthful concise limitations and links. After completion and verified
handoff, remove the plan without an archive; independently review the cleanup SHA
and final combined branch, including previous fixups and financial evidence handling.
