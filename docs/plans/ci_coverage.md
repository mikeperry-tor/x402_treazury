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
