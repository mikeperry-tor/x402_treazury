# Review findings cleanup

## Intent, baseline and scope

Implementation started on branch `review_findings_cleanup` at
`afaf26b226bf46e9ecf0a0eb9d71d782c3061be0`. The initial tracked tree was clean;
untracked `TODO` and `x402_treazury_review.md` are unrelated operator files and
remain untouched. Independent read-only G0 reviews are in progress for C01–C04,
C05/C06/C08, and C07 respectively. C01 is implemented pending validation and exact-SHA review; C02–C10 remain
proposed. No validation completion is claimed. This execution baseline supersedes the earlier
planning baseline for source comparisons without changing the selected scope.

Implement the operator's selected production-release changes from the review of
`3ceb88f2e41038c4f6ef6959494c2f501373f1cd`, using current source rather than the
review's historical locations or proposed fixes. Planning baseline:
`e3b151d7651c1d6210c6ebd11a42b1e1c0e66ba1`. Capture the implementation branch,
HEAD, working-tree changes and source-matched validation evidence again before
starting. Preserve unrelated tracked, untracked and ignored files and all private
financial evidence. This document describes proposed work, not implemented behavior.

The operator-restored [original review](x402_treazury_review.md) is historical source
input. Its recommendations are not implementation instructions, and its old code
locations, counts and verification claims do not describe the current baseline.
The decisions and coverage map here, together with maintained guides, govern work.

The selected changes are Rust support/build arithmetic, explicit payment and
listener authority, Store failure handling, removal of the foreground count gate,
wallet-owned metadata transports, unnecessary transparent discovery, supported-OS
private storage, and a one-hour requested NEAR quote window. Accepted policy stays
in owning guides. Unfinished work moves to a focused active or deferred plan;
documentation cleanup must not pretend a deferral fixes a valid finding.

The operator reports NU7 activation on November 5, 2026. Treat that as an urgent
reason to execute the separate [NU7 release-readiness plan](nu7_release_readiness.md),
not as an activation date/height to hardcode. That plan owns consensus/cohort
verification and B1's dependency-advisory assessment. Linux CI automation and
Windows implementation remain [deferred release work](deferred/release_followups.md).
Neither this plan nor those plans authorize funded swaps, paid API calls, secret
access, commercial signup, upstream submission or deployment.

## Preserved contracts and owning guides

Read [AGENTS.md](../../AGENTS.md), [development](../development.md),
[testing](../testing.md), [build entrypoints](../../scripts/README.md) and
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline) before work.
Area-specific owners are:

- [Wallet rules, observations and reconciliation](../wallet-rotation.md),
  [wallet administration](../wallet-cli.md), [SDK integration](../zingo-integration.md)
  and [lifecycle coverage](../../tests/LIFECYCLE.md).
- [Network policy](../network-egress.md), [local resource inventory](../local-resource-limits.md),
  [runtime architecture](../architecture.md), [configuration](../configuration.md)
  and [startup coverage](../../tests/STARTUP.md).
- [Agent sources](../agent-sources.md), [provider policy](../../providers/README.md#provider-policy),
  [source maintenance](../provider-updates.md), [portability](../development.md#portability)
  and [storage assessment](portability_fixes.md).
- [Dependency provenance](../../vendor/README.md), [reproducible builds](../reproducible-builds.md)
  and [documentation policy](../README.md).

Preserve journal-before-disclosure; exact accepted inputs, identities, signatures,
transaction bytes and liabilities; no top-ups or replay from uncertainty; SDK-owned
ZEC spendability; canonical Base proof semantics; immutable source/refund bindings;
authenticated listener ownership; and independent endpoint availability. Store
death, disconnect, cancellation or shutdown cannot report abandoned work as success.
All production HTTP/gRPC constructors remain in `src/network.rs`, with HTTPS,
public-destination checks, identity-bound pooling, remote DNS and no direct Tor
fallback. Keep stable formats, Tor token domains and `X402_*` environment contracts.

Remove responsibilities before adding abstractions. Do not replace the removed
foreground gate with a larger count, queue, whole-call deadline or retry framework.
Do not derive financial authority from advisory balances or introduce ZEC budgets.
Any new check needs its concrete protected authority/resource and existing guards
reviewed first. New vendoring requires the dependency-policy discussion before it
is introduced; these changes do not presume another vendored package.

## Selected decisions

| Finding / area | Current behavior and selected outcome |
| --- | --- |
| B10 Rust baseline | Raise `rust-version` from 1.94.1 to the already pinned 1.99.1. This selects the supported baseline; it does not claim older compilers demonstrably fail or change the compiler pin. Zingolib/Pepper declare no MSRV and their pinned upstream toolchain is 1.99.0. |
| B10 release arithmetic | Enable `overflow-checks = true` in the root release profile, retaining optimization and explicit checked/wrapping/saturating arithmetic. Inspect dependency effects and optimized financial consumers; no unmeasured performance claim or blanket panic recovery. |
| S1 | Reject empty spending-cap strings in every config/env/CLI/library path. Retain explicit `none`/`off` unlimited authority and existing numeric limits/defaults. Missing and present-but-empty values are different. |
| C1 | Missing, empty or whitespace-only required tokens fail closed, including the conventional token variable. Only explicit `auth = false`/`--no-auth` disables HTTP authentication. Reject configurations that disable both authentication and Host checking; preserve loopback binding and independent listeners. |
| A6 | Supervise Store worker termination in treasury and store-only serving. Report capability loss promptly, stop dependent admission and fail affected operations truthfully while independent static/other-Store work remains usable. Do not restart/replay accepted closures after panic. |
| A3 | Remove the 128 process-wide foreground semaphore and its task-local permit machinery. Preserve accepted-work ownership/tracking, draining, journal authority and justified operation-specific bounds. This removes a shared rejection point, not all memory/DoS risk. |
| P3 | Use the relevant EVM wallet/recipient identity for owned NEAR token metadata and single-wallet Base canonical headers, reads and rechecks. Audit multi-wallet consumers before selecting their representation; never assign sibling work to an arbitrary wallet. |
| P5 | Keep wallet-owned discovery on the relevant wallet identity. Coinbase already uses the selected wallet documentation identity, and cross-origin participating transports already isolate scopes. Verify callers and same-origin reuse; do not add a redundant shared discovery domain or claim a new cross-origin isolation fix. |
| P4 | Verify current production quote acceptance of our exact shielded-only refund UA format, then disable speculative transparent discovery for newly created shielded-only application wallets. SDK defaults currently derive/query unused transparent addresses. Preserve justified legacy/imported discovery and late receipts; no transparent refund fallback. |
| R7 | Change the requested quote-window default from 7,200 to 3,600 seconds. The returned executable quote deadline and SDK-derived signed block expiry remain independent. Use consensus/SDK parameters across NU7; no legacy seconds-per-block conversion or local freshness margin. |
| B8 | Fix private creation/reopen and inherited-ACL exposure on supported Linux/macOS through a small shared boundary covering actual consumers. Unsupported operations must error instead of succeeding through permission no-ops; this does not implement Windows support. |
| S2 | Accept the existing structured insufficient-funds traversal. Typically about two or three wallets are expected to have sufficient funds, potentially fewer without large wallets; this is operational rationale, not a maximum. Low/unknown estimates can still be eligible. Every signature remains chargeable, each captured identity is visited once, and traversal never allocates, tops up or waits for funding. |

P3 does not promise unlinkability from a service receiving common API/session
credentials, identical content or correlated timing. P4 quote acceptance does not
establish actual refund delivery. Preserve these distinctions in user-facing claims.

## Coverage of the original findings

Every original finding has an owner below. "Addressed" means the current code and
maintained contract cover the original defect, not that all broader qualification
is complete. Mixed findings retain their separate dispositions; planned work is
unfinished until its acceptance evidence exists. This map is temporary planning
material and is removed with the completed plan, not kept as a report archive.

| Disposition | Findings and owning evidence/work |
| --- | --- |
| Already addressed | S3, S4, S7, A1, A2, A4, A5, P6, R4, R8, R10: current payment/expiry, source-recovery, HTTPS, catalog, SDK/proving, scoped-failure, private-preflight and OpenAPI contracts/tests in the owning guides. Do not reopen these as blanket caps or rewrites. |
| Material defect addressed; retained behavior is intentional | C2: loopback validation and Bearer parsing fixed; CLI token input retained with explicit exposure warning in configuration. R2: registry rebuild/locking improvements; no count cap selected under agent-source/resource policy. R9: regex/client-lock/admission fixes; exact history-derived qualification budgets remain intentional. B5: typed financial control flow and named dead arms corrected; safe diagnostic-label matching is not queued for a blanket rewrite. |
| Documented non-action decisions | S2: accepted traversal in wallet guidance. S5/S6/R1: treasury capital, compound quote economics and one-time advisory-credit rules in wallet guidance. A7: accepted-result publication ownership in wallet/SDK guidance. P1/P2: limited experimental cover contract. R5: explicitly configured paid probes in agent-source guidance. B3/B6: complexity and focused test-hook discipline. B4/B7/B9: no demonstrated functional defect or blanket refactor; rationale in development. Other B10 lock/pin/hash/panic-count/key-placement observations: development and wallet-administration rationale. |
| Selected implementation | S1, C1, A3, A6, P3/P5, fresh-wallet P4, supported-platform B8, R7 requested default and B10 MSRV/overflow: C01–C08 here. P5 verification may require no behavior change where wallet identity is already correct. |
| Near-term follow-up | B1 dependency security, R7 external service contract and shielded-refund/NU7 qualification: N1–N5 in the NU7 plan. |
| Intended deferred work | B1 Linux CI and B8 Windows: D1/D2; R3 provider-output assessment: D3; R6 operator visibility: D4; B10 archive provenance: D6; residual legacy P4 discovery assessment: D7. Snapshot pruning is not selected. B2 upstream patch maintenance remains owned by the vendor ledger/retirement plan; its A2 correctness defect is addressed. |

Provider-output and legacy-discovery assessments are actual future work with a
required evidence-backed decision, not conditional placeholders. Their eventual
decision may select a focused fix or document acceptance of the current boundary.
Do not claim they are fixed or accepted before that assessment is complete.

## Ordered implementation commits

Stable IDs below name future coherent commits, not already-created commits. Each
commit includes its owning documentation and focused regression evidence. Resolve
the applicable independent design findings before implementing the affected choice.

### C01 — Supported compiler and release overflow behavior

Update root `Cargo.toml` and maintained build/dependency guidance for MSRV 1.99.1
and release overflow checks. Preserve `rust-toolchain.toml`, lockfile/provenance and
reproducible-build constraints unless separate NU7 work justifies a reviewed change.
Check effective release-profile settings and real optimized application consumers;
ordinary debug tests alone do not exercise the changed profile. Run the production
compiler/all-target warning gate and compatibility checks, then relevant optimized
payment/treasury coverage. Review arithmetic panic failure paths with C03; do not
convert financial `checked_*` errors to panics. Assess actual proving/scanning cost
only if a material regression appears. Update development, testing and release
guidance in this commit, not at the end of the plan.

### C02 — Explicit payment and listener authority

Implement S1 and C1 together across `SpendPolicy`, static/managed config,
standalone CLI/env, deployment validation, bind/startup and inspection consumers.
Verify missing versus empty cap/token values, explicit unlimited/zero caps, custom
and conventional token names, explicit no-auth, and both Host-check/auth disable
paths. Fail applicable startup validation before signing/funding or provider I/O;
do not require secrets for offline inspection that previously needed none.
Preserve stdio, loopback validation and case-insensitive Bearer scheme handling.
Update README, configuration, wallet CLI where relevant, `.env` templates and
shipped examples without reading/writing real secrets. Remove stale optional-token
and empty-unlimited guidance in the same commit. Tests must assert absent financial
side effects, truthful errors and independence of unaffected configured endpoints.

### C03 — Store capability-loss ownership

Monitor the exact Store worker handle in both `StoreOnlyOwner` and treasury
lifecycles. Independently challenge the smallest failure-propagation design before
implementation. Inspect payment admission, reconciliation, funding, SDK calculation,
completed-result publication, CLI/recovery and shutdown consumers together.
Terminate affected Store capability honestly; do not continue on a possibly
inconsistent Store, replay closures, calculate another transaction, release accepted
inputs, or discard already-calculated unpublished bytes. Preserve independent
static/other-Store endpoints and the existing forced-shutdown contract. A dead
worker must not be described as a healthy Store merely needing another SQL retry.
Require prompt sanitized cause/impact diagnostics and consumer-visible failure.
Deterministic panic/worker-exit fixtures cover treasury and store-only lifecycles,
accepted/disconnected callers, publication ownership, absence of duplicate financial
effects, unrelated progress and truthful drain/restart behavior. Update architecture,
wallet recovery, development severity and lifecycle/startup test guidance.

### C04 — Remove generic foreground admission

Remove the count gate and permit-reuse plumbing without removing `Work` tracking,
task retention or nested ownership. Review direct library, MCP, directory, raw,
relay and output-conversion paths; leave concrete I/O/storage guards intact.
Replace tests asserting the former saturation error with functional assertions:
more than the old 128 delayed owners can coexist, unrelated cheap and paid work
progresses, disconnected waiters retain exact accepted work, signed dispatch is
not duplicated, and all released work drains. Use deterministic release points;
no throughput claim, replacement count or whole-work deadline. Update both resource
inventories, architecture, network policy and test maps with the lost aggregate
task/memory bound stated honestly. Complete gate G1 before proceeding.

### C05 — Wallet-owned metadata and canonical network views

Trace all Base `ChainQuery` producers and NEAR assets/quote callers. Bind NEAR
metadata to the already-known job recipient and keep single-wallet Base header,
balance/nonce, canonical recheck and fallback traffic under that wallet identity.
Ordinary scalar balance reads already do this; do not add a header prerequisite.
Before coding multi-wallet handling, the independent reviewer must settle whether
production callers can use existing per-wallet workflows or require a coherent
multi-wallet view. Preserve finality, captures, reorg detection and endpoint-first
fallback; do not invent a wallet owner, silently narrow multi-wallet proof or add
unnecessary mandatory probes. Any retained shared-view exception needs concrete
consumer rationale and explicit scope in network policy; unresolved material
ownership decisions block this commit, not later documentation.

Verify P5's existing wallet selection, same-origin versus cross-origin cache keys,
runtime/timeout/transport separation, retries, promotion and scope-matched cover
owners. Change only demonstrated missing identity propagation. Fake SOCKS/TLS
fixtures must observe actual request scopes, shared-key/identity boundaries,
canonical failures, unrelated-wallet isolation and absent payment/funding effects.
Document common credentials and timing as residual service-visible correlation.
Update network policy, architecture, wallet/agent-source guidance and network maps.

### C06 — Shielded refund compatibility and fresh-wallet discovery

First reconcile existing evidence: SDK derivation/binding/seed-restoration tests,
the NEAR bridge implementation and Zodl precedent, and the documented public dry
quote checks in the SDK guide. Recheck current native-ZEC/Base-USDC dry-quote
acceptance using an unfunded public deterministic shielded-only UA and the existing
diagnostic/network factory; preserve exact receiver set, route, mode and echo
binding in safe ignored evidence. Do not use real wallet secrets. A rejection
requires resolving that concrete compatibility issue before assuming this contract;
it cannot enable a silent transparent fallback.

Use the existing SDK transparent-discovery configuration to disable unnecessary
known/gap-address queries for newly initialized shielded-only application wallets.
Audit all application-exposed receive addresses as well as refund generation.
Keep legacy/imported snapshots, issued ranges, accepted refund bindings and their
needed discovery/shielding intact. Empty current balance or absence of observed
transactions does not prove no address was disclosed. Distinguish new initialization,
mnemonic import and reopening; do not retroactively erase capabilities or issuance
history. SDK-created unused transparent keys alone need not create network demand.
Avoid a new migration flag/ledger if existing configuration/ownership suffices.

Test zero transparent-address RPCs for a fresh shielded-only wallet, shielded
receive/refund discovery and spendability, restart, ordinary source deposits,
legacy transparent late receipts and imported/recovery coverage. Deposit addresses
remain transparent-only; refunds remain shielded-only for new operations. Update
wallet CLI, wallet rotation, SDK/network guidance and lifecycle coverage.
Actual funded shielded refund delivery remains separately authorized qualification
in the NU7 plan; a dry quote or successful swap without a refund cannot qualify it.

### C07 — Supported-platform private storage

Execute the macOS ACL work currently specified as U1 in
[portability_fixes.md](portability_fixes.md), under one implementation owner here.
Review keys, Store/SQLite and sidecars, registry/discovery/cache files, temporary
preflight copies, lock files, publication and backup consumers before choosing the
smallest shared Linux/macOS abstraction. Establish privacy before sensitive bytes
are written; validate reopen through the actual descriptor and handle ACL-inspection
errors truthfully. `chmod 0600` alone is insufficient. Preserve legitimate system
aliases, ownership/locking, atomic publication and accepted financial evidence.
Replace unsupported-platform successful permission no-ops with operation errors
without claiming Windows support or building a general filesystem framework.

Run native disposable inherited-ACL positive and denied-read controls on macOS,
Linux equivalents, SQLite WAL/SHM/journal creation and reopen, symlink/replacement,
preflight, restart and read-only inspection consumers. Use no real keys/state.
Reconcile the older plan's U1 ownership rather than keeping duplicate active work;
its separate U2 namespace-authority threat assessment remains owned there unless
this review demonstrates an affected correctness gap requiring inclusion.
Update portability, wallet administration, storage-owning guides and relevant tests.

### C08 — One-hour requested swap window

Change the default `funding.quote_deadline_seconds` to 3,600, including effective
configuration, examples/help and authored expectations that describe the default.
Preserve explicit operator overrides and original accepted quotes/transactions
across restart. Do not rewrite existing deadlines or signatures.
Keep returned quote deadline checks before preparation/disclosure and SDK-derived
signed block expiry independently authoritative. Use the pinned SDK's upgrade-aware
expiry/consensus parameters, never `blocks * 90`, a fixed pre-NU7 delta, or a new
local margin. Cover defaults/overrides, returned deadlines differing from request,
proving crossing the deadline, cancellation/restart and activation-boundary signed
expiry using synthetic parameters. A shorter preference is not a refund completion
guarantee. Update configuration, wallet CLI/rotation, both timer inventories and SDK
guidance. The NEAR arrival/confirmation/late-refund contract stays in the NU7 plan.
Complete gate G2 before closing implementation findings.

### C09 — Review dispositions and documentation handoff

Maintain S2's accepted behavior and operational rationale in wallet/architecture
guidance without a two/three-attempt claim, new wallet cap or balance-read gate.
Verify all original finding IDs have a concrete disposition and owning destination.
Already-fixed findings need current contracts and tests, not another historical
review narrative. Accepted behavior needs durable rationale, not a future-fix TODO.

Move accepted-valid unfinished work out of prose backlog paragraphs into focused
plans. The [deferred release plan](deferred/release_followups.md) owns Windows,
Linux CI, provider-output and legacy-discovery assessments, snapshot visibility
and the concrete archive-provenance correction. No blanket SQL/public-surface/
loader refactor or snapshot-pruning work is selected; maintain their non-action
rationale in the appropriate guides.
The [NU7 plan](nu7_release_readiness.md) owns near-term dependency/security and live
bridge qualification. The existing portability plan owns U2; the vendor ledger and
retirement plan own remaining upstream patch work. Keep only truthful current
behavior/limitations and concise links in maintained guides. Do not strip caveats
before evidence supports their removal, or present unresolved assessments as proven
bugs. Verify README, development/testing, network, wallet and agent-source handoffs
and the relevant existing AGENTS.md links. Add an AGENTS link only for a new enduring
rule not already reached through its owning guide. Complete G3 before cleanup.

### C10 — Remove completed planning material

Only when C01–C09 are complete and all unfinished qualification/deferred work has
an owner, remove this completed plan and repair its incoming links. Do not retain
an archive/report copy. Have a non-author independently review the exact cleanup
SHA and then the final combined branch at G4. Do not remove the separate unfinished
NU7, portability or deferred plans, their release conditions, or financial evidence.

## Independent reviews and gates

Before substantive implementation, a read-only design reviewer challenges necessity,
ownership, resource/authority enforcement and opportunities to delete responsibilities.
The planning design review identified the P3 multi-wallet boundary, P4 quote-versus-
delivery distinction, A6 publication ownership and B8 supported-platform scope;
the commit requirements above incorporate them. Revalidate against implementation
HEAD and resolve material choices before affected code is built.

Independently review every exact implementation SHA and its relevant consumers;
the reviewer cannot be its author. Use `git commit --fixup=<introducing-SHA>` for
findings and independently review each exact fixup SHA. Preserve unsquashed history
through final integration. Give reviewers intent, scope, baseline, source/feature/
profile identity and actual validation evidence; ask them to verify claims and
challenge needless enforcement. Keep audit/review agents read-only by default;
assign explicit files if edits are delegated and serialize builds/source changes.

| Gate | Boundary, combined scope and exit criteria |
| --- | --- |
| G0 | Before affected implementation: resolve design findings, storage mechanism, failure ownership and multi-wallet identity choices using actual consumers. No material unresolved decision may be hidden in a later cleanup task. |
| G1 | After C04: review all C01–C04 commits/fixups through compiler arithmetic, listener/payment authority, Store death, accepted SDK/publication work, foreground retention and shutdown. Independent operations remain available; no replay, silent success or substitute admission gate. |
| G2 | After C08: review all preceding commits/fixups through network identities, canonical Base proofs, fresh/legacy discovery, provider receiver evidence, private storage, source/refund recovery and NU7-aware expiry. Verify full call chains and honest qualification limits. |
| G3 | After C09: integration review of the complete implementation, disposition coverage, exact validation evidence and permanent-guide/AGENTS handoff. Unfinished work has focused owners and does not linger as a prose backlog. |
| G4 | After C10: independently review the cleanup SHA and final combined branch, including every earlier fixup, maintained guidance, links and remaining plans. Documentation removal cannot erase authority or evidence. |

Every wider gate reviews all earlier commits and fixups alongside complete call
chains, not only the latest diff. Analyze fixup causes, repeated assumptions,
sibling hazards and missing regressions; decide whether they require wider audit,
regression coverage, simplification or a design change. Earlier per-SHA approval
does not exempt work from accumulated review. Later material changes reopen affected
gates and receive appropriate revalidation. Resolve material findings before completion.

Use source-matched CogC/CycC for substantive Rust refactors and architecture gates:
compare the agreed baseline with changed functions and complete ownership/network/
financial chains using the same pinned analyzer. Inspect relevant warnings and
unchanged high-risk hotspots, evaluate concrete simpler representations and record
their dispositions under ignored `target/`. Advisory scores and passing tests do
not justify organization by themselves; do not extract helpers merely to lower scores.

## Validation and release boundaries

For this planning-only change, validate document links, scope and independent design
review; no application build is required. During implementation use Make entrypoints
and the [ordered continuation policy](../testing.md#validation-order-and-continuation):

1. Capture source/executable/features/profile and run affected cheap contracts;
   complete design review, `make check-static` and applicable complexity review.
2. Run `make check-clippy check-compat` before broad runtime suites. Inspect actual
   production/dependency diagnostics, not just exit status. C01 also needs optimized
   evidence because the normal default suite uses the development/test profile.
3. With sources/executables frozen and feature configurations sequential, complete
   `make check-default-remaining check-proving check-lifecycle check-codec`.
   Revalidate affected consumers after fixes and resume unfinished named stages;
   carry forward only evidence whose inputs remain valid.
4. Choose socket-capable execution before localhost fixtures; socket-free tests stay
   sandboxed. Coordinate editor/other Cargo activity and never delete `target/` to
   obtain a clean-looking result. Native ACL tests require actual supported OSes.
5. Record commands, exact revisions/input hashes, nonzero test counts, features,
   profiles, warnings, failures and focused continuations under ignored/private
   evidence. A filtered zero-test success or compile-only check is insufficient.

Full application checks do not qualify mainnet refund delivery, real Tor, Docker
consensus or a fresh reproducible release. Execute those only under their existing
separate authorization/runbooks. A release claiming NU7/native-ZEC funding readiness
must also meet the applicable evidence and unresolved-contract conditions in the
NU7 plan; do not silently call a pending funded-refund check complete.

## Execution evidence

G0 C01 design review verified the existing compiler pin, root profile and optimized
financial consumers. Root MSRV now matches 1.99.1 and release overflow checks are
enabled without changing explicit arithmetic or panic recovery. Independent
reviewers also identified and resolved the C03 unpublished-byte retention boundary,
C05 coherent multi-wallet header exception and C07 atomic ACL creation requirement
before affected implementation. Validation and exact-SHA reviews remain pending.

C01 exact-SHA review: `298005561f5699798f7904cb37900310c807a4fa`, independent
runtime reviewer, no material source findings. Optimized payment and publication
coverage remains pending; no performance or release-reproducibility claim.
C02 implementation rejects empty caps at the shared parser, removes conventional
token absence bypass, validates authentication/Host policy before serving I/O,
and makes HTTP app construction fallible for direct library consumers. Offline
inspection remains secret-free. Focused regressions are authored; validation and
exact-SHA review remain pending.

C02 independent review found inspection skipped cap parsing along with secret
validation. The fix parses nonsecret authority unconditionally and validates the
structural Host/auth combination separately from serving-only token lookup.
Inspection still needs no token/key secrets. A focused CLI regression covers empty
environment caps before catalog I/O; the serve-only CLI cap is covered separately.

Corrected source compiler/all-target Clippy passed (`check-clippy-native-corrected.log`).
Compatibility passed crypto 2/2, payments 15/15 and SDK 2/2; server passed 10/11.
The existing non-loopback fixture omitted the now-required HTTP token, so it observed
that earlier validation rather than its intended loopback guard. Supply a public
fixture token and preserve all rejection/pre-provider-I/O assertions; no production
validation precedence change. Carry forward the unchanged compatibility targets and
full compiler/dependency warning evidence, revalidate changed server target with
Clippy and all server tests, then resume named remaining stages. Focused cache 1/1
and standalone CLI 6/6 passed with corrected source. Linker warning remains recorded.

Default library stage executed 1,242 cases: 1,217 passed, 22 intentional children/
opt-in cases ignored, 3 failed on stale expectations. C02 auth cross-product now
expects dual-disabled rejection; C03 atomic panic now asserts typed WorkerStopped;
C05 SOCKS proofs now compare actual selected transport scope credentials instead
of logical EVM-id credentials. All behavioral/side-effect assertions remain.
Focused auth, atomic-panic and Base-scope continuations pass 1/1 each. The repeated
cause prompted a wider affected-consumer scan, which found `host_policy.rs` still
expecting dual-disabled startup success. Replace that case with explicit no-auth
plus a real Host allowlist; existing pre-provider-I/O negative coverage remains.
Pause queued runtime stages only while the active compiler finishes, then validate
this fixture and resume, without changing sources during any active build/test.

Post-FFI compiler/all-target Clippy passed without warnings; native macOS5/5 and
registry-parent1/1 passed. Full compatibility passed crypto2/payments15/SDK2/server11.
Default library again passed1222 (22 intentional ignores); integration continued
through deployment, where an older conventional-token fixture still expected
implicit unauthenticated serving when the named token was absent. Correct this
consumer to assert missing-token bind rejection, then supply a public fixture token
and verify research authentication and sibling-listener isolation independently;
explicit auth=false remains usable. This is another stale C02 consumer expectation,
not a production authority change. Revalidate deployment Clippy/tests, preserve
unaffected evidence and continue the remaining named stages.

C03 implemented: the exact Store worker is supervised, typed loss stops affected
operations, and StoreHandle clones retain the existing exclusive lock. Completed
unpublished results stay with their original blocking owner on terminal Store loss;
callers fail promptly, ordinary drain remains pending and explicit force retains
its documented uncertainty. Independent design and initial implementation reviews
traced cancellation/publication/other-Store consumers. Added deterministic worker,
store-only, actor and isolated real in-flight calculation fixtures; runtime evidence
is pending. Safe tracing/JoinError categories do not claim to sanitize the existing
Rust panic hook's stderr output. Fresh exact-SHA/G1 review remains pending.

C04 implemented with no replacement count/queue: retained Work, nested ownership,
qualification context and output/drain remain. Initial independent review caught
context capture moving to construction time; corrected before commit, strengthened
existing case fixture to construct outside/poll inside and removed redundant empty
nesting coverage. MCP/direct fixtures now hold 132 accepted owners and verify
independent cheap/paid progress plus journal uniqueness and forbidden funding.
G1 is pending fresh exact-SHA, complexity and runtime evidence. C05–C08 independent
file-owned implementation was prepared concurrently with C01–C04; this departs
from the original strictly serial preparation boundary, but no dependent financial
work or validation is accepted through G1 until its material findings are resolved.

C05 implemented per independent consumer audit: single-owner Base complete views
use that owner, genuinely multi-owner qualification/library headers retain a
coherent RPC-origin view while individual reads retain each payer. NEAR metadata
uses the known job recipient. Existing Coinbase/cross-origin scopes verified;
no redundant discovery identity added. SOCKS regression coverage is authored and
initial independent source review found no material issue; validation pending.

C06 implemented fresh-seed-only persisted transparent-discovery disablement and
shielded-only exposed receive addresses; imports and reopen preserve saved policy.
Actual SDK/synthetic-indexer fixtures cover fresh shielded funds, zero transparent
RPCs, restart and legacy transparent late receipts. Current public dry quote
accepted our exact Sapling+Orchard/no-transparent UA with echoed binding, native
ZEC/Base-USDC, public EXACT_OUTPUT dry mode and no deposit allocation (HTTP 201).
Ignored evidence: `target/review-findings-cleanup/shielded-dry-quote-100.json` and
`.log`; diagnostic executable SHA256
`98a862285b06d5ec95e7b130de76050f04b54d8e66c0ae8651f216e7b69f5b36` predates
these source edits and exercises the existing factory/refund-format contract.
The initial 1-USDC request hit the explicit route minimum; unfunded synthetic
100-USDC dry quote succeeded. This is quote compatibility, never refund delivery.
Rust validation and exact-SHA review remain pending.

Default acceptance is now assembled: current production/native/compatibility pass;
unchanged default library1222+bin4 and11 early integrations carry forward from
`check-runtime-ffi-owner.log`; metadata-derived remaining44 targets passed593
cases with20 intentional ignores (`default-metadata-continuation.log`, exact
selection/input evidence in `default-continuation-selection.json`). Proving4/4
passed. Lifecycle56 passed/1 intentional ignored; fresh receipt failed at its
per-address Sapling assertion. Codec and optimized acceptance remain pending.

Independent pinned-SDK diagnosis rejected a proposed height workaround: BlockHeight
subtraction is saturating, so no new bound or birthday change is justified. Actual
SDK default_receivers is Orchard-only; application initialization adds all_shielded.
Require every published address noT+usable shielded and require/select the dual
Sapling+Orchard address for the actual receipt. More importantly, the authored
initial-mixed-UA assumption was false for the pinned cohort: ReceiverSelection has
only Orchard/Sapling and generation plus disk restore use no transparent receiver.
Remove the resulting redundant address_info filter and wallet lock, correct owning
guides, and preserve disabled fresh discovery, import/reopen settings and exact
legacy refund identities. This supersedes earlier mixed-UA/filter rationale in this
plan. G2 independently checked current generation and all disk reconstruction paths;
no hypothetical SDK boundary guard is retained. Revalidate affected compiler,
address/receipt/refund and CLI consumers, carry forward unrelated lifecycle/default
results, then finish codec and optimized evidence before final gates.

C07 implemented a shared private-file/directory/temporary boundary without another
crate: Darwin atomic filesec ACL_NO_INHERIT, Linux mode-masked creation with default
ACL removal, descriptor reopen validation and explicit unsupported errors. Keys,
Store/SQLite and sidecars, registry/cache/locks, backups, mnemonic reads and SDK/
preflight scratch use it. U1 was consolidated out of the portability plan, which
retains U2's separate configured-name/rollback assessment. Initial native macOS
libc probes passed; Rust/native Linux acceptance remains pending. Full check-static
found the analyzer grammar cannot parse edition-2024 unsafe extern blocks; independent
review approved narrowly masking the validated foreign-module unsafe token for
metrics with explicit evidence and syntax probes. No exclusion may be accepted as
a successful complexity review. Tooling correction and native validation pending.

C07 tooling follow-up: the independently approved analyzer compatibility correction
masks only AST-validated foreign-module `unsafe` tokens for the older grammar,
preserving locations/newlines and recording transformed lines. Equivalence,
unrelated unsafe syntax and malformed-original probes are authored; no application
syntax, toolchain/analyzer dependency pin or financial behavior was weakened.
Native Linux testing is now available via disposable Docker Rust 1.99.1, using only
an exact public helper-source copy and isolated Cargo/SQLite evidence, without
mounting application state/secrets. Results pending.

Compiler continuation: production compiler passed; all-target Clippy identified
unneeded unit arms (C07), async-wrapper syntax (C04), and C03's enlarged preflight
enum plus a test-only Debug bound. Focused fixups `1fcee0c6`, `c497fa52`, `99c562fe`
resolve them; fresh exact-SHA review found no material issue. Analyzer probes then
passed 9/10: syn accepts a free extern function declaration without a body as a
verbatim item, so that test did not demonstrate malformed syntax. Replace it with
an actually malformed foreign return type; no production/parser contract loosened.
Carry forward the passing 92 native Python/tooling fixtures and coverage-publication
checks; rerun analyzer/provenance remainder and resume the compiler gate.

Complexity review verified the complete 465-file/10,268-callable report at
`target/complexity-runs/20261106T013702Z-9750015e` against execution baseline;
no exclusions. Publisher CogC 17→23 is retained because the existing owner alone
holds exact bytes/client/snapshot/lock through consumer notification and terminal
non-drain; helper/state extraction adds choreography without removing a duty.
For `run_standalone` CogC 19→24/CycC 38→48, independent review selected a real
simplification: build HTTP Host policy once and separate inspection structure from
serving token validation in that branch. Apply as C02 fixup; regenerate affected
metrics. Unchanged deployment/payment/relay ownership hotspots were reviewed without
material findings or further compelling simplifications.
The resumed Clippy gate found only three macOS test permission literals requiring
octal syntax; C07 fixture-only fixup changes `0` to `0o0`, preserving behavior.

The next compiler continuation exposed a real consumer-layout dependency: removing
the wrapper's pre-poll `Box::pin(work)` made `tests/server.rs` exceed the existing
layout-query depth. Restore that existing indirection outside the async block;
retain poll-time qualification capture and remove only admission machinery. No
recursion-limit increase or runtime gate is introduced. G2 also encountered two
affected cache persistence WARNs without safe causes; C07 fixup adds typed I/O/
SQLite/worker causes (explicit unknown fallback), best-effort origin results and
focused corruption/filesystem diagnostic assertions. Raw error prose stays excluded.

Native C07 correction: focused CLI filter passed 5/6; an existing Store fixture
exposed descriptor ACL retrieval NULL/ENOENT for a valid newly-created directory.
Independent native/API audit traced Apple Libc `acl_file.c` → `statx_np.c` →
`filesec.c`: descriptor retrieval uses ENOENT for absent FILESEC_ACL, while invalid
fds return EBADF. Native clean file/dir, inherited-grant and invalid-fd probes
confirmed that contract (`macos-acl-api-contract.json`). Accept only NULL+ENOENT;
retain every other inspection error and grant rejection. Explicit Rust no-ACL
file/directory reopen and invalid-descriptor controls added; inherited positive and
denied-read controls retained. This is a functional portability fix, not a broad
error suppression. Revalidate native fixtures and affected compiler/consumers.

Native macOS Rust controls passed 4/4 after `75f7b526`, including real EBADF,
explicit no-ACL file/dir reopen, inherited grant/denied-read, link/mode checks and
SQLite sidecars/temp cleanup. Linux exact copied helper again passed 3/3.
Focused cache exercise returned complete origin data and safe filesystem causes;
its assertion incorrectly omitted tracing's quoted string format. Correct it to
check the field plus typed kind without imposing formatting. SQLite continuation
remains pending. The development lib-test linker emitted `__eh_frame section too
large ... compact unwind` performance warning; retain it for independent diagnostic
review and panic-ownership regressions. Do not suppress it or change build pins.

C07 necessity review supersedes the earlier Linux default-ACL rejection/removal:
pinned SQLite 3.51.1 copies validated DB modes to WAL/SHM/rollback journals; Linux
0600/0700 modes constrain POSIX ACL class masks. Remove redundant xattr inspection/
mutation and preserve real descriptor/mode/link/directory checks. On macOS, reject
file-inheriting parent ALLOW entries rather than harmless non-inheriting parent
grants; private objects still reject all granting ACLs. Independent native/source
probes established both distinctions before implementation. Add actual bundled-
rusqlite inherited/non-inherited controls and registry-before-bytes assertions.
No namespace/U2 work or general ACL framework is selected. Owning guides updated.
The corrected default library ran 1,220 passes/22 intentional ignores; default
continued through early integration targets and found three CLI consumer failures:
two handcrafted legacy registry fixtures were created 0644 rather than privately,
and directory guidance had unnecessarily changed wording. Precreate synthetic
registries privately before SQLite writes; restore maintained init guidance using
the actual descriptor error kind (not `path.exists`, which conflates access denial
with missing). Native and affected consumer/compiler continuations remain pending.

Narrowed native controls passed macOS 5/5 plus actual registry-parent 1/1 and
Linux 3/3 with bundled SQLite. Full CLI now passes 24/25; independent exact review
caught the remaining fixture already precreated its registry before calling the
new private precreation helper. Remove that duplicate, preserving one private
creation path. Linux native fixture now explicitly chooses a named UID distinct
from its ordinary owner and exercises a fresh denied read at mode 0000, while
retaining default/access ACLs and actual SQLite mode checks. It does not claim an
alternate-UID read trial. Revalidate these focused fixtures, preserve prior results
where inputs remain valid, then finish compiler/runtime/optimized acceptance.

The operator's question about the new unsafe boundary prompted an additional
independent native-API ownership audit. Atomic macOS no-inherit creation still
requires the isolated Darwin APIs; pinned std/libc offer no safe equivalent.
Reviewer found `Security` constructed before checking `filesec_init()` for NULL;
its destructor would pass NULL to Apple's dereferencing `filesec_free` on allocation
failure. Check the raw pointer before constructing the owner. `acl_free(NULL)` is
valid; copied FILESEC_ACL property, borrowed ACL entries/flags, CString lifetimes
and newly owned descriptors were independently checked against Apple sources.
No extra allocation state/framework is needed. Stop the unfinished validation
build before editing, retain its interrupted log without a pass claim, and rerun
affected native/compiler consumers before continuing runtime and optimized checks.
