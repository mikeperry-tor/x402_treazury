# Unix configured-name authority assessment

## Intent, scope and baseline

U2 retains the focused unfinished namespace-authority assessment identified at
`51549a453b5612b02332fa91625358e1ec568c72`. C07 of the
[review cleanup plan](review_findings_cleanup.md) owns Linux/macOS ACL creation and
reopen privacy implementation and its native acceptance; it is no longer duplicate
active work here. Current privacy contracts belong in
[development](../development.md#portability) and [wallet administration](../wallet-cli.md).
Windows implementation remains [deferred release work](deferred/release_followups.md).

Preserve state formats, locks, exact accepted inputs/bytes, encrypted snapshots,
financial evidence and SDK-owned spendability. Assessment or cleanup grants no
replay, abandonment, input release, top-up or replacement authority. No general
filesystem framework, monetary ledger or new spending cap is preselected.

## U2 — configured-name authority

Source/design analysis identified that an attacker-controlled initial alias could
select an older owner-private authenticated state copy. Final permissions, ownership
and a valid treasury identity do not prove intended configured-name authority.
This is an unresolved threat analysis, not a demonstrated breach of normal
private-parent deployments. Atomic private creation and descriptor ACL checks do
not detect authenticated-state rollback.

Next, audit actual Store, registry, temporary and publication consumers against that
concrete older-state alias scenario. Determine whether existing protected parents
suffice and repair demonstrated gaps only. Preserve legitimate system aliases such
as macOS `/var` and sticky owned temporary directories. Do not assume every ancestor
must be 0700 or canonicalization alone proves original configured-name authority.

Acceptance requires an evidence-backed threat decision with complete affected
create/open/replace/backup/recovery chains and independent design/implementation
review. Any correction needs replacement/alias and restart regressions, owning-guide
updates, exact-SHA/fixup review with preserved history, appropriate native validation
and source-matched complexity review for substantive Rust changes. Source analysis
or compilation alone does not qualify power-loss recovery or namespace semantics.
Record native host, exact revision, commands and nonzero execution counts in ignored
or private evidence. Remove this plan only under the
[planning workflow](../planning-workflow.md) once U2 is resolved.
