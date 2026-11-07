# Remove the managed funding recovery limit

Branch: `direct_swap_control`. Baseline:
`22f7bf1f33372d0b9f373a414ec44a45b2b705db`.

The user requests removal of the application `max_attempts` limit and its durable
recovery-count enforcement because it can permanently obstruct otherwise safe
managed funding. Currently a derived count of archived operations excludes proven
never-disclosed resets and gates quote selection, preparation, recovery and startup.
Remove that responsibility rather than replacing it with another budget or limit.

Preserve original operation history, signed bytes, accepted inputs and identities,
canonical confirmation/finite-expiry closure, current enabled unconsumed claims,
SDK spendability, qualification authority and independently pending bridge work.
Keep nondisclosure markers where existing ownership/recovery/retirement consumes
them. No evidence deletion, schema migration, legacy limit adapter or SDK/dependency
change is needed. Network retry counters named `max_attempts` are unrelated.
Preserve unrelated research-desk configuration, .vscode and TODO files.

Owning guidance: AGENTS.md, planning-workflow, wallet-rotation best-effort,
observation and reconciliation policies, wallet-cli, configuration, testing,
tests/COMPLEXITY.md and the timer/resource inventories where applicable.

## Commit boundaries and gates

1. **C1 — coordinated removal.** Remove the configuration/default/validation,
   transaction-authority and actor/backend parameters, derived count query/helper,
   recovery-limit error and startup/scheduling obstruction. Preserve distinct
   current-authority checks. Update all library/CLI/tool consumers and authored
   configuration, owning guides and diagnostic text in the same coherent commit.
   Replace exhaustion tests with regressions proving repeated safe recovery and
   restart/old history cannot obstruct progress. Preserve unknown or unexpired
   source ownership, bound provider outcomes, disabled funding, qualification and
   no-refund-replay assertions. Test removal of obsolete configuration separately.
2. **G1 — integration.** Independent design review precedes code edits. Review the
   exact C1 SHA and full call chains, including archives, readiness, CLI recovery,
   disabled funding and direct-tool isolation. Complete source-matched CogC/CycC
   review, production compiler and all-target Clippy before broader runtime suites.
   Run cheap configuration/contract tests first, then affected funding/recovery,
   startup/CLI and selected real SDK synthetic-chain regressions. Verify nonzero
   execution and absence of duplicate disclosure and unrelated side effects.
   Use socket-capable execution upfront for local fixtures; no real wallet secrets,
   paid calls, service restart, actual Tor-network or Docker consensus tests.
3. **C2 — completion.** Independently review fixups and the combined implementation,
   resolve material findings and compelling simplifications, and record evidence
   here. Hand off enduring contracts to maintained guides, remove the completed
   plan preserving Git history, and review the exact cleanup SHA and final branch.
   No push or merge is authorized.

## Evidence

- Independent reviewer `recovery_limit_review` accepted the design and exact plan
  commit `80668470` before implementation. Nondisclosure markers have independent
  ownership consumers and remain. ManagedFundingAuthority is transient; no
  serialized limit adapter or schema change is needed.
- C1 implementation is committed at `753a5fc1`. Independent exact-SHA and combined
  source review accepted it and subsequent fixups. Root integrated Store and Treasury
  author changes, removed the placeholder Backend policy tuple, and retains managed
  profile validation with actual claim ceilings. Tests cover repeated safe closure,
  reopen, old unclassified archives and obsolete limit diagnostics while preserving
  financial history. Counter proxies now assert operation/archive/journal facts.
  HTTP retries and factual submission-attempt evidence remain outside this change.
- Compiler-driven integration corrections are retained unsquashed and independently
  reviewed: `ff4b884c` restores the free CLI recovery function's network argument
  accidentally removed by a method-call transformation; `20613045` keeps the private
  bootstrap helper private by selecting the exact job through existing group/claim
  APIs, and updates one SDK fixture's obsolete argument; `0999fe14` borrows that
  test's one-element slice without cloning; `10934de7` removes an integration
  fixture's unsupported selected-input assertion (it has no calculation manifest),
  retaining exact bytes/identity/pending ownership, zero broadcasts and rejection of
  fresh preparation. Real SDK tests retain actual selected-input assertions.
  Wider visibility/signature/fixture audits found no further source issues; source
  review alone did not establish compilation and the failed logs remain retained.
- Runtime-driven assertion fixes are retained in `cbc7d835` and `d5bc3ba5`.
  Four tests incorrectly queried the current-operation facts API using an archived
  operation identity. They now inspect current facts before reset and retained
  archives afterward, using existing APIs or the integration fixture's read-only
  database. The SDK fixture keeps Store internals private. These are test-only
  corrections; production code is unchanged after `ff4b884c`. Independent exact
  fixup and combined review accepted both corrections.
- Configuration regression suite: 17 passed in
  `target/remove-recovery-limit-config-fixed.log`; obsolete setting is rejected.
- Final source-matched complexity at `d5bc3ba5`:
  `target/complexity-runs/20261107T191121Z-b6650636`, 486 files, 10,713 callables,
  no exclusions or warnings; independent review verified every source/lock hash.
  Relevant production hotspots are unchanged or improved. The retained source
  observation/recovery, publication/input-acceptance and scoped inventory ordering
  remains cohesive; no compelling responsibility deletion or extraction was found.
- Compiler and all-target Clippy passed without diagnostics at `10934de7` in
  `target/remove-recovery-limit-clippy-complete.log`; the exact `d5bc3ba5` rerun
  also passed without diagnostics in `target/remove-recovery-limit-clippy-exact.log`
  (production 30.59 seconds, all targets 4 minutes 31 seconds).
- Compatibility: 31 passed in `target/remove-recovery-limit-compat.log`.
  Affected library tests: 184 cumulative passes, one intentional isolated-helper
  ignore, from `target/remove-recovery-limit-runtime-lib.log` and its
  `-fixed2.log` continuation. The original two failed assertions are corrected
  above; the other 182 passes remain valid.
- Six default integration targets (assignment, cli_contracts, funding,
  integration_preparation, rotation, treasury_sync): 352 passed, ten intentional
  ignores, in `target/remove-recovery-limit-consumers.log`. Full SDK lifecycle:
  65 passed, one intentional helper ignore, in
  `target/remove-recovery-limit-lifecycle.log`. This existing full lifecycle suite
  was broader than necessary for removing the counter; future continuation uses
  focused affected checks. This task adds no SDK lifecycle test file or case.
- The known macOS debug test-linker `__eh_frame section too large` diagnostic is
  retained. `target/remove-recovery-limit-unwind.txt` records full `__eh_frame`
  and `__unwind_info` sections in the actual SDK test executable. Affected panic,
  cancellation and restart lifecycle cases passed. The focused optimized follow-up
  passed all four selected tests, including the three removal-specific
  startup/Store/recovery reopen cases, in
  `target/direct-sdk-test-cleanup-optimized.log` (release build 1 minute 13 seconds,
  runtime 0.10 seconds, no diagnostics). No panic/profile/toolchain changes or
  warning suppression.
- Independent combined review at `d5bc3ba5` verified the source/complexity hashes,
  prior fixups and runtime logs and found no unresolved material source finding.
  Review also accepted the exact evidence update `3d53f74b` and carried production
  review through the separately reviewed SDK-test cleanup and fixup `5e1f2d2a`,
  which change only tests and their guide. Independent G1 review accepted all
  recovery evidence at `5e1f2d2a`. Final evidence commit and cleanup review remain.
