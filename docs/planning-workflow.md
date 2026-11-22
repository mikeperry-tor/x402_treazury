# Planning workflow

This guide owns the lifecycle of implementation plans. Use it alongside
[AGENTS.md](../AGENTS.md),
[validation order and continuation](testing.md#validation-order-and-continuation),
and the [documentation policy](README.md). It is permanent guidance, not a task
plan.

Write a committed plan when the user asks for one, when continuing an existing
plan, or when staged work spans multiple dependent phases with distinct design
decisions, commit boundaries, and review gates. Standalone non-trivial changes
and substantive documentation updates use the
[independent subagent review](../AGENTS.md#independent-subagent-review)
workflow in `AGENTS.md` without requiring a plan file; trivial edits (such as
typos or small documentation tweaks) need neither.

This workflow addresses the coordinating agent. Follow the
[independent subagent review](../AGENTS.md#independent-subagent-review),
[review method and findings](../AGENTS.md#review-method-and-findings), and
[writing style](../AGENTS.md#writing-style) rules in `AGENTS.md`, and include
the applicable guidance and context in every reviewer brief.

## Create and commit the active plan

Keep active plans under `docs/plans/`. Before implementation, report the working
branch, starting baseline, and plan path, then commit the plan while preserving
unrelated workspace changes. A complete plan must:

- State user intent, scope, starting baseline, preserved invariants, and owning
  guides, distinguishing current behavior from proposed behavior.
- Define ordered commit boundaries with stable identifiers, coherent behavior,
  consumers to update, and acceptance evidence. Include tests covering the
  change and updates to affected permanent documentation with each
  implementation commit, and keep behavior removals paired with their
  replacement or recovery path.
- Name independent design and final integration gates, plus intermediate
  integration gates at commit boundaries with meaningful ownership or
  architecture changes. State each gate's combined scope, evidence, and exit
  criteria, and scale gate count to the work.
- Specify evidence and ordering for exact-SHA, fixup, complexity, and
  integration reviews, following the validation order in `docs/testing.md`
  rather than creating a second test matrix.
- Identify permanent documentation destinations and required `AGENTS.md` guide
  links for enduring rules. Include documentation review, writing-style
  compliance, and completed-plan cleanup in acceptance criteria.

Arrange independent design review before substantive implementation. Resolve
material findings before building on those decisions, and update and commit the
plan whenever findings change the approach.

## Implement and maintain review evidence

Arrange exact-SHA and fixup reviews per
[independent subagent review](../AGENTS.md#independent-subagent-review) as
implementation proceeds, identifying exact commit SHAs and prior findings in
each brief. Later changes invalidate affected review evidence.

Keep concise, committed evidence in the evolving plan:

- Reviewed SHAs, finding dispositions, root causes, resolutions, and fixup
  references.
- Validation scope, source and build configuration, results, and remaining work
  (unavailable checks are missing coverage, never passes).
- Evidence locations and configuration details sufficient to make validation
  claims verifiable, without duplicating generated reports or private financial
  evidence.

A separate review record is optional when volume warrants one and shares the
plan's lifecycle. Reuse still-valid evidence after localized changes and finish
any remaining required checks.

Treat the committed plan as an evolving review artifact until final cleanup:

- Retain implemented steps, stable identifiers, intended behavior, and
  acceptance criteria; do not progressively delete finished work.
- Update status and evidence to separate proposed, implemented, validated, and
  reviewed work.
- Record material direction changes and their rationale, marking superseded
  decisions clearly rather than silently rewriting original intent.
- Keep the plan accurate to the branch and explicit about remaining differences,
  update it when scope, commit boundaries, or acceptance evidence change, and
  explain material departures at handoff.

Update maintained guides alongside the implementation they describe. Retaining
implemented notes in a plan never replaces maintained documentation. Remove the
completed plan only through the completion workflow below.

## Pass integration gates

At each named gate, run an independent combined review following
[review method and findings](../AGENTS.md#review-method-and-findings). Include
the net diff from the starting baseline, all accumulated plan commits and
fixups, complete call chains, prior findings, validation evidence, and
complexity review results per
[complexity review discipline](../tests/COMPLEXITY.md#review-discipline) (for
substantive Rust refactors and architecture gates). Earlier per-commit approval
never exempts a branch from combined review.

Record the reviewed scope, SHAs, findings, and acceptance evidence in the plan.
Exit a gate only when material findings are resolved and required evidence is
satisfied. Later substantive changes reopen affected gates and require
appropriate revalidation; never build dependent work on unresolved material
findings. The final implementation gate covers the full planned change and all
prior fixups before plan cleanup.

## Review the whole changeset until clean

Preserve the committed plan, review evidence, and unsquashed fixups through
final implementation review and user review rounds. After the final
implementation gate, run the following whole-changeset review loop before plan
removal (the gate review may serve as the first round; skip to Step 2 when the
gate review already covered the unchanged net diff, and do not duplicate review
on an unchanged snapshot):

1. Have a fresh independent reviewer inspect the **net combined diff from the
   starting baseline (`git diff <base>..HEAD`) as a single unit**, alongside
   all prior findings and fixups. Reviewing the net changeset as one unit
   exposes defects, cross-file inconsistencies, and simplification opportunities
   that are easy to miss when wading through sequential fixup commits.
2. Assess findings, documentation-review items, and simplification proposals
   against the authorized scope and preserved contracts using
   [review method and findings](../AGENTS.md#review-method-and-findings).
3. Address material findings and compelling simplifications worth resolving
   before integration in focused `git commit --fixup=<introducing-SHA>` commits
   with regression coverage and documentation updates (keeping coupled fixes
   together when coherence requires it), and independently review each fixup
   SHA.
4. Reuse still-valid validation evidence: rerun affected checks and reopen gates
   whose inputs changed, and finish any remaining checks without restarting the
   full suite solely because another review round ran.
5. Repeat Step 1 with a fresh reviewer over the updated whole-changeset diff
   until a clean pass finds no remaining material findings or compelling
   simplifications worth resolving before integration.

Record each finding's evidence and disposition, explaining rejected
recommendations. Give genuinely deferred work a focused plan, concrete next
action, and acceptance evidence; deferring a material finding never waives
release conditions. Broader work requires a concrete link to the change or a
recurring failure pattern; record unrelated ideas separately rather than
silently expanding scope.

Exit when the exact reviewed changeset has no unresolved material findings or
compelling simplifications worth resolving before integration and all acceptance
evidence is satisfied. No fixed iteration count or search for hypothetical
improvements is required, and missing coverage is always reported as a
limitation, never a pass or evidence of convergence. This loop needs no
framework-specific goal command and grants no extra execution or spending
authority.

### Report state at handoff

At each handoff, report the working branch, reviewed SHA (or identified
uncommitted snapshot), plan status, whether plan cleanup is complete or still
pending, unresolved findings, missing validation, and remaining work.
Distinguish implementation completion from review completion and from
authorization to push or merge, and never describe an unreviewed change as fully
reviewed.

## Complete the plan and preserve its history

Once the whole-changeset review loop and applicable user review rounds pass
cleanly:

1. Verify that enduring rules, contracts, and rationale live in maintained
   guides (matching [writing style](../AGENTS.md#writing-style) and with
   required `AGENTS.md` links). Move genuinely unfinished work into focused
   active or deferred plans with concrete next actions and acceptance evidence.
   Deferring a finding never fixes it or waives its release conditions.
2. Delete the completed plan and any temporary review record, repair inbound
   links, verify `git diff --check`, and commit the cleanup. Do not create a
   completed-plan archive or defer permanent documentation to this commit.
3. Run a fresh independent review of the exact cleanup SHA and the final
   plan-free combined branch (`git diff <base>..HEAD`), supplying earlier
   commits, fixups, prior findings, maintained documentation, and remaining
   plans. If substantive findings emerge, resolve them and reopen the review
   loop and affected gates until clean.

Commit, update, and delete the plan on the same working branch, preserving that
ancestry through integration so its history and review evidence remain
retrievable in Git. Do not squash away plan commits, build a plan-free delivery
branch, or rewrite history unless the user requests a different history policy.
Routine local cleanup needs no separate delivery branch or approval gate;
pushing and merging remain subject to user authorization, and plan completion
never authorizes either.

Documentation or Git cleanup never deletes private financial evidence or grants
replay authority. Generated reports and private ledgers keep their storage and
ownership rules.
