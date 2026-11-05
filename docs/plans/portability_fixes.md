# macOS ACL and Unix storage follow-up

## Intent, scope and baseline

This focused unfinished plan preserves two storage concerns from the review work
at `51549a453b5612b02332fa91625358e1ec568c72`. They are separate follow-up work,
not prerequisites for the remaining financial, catalog, registry and R9 milestone.
No storage repair described here is implemented or qualified by this plan.

The user explicitly deferred Windows runtime, private-storage, supervision and
native acceptance work and removed its dedicated CI/experiment drivers. Windows
work is deferred, not complete. Portable ErrorKind reporting, unfunded connectivity
controls, generic fixtures and strict binary vendor verification remain. Existing
guarded platform fallbacks do not establish Windows support. Optional qualification
backends and live-funded qualification are outside this work.

Preserve existing state formats, locks, exact accepted inputs and bytes, encrypted
snapshots, financial evidence and SDK-owned spendability. Storage cleanup grants
no replay, abandonment, input release, top-up or replacement authority. Do not add
monetary ledgers, caps, deadlines or a general filesystem framework to resolve these
focused issues. Preserve unrelated local files, dependency/toolchain pins and the
reproducible-build sandbox.

## Unfinished findings

| Concern | Evidence and remaining contract |
| --- | --- |
| macOS inherited ACLs defeat mode-only privacy | A non-root public disposable-file reproduction denied a fresh read of an ordinary mode-0000 file, but allowed a fresh read of a mode-0000 file inheriting Everyone READ. After chmod 0600, the current mode-only check accepted it while the inherited ACL remained. No real state or keys were accessed. Require private creation before sensitive bytes, truthful descriptor-based reopen validation and SQLite sidecar behavior; chmod alone is insufficient. |
| Original Unix namespace authority | Source/design analysis identified that an attacker-controlled initial alias could select an older owner-private authenticated state copy. Final mode, ownership and valid treasury identity alone do not prove the intended configured-name authority. Assess actual callers and protected-entry assumptions, preserving legitimate OS aliases such as macOS `/var` and sticky owned temporary directories. This is an unresolved threat analysis, not a demonstrated breach of the normal private-parent case. |

The public reproduction and source research remain in ignored/private evidence;
Git and evidence retention are not replaced by a completed-plan archive.

## Ordered corrective boundaries and review

1. **U1 — macOS ACL creation and reopen:** independently review the smallest
   correction in the actual private-file/directory consumers before implementation.
   Cover keys, journals, source/cache/registry files and backup/publication paths
   where affected. Establish privacy at creation before bytes, reject unsafe
   inherited access on reopen, and distinguish authoritative ACL absence from
   inspection errors. Include actual SQLite WAL/SHM/journal children, read-only
   inspection, lock/exclusive reopen, atomic no-clobber publication and restart.
   Consolidate faithful native disposable fixtures, including inherited-ACL positive
   and denied-read controls; mode checks or a successful build alone cannot qualify
   privacy. Update the owning guides in the same coherent implementation commit.
2. **U2 — Unix configured-name authority:** first audit current Store, registry,
   temporary and publication consumers against the concrete older-state alias
   threat. Resolve whether existing protected parents suffice and repair only
   demonstrated gaps. Preserve legitimate trusted aliases; do not assume every
   ancestor must be 0700 or that canonicalization alone proves original authority.
   Couple any correction with replacement/alias and restart regressions and owning
   documentation. Do not preselect a manual path-walking framework.

Each boundary needs independent necessity/design challenge, exact-SHA and fixup
review with unsquashed history, relevant compiler/compatibility/native tests and
source-matched complexity review for substantive Rust changes. A combined storage
review must inspect complete create/open/sidecar/replace/backup/recovery consumers
and accepted financial ownership after the affected corrections. Review recurring
fixup causes and simpler representations; resolve material findings before
claiming completion. Evidence must name actual source, host, commands and nonzero
execution counts. No additional platform lifecycle gate is imposed on the current
financial/catalog/registry/R9 milestone by this unfinished follow-up.

## Owning guidance and primary references

Keep enduring policy in [development](../development.md#portability),
[testing](../testing.md), [wallet administration](../wallet-cli.md),
[financial reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy)
and the affected source/cache guides. AGENTS links the development policy. Before
removing this plan, verify that handoff, independently review its exact cleanup
SHA and the assembled implementation, and leave any unresolved work explicit.

Apple documents [file security and permissions](https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/FileSystemDetails/FileSystemDetails.html)
and [authorization permissions](https://developer.apple.com/library/archive/documentation/Security/Conceptual/AuthenticationAndAuthorizationGuide/Permissions/Permissions.html).
The [descriptor ACL API](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man3/acl_get_fd.3.html)
and [pinned XNU authorization source](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/kern_authorization.c)
provide starting contracts; implementation must verify actual ACL/error semantics
and native creation/inheritance behavior rather than infer privacy from mode bits.
