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

- State user intent, scope, current status, starting baseline, preserved
  invariants, and owning guides, distinguishing current behavior from proposed
  behavior.
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
- Include an `## Agent handoff protocol` section with a
  `### Execution checkpoints` subsection near the top of the plan so a fresh
  independent agent can resume work without prior conversation history (see
  [Agent handoff and progress updates](#agent-handoff-and-progress-updates)).

Run an independent design review before substantive implementation. Resolve
material findings before building on those decisions, and commit plan updates
whenever findings change the approach.

## Agent handoff and progress updates

Assume any execution turn can stop mid-plan (for example, due to usage limits
or switching agents). Structure every plan so an incoming agent can determine
the exact state from Git and the plan file alone, without scraping prior
transcripts or redoing finished work.

### Required handoff structure

Place `## Agent handoff protocol` directly after the opening objective and
status block, using two standard parts:

1. **Resume steps (under `## Agent handoff protocol`):**
   - List the exact read-only commands an incoming agent runs first to reconcile
     the checkout with the plan (for example, `git status`, `git log`, or branch
     ref listings for multi-repo work).
   - State the rule for skipping items already marked `Done` whose recorded
     commits and evidence match the checkout.
   - Reference or inline the self-contained build, test, and verification
     commands (such as `make` targets, `CARGO_ARGS`, `TEST_ARGS`, feature flags,
     or fixture commands) so the incoming agent does not rediscover flags,
     feature sets, or paths.
2. **Checkpoint table (under `### Execution checkpoints`):**
   - Maintain one canonical Markdown table covering every phase, commit
     boundary, integration gate, planned pause, or major work item in execution
     order. Include columns for `Phase or item`, `Target`, `Commit or ref`,
     `Validation`, `Review`, and `Status` (plans that track extra per-item
     artifacts, such as target bases or draft files, may add concise columns
     while keeping these core fields).
   - Restrict `Status` to four values: `Pending`, `In progress`, `Blocked`, or
     `Done`. Never mark an item `Done` until its required checks and review
     gates have passed. When an item is `Blocked`, keep `Status` set to
     `Blocked` and record the one-line blocker under
     `#### Open findings and next action`.
   - Include a short `#### Open findings and next action` bullet list directly
     below the table whenever a mid-phase next step, active blockers, or
     unresolved review findings exist.

### Updating progress in place

Keep plan updates uniform across agents so plans do not accumulate ad-hoc
diaries or noisy commit churn:

- **Edit checkpoints in place:** Update the matching row in
  `### Execution checkpoints` with the new commit SHA, validation outcome,
  review status, and `Status` value instead of appending dated status
  paragraphs, chronological prose logs, or per-command narratives to the plan.
- **Prune resolved items:** Remove fixed blockers, resolved review findings,
  and superseded next actions from `#### Open findings and next action` once
  verified, rather than crossing them out or keeping a running diary. Keep
  concise review dispositions, root causes, resolutions, and fixup SHAs in the
  checkpoint row, the plan's review record, or a companion review record.
- **Keep raw evidence out of Git:** Store verbose test logs, generated reports,
  diagnostic outputs, and private financial ledgers in ignored local directories
  (such as `target/`) or private storage, and record only a brief outcome
  summary and evidence path in the checkpoint row.
- **Batch checkpoint commits:** Avoid standalone progress-only commits after
  small edits. Fold prior reviewed SHAs and review outcomes into the next phase
  or milestone commit (never into `--fixup` commits). Commit a standalone
  checkpoint update when pausing mid-phase, or after a review, fixup round, or
  integration gate if no immediate implementation commit follows, so the next
  agent inherits committed progress.

### Planned pauses and multi-stage transitions

When a plan pauses for separate intervening work (such as a release, dependency
upgrade, or separate plan before a later phase), keep the handoff bounded in
three steps:

- **Before pausing:** Update the opening status block, completed checkpoint
  rows, and `#### Open findings and next action` in place. Keep verification
  commands, test lists, conflict rules, and later-phase instructions intact so
  the resuming agent does not have to rediscover them.
- **During intervening work:** Restrict edits in the paused plan to updating
  the opening status block, marking any pause row `Done`, recording follow-up
  fixes and the next action for resuming under
  `#### Open findings and next action`, and updating owned provenance or
  baseline references.
- **On resume:** When a resumed phase re-runs per-item rows against a new
  target base, update the opening status block and mark that phase row
  `In progress` with the target ref. Update each per-item row's `Target` or
  `Target base`, keep the prior ref in `Commit or ref` until replaced, and
  reset `Validation`, `Review`, and `Status` to `Pending` while leaving earlier
  phase, gate, and pause rows `Done`.

## Implement and maintain review evidence

Run exact-SHA and fixup reviews per
[independent subagent review](../AGENTS.md#independent-subagent-review) as work
progresses, citing exact commit SHAs and prior findings in each brief. Later
edits to reviewed work invalidate prior review evidence.

Update `### Execution checkpoints` and `#### Open findings and next action` in
place (see [Updating progress in place](#updating-progress-in-place)), and
record concise review details in the evolving plan (or a companion review record
with the same lifecycle):

- Reviewed SHAs, finding dispositions, root causes, resolutions, and fixup
  commits.
- Validation scope, build configuration, results, and remaining work
  (unavailable checks count as missing coverage, never passes).
- Evidence locations sufficient to verify claims.

Reuse still-valid evidence after localized edits and finish remaining checks.
Keep the plan accurate until final cleanup:

- Retain implemented steps, identifiers, intended behavior, and acceptance
  criteria instead of deleting completed steps mid-flight.
- Separate proposed, implemented, validated, and reviewed status clearly, and
  reopen matching checkpoint rows when later edits invalidate prior validation
  or review.
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

Update the gate's checkpoint row in place and record the reviewed scope, SHAs,
findings, and acceptance evidence in the plan. Exit a gate only when material
findings are resolved and required evidence is met. Later substantive changes
reopen dependent gates; never build dependent work on unresolved material
findings.

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

Before handing off, ensure the opening status block,
`### Execution checkpoints`, and `#### Open findings and next action` match the
checkout per [Updating progress in place](#updating-progress-in-place). Report
the working branch, reviewed SHA (or uncommitted snapshot), plan status, whether
plan cleanup is finished, unresolved findings, missing validation, and remaining
work. Distinguish implementation completion from review completion and from
permission to push or merge.

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
