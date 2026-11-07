# Independent review after upstream rebase

Branch: `direct_swap_control_squashed`. Starting tip:
`e6bc8a4f2f5cabdb8814080509e83d7289700ffc`.
Review baseline/local `main`: `b708a06b340dfc7ee695eab5b66a6120c152af61`.
Upstream boundary `mikeperry/main`:
`72a04ce2b4168d0a16b92c3d7ef73a3aad1a5275`.

The user requests fresh independent reviews of every change since local main for
issues and compelling simplifications, with warranted corrections. Preserve local
main, upstream and original source branch, unrelated `.vscode/` and `TODO`, exact
accepted financial evidence, listener ownership, Ironwood-only/privacy policy,
network isolation and existing valid authority. Do not add speculative enforcement,
new maintained dependencies, live spending, or new test frameworks.

Owning guidance: AGENTS.md, planning-workflow, testing/complexity discipline,
wallet-rotation ownership/observation/reconciliation, zingo-integration, network
egress/resource inventories, deployment/catalog/configuration, provider maintenance
and agent-source discovery policy.

## Ordered work and gates

1. **R0 — independent audit.** Three fresh conversations, without inherited turns,
   examine distinct concerns: direct protocol/Store/MCP; SDK/treasury/managed funding;
   network/deployment/discovery/upstream providers and vendored transport. Review
   relevant exact commits and combined call chains. Record initial assessment before
   author rationale, then reconcile prior plan/fixup history. Supply concrete
   triggers, evidence, introducing commits, coverage and limitations. Generate a
   source-matched complexity comparison against local main for hotspot review.
2. **R1 — warranted corrections.** Investigate findings and independently review
   the selected design before editing. For a feature-parent issue above upstream,
   use `git commit --fixup=<introducing-SHA>`. Issues introduced at/below upstream
   receive descriptive independent commits as requested. Keep coupled correctness,
   focused regression coverage and maintained guide updates together. Independently
   review every exact fixup/new SHA. Preserve unsquashed review history.
3. **G1 — final combined review.** Resolve material findings and compelling
   simplifications, review updated source-matched CogC/CycC, then complete selected
   production compiler/all-target Clippy and dependency compatibility checks before
   focused affected runtime suites. Choose tests from actual contracts; do not
   repeat unchanged full SDK suites or claim old evidence validates new upstream
   dependencies. Preserve diagnostics and identify missing live/consensus/Tor
   qualification. Reopen affected gates after changes and reuse valid results.
4. **R2 — handoff and cleanup.** Commit concise audit/finding/validation evidence
   here, keep enduring contracts in owning guides, remove this completed plan and
   independently review exact cleanup plus the final combined branch. No push,
   merge or deployment is authorized.

## Evidence and remaining work

- Fresh read-only agents `review_direct_protocol`, `review_treasury_financial` and
  `review_network_deployment` began at the starting tip with separate scopes and no
  inherited turns. They are not authors of the implementation under review.
- Prior rebase was mechanically patch-equivalent; this review assesses correctness
  and simplification independently. No post-upstream compiler/runtime pass is yet
  claimed at that point. Subsequent audit, correction and validation evidence follows.
- R0 initial assessments and full assigned call-chain/history audits found no
  material financial or protocol defect. A preliminary native-send scheduling
  concern was withdrawn after the existing atomic index-clear/prepared-only
  selection and its tests were traced; no correction is warranted there.
- Selected R1 corrections, independently designed before implementation:
  - CLI balance guide: `ce1d4399` changed reporting to Ironwood totals including
    dust and unknown unsupported transparent fields. Correct the stale paragraph
    without changing code or adding a test for prose.
  - Local configuration diagnostics: Base preflight discards its error (already
    present before local main, retained by the `8dfc6197` refactor); adjacent
    NEAR/treasury catches (`085a6f5c`) incorrectly pass local
    errors through a chain-verification classifier. Log only the safe top-level
    repository-authored preflight error, never its chain or endpoint/header values.
    Cover meaningful causes and secret-value absence with cheap local fixtures.
    Do not introduce a new classifier or modify capability degradation.
  - Unix SOCKS TLS validation: no implementation defect was established, but the
    distinct new vendor HTTPS/ALPN branch has only plaintext fixture coverage.
    Reuse authenticated SOCKS/TLS fixtures for trusted HTTPS/H2, untrusted-root
    rejection, remote DNS/identity and missing-socket no-direct-fallback checks.
    This belongs to upstream `72a04ce2`, so use a standalone regression commit.
- Initial complexity `target/complexity-runs/20261107T195410Z-bd266a58` is complete:
  486 files, 10,738 callables, no exclusions, baseline local main. All reviewers
  verified their affected hashes. `prepare_with_observation`, `serve`, live receipt
  recovery and owned Tor supervision warnings retain distinct ownership/validation
  ordering; no compelling extraction or responsibility removal was found.
  Exact plan `6616c132` passed independent review. Post-correction exact review and
  selected compiler/runtime validation remain.
- Exact corrections independently accepted: CLI fixup `ae061db4` to `ce1d4399`;
  standalone Base diagnostic correction `5c204a9a`; NEAR/treasury diagnostic fixup
  `a2236695` to `085a6f5c`; standalone upstream regression `6c41cf47`.
  Diagnostics use existing safe local error messages instead of adding a classifier.
  One isolated fixture reuses a cheap encrypted Store for six missing/invalid
  configuration cases; it never reconstructs an SDK wallet or performs network I/O.
  The Unix regression reuses real authenticated SOCKS and TLS fixtures, with no new
  production transport, dependency or authority. Maintained guide handoffs are in
  wallet-cli, tests/STARTUP and tests/TOR. Exact evidence/design update `0c921da1`
  passed independent review.
- Parent attribution correction: current-line blame initially attributed the Base
  omission to `8dfc6197`, so `9bcf5987` was incorrectly named as its fixup. Comparing
  local main confirms the same discarded error and warning predate that refactor;
  the independent reviewer verified the correction. After validation, metadata-only
  rewording mapped `9bcf5987` to standalone `5c204a9a`, `cff3f989` to `a2236695`,
  and `9d128c00` to `6c41cf47`. Independent exact review verified each raw patch,
  tree and parent tree is identical; all boundaries remain unsquashed. Evidence
  from the actual tested tip `9d128c00` therefore carries to `6c41cf47`. No
  revert/reapply pair was added. NEAR/treasury classifier misuse was introduced in
  `085a6f5c` and retains its fixup target.
- Final source-matched complexity at `9d128c00`:
  `target/complexity-runs/20261107T195957Z-02858869`, complete 486 files and 10,741
  callables, no exclusions. Changed hashes and lockfile independently verified;
  all production warning objects match the initial report and prior named
  dispositions carry. No substantive Rust refactor was warranted.
- `vendor/verify.py` passed published-source-plus-reviewed-patch verification for
  hyper, h2 and reqwest (`target/post-rebase-review-vendor.log`). Formatting and
  correction diff checks passed. The upstream reqwest patch's unchanged whitespace
  context is retained for exact provenance rather than rewritten for diff hygiene.
  Production compiler and all-target Clippy passed without diagnostics in
  `target/post-rebase-review-clippy.log`. Subsequent compatibility and focused
  runtime results follow. Prior SDK lifecycle evidence is not a new post-upstream
  pass.
- Validation on Rust source `9d128c00`: production compiler and all-target Clippy
  passed without diagnostics (2 minutes 21 seconds and 6 minutes 28 seconds) in
  `target/post-rebase-review-clippy.log`; compatibility passed 31 in
  `target/post-rebase-review-compat.log`. Focused default library selection passed
  62 with no ignores in `target/post-rebase-review-focused-lib.log`, including the
  actual isolated configuration fixture and three TLS/SOCKS contracts, direct
  protocol/Store/server/deployment and bundled-provider contracts. Network passed
  16, with four explicitly ignored live Tor controls, and network audit passed 2
  in `target/post-rebase-review-network.log`. No live test was invoked.
- The known macOS debug library-test linker unwind-size warning remains visible.
  `target/post-rebase-review-unwind.txt` records the actual current binary's
  retained `__eh_frame` and `__unwind_info`. The existing exact Store worker-loss
  test passed after actual panic/cancellation/reopen and independent Store progress
  (`target/post-rebase-review-unwind-debug.log`). Three focused optimized cases
  passed in `target/post-rebase-review-optimized.log` (release build 1 minute
  8 seconds, runtime 0.09 seconds), without diagnostics. No panic strategy,
  profile, toolchain pin or diagnostic suppression changed. Source reviews and
  ordinary checks do not qualify live settlement, real Tor or consensus behavior.
- Scoped recurring-cause review retained terminal background sanitizers: unlike
  local preflights, these can carry private Store or external evidence errors.
  Worker loss already emits its precise safe cause; other uncertainty remains
  explicitly unclassified. Replacing those with raw error display is unjustified,
  and no classifier/framework or broader logging cleanup is warranted.
- Scope C final combined review accepted `6c41cf47`, its exact metadata mapping,
  112 selected default/debug executions and three optimized passes. Four live Tor
  controls remain explicitly ignored.
- G1 final combined review accepted `6c41cf47` plus the sole modified plan snapshot
  (`aa12650101c4a0f91dc832e9c47a4018edd0801c36eb280ba7af5c3e09c8692e`). The
  integration reviewer independently verified all 486 current Rust source hashes
  and lockfile, unchanged warning dispositions, all metadata mapping trees/patches
  and actual validation logs. Financial scope B also accepted the final combined
  state with all 125 scoped hashes unchanged. No material finding or compelling
  simplification remains. Local main/upstream/source refs and unrelated files are
  preserved. Only evidence commit, completed-plan removal and exact cleanup review
  remain; enduring contracts are already in maintained guides.
