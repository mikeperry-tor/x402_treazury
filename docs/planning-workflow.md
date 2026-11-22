# Planning workflow

This guide owns the lifecycle of implementation plans. Use it alongside
[AGENTS.md](../AGENTS.md),
[validation order and continuation](testing.md#validation-order-and-continuation),
and the [documentation policy](README.md).

Write a committed plan under `docs/plans/` when the user asks for one, when
continuing an open plan, or when staged work spans multiple dependent phases
with distinct design decisions, commit boundaries, and review gates.
Standalone non-trivial changes and substantive documentation updates use the
[independent subagent review](../AGENTS.md#independent-subagent-review)
workflow directly without a plan file; trivial edits (such as typos or tiny
wording tweaks) need neither.

Follow the
[independent subagent review](../AGENTS.md#independent-subagent-review),
[review method and findings](../AGENTS.md#review-method-and-findings), and
[writing style](../AGENTS.md#writing-style) rules in `AGENTS.md`, and include
the needed context in every reviewer brief.

## Create and commit the active plan

Before implementation, report the working branch, starting baseline, and plan
path, then commit the plan while preserving unrelated workspace changes. A
complete plan must:

- State user intent, scope, starting baseline, preserved invariants, and owning
  guides, distinguishing current behavior from proposed behavior.
- Define ordered commit boundaries with stable identifiers, coherent behavior,
  consumers to update, and acceptance evidence. Pair each implementation commit
  with its tests and permanent documentation updates, and pair behavior
  removals with their replacement or recovery path.
- Name independent design and final integration gates, plus intermediate
  integration gates at major ownership or architecture boundaries, stating each
  gate's scope, evidence, and exit criteria.
- Specify exact-SHA, fixup, complexity, and integration review evidence
  following the validation order in `docs/testing.md` rather than inventing a
  second test matrix.
- Identify permanent documentation destinations and `AGENTS.md` guide links for
  enduring rules, including writing-style review and completed-plan cleanup in
  acceptance criteria.

Run an independent design review before substantive implementation. Resolve
material findings before building on those decisions, and commit plan updates
whenever findings change the approach.

## Implement and maintain review evidence

Run exact-SHA and fixup reviews per
[independent subagent review](../AGENTS.md#independent-subagent-review) as work
progresses, citing exact commit SHAs and prior findings in each brief. Later
edits invalidate prior review evidence.

Record concise evidence in the evolving plan (or a companion review record with
the same lifecycle):

- Reviewed SHAs, finding dispositions, root causes, resolutions, and fixup
  commits.
- Validation scope, build configuration, results, and remaining work
  (unavailable checks count as missing coverage, never passes).
- Evidence locations sufficient to verify claims without copying generated
  reports or private financial ledgers into Git.

Reuse still-valid evidence after localized edits and finish remaining checks.
Keep the plan accurate until final cleanup:

- Retain implemented steps, identifiers, intended behavior, and acceptance
  criteria instead of deleting completed steps mid-flight.
- Separate proposed, implemented, validated, and reviewed status clearly.
- Mark superseded decisions and their rationale rather than silently rewriting
  original intent.
- Update maintained guides alongside the implementation they describe; plan
  notes never replace maintained docs.

## Pass integration gates

At each named gate, run an independent combined review following
[review method and findings](../AGENTS.md#review-method-and-findings) covering
the net diff from the starting baseline, all accumulated plan commits and
fixups, complete call chains, prior findings, validation evidence, and
complexity review results per
[complexity review discipline](../tests/COMPLEXITY.md#review-discipline) (for
substantive Rust refactors and architecture gates). Per-commit approval never
replaces combined review.

Record the reviewed scope, SHAs, findings, and acceptance evidence in the plan.
Exit a gate only when material findings are resolved and required evidence is
met. Later substantive changes reopen dependent gates; never build dependent
work on unresolved material findings.

## Review the whole changeset until clean

Keep the committed plan, review evidence, and unsquashed fixups through final
implementation review and user review rounds. After the final implementation
gate, run this whole-changeset loop before plan removal (if the gate review
already covered the unchanged net diff, skip directly to step 2):

1. Have a fresh independent reviewer inspect the **net combined diff from the
   starting baseline (`git diff <base>..HEAD`) as one unit**, along with all
   prior findings and fixups, to catch cross-file defects and simplification
   opportunities hidden across sequential fixups.
2. Evaluate findings, documentation issues, and simplification proposals against
   authorized scope and preserved contracts using
   [review method and findings](../AGENTS.md#review-method-and-findings).
3. Fix material findings and compelling simplifications in focused
   `git commit --fixup=<introducing-SHA>` commits with regression tests and doc
   updates, and independently review each fixup SHA.
4. Reuse still-valid validation evidence: rerun dependent checks, reopen gates
   whose inputs changed, and finish remaining checks without needlessly
   restarting unrelated stages.
5. Repeat step 1 with a fresh reviewer over the updated combined diff until a
   clean pass finds no remaining material findings or compelling
   simplifications worth resolving before integration.

Record each finding's evidence and disposition, explaining rejected proposals.
Move genuinely deferred work into a focused plan with a concrete next action
and acceptance criteria; deferring a finding never waives release conditions.
Keep scope bounded to the user's request and recurring failure patterns.

### Report state at handoff

At each handoff, report the working branch, reviewed SHA (or uncommitted
snapshot), plan status, whether plan cleanup is finished, unresolved findings,
missing validation, and remaining work. Distinguish implementation completion
from review completion and from permission to push or merge.

## Complete the plan and preserve its history

Once the whole-changeset review loop and user review rounds pass cleanly:

1. Confirm all enduring rules, contracts, and rationale live in maintained
   guides (compliant with [writing style](../AGENTS.md#writing-style) and
   linked from `AGENTS.md`), and move any genuinely unfinished work into a
   focused active or deferred plan.
2. Delete the completed plan and any temporary review record, repair inbound
   links, verify `git diff --check`, and commit the cleanup without creating a
   completed-plan archive.
3. Run a fresh independent review of the cleanup SHA and the final plan-free
   combined branch (`git diff <base>..HEAD`). If substantive findings emerge,
   resolve them and reopen the review loop until clean.

Commit, update, and delete the plan on the same working branch so its history
and review evidence remain retrievable in Git. Do not squash away plan commits,
build a separate plan-free branch, or rewrite history unless the user requests
it. Pushing and merging always require explicit user authorization.
Documentation and Git cleanup never alter or delete private financial ledgers.
