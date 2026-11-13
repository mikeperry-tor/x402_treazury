# GitHub CI and octocov

## Intent and baseline

Implement GitHub Actions test status and octocov default line coverage, with
pre-merge results on `ci_coverage`. Start on `ci_coverage` at
`fffa29e6ada98943dc9e83d7f4f92f5776c244a0`. No workflows currently exist;
`make check` and `make coverage` already own validation and coverage collection.
Preserve unrelated untracked files, application behavior, pinned toolchain,
financial/network authority, test selections and coverage exclusions. No funded,
live Tor or Docker qualification is selected. No dependency forks or runtime
limits are introduced.

Owning guides: [development](../development.md), [testing](../testing.md),
[scripts](../../scripts/README.md), [complexity](../../tests/COMPLEXITY.md) and
[planning workflow](../planning-workflow.md). The broader advisory automation and
actual Linux execution acceptance in deferred release track D2 are not retired by
writing this workflow.

## Ordered implementation

- C1: Export filtered LCOV within the existing atomic coverage publication path.
  Extend the offline shell regression to verify the report/exclusions and failed
  LCOV export preservation; document the new report in owning coverage guides.
- C2: Add SHA-pinned Actions and octocov configuration, main and `ci_coverage`
  push/PR triggers, independent Linux check and coverage jobs, summaries and
  resolved report artifacts. Fetch merge-base history for complexity. Use the
  pinned Rust toolchain, Python 3.11+ and matching LLVM tools. Store main coverage
  baseline in Actions artifacts; publish the permanent coverage SVG from main
  pushes only, through a separate job with contents-write authority. Branch
  previews retain generated SVG and HTML artifacts. Document branch preview,
  missing/expired baseline, fork permissions and exact coverage scope; add README
  badges for main. No arbitrary coverage or test-time thresholds.
- C3: Record validation/review evidence and completion status. If all acceptance
  evidence is available, remove the completed plan with independent cleanup review;
  otherwise retain it with concrete remaining acceptance work.
- C4: Repair a stale credit-pricing regression exposed by hosted coverage. Keep
  provider inheritance, tag-subset selection, estimated/observed rendering and
  operator override assertions, using a small synthetic source/endpoint table
  instead of the comprehensive deployment's changing provider inventory. Review
  the design independently, commit the correction as a fixup to `920fced2`, run
  affected test/lint consumers and review the exact fixup plus combined state.
- C5: Expose all seven existing check stages as separate sequential Actions steps.
  Capture each stage's stdout/stderr, elapsed time and exit status under
  `target/ci-checks/`, summarize outcomes and retain logs after failure. Preserve
  exact Make selections and failure propagation. Retain initial coverage listing
  logs (`tests-*.txt`) as diagnostics too. Update owning CI/tooling documentation.
- C6: Synchronize the maintenance fixtures before pausing time, using a test-only
  signal after the finite initial inventory and active read/retirement futures
  drain. Resume time before subsequent TLS or teardown. Synchronize the treasury
  interruption fixture at the actual registered recovery sleep, advance the
  existing interval virtually and resume before SDK reconstruction. Preserve
  all financial, cancellation, ownership and forbidden-replay assertions and
  production cadence. Keep test hooks private and absent from production builds.
- C7: Independently review exact C5/C6 commits and any fixups plus the complete
  combined branch. Carry valid prior evidence forward. Retain the plan for hosted
  acceptance; the user will push the branch. Evaluate compiled caching only after
  hosted stage timings establish its value. Keep the locally qualified eight
  test threads; no concurrency remeasurement or tuning is selected.
- C8: Isolate each injected HTTP/2 paid-response fault with a distinct payer/client,
  so a preceding GOAWAY cannot prevent the next fault's signed request from reaching
  the fixture. Keep the shared-client stream-preemption scenario, exact one-signed-
  request assertions, padding and SOCKS isolation checks. Update identity accounting
  for the three separate fault payers and document the fixture boundary. Commit
  as a fixup to `22f12389` and independently review the exact fixup and consumers.
- C9: Put the cached-JSON diagnostic regression behind the existing exact-child
  log-capture guard, preserving freshness, typed generation failure, DEBUG capture and
  private-prose/URL assertions. Commit as a fixup to `47c99674`; independently
  review the exact fixup and final combined branch. Preserve eight test threads,
  production transport/replay/cache/logging policy and retained CI stage selections.
- C10: Repair TLS fault-fixture publication and partial-write ordering. Protect
  corrupted TCP acceptance, count publication and count snapshots with one local
  mutex, held only during synchronous polling. Release the body header gate only
  after the complete original TLS output chunk is accepted. Use a generic private
  fixture writer to test pending, zero, failed and partial writes and publication
  exclusion deterministically without sockets. Preserve real ciphertext corruption,
  all exact connection/call/flip counts and existing mode scenarios. Commit the
  correction as a fixup to `2c83d5c7`, independently review the exact fixup and
  combined branch, and retain hosted acceptance for the user's push.
- C11: Reuse startup-drained notifications in the retirement fixture, replacing
  the entry-plus-yield guess before allocating a later wallet and awaiting complete
  startup retirement before idle-rescan assertions. Keep zero-scalar/request counts
  and all financial assertions. Audit adjacent users of those boundaries; do not
  replace distinct blocked-acquisition or SOCKS checks with the wrong signal.
- C12: Add test-only final-Manager-drop acknowledgement and a shared registry-test
  teardown helper. After listener shutdown, drain accepted Manager work, drop the
  fixture handle and await real Store/lock destruction before reopening. Verify
  a deliberately retained owner keeps teardown pending and registry locked, then
  releases both without retrying open. Apply to the demonstrated HTTP/restart
  fixture; audit nearby direct restarts without blanket conversion. Document
  reusable completion-boundary testing guidance, review exact fixups and combined
  state, then retain hosted acceptance for the user's push.
- C13: Correct the public-demo integration test's stale default reserve ceiling
  from $4 to the documented $3, preserving independent literal financial and
  offline-inspection assertions. Audit related profile/default consumers, document
  configuration-contract synchronization, and validate integration targets plus
  the existing default-test remainder at eight threads. Commit as a fixup to
  `ac369750`; independently review the exact fixup and combined branch. Carry
  unchanged compatibility evidence forward without claiming full `make check`.
- C14: Replace the startup failure fixture's redundant log-text assertions with
  its functional CLI contract: unsuccessful exit, empty stdout and no private
  address disclosure. Retain the adjacent structured-JSON parse-stage and absent-
  inventory assertions. Reproduce with inherited `NO_COLOR` absent, audit sibling
  diagnostic consumers and validate the default remainder without the locally
  inherited color override. Keep production formatting and CI color settings
  unchanged. Independently review the exact fixup and combined state; retain
  hosted acceptance for the user's push.
- C15: Add a focused Ubuntu treasury-sync diagnostic workflow, bootstrapped by
  scoped `ci_coverage` pushes and manually dispatchable after default-branch
  availability. Compile the exact integration executable through Make before
  starting a diagnostic-only five-minute watchdog; execute the whole target at
  eight threads with uncaptured output and opt-in static phase breadcrumbs.
  Capture owned process state and GDB thread stacks before terminating stalled
  fixtures. Keep cancellation capture within one short aggregate budget and
  upload selected diagnostics with `always()`. Preserve real test exit status,
  fail on diagnostic timeout, and document force-cancel/runner-loss limitations.
  Validate the monitor with synthetic success/failure/stall/cancel/debugger-error
  and process-group isolation controls, plus affected Rust lint/tests and workflow
  syntax. Independently review exact commits and the combined branch. No push,
  production timeout, test-thread tuning or deadlock-fix claim is authorized.
- C16: Use the captured Linux wait state to instrument the newly implicated
  actor-dispatch fixture. Share opt-in command phase breadcrumbs with the existing
  reuse fixture; add an optional static Mock label for closed-name RPC entry and
  prepared-response observations, including its intentionally pending mempool
  request. Enable only existing treasury DEBUG and Pepper sync INFO events through
  one diagnostic-only process subscriber so spawned runtimes report their stages.
  Preserve ordinary-test logging, all financial/ownership assertions and runtime
  behavior. Independently review exact changes and combined consumers; validate
  selected test Clippy and the full eight-thread treasury target before the user's
  next hosted diagnostic run. Do not infer an async wait location from a wrapper
  stack frame or claim an unproven deadlock fix.
- C17: Make lifecycle panic output immediate and add a focused lifecycle diagnostic
  workflow using the existing monitor. Preserve `zcash-testutils`, the library
  lifecycle filter and eight threads. Unlike the finite treasury-sync experiment,
  lifecycle diagnostics take numbered CPU/memory/thread snapshots every five
  minutes without terminating active proving. Keep actual test exits, bounded
  capture/cancellation cleanup and artifact privacy. Independently review exact
  changes and combined consumers; validate synthetic monitor controls, fixed Make
  selection, Cargo artifact selection and Actions syntax without repeating the
  expensive lifecycle suite solely for output/tooling changes.
- C18: Remove cooldown timing from the deployment funding-stop isolation fixture.
  Start an empty managed group, await existing full startup completion, acknowledge
  the applied test funding cancellation, then explicitly create one synthetic
  funded recipient and request its first immediately-due Base read. Retain both
  MCP listeners and exact one-job assertions. Reuse the startup-drained signal in
  the two adjacent maintenance fixtures whose cooldown assertions currently join
  only individual observations. Add a blocked-startup/sibling-publication ordering
  witness using existing inventory RPC controls. Keep hooks crate-private and
  test-only, preserve production cadence/authority and eight threads, review exact
  commits and combined consumers, then validate affected default-library behavior.
- C19: Correct SDK loader/worker busy publication before task enqueueing, using
  one private shared primitive. Force consumer completion before the publication
  closure returns in a deterministic real-channel/thread regression, verify idle
  completion and another dispatch, and retain a held-work positive control.
  Include that exact source module from existing SDK compatibility tests; do not
  expose SDK APIs or change workspace/dependency resolution. Fold the change into
  the maintained Pepper progress patch and its exact provenance, allowing declared
  patch-created files while retaining maintained hashes and strict restored
  upstream inventory. Independently review exact changes and combined consumers;
  run provenance controls, compiler/Clippy gates, compatibility, treasury-sync and
  the two stalled lifecycle cases. Preserve all work, channel capacities, timers,
  authorities, assertions and eight-thread settings.

## Review and validation gates

Independent fresh-conversation design review precedes substantive implementation.
Review each exact C1/C2 SHA and any fixup without rewriting history. Final combined
review covers all accumulated commits, workflow permissions/triggers, coverage
export/publication/failure paths and consuming documentation. No Rust refactor or
new complexity architecture is selected, so no new Rust CogC/CycC analysis is
needed locally; the CI workflow must run the existing analyzer stage.

Run affected cheap checks first: `scripts/tests/coverage.sh`, relevant existing
Python tooling tests and shell syntax; validate Actions syntax with actionlint and
octocov configuration/LCOV compatibility where tools are available. No production
Rust changes justify rebuilding the entire application locally for this edit.
Hosted execution is separate evidence: after authorized push, inspect branch
check results, coverage summary and downloadable SVG/HTML, failed-stage propagation
and baseline-absent behavior. A successful main run establishes the comparison
baseline and permanent badge. Do not claim remote success or publish without
authorization. Generated evidence belongs under ignored `target/`.

## Evidence and remaining work

Independent design review covered exact plan SHA `5e187e9a` and the existing
coverage/Makefile consumers. No material plan/C1 findings. C2 must pin octocov's
binary and checksum, reject absent LCOV data, and serialize only badge publication
with a current-main check so older runs cannot overwrite newer published badges.

C1 implemented: LCOV shares summary exclusions and the existing atomic publication
boundary. Offline regression includes LCOV-only failure preservation.
`scripts/tests/coverage.sh`, shell syntax checks and `git diff --check` pass.
Push and merge are not authorized by plan completion.

C1 exact SHA `81333766` independently reviewed, including common coverage modes,
failure trap, source verification and report consumers: no material findings.
C2 implemented with pinned action SHAs, cargo-llvm-cov 0.9.1 and octocov 0.83.5
plus archive checksum. Publication alone is serialized, skips superseded main
revisions and never force-pushes. The publisher's exact inline shell was exercised
in disposable local Git repositories: empty-branch bootstrap, update with retained
history, stale-revision skip and unchanged repeat all passed. No user remote was
written. Exact octocov config and synthetic LCOV were parsed by checksum-verified
octocov 0.83.5 on macOS, producing the expected 1/2-line report and 50.0% badge.
Actions syntax passes actionlint 1.7.7; 32 existing Makefile/default-target/compiler
warning tooling tests pass. Shell syntax and diff checks pass. These are local
tooling checks, not a hosted run or application-coverage measurement.

Independent final integration review covered exact C2 SHA `3d27cf54` and all
accumulated commits from `fffa29e6`, including the complete Make/coverage/report/
publication chains and documentation. No material findings or compelling
simplifications remain. The reviewer independently passed coverage shell regression,
shell syntax, actionlint and diff checks. Inspection of pinned octocov source
confirmed that missing baselines and fork comment/viewer failures are nonfatal,
and main-only report storage excludes fork baseline writes.

## Handoff and remaining acceptance

The initial CI implementation and review are complete on `ci_coverage`. The
operator's hosted branch run exposed the regression addressed in C4 below.
The agent has not pushed a remote branch/badge or changed branch protection.

After the correction is pushed, inspect the `ci_coverage` Actions check/coverage jobs,
nonzero failure propagation, summary and SVG/HTML artifacts without a main
baseline; fix and independently review any concrete failures. Validate fork
read-only behavior when such a PR is available. After integration, verify a
successful main baseline and badge publication, then remove this completed plan
and repair its deferred-plan reference through independently reviewed cleanup.
Hosted acceptance remains missing evidence, so the plan is retained. The broader
D2 advisory automation remains separately deferred.

## Hosted failure follow-up

Run `43364436717`, coverage job `131685388054`, executed source `4a56753b`.
The `coverage-diagnostics` artifact retained the actual failure: after 1451 library
tests passed, `credit_pricing` failed at a missing `sources.social` lookup in
`comprehensive_subsets_inherit_conversion_despite_endpoint_instruction_overrides`.
`920fced2` removed the example's SocialFetch sources without adapting that test.
Coverage export and octocov were not reached; no coverage percentage is available.
Run metadata and extracted diagnostics are private ignored evidence under
`target/ci-diagnostics/43364436717/`. Public metadata/artifacts were read without
changing the run or repository.

Independent C4 design review covered source `4a56753b`, removal commit `920fced2`,
provider/config composition and complete rendering assertions. No material design
findings. The inline two-source table keeps actual SocialFetch settings/fixture
inheritance and existing help exclusion. Endpoint instruction assertions remain
a separation check, not runtime listener-composition qualification. Validation is
limited to the affected `credit_pricing` and `socialfetch` targets and changed-test
Clippy; the unchanged full `config` target has socket fixtures and is not selected.
The test repair is committed as fixup `c79c70d5` against `920fced2`.
`make fmt-check`, `make clippy CARGO_ARGS='--test credit_pricing'` and
`make test CARGO_ARGS='--test credit_pricing --test socialfetch'` passed on macOS:
12 credit-pricing and two SocialFetch tests executed, with no compiler/Clippy
diagnostics. Logs are `clippy-fix.log` and `test-fix.log` beside the hosted evidence.
Independent exact-fixup and combined review covered `c79c70d5` and accumulated
changes from `fffa29e6`, including config/catalog/rendering and unchanged CI
callchains: no material findings. Earlier workflow/tooling evidence remains valid.
No instrumented coverage or full-suite rerun was performed; hosted retry and
eventual main baseline/publication remain pending. The separate Linux-check job
was still running when its status was inspected. No fix has been pushed by the
agent.

## CI timing and fixture follow-up

Starting C5/C6 baseline is `844bc2ac` on `ci_coverage`. User authorization covers
stage visibility, diagnostics and test synchronization; the user retains pushing.
The locally qualified eight-thread setting stays unchanged, without remeasurement.
Independent design review inspected the Make sequence, complete maintenance
observation/retirement chains, actor unwind/recovery and pinned Tokio timers.
The design requires fully drained initial observations before paused time, and
registration/release synchronization around the real recovery Sleep before
virtual advancement. Neither hook requests work or changes production authority.
No material design findings remain with those corrections incorporated.

Validate C5 with actionlint, focused subprocess fixtures for stage logging,
nonzero exit propagation and summaries, plus existing tooling checks. Validate C6
with formatting, selected library-test Clippy, source-matched complexity review of
affected hook-bearing callables and focused maintenance/actor consumers. Localhost
fixtures require socket-capable execution up front. Complete design/complexity and
selected compiler/lint checks before runtime suites, and freeze sources during
validation. No full-suite performance or GitHub speedup is claimed from local
functional checks. New hosted stage timings, compiled-cache value and final
coverage/badge publication remain remote evidence.

Exact revised plan `2338a7ce` independently reviewed with no material findings.
C5 implements seven sequential steps and the allowlisted stage helper, with
combined output, elapsed/status files and failure summaries. Six new subprocess
regressions and 28 existing Make/target-selection tests pass, as do actionlint,
Bash syntax and diff checks. No compiled cache or concurrency changes were made.
C5 is committed as `331feab4`. C6 fixups are `aba81b37` against `dac55936`
(cancellation fixture and shared completion signal), `2eae8f43` against
`116196f4` (startup-failure fixture), and `f4ab8e20` against `8da347fd`
(interrupted actor recovery fixture). No production interval, authority, queue
or observation selection changed. Sources were frozen at `f4ab8e20` for validation.

The source-matched complexity comparison against `844bc2ac` is complete with no
exclusions (511 files), retained under
`target/complexity-runs/20261113T050534Z-df5b806b/`. Independent complexity review
dispositioned the sole increase, `ManagedPool::maintenance` CogC 52 to 54/CycC 57
to 60, as the test-only drained-state notification. Keeping the signal beside the
existing completion boundary avoids a new scheduler/observer responsibility;
extracting it would only move decisions. Actor ownership and unchanged observation,
retirement and Store consumers were reviewed as well.

At `f4ab8e20`, `make check-clippy` passed both production compiler and all-target
`zcash-regtest` Clippy checks without diagnostics; `make fmt-check` passed.
Focused library execution selected maintenance and treasury actor modules with
eight test threads: 28 passed and two isolated child fixtures were ignored at the
top level and invoked by their passing parents. All three changed fixtures ran.
The test linker emitted the already documented macOS large `__eh_frame` warning;
no runtime failure occurred. Logs are under `target/ci-timing-validation/`.
This is functional validation, not a concurrency or GitHub speedup measurement.

Independent review cleared every exact C5/C6 commit/fixup and the combined
`fffa29e6..f4ab8e20` branch, reconciling prior findings and source-matched validation.
No material findings or compelling simplifications remain locally. C5/C6 are
implemented and locally validated/reviewed; C7 hosted acceptance remains pending.
The plan stays active for the user's next push and eventual main publication.
Compiled caching and concurrency remain unchanged; no new tuning measurement is
selected. The remaining work is to inspect hosted stage/log artifacts and complete
the already listed remote coverage/baseline/badge acceptance.

## HTTP/2 fault and tracing follow-up

Starting C8/C9 baseline is `ab19e36d`. The reported Linux library run passed 1449
tests and failed the cover protocol fault fixture (`/lost` signed count absent)
and cached-JSON diagnostic fixture (missing cache-hit DEBUG message). No replay
or production cache failure is established by those assertions.

Independent concern-specific design reviews traced pinned HTTP transport behavior
and tracing/cache consumers. Automatic request retries are disabled; HTTP/2 GOAWAY
pool teardown is asynchronous, so reusing that transport can prevent the next fault
from reaching its signed dispatch. Separate existing payer identities isolate the
faults without retries or weaker counts. The diagnostic callsite is shared with
subscriberless cache tests; the maintained tracing policy requires the existing
exact-child guard. No material design findings remain with those corrections.

Validate formatting and selected test-target Clippy before focused runtime
execution of the affected cover and startup modules. Use socket-capable execution
up front for SOCKS/TLS fixtures. Inspect exact signed counts, actual child execution
and retained privacy assertions; do not add timing sleeps, larger watchdogs or a
concurrency benchmark. Preserve earlier source-matched compiler/workflow evidence
where the test-only edits do not invalidate it. Hosted full-stage acceptance still
requires the user's next push.

Exact plan `c6fcc7e3` received independent C8 and C9 concern-specific reviews with
no material findings. One local baseline cover pass (0.22 seconds) did not
reproduce the CI interleaving; the repair addresses the demonstrated shared-pool
sequencing fragility. C8 is implemented with fault payers 3/4/5, preserving the
normal/second payer and discovery scenarios, exact signed counts and all padding
checks. Implementation validation/review remain pending.

C8 is fixup `535fe702` against `22f12389`; C9 is fixup `3bd89da8` against
`47c99674`. Sources were frozen at `3bd89da8` for validation. `make fmt-check`
and `make clippy CARGO_ARGS='--lib --tests'` passed without diagnostics. Focused
cover protocol/startup execution passed 17 tests, including both changed fixtures.
A full default library run with the established eight threads then passed 1454
tests, with zero failures and 24 ignored. The test linker emitted the documented
macOS large `__eh_frame` warning. Logs are under `target/ci-new-failures/`.
This is local functional acceptance, not a Linux interleaving reproduction,
concurrency benchmark, full `make check` or live qualification.

Independent review cleared exact plan `c6fcc7e3`, both fixups and the combined
`fffa29e6..3bd89da8` branch and relevant unchanged consumers. The reviewer verified
actual child execution, exact signed counts/SOCKS identities, retained cache and
privacy assertions and source-matched execution logs. No material findings or
compelling simplifications remain. Related startup diagnostic consumers were
checked: the warm diagnostic already uses the child guard; subscriberless cache
fixtures need no diagnostic isolation themselves. No production policy or
concurrency changes were made. Prior unaffected CI/compiler/complexity evidence
is retained; hosted full-stage/coverage and main badge acceptance still await the
user's push. The plan remains active for that remote acceptance.

## TLS fault publication follow-up

Starting C10 baseline is `741bae98`. The reported Linux library run passed 1450
tests and failed the isolated TLS retry child in `tor1`: the header-corruption
counter was zero rather than one. Independent low-level and scenario audits found
no scenario identity/fault-selection defect. The wrapper exposes corrupted bytes
before publishing its atomic counter, allowing a different runtime worker to
observe the client failure before that publication. Atomic memory ordering alone
cannot close this interval. Partial initial header writes also release the body
gate before the original chunk is completely accepted.

The design uses accepted-byte bookkeeping under a shared synchronous mutex,
never counting an attempted mutation as delivered. Header-prefix progress is
retained across partial and pending polls. Pinned Tokio-rustls completes and
flushes the handshake before returning the accepted stream; pinned Rustls drains
accepted prefixes of its first output chunk. No parser, timer, scenario split,
production retry or concurrency change is selected. The exact reported Linux
interleaving is not claimed to be reproduced by source inspection.

Validate focused socket-free write regressions and test-target lint/format checks
before the existing TLS retry and affected network/socket tests. Include a full
local default library run if focused tests pass, since the issue was exposed by
that harness. Keep eight threads and existing child modes. Retain all exact
financial no-replay/connection and actually-corrupted-byte assertions. Generated
logs remain under ignored `target/ci-tls-failure/`.

Exact plan `f18308c8` was independently reviewed with no material findings.
The baseline isolated TLS fixture passed locally in all four child modes; the
reported Linux scheduling was not reproduced. C10 is implemented in the test-only
socket wrapper, with three deterministic writer controls and unchanged scenario
assertions. Validation and review evidence follow.

C10 is fixup `b4f203e2` against `2c83d5c7`. Sources stayed frozen at `b4f203e2`
for validation: formatting and `make clippy CARGO_ARGS='--lib --tests'` passed
without diagnostics; all three socket-free writer controls passed. A focused
network run passed 83 tests with one ignored, including the TLS parent that
executes the four existing child modes. The full default library suite passed
1457 tests, with zero failures and 24 ignored, using the unchanged eight threads.
The test linker emitted the documented macOS large `__eh_frame` warning. Logs
are under `target/ci-tls-failure/`.

Independent review cleared exact plan `f18308c8`, exact fixup `b4f203e2` and the
combined `fffa29e6..b4f203e2` branch. It verified pinned writer contracts, actual
accepted-byte publication, no mutex guard across an await, complete header-prefix
progress and every count snapshot. Relevant unchanged payment/acquisition/manual
write, catalog/Base and earlier CI fixture consumers were inspected. Source-matched
logs confirm all three new tests and the original TLS fixture executed. No material
findings or compelling simplifications remain. These are local functional results,
not reproduction of the reported Linux schedule, a full `make check`, coverage
acceptance or live qualification. Production and concurrency policies are unchanged;
hosted acceptance and eventual main coverage/badge publication await the user's push.

## Completion-boundary testing follow-up

Starting C11/C12 baseline is `1d3f6754`. The user requested a systematic pattern
instead of waiting for individual flaky failures. The reported Linux run passed
1452 tests and failed a registry restart with a retained exclusive lock and an
empty-startup retirement test with one unexpected scalar request.

Independent audits traced two concrete missing boundaries. The reconciliation
entry hook fires before the awaited wallet-boundary capture, so yielding cannot
prove the pass was empty; the existing startup-drained hook can. A second retirement
fixture observes balance evidence before the enclosing retirement future finishes;
it needs that same startup completion before its idle assertions. Distinct blocked
nonce and SOCKS cases were examined and must not await a drain while intentionally
holding active work.

Pinned Axum graceful shutdown can signal connection closure before destroying its
service/router and its Manager clone. A test-only sender declared last on Manager
closes only after all prior fields, including Store lock files, are destroyed.
Its receiver therefore acknowledges the actual ownership boundary. The helper
will register that signal, drain accepted work, drop its Manager and await closure.
A retained-owner negative control must prove the helper cannot complete or release
the registry lock early. Neighboring direct awaited registration/restart cases
showed no equivalent HTTP owner gap and are not mechanically rewritten.

This is a bounded audit by failure class, not repository-wide assurance or a new
fixture framework. Keep eight threads, production locks/cadence/authority and
all existing assertions; add no sleeps, retry-open loops or assertion retries.
Validate formatting and selected test-target Clippy, the controlled teardown
regression, affected discovery/retirement/maintenance consumers and a full default
library run. Localhost fixtures require socket-capable execution up front. Keep
evidence under ignored `target/ci-completion-validation/` and independently review
the plan, exact changes and combined branch before reporting completion.

Exact plan `e458db68` received separate independent C11 and C12 design clearance.
C11 is fixup `5cf61b3e` against `a0d8d3e8`, reusing one retirement fixture helper
in two demonstrated startup/idle cases. C12 uses the real Manager's final test-only
field rather than a proxy service sentinel or a new service wrapper. Declaration-
order destruction makes channel closure follow registry lock destruction; the
helper asserts that its signal slot is unused and requires that expected closure.
Source changes and the retained-owner control are implemented; validation and
review evidence follow.

C12 is fixup `25627905` against `801ad389`; comment-only fixup `8518af01`
clarifies that HTTP graceful completion precedes the final-owner acknowledgement.
Selected test-target Clippy passed without diagnostics at `25627905`, with valid
carry-forward across that comment-only change. Formatting, the retained-owner
control (one test), and 199 affected discovery/retirement/maintenance tests passed
at `8518af01`. Four isolated child cases were ignored at the affected parent level.
The full default library run passed 1458 tests with zero failures and 24 ignored,
using the unchanged eight threads. The linker emitted the documented macOS large
`__eh_frame` warning. Logs are under `target/ci-completion-validation/`.

Independent review cleared exact plan `e458db68`, fixups `5cf61b3e`, `25627905`
and `8518af01`, and the combined `fffa29e6..8518af01` branch. It verified actual
source-matched execution, the retained-owner negative and immediate-reopen positive
controls, all scalar/retirement assertions and final-field destruction ordering.
No material findings or compelling simplifications remain in this bounded scope.
The broader pattern is documented in the testing guide: identify the true completion
event, add a controlled ordering witness, and audit demonstrated matches. This
does not claim repository-wide flake freedom or reproduce every Linux scheduling
interleaving. No production authority, locking, retry, timer or concurrency policy
changed. Hosted Linux full-stage/coverage acceptance and eventual main badge
publication remain pending for the user's push; the active plan is retained.

## Demo configuration-contract follow-up

C13 starts at `92712ffb`. The reported failure is deterministic configuration/test
drift, distinct from scheduling failures. Independent fresh-conversation design
review traced `ac369750`: it lowered the default small maximum and updated the
profile comment and other contract tests, but omitted this integration assertion.
Related managed-config defaults already assert $2–$3; explicit synthetic ranges
and live API-reservation totals express different contracts and remain unchanged.
Keep the literal $3 expectation independent of the configuration being tested.
Add the profile's documented zero retirement tolerance to its safety assertions.

Recent broad local evidence covered the library, which cannot execute root
integration targets. Run selected test-target Clippy and the demo/config/CLI
consumers first, then complete `make check-default-remaining` with socket-capable
execution. The reported hosted run already passed its preceding compatibility
stage; this test/documentation-only correction does not invalidate those inputs.
This continuation is not full `make check`, hosted Linux, or coverage acceptance.
Logs belong under ignored `target/ci-profile-validation/`. No runtime defaults,
financial authority, test selection or concurrency setting changes.

Independent review cleared exact C13 plan `8a3cd126` and fixup `4d008b78`
against `ac369750`, including the bounded combined `fffa29e6..4d008b78` branch,
profile/default serialization, inspection consumers and accumulated prior findings.
No material findings or compelling simplifications remain in that scope.
Formatting and selected `public_demo_profiles` Clippy passed without diagnostics;
the only subsequent Rust edit was formatting. Focused CLI/config/demo execution
passed 22 tests. `make check-default-remaining` completed successfully at
`4d008b78`: 2169 passed, 44 ignored across 59 test binaries, including 1458
library tests and the formerly failing demo integration test. Eight threads and
the fixed stage selection were unchanged. Its only compiler diagnostic was the
documented macOS large `__eh_frame` linker warning. Logs are under
`target/ci-profile-validation/`. This is local default-remainder evidence, carrying
the unchanged preceding hosted compatibility evidence; it does not establish full
`make check`, hosted Linux or instrumented coverage. Hosted acceptance and eventual
main baseline/badge publication remain pending; the user retains pushing.

## Diagnostic color-environment follow-up

C14 starts at `e531c48b`. The reported startup assertion reproduces locally when
`NO_COLOR` is removed from the test environment. Pinned tracing-subscriber 0.3.23
enables ANSI by default unless that variable is nonempty; ANSI separates the
field name, equals sign and value. The actual parse-stage diagnostic is correct.
The inherited local `NO_COLOR=1` explains why earlier default-remainder evidence
did not expose this environment dependency. The original proposed correction was
explicit child `NO_COLOR=1`; user review instead requested functional assertions
under the existing testing-efficiency policy. Supersede that formatting patch:
remove redundant lifecycle text matches, retain the failing CLI's unsuccessful
exit, empty stdout and private-address exclusion, and rename the test accordingly.
The adjacent qualification-snapshot test already asserts structured parse failure,
an independently completed source and absence of partial inventory. No new parser,
formatter override or duplicated scenario is needed. Document these actual
boundaries in `tests/STARTUP.md`. The bounded root CLI audit found no additional
demonstrated ANSI-sensitive assertion: custom subscribers disable ANSI and wallet
birthday reason text also appears in the plain error cause chain.

Run formatting and selected startup-test Clippy before the affected startup tests,
then continue the default remainder with inherited `NO_COLOR` absent at eight
threads. Socket fixtures require socket-capable execution up front. Logs belong
under `target/ci-startup-color-validation/`. Prior unchanged compatibility inputs
remain valid; full `make check`, hosted Linux and coverage acceptance remain open.

Independent revised-design review approved the functional simplification. Exact
plans `0c39b05f`/`e83625bf` and fixup `e6d4214e` against `c673f4ea` received
independent review, including the bounded combined `fffa29e6..e6d4214e` branch
and complete startup failure/inspection consumers. No material findings remain.
The renamed CLI test retains failure status, empty stdout and private-address
exclusion; the neighboring structured snapshot test retains parse failure,
independent completion and absent-inventory assertions. No formatting override
was implemented. Formatting and selected startup-test Clippy passed without
diagnostics. With inherited `NO_COLOR` absent, the startup target passed 19 tests
with one ignored, and `make check-default-remaining` passed 2169 with 44 ignored
across 59 binaries at `e6d4214e`, using unchanged eight threads. Logs are under
`target/ci-startup-color-validation/`. This confirms the functional tests no longer
depend on inherited color suppression; it is local continuation evidence, not
hosted Linux, full `make check` or instrumented coverage acceptance. The retained
plan still awaits hosted acceptance and eventual main baseline/badge publication.

## Focused Linux hang diagnostics

C15 starts at `1bfbbb4f`. Run `43389768250` passed line coverage but its Linux
default-test stage remained active for over an hour at treasury command-sync reuse.
That exact case passed locally in 30 seconds and its whole target passed 27 tests
with one ignored in 35 seconds at eight threads. Successful serial hosted coverage
executed that target in 172 seconds. These observations establish an abnormal
stall, not a demonstrated deadlock or its cause. Private investigation evidence
is under `target/ci-sync-hang-validation/`. The fixture audit found no mutex held
across an await; production synchronization remains unchanged.

Independent fresh-conversation design review approved focused Linux diagnostics
with corrections: GDB must omit frame arguments as well as locals, disable
auto-loading/debuginfod and target only the owned test process tree. Compilation
precedes the five-minute fixture watchdog. Debugger collection has an aggregate
budget and its own process group so failed attaches cannot block termination.
Cancellation reaches the Python monitor directly using `exec`; write process
state first and bound capture plus teardown inside GitHub's initial signal grace.
Only artifact upload uses `always()`. Force cancellation or runner loss can still
prevent artifacts. A scoped branch-push trigger handles initial availability;
manual dispatch depends on default-branch workflow registration. Existing checks
and coverage selections remain unchanged and may run alongside the bootstrap.

Permanent usage and limits belong in `docs/testing.md`, `scripts/README.md` and
`tests/STARTUP.md`. No fixture output is an assertion or a production diagnostic.
Retain exact financial, cancellation, observation and forbidden-send assertions.
Test the monitor with small socket-free subprocess fixtures, including descendants
and an unrelated-process survivor. Lint the changed Rust target, run the affected
treasury-sync target once, and check Actions syntax. Hosted reproduction, native
Linux ptrace capture and eventual main acceptance remain separate evidence.

C15 exact implementation `01b82fc3` received independent review. One material
finding invalidated the initial privileged-debugger cleanup assumption: a
runner-owned process cannot reliably SIGKILL UID-0 GDB through sudo. The reviewed
replacement enables same-user ptrace once on the dedicated ephemeral diagnostic
runner and keeps GDB unprivileged. Add a bounded native attach smoke check before
compilation, plus cancellation-during-capture regression proving debugger and
descendant termination. Explicit Bash selection preserves compilation pipeline
failure propagation. These corrections are a fixup against `01b82fc3`; no
application security policy or ordinary CI selections change.

C15 correction is fixup `e9d96ca2`. Independent review cleared exact plan
`dcee2e32`, implementation `01b82fc3`, that fixup and the bounded combined
`fffa29e6..e9d96ca2` branch. The privileged-debugger finding is resolved; no
material findings or compelling simplifications remain in the reviewed scope.
Formatting and selected `treasury_sync` Clippy passed without diagnostics.
The affected target passed 27 tests with one ignored in 34.60 seconds at eight
threads with phase breadcrumbs enabled; Rust inputs stayed unchanged across the
tooling-only fixup. Seven monitor regressions passed at `e9d96ca2`, including
capture before cancellation teardown, cancellation during stalled debugger
capture, debugger/fixture descendant cleanup, unrelated-process survival,
original test status, exact Cargo selection, and process-group snapshot filtering.
The actual pinned Make no-run JSON was also consumed successfully by the selector.
Both workflows pass actionlint and diff checks pass. Local subprocess controls
needed process-inspection permissions; their debugger and proc fixtures are
synthetic. Logs are under `target/ci-hang-diagnostics-validation/`.

Native Linux ptrace and the hosted hang remain unexecuted locally: the diagnostic
workflow's bounded GDB smoke supplies native acceptance on the next scoped push.
The agent has not pushed or dispatched a run. The user will push `ci_coverage`,
inspect Treasury sync diagnostics and download `treasury-sync-diagnostics`, then
use its last phase/process stacks to localize any reproduced stall. Existing CI
runs alongside that bootstrap; the focused workflow can be retried independently.
Force cancellation/runner loss can prevent evidence, and native stacks alone may
not identify an async waiter. Hosted full checks, final coverage/main publication
and active-plan cleanup remain pending; no deadlock fix is claimed.

## Actor scan wait localization

C16 starts at `6ff61887`. Diagnostic run `43402324149`, job `131807960376`,
executed that revision and captured the actor-dispatch fixture after 301 seconds.
The command-reuse case completed in 31 seconds. Native smoke/ptrace and GDB capture
succeeded (debugger exit zero), with artifacts retained privately under
`target/ci-diagnostics/43402324149/`. This supplies C15's native capture evidence;
it does not establish cancellation capture or eliminate the observed stall.

The live `SyncSession::run` stack is inside `runtime.block_on` at treasury/mod.rs
724, before runtime teardown. The Store worker is idle. Parked executor threads
do not expose suspended async futures; the test wrapper's ending line cannot
identify which actor command is waiting. Independent command-chain and SDK-lock
audits found no wallet/store lock cycle for this fixture. Prepare, submit,
reconcile, explicit sync and startup can each request a scan, so the next evidence
must distinguish those commands and their request boundaries.

Independent design review approved a shared opt-in phase helper, one narrowly
filtered global subscriber only in the diagnostic experiment, and optional static
Mock labels with defaults in existing constructors. Record only known RPC method
names, entry/prepared response and the deliberate pre-header mempool wait; no
URLs, request bodies, headers, arbitrary method strings or cancellation inference.
Existing treasury events show outer scan/checkpoint stages; Pepper INFO shows
scan start/end, not every SDK-internal wait. Avoid RAII tracking or a new framework.
Use existing five-minute capture and unchanged eight threads, with logs under
`target/ci-actor-diagnostics-validation/`; hosted reproduction remains required.

The SDK audit also identified a separate bounded mempool-notice shutdown cycle:
`NewBlockMined` can block sending to a full channel after the scan stops draining
and joins its monitor. The actor fixture cannot exercise it because its mempool
handler never returns headers. Keep this outside the actor-cause claim. Concrete
remaining SDK work is a controlled full-notice-channel shutdown reproduction,
followed by independently reviewed correction/provenance and transaction-drain
regressions; it is not resolved by this diagnostic-only change.

C16 is implementation `891d0a70`, independently reviewed with exact plan
`244e7858` and the bounded combined branch. No material implementation findings
or compelling simplifications remain. Formatting and selected treasury-test Clippy
passed without diagnostics. The full target passed 27 tests with one ignored in
34.62 seconds at eight threads with diagnostic logging enabled; actor command/RPC
and existing scan events all executed. The actor fixture also passed individually
with the diagnostic flag absent (one test, 6.19 seconds). Runtime inputs match
`891d0a70`; logs are under `target/ci-actor-diagnostics-validation/`.

Complexity comparison against `244e7858` is complete and source-matched at
`target/complexity-runs/20261113T134246Z-9193388f/` (511 files, 11717 spaces).
The independent review dispositioned test-only RPC inclusive CogC 26→31/CycC
40→46: its own scores remain 21/27; the added closure owns only gated, closed-name
telemetry and adds no functional or concurrency responsibility. Extracting it
would move complexity without removing that necessary privacy responsibility.
The shared helper is 3/6; reuse-fixture scores improve from 7/14 to 5/11, while
actor-dispatch remains 0/1. No production warning or source changed.

Push the updated branch to obtain command/scan/RPC evidence from Linux; the existing
focused workflow triggers on this fixture change. A local pass does not reproduce
the captured stall, and no application deadlock fix is claimed. The separately
identified SDK notice-channel cycle remains concrete unfinished work. Hosted
checks and eventual main publication/plan cleanup also remain pending.

## Lifecycle progress and failure visibility

C17 starts at `a343a838`. Focused run `43436700710` passed all 27 treasury-sync
tests with one ignored in 36 seconds, including both previously implicated cases.
Ordinary CI run `43436700708` passed static, compiler/Clippy, compatibility, default
remainder and proving stages, then remained in lifecycle testing for over 52
minutes. User output showed two failed direct-service tests and later successful
terminal/worker/reorg tests, followed by at least five minutes without a new
completion. The API cannot expose this running console or its remaining test set.
Private public-API/artifact evidence is under `target/ci-diagnostics/43436700708/`
and `target/ci-diagnostics/43436700710/`.

There are 68 authored lifecycle test attributes. Real cryptographic proofs, scans
and accepted-work drains make this stage heavier than default coverage; elapsed
time alone does not prove a hang. Both failed direct-service tests have existing
180-second submission handoff guards, but no panic details are available yet.
Already reported failures have completed their test wrappers; they are not proof
that those wrappers remain blocked in teardown. Independent read-only triage found
no demonstrated release-gate cycle. Preserve financial assertions and limits.

Independent design review approved `--nocapture` for the existing lifecycle stage
and one lifecycle mode in the existing monitor. Compile through Make before
sampling; select exactly the feature-enabled library executable. Take snapshots
every 300 seconds after the preceding capture completes, including process CPU
ticks/RSS with explicit units. Do not apply the treasury-sync termination policy
to real proving. On cancellation during a periodic capture, shorten that same
capture and proceed to cleanup without another attach. Preserve zero and nonzero
test exits, including test completion while captured; record capture errors and
continue valid work. Use scoped branch-push bootstrap, native attach smoke and
`always()` upload. GDB can perturb fixture timing, so this experiment does not
replace ordinary lifecycle qualification.

Validate the monitor's continuation, failed capture, completion-during-capture,
nonzero exit and cancellation paths with small subprocesses. Verify exact Make
selectors, actual Cargo JSON library selection and Actions syntax. No Rust source,
profile pin, test thread count or application timeout changes. Generated evidence
belongs under `target/ci-lifecycle-diagnostics-validation/`. Hosted failure
assertions and the root causes of remaining stalls still require real evidence.

C17 is implementation `0104d1c1`, independently reviewed with exact plan
`7381f986` and bounded combined `fffa29e6..0104d1c1`. No material findings or
compelling simplifications remain in the reviewed scope. Eleven monitor controls
and 20 Make regressions passed. They verify legacy timeout behavior, periodic
continuation after capture errors, exact library/feature/filter/eight-thread
selection, original nonzero completion during capture and cancellation without
duplicate attachment. All three workflows pass actionlint and diff checks pass.
Actual pinned Make no-run Cargo output selected the feature-enabled library
executable, and harness listing confirmed 68 tests with zero benchmarks. Listing
did not execute those tests. Compilation succeeded with the documented macOS
large `__eh_frame` linker warning emitted in Cargo JSON. No expensive lifecycle
rerun or Rust/profile change was selected for this tooling/visibility edit.
Logs are under `target/ci-lifecycle-diagnostics-validation/`.

A pinned-checkout applicability audit also excluded stable-tip MockNet EOF flooding
as a cause of the separately identified notice-channel risk: its empty mempool
subscription stays open until a later block ends it, and synchronous block batches
close an existing subscription once. This does not diagnose the current lifecycle
wait or retire the separate SDK hazard. The user will push the reviewed branch
and inspect immediate panic details plus the lifecycle snapshots; a normal cancel
after snapshots allows the artifact upload attempt without waiting for completion.
The already-running ordinary job cannot acquire these diagnostics retroactively.
No underlying failure/hang fix, native lifecycle capture or full CI success is
claimed. Remaining SDK work and hosted/main acceptance remain explicit above.

## Deployment reconciliation completion follow-up

C18 starts at `51b80da5`. The reported default-library failure is the second RPC
wait in `funding_failure_keeps_listeners_and_base_reconciliation_available` at
deployment.rs2731, after a scalar cooldown publication and a virtual clock jump.
Independent source review found that scalar publication precedes retirement and
maintenance consumption of its Completion. Pending cursors can skip an active
wallet and wrap on another maintenance tick. A single large virtual-time advance
also does not execute all intermediate loop iterations. The source exposes this
ordering vulnerability; it does not reproduce the exact hosted interleaving.

Independent revised design removes the unrelated cooldown responsibility from
this deployment isolation test. Declare empty capacity, install and await the
existing full-startup signal, await a test-only acknowledgement after funding's
stop token is cancelled, then allocate/credit/request one synthetic recipient
within one Store call. This is explicit fixture setup, not runtime auto-funding
authority. Its first read has no cooldown. Preserve real RPC/MCP assertions and
exactly one funding job, with no new timer, longer guard or log-text assertion.
Expose an existing startup-hook installer only under cfg(test), retaining private
fields and the existing signal's complete-pass semantics.

The bounded audit found two additional owner-completion gaps: zero-refresh idle
maintenance and held pending inventory. Await startup drain before their existing
clock assertions. The inventory fixture already holds one RPC while siblings
finish: verify a healthy sibling's scalar cooldown is published but startup is
still incomplete, then release the existing gate and await startup. No new RPC or
Store gate is needed. Other users already join reconcile/startup, perform no clock
advance, or intentionally observe independent work before SOCKS readiness.

Update owning startup/testing guidance, run selected library-test Clippy and
source-matched complexity review before affected cheap tests and the full default
library suite. Localhost fixtures require socket-capable execution up front.
Evidence belongs under `target/ci-deployment-completion-validation/`. The separate
slow lifecycle diagnostics remain independent, and no lifecycle failure fix is
implied. The user retains pushing; hosted acceptance remains necessary.

C18 fixups are `b1972ad4` against `a6428e3b`, `c22df140` against `116196f4`,
and `950f4173` against `1d242e32`. Independent review cleared exact plan
`54c65f96`, all three fixups and the bounded combined `fffa29e6..950f4173`
branch. No material findings or compelling simplifications remain in this scope.
The deployment fixture removes the unrelated clock responsibility and proves
applied funding cancellation before creating fresh reconciliation work. The
inventory witness proves sibling scalar publication versus blocked startup; it
does not separately isolate a retirement-stage delay. Existing financial and
forbidden-side-effect assertions remain intact.

Formatting and selected library/test Clippy passed without diagnostics. The
deployment fixture passed once in 0.06 seconds; the maintenance group passed 17
tests with one isolated child ignored in 5.47 seconds. Continuing the failed named
`make check-default-remaining` stage passed 2169 tests with 44 ignored across 59
binaries, including all 1458 local default-library tests. Its only underlying
compiler diagnostic was the documented macOS large `__eh_frame` linker warning.
Hosted run `43451745762` executed baseline `51b80da5` and passed compatibility
before failing default remainder; unchanged compatibility/production inputs
retain that evidence across these cfg(test)-only fixes. This continuation is not
full `make check`, hosted Linux success or a lifecycle-failure resolution. Logs
and public run metadata are under `target/ci-deployment-completion-validation/`.

Complexity is complete and source-matched at `950f4173`, compared to `54c65f96`,
under `target/complexity-runs/20261113T160130Z-1ab71d2e/` (511 files, 11718
callable spaces, no exclusions or advisory regressions). Independent review
examined unchanged serving 32/43 and maintenance 54/60 CogC/CycC hotspots and
complete owner/Store/retirement consumers. The deployment fixture changes 8/11
to 7/12 by replacing a clock loop with explicit synthetic setup; the startup
installer is 0/1, zero-refresh remains 2/3 and inventory changes 4/13 to 5/14 for
the ordering witness. No production scheduling responsibility was added.
The active plan remains for hosted acceptance and its separate unfinished work.

## SDK task-publication race

C19 starts at `0e7a3a56`. Cancelled lifecycle diagnostic run `43451746068` at
`51b80da5` uploaded 24 snapshots under
`target/ci-diagnostics/43451746068/extracted/`. It completed 66 of 68 tests with
no FAILED results. Birthday reorg-before-declared-coverage and pruned-checkpoint
rebuild remained unfinished. CPU was busy during early work, then rose only about
36 CPU-seconds over the final 81 wall-clock minutes, with the same 15 threads and
two live scan runtimes. This establishes a persistent wait rather than ongoing
proving. The isolated birthday baseline passed locally in 132.28 seconds, so that
run did not reproduce the hang. The old run used the earlier pushed revision;
later local commits did not alter it.

Independent SDK and fixture audits excluded explicit fixture wallet guards,
retained Store senders, stalled-response injection and a demonstrated repair-lock
cycle. They found a concrete loader/worker publication race in
`vendor/pepper-sync/src/scan/task.rs`: enqueue precedes busy=true, while a separate
consumer can finish and publish false before enqueueing returns. A later true
store loses that completion. Loader completion and further work are gated on idle,
so the stale flag can leave even finished scans polling indefinitely. This code
interleaving is proven; its occurrence in the captured CI future is not.

Independent design review approved publishing busy before enqueueing via a small
private std-only helper shared by both actual dispatch sites. Retain existing
channel panic contracts, capacities and consumer completion stores; add no rollback
state, timeout or recovery loop. Force an actual consumer to finish before the
enqueue closure returns, assert idle and repeat, and verify held unfinished work
remains busy. A negative mutation restoring the old order must fail.

Direct `-p pepper-sync` testing was checked and is unavailable because the excluded
dependency is not a workspace member. The reviewed alternative compiles the exact
private module into existing `tests/sdk_compat.rs`; no implementation is copied,
public API added, dependency checkout introduced or lock graph changed. A reversible
creation hunk belongs in the existing progress patch. Remove only the redundant
pre-reversal assumption that the patched inventory equals upstream: existing
maintained-file hashes already enforce its inventory, and post-reversal inventory
and hashes must still match upstream exactly. Add focused provenance rejection
controls and update the owning SDK/checkout guides.

Upstream investigation confirms the same race in Zingolib commit
`3bc86bec73db704e5fba242dc6221497da73e429` (November 12): its commit message
describes final scan completion overwritten by the dispatcher's later busy store
and shutdown-on-completion polling forever. Follow-up `4f411a573c792925aa4a58ad9fca166e72e7a379`
and subsequent encapsulation replace shared flags with scanner-owned keyed task
activity. Published tag `zingolib_v8.0.0`, resolved to `0df84af1016d243f9bf932a829756d87ce0a74e2`,
still contains both faulty enqueue-before-store sites. Inspected dev
`f930d80b6310a35c06e52408045f372f3c8ae261` is 72 commits and 71 files ahead of
our pin. No existing `reference_repos` checkout was available; primary upstream
sources and commit patches were inspected directly without creating one.

Independent design assessment retains the minimal local ordering correction:
the existing scanner dispatches only after observing idle, so publishing busy
first cannot be cleared by a preceding still-running task. Upstream's final
protocol also changes result-consumption and ended-runner behavior and is a
candidate for a separately qualified coherent cohort upgrade. This correction is
not an upstream cherry-pick, and neither it nor upstream confirmation establishes
which future the captured CI run awaited. Existing checkout retirement blockers
and the separate mempool-notice channel hazard remain open.

Run socket-free deterministic/provenance tests, selected production compiler and
all-target Clippy gates and focused source-matched SDK complexity review before
broader runtime tests. Then SDK compatibility, real treasury-sync consumers and
both stalled lifecycle cases execute sequentially on frozen sources. Generated
evidence belongs under `target/ci-lifecycle-wait-validation/`; no real funds,
consensus services or live Tor are selected. Hosted lifecycle/default acceptance
and the separate mempool-notice channel issue remain unfinished. User retains push.
