# Deferred release and review follow-ups

## Scope and ownership

This plan owns work intended for eventual implementation or a defined assessment,
outside the current cleanup. Assessment completion requires an evidence-backed
decision, not necessarily a code change. Current contracts and decisions not to
change behavior remain in their owning guides. No item authorizes spending, secret
access, deployment, a broad refactor or a new resource limit.

The selected current fixes belong to [review cleanup](../review_findings_cleanup.md).
Near-term consensus/cohort compatibility, B1 dependency-security evaluation and
bridge refund/deadline evidence belong to [NU7 release readiness](../nu7_release_readiness.md).
Supported Linux/macOS ACL work is selected cleanup work; it is not deferred with
Windows. The separate [Unix namespace assessment](../portability_fixes.md) retains
its own concrete threat question. Vendor retirement remains in its existing
[assessment plan](../vendor_retirement.md) and [checkout ledger](../../../vendor/README.md).

## Intended future work and acceptance

| ID / concern | Intended work and acceptance |
| --- | --- |
| D1 — Windows support / B8 | Deferred implementation and native acceptance. Inventory runtime, private files/ACLs, SQLite sidecars, locking, supervision, process/network semantics and SDK/native dependencies; implement real contracts and test native positive/negative controls before claiming support. Current unsupported-operation errors are not a Windows implementation. Preserve existing portable improvements. |
| D2 — Linux CI / B1 | Build automated Linux fmt/tooling/vendor integrity, production warnings/all-target Clippy, compatibility and runtime stages, with exact source/toolchain identities and efficient continuation. Integrate advisory evaluation including source versions hidden by path patches; demonstrate actual workflow execution and failure propagation. CI deferral does not defer N2's current dependency-security assessment or local release checks. The cleanup raises the supported compiler to the pinned compiler, so no obsolete 1.94.1 matrix is promised. |
| D3 — Provider-output assessment / R3 | Examine descriptions, control characters and unfamiliar pricing JSON through actual MCP output and consumers, using small synthetic cases. Identify harmful rendering, loss of useful meaning or avoidable output amplification, then decide which focused transformations are warranted. Exit with supported output policy in the agent-source guide and regressions for any selected changes, or explicit evidence-backed acceptance of current behavior. A generic description cap is not presumed necessary and does not solve instruction injection. This assessment is planned work, not contingent on receiving a new report. |
| D4 — Snapshot visibility / R6 | Add derived operator visibility for database/WAL sizes, unique snapshot count/bytes, current snapshot size and reference reasons separating unresolved work from resolved history without double counting. Validate actual shared references with small fixtures and explain results in wallet administration. Use existing metadata; add no permanent accounting ledger, quota or implicit deletion. Snapshot pruning is not selected work: retention remains the current recovery contract. |
| D6 — Archive provenance / B10 | Correct unknown-versus-dirty reporting for builds without Git metadata. Define honest provenance using the existing versioned build-identity contract and inspect qualification/reproducibility consumers before choosing a compatible representation. Validate clean, dirty and repository-free inputs with faithful fixtures and update development/release guidance. Preserve hashes and do not fabricate a clean revision. Other B10 observations have documented non-action dispositions, not a deferred catchall refactor. |
| D7 — Legacy transparent discovery assessment / P4 | Assess isolation for necessary known/issued legacy transparent queries, including what the indexer sees, current SDK transport hooks and the cost/correctness of per-address isolation or another supported discovery method. Preserve late receipts, issuance coverage and complete recovery. Exit with a concrete supported correction and focused regression plan, or an explicit evidence-backed acceptance of the shared-indexer boundary in network/wallet guidance. Fresh-wallet probing removal does not close this assessment; separate circuits alone do not guarantee timing unlinkability. |

Track identifiers retain gaps when a proposed track is removed rather than
renumbering existing work. Non-action decisions belong in owning guides, not this
future-work queue.

## Workflow when a track is scheduled

Capture current intent, exact baseline and relevant sources/consumers. Independently
challenge the design and necessity before implementation; resolve material findings.
Revise the selected track into ordered coherent commit boundaries with stable IDs,
owning guide changes and acceptance evidence. Do not execute a broad cleanup based
only on this deferred inventory.

Require non-author review of every exact committed SHA and `--fixup` SHA, preserving
unsquashed history through integration. Use source-matched CogC/CycC for substantive
Rust refactors and architecture gates; review complete chains and unchanged hotspots,
not metric-driven helper extraction. Combined gates inspect all prior commits and
fixups, repeated assumptions, side effects and missing regressions and resolve
material findings before dependent work. Follow [testing order](../../testing.md#validation-order-and-continuation),
[development policy](../../development.md) and [documentation handoff](../../README.md).

Retain only concise current behavior/limitations and links in guides, not duplicated
backlog essays. Record rejected proposals as decisions, not perpetual TODOs. When a
track completes, move enduring rules to its owning guides, verify AGENTS links and
remove that completed planning material; independently review the cleanup SHA and
final assembled work. Keep no completed-plan archive and never delete financial
evidence as documentation cleanup.
