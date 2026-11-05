# Planning workflow

This guide owns the lifecycle of implementation plans. Use it with the
[independent review requirements](../AGENTS.md#independent-subagent-workflows),
[validation guidance](testing.md#validation-order-and-continuation) and
[documentation policy](README.md). It is permanent guidance, not a task plan.

This workflow addresses the coordinating agent. Arrange reviews using the
[independence and coordination requirements](../AGENTS.md#independence-and-coordination),
[required reviews](../AGENTS.md#required-reviews) and
[review method](../AGENTS.md#review-method) in AGENTS.md; include the applicable
guidance in reviewer briefs.

## Create and commit the active plan

Keep active plans under `docs/plans/`. Before implementation, report the working
branch, starting baseline and plan path, and commit the plan. Preserve unrelated
workspace changes. The plan must:

- State user intent, scope, starting baseline, preserved invariants and owning
  guides. Distinguish current behavior from proposed behavior.
- Define ordered commit boundaries with stable identifiers, coherent behavior,
  relevant consumers and acceptance evidence. Include applicable tests and affected
  permanent documentation with each implementation commit. Keep removal of existing
  behavior with its necessary replacement or recovery path.
- Name independent design and final integration gates, plus intermediate
  integration gates where warranted by the work.
  Place integration gates at specified commit boundaries based on meaningful
  ownership or architecture changes. State their combined scope, evidence and exit
  criteria; scale their number to the work.
- Specify the evidence and ordering for exact-SHA, fixup, applicable complexity and
  integration reviews. Follow the existing validation order rather than creating a
  second test matrix.
- Identify permanent documentation destinations and required AGENTS.md links for
  enduring rules. Include documentation handoff and completed-plan cleanup in the
  acceptance evidence.

Arrange the required independent design review before substantive implementation.
Resolve material findings before building on affected decisions, and update and
commit the plan when those findings change its approach.

## Implement and maintain review evidence

Arrange the [required exact-SHA and fixup reviews](../AGENTS.md#required-reviews)
as implementation proceeds, supplying identified revisions and the
[review briefs and method](../AGENTS.md#independent-subagent-workflows).
Later changes invalidate affected review evidence.

Keep concise, committed evidence in the evolving plan: reviewed SHAs, finding
dispositions, causes, resolutions, fixup references, validation scope and results,
and remaining work. A separate review record is optional when the volume warrants
one; it has the same lifecycle as the plan. Do not duplicate generated reports or
private financial evidence. Evidence locations and relevant source/build
configuration must make validation claims verifiable; unavailable checks are
missing coverage, not passes. Reuse still-valid evidence after localized changes
and complete remaining required checks.

Treat the committed plan as an evolving review artifact throughout implementation
and review. Retain implemented portions, stable step identifiers, intended behavior
and acceptance criteria until final plan cleanup; do not progressively delete
completed work. Update status and supporting evidence to distinguish proposed,
implemented, validated and reviewed work. Record material changes in direction and
their rationale, marking superseded decisions clearly rather than silently
rewriting the original intent. Keep the plan accurate to the current implementation
and explicit about remaining differences. Update it when scope, commit boundaries
or acceptance evidence changes, and explain material departures at handoff.

Update permanent guides alongside the implementation they describe. Retaining
implemented material in the plan does not replace maintained documentation. Remove
the completed plan only through the completion workflow below.

## Pass integration gates

At each named gate, arrange the required independent combined review using the
[shared review method](../AGENTS.md#review-method). Identify all accumulated plan
commits and fixups, complete affected call chains, prior findings and relevant
validation evidence in the brief. Supply applicable complexity review evidence
under the [required reviews](../AGENTS.md#required-reviews). Earlier per-commit approval does
not exempt code from combined review.

Record the reviewed scope and SHAs, findings and acceptance evidence. Exit only
when material findings are resolved and required evidence is satisfied. Material
later changes reopen affected gates and require appropriate revalidation; do not
build dependent work on unresolved material findings. The final implementation
gate covers the full planned change and all prior fixups before plan cleanup.

## Continue review rounds and report state

Preserve the committed plan, review evidence and unsquashed fixups through final
implementation review and subsequent user review rounds. After the final
implementation gate, use the following review and correction loop as the last
substantive phase before plan removal. The gate's review may serve as the first
round; no duplicate review is required for an unchanged snapshot.

1. Assess findings and simplification proposals against the authorized scope and
   required contracts, using the
   [shared simplification criteria](../AGENTS.md#review-method).
2. Address material findings and selected compelling simplifications in focused
   fixup commits against their introducing changes, with appropriate regression
   coverage and documentation. Keep coupled corrections together when required for
   coherent behavior. Independently review each exact fixup SHA.
3. Reuse valid validation evidence. Rerun affected checks and reopen affected gates
   when code, tests, dependencies or build inputs invalidate their evidence;
   complete remaining required checks. Do not restart the full suite solely because
   another review round occurred.
4. Arrange an independent combined review of the resulting implementation using
   the shared review method and all prior findings and fixups. Repeat when material
   findings or compelling simplifications warrant correction before integration.

Record each finding's evidence and disposition. Explain rejected recommendations;
give genuinely deferred work a focused plan, concrete next action and acceptance
evidence. Broader work needs a concrete connection to the change or a recurring
failure pattern; record unrelated opportunities separately without silently
expanding the task. Deferring material findings does not waive release conditions.

Exit when the exact reviewed state has no unresolved material findings or
compelling simplifications worth completing before integration, and required
acceptance evidence is satisfied. No fixed iteration count or exhaustive search
for hypothetical improvements is required. Report missing coverage as a limitation,
not a pass or evidence of convergence. This loop requires no framework-specific
goal command and grants no additional execution or spending authority.

At each handoff, report the working branch and reviewed SHA or identified
uncommitted snapshot, the plan's status, unresolved findings, missing validation
and remaining work. Distinguish implementation completion from review completion
and authorization to push or merge. State whether cleanup is complete or still
pending; do not describe an unreviewed change as fully reviewed.

## Complete the plan and preserve its history

Once the review and correction loop and applicable user review rounds are complete:

1. Verify that enduring rules and rationale are in maintained guides, with required
   AGENTS.md links. Move genuinely unfinished work into focused active or deferred
   plans with concrete next actions and acceptance evidence. Deferral does not fix
   a finding or waive its release conditions.
2. Delete the completed plan and any accompanying temporary review record, repair
   links and commit the cleanup. Do not keep a completed-plan archive or defer all
   permanent documentation to this commit.
3. Arrange independent review of the exact cleanup SHA and final combined branch,
   supplying earlier commits and fixups, permanent documentation and remaining
   plans under the shared review requirements. Resolve material findings and reopen
   the review loop and affected gates when substantive findings emerge.

The normal workflow commits, updates and ultimately deletes the plan on the same
working branch. Preserve that ancestry through integration so the plan and review
evidence remain retrievable from Git history. Do not squash away those commits,
reconstruct a plan-free delivery branch or rewrite away plan history unless the
user explicitly requests a different history policy. Routine local cleanup needs
no separate delivery branch or approval gate. Pushing and merging remain subject
to the user's authorization; plan completion does not authorize either.

Documentation or Git cleanup never deletes private financial evidence or grants
replay authority. Generated reports and private ledgers retain their existing
storage and ownership rules.
