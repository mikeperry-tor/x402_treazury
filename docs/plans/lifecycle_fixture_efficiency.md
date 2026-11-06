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
| F1 | Shorten only the manually missing-checkpoint SDK reorg variant to rollback20 (tip43 replaced by33, checkpoint23 manually absent). Retain actual1000-window boundary999, deep1011 and same-height1001 cases, exact divergent hash, reset/reopen, byte/input/liability and second-scan assertions. |
| F2 | Expand existing cheap Store projection test to cover JSON0, maximumu32 and invalid string (retain existing10000 if useful), authenticating signed expiry/txid and preserving complete facts/encrypted bytes. Reduce expiry integration from3 to2 cold/warm journeys; retain actual one-scan count, no submit/lookup and malformed sequencing preservation in the warm case. |
| F3 | Reduce confirmation integration from4 to3 journeys, combining malformed sequencing with alternate authorization. Retain clean never-disclosed attempts0, submitted attempts1, real ZIP244 alternate bytes and zero lookup/replay, original facts/bytes and damaged-counter preservation. Update owning lifecycle guidance alongside these boundaries. |

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

## Completion and handoff

Current fixture rationale belongs in `tests/LIFECYCLE.md`; existing AGENTS/testing
links already identify that owner. Commit concise reviewed SHAs, findings,
validation/configuration and evidence reuse here as work proceeds. After combined
review and user scope are satisfied, remove this completed plan, preserve its Git
history and independently review exact cleanup/final branch. No push, merge,
financial execution, live qualification or history rewrite is authorized.
