# Simplify settlement and chain recovery

The operator wants automatic progress from blockchain/SDK facts, finite transaction
expiry and NEAR-owned swap settlement. The current branch overextended source
reorg recovery into lifetime historical swap reconciliation. Reform this branch
in place; the recovery checkpoint is
`review/usdc-reorg-before-model-reform-20261028` at `973d0574`.
The original review baseline remains `0a6dbb93` for accumulated complexity review.

Preserve journal-before-disclosure, exact accepted inputs and original signed
bytes, actual pending transaction ownership, first-positive-credit claim
consumption, retired signer exclusion, configured payment/funding/fee authority,
network isolation and existing observation owners. Never infer another deposit
from refunds or source expiry. A provider operation may remain unresolved after
its source transaction expires. No new ledger, watcher, timer or zero-expiry
framework is proposed. Production deposits already use ordinary SDK finite expiry.

Owning maintained guides: wallet-rotation.md, wallet-cli.md,
zingo-integration.md, configuration.md, network-egress.md, vendor/README.md and
tests/LIFECYCLE.md. AGENTS.md links these guides. Completed plans are review
artifacts, not maintained policy, and this file will be removed after handoff.

Ordered coherent changes:

1. **M1 Delegate settlement and remove lifetime source reconciliation.**
   A validated, bound NEAR SUCCESS closes source work without fabricating Zcash
   confirmation or Base credit. First-positive Base observation remains advisory
   readiness and consumes the initial claim; the existing credit lane finishes
   independently of source confirmation. Prevent a new calculation after SUCCESS,
   retain an already accepted calculation until its exact result is journaled,
   close late results without disclosure, and let already accepted I/O drain.
   Provider FAILED/REFUNDED closes bridge observation, while genuinely pending
   source work retains its original owner until canonical confirmation/finite
   expiry. INCOMPLETE/unknown remain unresolved. Remove historical terminal-source
   rescans, reopening and app-level chain-replacement event plumbing. Keep real SDK
   missing-boundary/pruned-checkpoint/deep-rescan repairs; restore upstream rolling
   retention instead of equating a node retention window with finality. Preserve
   schema-22 history, snapshot pins and the read-only no-automatic-disclosure guard
   for previously reopened originals; tolerate multiple legacy pending originals.
   Update permanent guides with this commit. Cover SUCCESS races, delayed credit,
   restart, pending/terminal provider combinations, unchanged terminal work across
   SDK rescan and genuinely unfinished original preservation.
2. **M2 Remove redundant historical Base consistency authority.** Independently
   approved scope deletes historical balance-support and archived-resolution
   vetoes and retired-support anchor RPC revalidation. Preserve all audit bytes.
   Retain current authorization binding, generation fences, finalized/depth RPC
   tuples and checkpoint/debit publication ordering. Fix equal-confirmed-height
   cache ordering using latest height, with an old-zero cache, newer positive
   latest read and out-of-order retiring publication regression. M2 is independent
   of M1 source semantics and can be prepared while its final focused regression
   work finishes; commit M1 before M2, then validate frozen combined sources. No
   generic corruption-repair framework is proposed.
3. **M1b Recover unpublished calculations from the disclosure boundary.**
   Current intent-only crash recovery still requires operator review. Production
   calculation is offline; only an atomically committed outgoing journal can
   supply broadcast authority. Remove the parallel global intent-only fence,
   including its schema uniqueness constraint; preserve the raw PREPARED fence,
   serialized SDK mutation owner, completed-result publication and exact old-ID
   guards. After successful exclusive offline reconstruction with no completed
   result retained, an intent with neither outgoing nor factual source operation
   can close without a chain-absence proof. Preserve its encrypted inputs and use
   existing funding recovery archival and fresh-operation/current-authority paths.
   Orphan factual operations remain corruption. Jobless shielding needs no fake
   funding row or closure ledger; its next calculation uses a new identity and SDK
   spendability. Classify never-disclosed resets in the existing durable exclusion
   from transaction-recovery attempt counts, without charging local crashes as
   signed transaction recovery. Keep persisted table names compatible and explain
   their expanded nondisclosure meaning in the owning guide. Update wallet
   ownership, CLI and SDK guides and the lifecycle matrix with this change.
   Independent design review must challenge live cancellation/reconstruction,
   completed-result retry and lost acknowledgments; runtime regressions must cover
   pre-journal crashes, committed outgoing with lost acknowledgment, unrelated
   progress and preserved input evidence. Independently review this exact commit
   and all prior fixups, with complexity comparison, before final integration.
4. **M3 Integration and documentation handoff.** Independently review the combined
   branch and all previous fixups against complete consumers. Remove this file
   only after maintained docs match actual behavior and validate the exact cleanup
   SHA and final combined branch.

Independent read-only design review precedes substantive decisions. Implementation
ownership separates SDK/source recovery from NEAR/funding and root publication
boundaries. An independent reviewer who did not author the changes reviews each
exact committed SHA; discovered defects use fixup commits against their introducer
and receive independent exact-SHA review. History stays unsquashed.

Architecture gates after M1 and any M2 examine all prior commits/fixups, recurring
causes, concurrency/cancellation/restart, financial ownership, network policy and
permanent documentation. Compare CogC/CycC to both 973d0574 and the original
baseline, inspect unchanged high-risk chains and explicitly resolve warnings.
Prefer removing responsibilities over relocating complexity. Material findings
reopen the affected gate before dependent work.

Do not repeat the full suite already run by the operator. Freeze sources and Git
state for validation. Complete applicable static/provenance/complexity, production
compiler, all-target Clippy and compatibility checks before focused runtime tests.
Validate changed contracts and affected consumers with nonzero execution counts,
forbidden-side-effect assertions, cancellation/restart and unrelated progress.
Carry forward unaffected evidence and complete previously unfinished jobless-source
and wallet-workflow checks. Store generated evidence under ignored target output.
