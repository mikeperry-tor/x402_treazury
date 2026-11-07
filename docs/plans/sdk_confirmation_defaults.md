# SDK Zcash defaults and Base finality

Branch: `direct_swap_control`. Baseline: `f7fe67b1cfb41394b6c555ed6664d30e118e5005`.

The user requests the pinned SDK's trusted confirmation default instead of a
duplicate Zcash `3`, and removal of the arbitrary Base depth `12`. At the baseline,
Base required configured depth and RPC finality; depth never accelerated evidence.
Keep explicit Zcash overrides and the existing uniform SDK threshold. Do not add
trusted/untrusted classification, change SDK dependencies or introduce a finality
fallback. Preserve unrelated local configuration and files, retained financial
evidence, ownership, canonical rechecks, endpoint isolation and accepted work.

Owning guides: wallet-rotation, wallet-cli, configuration, network-egress, testing
and tests/COMPLEXITY.md. AGENTS.md independent review and planning workflow apply.

## Implementation and acceptance

1. **C1 — confirmation sources.** Read the SDK trusted default through its public
   API; remove Base depth configuration and constructor plumbing. Acquire financial
   Base evidence at the finalized anchor, retaining latest advisory reads,
   canonical validation, pending behavior and configured fresh-view failover.
   Keep serialized observation depth as evidence metadata, allowing zero and
   changing observed depths without making them identity or policy. Historical
   evidence stays readable without migration or rewritten liabilities. Update all
   callers, qualification tooling, faithful fixtures and owning documentation.
2. **G1 — exact and combined review.** Independent design review precedes edits.
   Review C1's exact SHA and complete consumers, particularly receipt inclusion,
   observation comparison/storage, unavailable finality and zero/changing gaps.
   Run source-matched CogC/CycC analysis, production compiler and all-target Clippy
   before broad selected runtime checks. Run focused configuration, Base evidence,
   receipts, failover and affected consumer regressions with socket-capable
   execution where needed. Report actual counts and missing coverage; no live
   payments, secrets, service restart, push or merge.
3. **C2 — completion.** Resolve material findings in independently reviewed fixup
   commits, record evidence here, then delete this plan after final integration
   acceptance. Review the exact cleanup SHA and combined branch. Preserve history.

## Review and validation evidence

- Independent reviewer `asset_search_review` accepted the Zcash public SDK accessor
  design at the baseline and accepted the complete Base design at `5816de41`.
  Its material observation-depth finding is incorporated: depth equality must stop
  acting as scope when recording actual gaps; zero finalized/latest gap is valid.
  Historical serialized depth remains readable without rewriting evidence.
- C1 is implemented at `8a4b8dfc`. The independent reviewer accepted that exact
  implementation and each fixup, plus the combined source through `6e78de80`.
  Findings and dispositions:
  - `c3746f37`: reject an explicitly null archived primary RPC setting rather than
    silently selecting defaults. Missing settings still use existing defaults;
    read-only archive extraction selects only endpoint settings, without retired
    limit compatibility logic or endpoint-policy duplication.
  - `6cbf135d`: clarify that new observations record actual depth while historical
    records keep original metadata.
  - `b0f57f7d`: all-target Clippy exposed a missed diagnostics-tool constructor.
    Remove its obsolete argument. A repository-wide consumer audit found no other
    production depth plumbing outside the initially delegated src/tests scope.
  - `6e78de80`: four runtime failures exposed stale fixtures. Keep exact canonical
    RPC order/count assertions while reflecting shared finalized/confirmed-anchor
    rechecks; move partial future-receipt fixtures beyond the observed tip while
    retaining identity-promotion and no-extra-proof assertions. In-chain receipts
    awaiting finality remain separately covered. No production correction needed.
- G1 passed at `6e78de809bfce5d5ada38d4059943537116ede92` with independent combined
  review and no unresolved material findings or compelling simplifications:
  - `target/sdk-confirmations-config.log`: 16 configuration tests passed.
  - `target/sdk-confirmations-runtime-lib.log`: 77 passed, four stale-fixture
    failures, two explicit parent-invoked helpers ignored. The four corrected
    cases passed in `target/sdk-confirmations-runtime-lib-fixed.log`; carry forward
    the unchanged 77 passes, including the real SDK worker/rotation fixture.
  - `target/sdk-confirmations-runtime-consumers.log`: 315 passed across assignment,
    Base, Base failover/Tor fixtures, confidential diagnostics, managed payments and
    integration preparation; 11 explicit helpers/opt-ins ignored. All three archive
    receipt-recovery tests executed. Total distinct selected passes: 412.
  - `target/sdk-confirmations-clippy-final.log`: production compiler 56.98 seconds
    and all-target zcash-regtest Clippy 5m06, clean emitted diagnostics. Earlier
    compiler failure from the missed diagnostic caller remains recorded in
    `target/sdk-confirmations-clippy.log`; the corrected intermediate gate also
    passed before runtime continuation. Final fmt-check passed.
  - Source-matched complexity report
    `target/complexity-runs/20261107T174838Z-6ec53c82`: 486 files, 10,729 callables,
    no exclusions; all 63 changed Rust source hashes independently verified.
    Sole warning: receipt-recovery reconcile CycC 57 to 58 from the additional
    fallible endpoint-settings reconstruction. Its provenance/confinement/Tor
    cleanup ordering remains cohesive; no policy or ledger was added, and moving
    the fallible call elsewhere would not simplify the responsibility.
  - Known macOS SDK debug-test linker diagnostic: retained `__eh_frame` size
    `0x1310b58` and `__unwind_info` size `0x367158` verified in
    `target/sdk-confirmations-unwind.txt`. No panic/profile/toolchain/SDK change.
    Affected cancellation/restart and ownership tests passed. The focused optimized
    Base/failover continuation passed eight checks after a 52.56-second release
    build, with clean diagnostics (`target/sdk-confirmations-release.log`).
- C2 documentation handoff is complete in the owning guides. Completed-plan removal
  and exact cleanup/combined review remain next. No full make check, Docker
  consensus, paid/live settlement or real Tor-network qualification was performed.
  No service restart, push or merge. Unrelated research-desk configuration, .vscode
  and TODO files remain untouched and unstaged.
