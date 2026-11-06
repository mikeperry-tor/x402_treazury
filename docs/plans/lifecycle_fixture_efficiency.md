# Synthetic lifecycle fixture efficiency

## Intent and baseline

Implement the operator's three selected reductions in synthetic lifecycle work,
without weakening recovery, original accepted work, disclosure evidence or negative
RPC assertions. Branch `nu7_release_readiness`; baseline
`19b2e3cb575e7a50190a2c25434af3ad69797dc4`. Preserve unrelated `.vscode/` and
`TODO`, private evidence and existing source/build pins. No production behavior,
Make feature selections, compiler gates, timers, limits or dependencies change.

Follow [planning workflow](../planning-workflow.md),
[testing efficiency](../testing.md#testing-efficiency),
[validation ordering](../testing.md#validation-order-and-continuation),
[lifecycle contracts](../../tests/LIFECYCLE.md),
[reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy) and
[wallet observation](../wallet-rotation.md#wallet-observation-policy).

## Ordered implementation and review gates

| ID | Commit boundary and preserved acceptance |
| --- | --- |
| F1 | Shorten only the manually missing-checkpoint SDK reorg variant to rollback 20 (tip 43 replaced by 33, checkpoint 23 manually absent). Retain actual 1000-window boundary 999, deep 1011 and same-height 1001 cases, exact divergent hash, reset/reopen, byte/input/liability and second-scan assertions. |
| F2 | Expand existing cheap Store projection test to cover JSON 0, maximum u32 and invalid string (retain existing 10000 if useful), authenticating signed expiry/txid and preserving complete facts/encrypted bytes. Reduce expiry integration from 3 to 2 cold/warm journeys; retain actual one-scan count, no submit/lookup and malformed sequencing preservation in the warm case. |
| F3 | Reduce confirmation integration from 4 to 3 journeys, combining malformed sequencing with alternate authorization. Retain clean never-disclosed attempts 0, submitted attempts 1, real ZIP244 alternate bytes and zero lookup/replay, original facts/bytes and damaged-counter preservation. Update owning lifecycle guidance alongside these boundaries. |

Independent read-only design review at the baseline passes without material
findings. It traced fork construction, pinned SDK verification/truncation,
authenticated recovery and existing Store malformed-health/disclosure coverage.
Existing cheap tests already cover counter permutations; expand the projection
case rather than create another fixture framework or validation matrix.

Independently review every exact implementation/fixup SHA and its relevant
consumers; preserve unsquashed history. Source-matched complexity review covers
changed test functions and meaningful surrounding recovery chains. Before
completion, independently review the combined F1-F3 implementation and evidence,
resolve material findings/compelling simplifications, then review exact cleanup.
Generated metrics/logs stay under ignored `target/lifecycle-efficiency/`.

## Validation and evidence reuse

Sources and executable configuration freeze during validation. Coordinate editor
Cargo processes; if rust-analyzer restarts Cargo, stop and notify the operator as
previously requested. Tests use only deterministic unfunded keys, temporary state
and localhost fixtures, never production wallets.

Run formatting and source-matched complexity; carry forward unchanged analyzer
qualification, Python tooling and four-vendor provenance evidence from
`target/nu7-readiness/vendor-removal-*` at code `d97b23b9` (later baseline commits
are documentation only). Complete independent design/complexity and the production
compiler/full all-target Clippy gate. Run the affected inexpensive Store group,
then one full timed lifecycle suite, which executes all three changed cases once.
Do not first repeat those expensive cases individually without a failure or other
concrete reason. Use the runner's socket-capable authority up front for fixtures.

Compare the same lifecycle selection/thread count (57 pass/1 ignored,
572.33 seconds at the prior frozen graph) and inspect changed scenario execution;
report observed time without a timing assertion or guaranteed percentage. Cheap
Store test count should remain stable; its input rows expand. Carry forward
unaffected default/compatibility/proving/codec/optimized evidence: only cfg(test)
fixture code and owning guidance change, no production arithmetic, graph, signing,
transport or proof algorithms. Revalidate affected consumers after any failure and
resume unfinished checks, without restarting passed unrelated stages.

## Implementation and validation checkpoint

F1 `de527d1d`, F2 `04faa90c`, F3 `8881fa6e`, formatting fixup `bfd02e2f`
and comment correction `f1010d9b` all pass independent exact-SHA and combined
source review. The source comment now correctly describes old tip 43 replaced by
33. No material findings remain in those reviews. Changes are confined to the
three lifecycle functions, the existing Store projection test and owning guidance;
production behavior, the dependency graph, Make and test counts are unchanged;
existing assertions are retained.

Formatting passes. Final source-matched metrics at frozen code `f1010d9b` are in
`target/complexity-runs/20261106T184820Z-6865b609/`: 470 files / 10,373 callable
spaces, no exclusions or baseline-comparison warnings. Independent complexity
review accepts reorg/expiry/confirmation/Store projection and the relevant
unchanged canonical recovery ordering without further refactoring.

All selected validation passes, with commands, logs and exits under
`target/lifecycle-efficiency/`:

- `store-tests`: all 11 inexpensive inventory tests pass in 0.72 seconds, including
  all four JSON expiry inputs, malformed health/disclosure and immutable bytes.
- `clippy`: production compiler and full all-target `zcash-regtest` Clippy pass
  without diagnostics (40.07 and 239 seconds respectively).
- `lifecycle`: the one complete `zcash-testutils` run at eight threads passes all
  57 cases, with the same one deliberately ignored case. It executes all three
  modified tests and all six real-SDK refund/discovery privacy regressions.
  Test execution is 538.30 seconds, versus the prior 572.33 seconds: an observed
  34.03-second / 5.95% reduction from one comparison, not a repeatability guarantee.
  Its executable rebuild takes 28.33 seconds; compilation and execution are reported
  separately. Both Store and lifecycle debug links retain the already assessed
  macOS large unwind-table warning; production/Clippy are warning-free.

Unchanged compatibility 31, default continuation 1914/42 ignored, proving 4,
codec 1, optimized compatibility 20/signer 2, Python 96/analyzer 10 and retained
vendor provenance evidence carry forward from the baseline's recorded
`vendor-removal-*` logs and final source review. These fixtures change no shared
production algorithms, feature/profile selection, helper implementations or tool
sources; no broad or optimized suite was repeated. No additional performance
matrix or production tuning is claimed. Independent final combined runtime/evidence review accepts the exact frozen
implementation, all accumulated commits/fixups and affected call chains with no
material findings or compelling simplifications. It verifies raw logs/exits,
actual scenario execution, comparison limits and the evidence carried forward.
Exact evidence-record and cleanup review remain pending.

## Completion and handoff

Current fixture rationale belongs in `tests/LIFECYCLE.md`; existing AGENTS/testing
links already identify that owner. Commit concise reviewed SHAs, findings,
validation/configuration and evidence reuse here as work proceeds. After combined
review and user scope are satisfied, remove this completed plan, preserve its Git
history and independently review exact cleanup/final branch. No push, merge,
financial execution, live qualification or history rewrite is authorized.
