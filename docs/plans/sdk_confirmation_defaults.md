# SDK Zcash defaults and Base finality

Branch: `direct_swap_control`. Baseline: `f7fe67b1cfb41394b6c555ed6664d30e118e5005`.

The user requests the pinned SDK's trusted confirmation default instead of a
duplicate Zcash `3`, and removal of the arbitrary Base depth `12`. Base currently
requires both configured depth and RPC finality; depth never accelerates evidence.
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
  design at the baseline. Base design review is in progress. It identified that
  observation depth equality must stop acting as scope when recording actual gaps;
  zero finalized/latest gap must also remain valid. No implementation yet.
