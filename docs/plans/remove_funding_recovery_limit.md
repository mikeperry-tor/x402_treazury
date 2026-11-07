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

- Independent reviewer `recovery_limit_review` is reviewing the design. Initial
  assessment confirms nondisclosure markers have independent ownership consumers
  and must remain. Code implementation has not started.
