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
- Exact plan `3d53f74b` and implementation `8c68210a` passed independent review.
  The new unfunded creation test owns request/address identity; successful real
  actor refresh after spent and reconstructed snapshots remains in the funded
  receipt test. The recipient matrix now decodes public addresses without creating
  legacy SDK receivers. Configured unfunded preparation checks the actual missing
  binding error before any network access. The native send test retains actual
  total-cap boundaries and real submission/confirmation, with cancellation/reopen
  coverage handed off to unchanged native calculation-owner tests.
- Source-matched CogC/CycC comparison against `d5bc3ba5` is retained in
  `target/complexity-runs/20261107T192104Z-73aa5660`: 486 files, 10,714 callables,
  no warnings or exclusions. Independent review verified changed hashes and
  accepted the reduced orchestration without production changes or helper
  frameworks. Exact compiler and focused runtime checks remain pending.
- The first compiler gate passed production but found one fixture argument type
  error: `SyncSettings::new` takes `String`, not `&str`. Fixup `5e1f2d2a` adds the
  required conversion. The failed log remains in
  `target/direct-sdk-test-cleanup-clippy.log`; the corrected gate runs in its
  `-fixed.log` continuation. Source review did not establish compilation.
- Independent review accepted exact fixup `5e1f2d2a` and the combined test change.
  Corrected source-matched complexity is
  `target/complexity-runs/20261107T192341Z-28ac3493`, with the same file/callable
  counts and no warnings or exclusions. All changed hashes match the exact fixup;
  prior complexity dispositions carry forward.
- Corrected production compiler and all-target Clippy passed without diagnostics
  in `target/direct-sdk-test-cleanup-clippy-fixed.log` (31.21 seconds and
  4 minutes 25 seconds). Formatting and diff checks passed. The socket-free
  recipient classifier passed in the optimized four-test continuation
  `target/direct-sdk-test-cleanup-optimized.log`; its three other cases complete
  the recovery-removal plan's optimized follow-up. All four changed native
  receive/send cases passed, with no ignores, in
  `target/direct-sdk-test-cleanup-native.log` (build 33.31 seconds, runtime
  35.18 seconds). Existing unchanged lifecycle/compatibility/recovery evidence is
  carried forward from the recovery-removal plan; its full lifecycle suite was not
  repeated for this cleanup.
- The known macOS debug library-test linker unwind-size warning remains visible.
  `target/direct-sdk-test-cleanup-unwind.txt` records the actual current executable's
  retained `__eh_frame` and `__unwind_info` sections. Unchanged real SDK panic,
  cancellation and restart controls passed in the prior lifecycle run; the current
  optimized continuation passed without diagnostics. No profile or panic-strategy
  change or warning suppression was introduced. No live-funded, consensus or
  actual Tor-network qualification is claimed.
- Independent final TG1 review accepted exact `5e1f2d2a` and the combined cleanup,
  verified compiler/runtime counts and complexity evidence, and found no unresolved
  material issue or compelling simplification. Coverage handoffs and permanent
  lifecycle guidance agree. Only evidence commit, plan removal and exact cleanup
  review remain.
