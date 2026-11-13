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
C5 exact-commit review and C6 compiler/runtime validation remain pending.
