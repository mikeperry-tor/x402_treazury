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
  claimed. Audit, warranted corrections, final validation and cleanup remain.
- R0 initial assessments and full assigned call-chain/history audits found no
  material financial or protocol defect. A preliminary native-send scheduling
  concern was withdrawn after the existing atomic index-clear/prepared-only
  selection and its tests were traced; no correction is warranted there.
- Selected R1 corrections, independently designed before implementation:
  - CLI balance guide: `ce1d4399` changed reporting to Ironwood totals including
    dust and unknown unsupported transparent fields. Correct the stale paragraph
    without changing code or adding a test for prose.
  - Local configuration diagnostics: Base preflight discards its error
    (`8dfc6197`); adjacent NEAR/treasury catches (`085a6f5c`) incorrectly pass local
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
