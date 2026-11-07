# Simplify direct-tool SDK integration tests

Branch: `direct_swap_control`. Baseline: `d5bc3ba5`.

The user requests cleanup of overgrown SDK-backed tests added for direct tools.
SDK compatibility and treasury lifecycle coverage predate this branch. Simplify
application test setup and duplicated scenarios; do not change production behavior,
SDK dependencies, financial authority, privacy policy or unrelated files.

Owning guidance: AGENTS.md, planning-workflow, testing efficiency and validation
continuation, tests/LIFECYCLE.md, tests/COMPLEXITY.md, zingo-integration and
wallet-rotation ownership/reconciliation policies. The parallel recovery-limit
removal plan retains its own evidence and completion gate.

## Boundaries and acceptance

1. **T1 — focused test simplification.** Independently review the design before
   editing. Replace the native send cancellation/reopen narrative with its distinct
   actual-fee total-cap and submission/confirmation assertions; existing calculation
   owner tests retain exact-byte cancellation/restart coverage. Move multi-address
   classification to existing cheap tests and retain representative preparation
   rejection with no ownership or network side effects. Simplify the native receive
   narrative using existing receipt projection coverage and focused service/address
   recovery where necessary. Preserve actual receipt attribution, fresh Ironwood-only
   addresses, partial/excess receipts, durable restart, snapshot invalidation and
   absence of bridge/spend/transparent RPC effects. Avoid new test frameworks or
   helpers that merely conceal large scenarios. Update lifecycle documentation.
2. **TG1 — exact and combined review.** Review the exact commit and affected
   consumers independently. Review source-matched complexity and existing coverage
   handoffs. Run selected compiler/all-target Clippy, then changed cheap contracts
   and focused SDK-backed tests. Reuse unaffected prior evidence, including the
   existing full lifecycle pass; do not repeat that whole suite. Preserve known
   macOS debug unwind diagnostics and complete the pending focused optimized check.
   Fix material findings in independently reviewed unsquashed fixups.
3. **T2 — completion.** Record evidence and remaining limitations, hand off current
   test contracts to maintained documentation, delete this completed plan preserving
   history, and independently review exact cleanup and final combined state.
   No live paid calls, production restart, push or merge is authorized.

## Evidence

- Independent read-only audit `sdk_test_scope_review` identified overgrown native
  receive scenarios, synthetic-chain setup for early address classification, and
  cancellation/reopen duplication in native send. Core service/SDK and privacy
  boundary tests remain useful. Independent design review accepted the focused
  approach: use existing constructed-address tests; retain configured-sync and
  actual missing-binding errors; preserve successful actor/service status refresh
  after spent and reconstructed SDK snapshots. Existing notes, Store and native
  calculation-owner tests own the duplicated projection/cancellation scenarios.
