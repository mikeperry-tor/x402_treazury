# Examples reorganization and the Zcash build

## Intent and baseline

Make `examples/` a small set of legible user-facing starting points. Keep a small
managed-wallet deployment and a comprehensive feature showcase. Preserve static
wallet runtime support without showcasing static serving. Make Zcash unconditional
in the application build, while retaining opt-in proving and consensus test features.

Starting baseline: `0aabd50f1d8833c43f91cc6e9061d131c770fac1`.
This request authorizes planning; implementation and the commits below are proposed.
The read-only audit and independent design review found maintained live testing
infrastructure, active maintenance utilities, overlapping deployments and two
obsolete utilities. No builds or live requests were run for the audit.

At this baseline `examples/` contains 92 tracked files and about 27,600 lines;
roughly 25,400 lines belong to live integration infrastructure. The framework is
current and included by `tests/integration_preparation.rs`; its broader automated
lifecycle and combined cover/payment qualification remain deferred. The default
build already includes Zcash, but the manifest and source still support a reduced
configuration. The changes below replace that configuration rather than merely
changing its documentation.

Read the owning guides before implementation: [development](../development.md),
[testing](../testing.md), [configuration](../configuration.md),
[wallet administration](../wallet-cli.md), [wallet ownership](../wallet-rotation.md),
[network policy](../network-egress.md), [architecture](../architecture.md),
[provider policy](../../providers/README.md#provider-policy),
[live runbook](../../tests/live/INTEGRATION.md), its
[technical reference](../../tests/live/INTEGRATION_REFERENCE.md), and
[complexity review discipline](../../tests/COMPLEXITY.md#review-discipline).

## Preserved contracts

- Static-wallet configuration, CLI behavior, payment signing and regression coverage
  remain supported. Compiling treasury support does not initialize it, load its
  keys or start workers for a static or unsigned deployment.
- Preserve accepted financial work, exact signed bytes, original identities,
  liabilities, evidence formats and observation-only historical reports. Moving
  source does not authorize replay, reset a registry or move private state.
- Preserve network-factory coverage, Tor identities and remote DNS/no direct
  fallback, authenticated listener ownership and startup availability.
- Preserve stable state formats, generated `auto_v1_` pool names, Tor token domains,
  `X402_*` environment contracts and toolchain/dependency pins. In particular,
  renaming `privacy.toml` must not rename its state directory, wallet identities,
  listeners or isolation namespace.
- Do not add limits, deadlines, funding gates, compatibility frameworks or new
  runtime responsibilities. Retain current qualification claims and gaps.
- Leave untracked `TODO`, `run_servers.sh`, local deployment overrides, ignored
  state, secrets and retained `target/` artifacts untouched. Do not mass-rewrite
  local configurations to follow the renamed tracked template.

## Intended layout

| Destination | Contents and role |
| --- | --- |
| `examples/README.md` | Explain the starter, comprehensive showcase and Tor policy, with setup links. |
| `examples/deployments/managed.toml` | Minimal managed deployment with one provider, one listener and one named pool; explicit wallet setup and funding guidance. |
| `examples/deployments/comprehensive.toml` | Renamed `privacy.toml`, preserving its curated sources, four listeners, two pools, Tor, relay and dynamic source management. Add a short relationship map and organized comments. |
| `examples/network/tor.toml` | Reusable standalone CLI network policy. |
| `tests/live/live_integration.rs`, `tests/live/runner/` | Existing runner entry point and modules, next to their runbooks and manifests. |
| `tests/live/deployments/` | `live-qualification.toml`, `live-tor-providers.toml` and the two public demo deployment profiles. |
| `tests/live/provider_preflight.rs` | Unsigned live diagnostic, retaining its existing manifest and qualification boundaries. |
| `tools/providers/` | `snapshot_spec.rs` and `snapshot_agentutility.rs` maintenance utilities. |
| `tools/diagnostics/diagnose_credit.rs` | Documented read-only operator diagnostic. |
| `tools/diagnostics/quote_near.rs` | Credential-free dry-quote diagnostic using public test identities. |

Keep internal Rust utilities as explicit root-package `[[example]]` targets with
their existing names and new `path` values. Cargo's target kind is a build mechanism;
it does not require sources to live in the user-facing `examples/` directory.
This preserves `--example` commands, application-only ordinary build behavior and
test wiring without a new workspace, dependency graph or auxiliary binary suite.
Explain the arrangement once in `tools/README.md` and the live runbook.

Remove `servers.toml`, `socialfetch.toml` and `agent-sources.toml` as standalone
static-serving showcases. Replace `servers-managed.toml` with the smaller starter;
fold the useful explanations from `servers-auto-wallets.toml` and
`agent-sources-managed.toml` into the configuration/source-management guides and
the comprehensive example where compatible. Automatic assignment and explicit
named pools are alternatives: document the substitution instead of pretending
every mutually exclusive mode can be active in one showcase.

Delete `convert_privacy_local.rs`, its module/tests and its exclusively used
`tests/fixtures/migration/privacy_schema12.sql`. The utility cannot convert the
known schema12 source and recognizes schema16 rather than current schema21.
Preserve production storage preflight and the rule against manufacturing missing
legacy evidence. Delete the undocumented, unreferenced `startup_probe.rs`.

## Ordered implementation commits

### R1 Remove obsolete utilities

Delete the converter, its dedicated fixture and the startup probe together with
their obsolete references. Update `docs/configuration.md` and `docs/wallet-cli.md`
in this commit; retain maintained legacy-state rejection/recovery guidance without
an incident narrative or a link to a nonexistent conversion tool.

Acceptance: check all fixture and utility consumers before deletion; search for
remaining live references; run focused production legacy-schema/preflight tests
and the network-constructor audit. Confirm no production preflight implementation
or independent fixture was removed. Do not add tests that merely assert absence.

### R2 Relocate maintained internal tooling and qualification inputs

Move the runner, its fixture and the active utilities into the destinations above.
Declare explicit Cargo paths without renaming commands or changing runner behavior.
Include `quote_near` with the operator diagnostics and update its wallet, public-demo
and live-preflight documentation references; no Rust utilities remain in `examples/`.
Move qualification/demo deployments and repair their provider, schema, network,
state and manifest paths according to each file's existing resolution rules.
Preserve the public demo profiles' distinct static-payment and managed-swap roles;
the static profile is test infrastructure rather than a serving showcase.

Update direct driver includes in `tests/integration_preparation.rs`, embedded
fixtures, provider maintenance references, demo-profile tests, all documented
commands and AGENTS implementation links in the same commit. Check wrapper build
profiles when updating commands: default wrapper builds use `target/release`,
whereas explicit developer builds use `target/debug`; neither uses the old
directory solely because a source file was moved.

Move the corresponding provenance inputs in **both** `build.rs` and
`src/build_identity/inputs.rs`, including rerun directives and Git dirty-path
inventory. Preserve runtime/preparation comparison and private executable pinning.
Do not broaden hashing to unrelated local files, secrets or generated artifacts.

Extend `tests/network_audit.rs` to cover the relocated operational Rust sources.
Update complexity inventory and classification for their new homes: operational
runner code under `tests/live/runner/` must retain the scrutiny previously applied
to operational examples, while genuine test items remain tests. Include relocated
tools in current and baseline discovery without including `tools/complexity/`.
Account explicitly for path-based unmatched metrics after moves. Keep maintenance
utilities outside the application coverage denominator and update its explanation;
do not claim a coverage or complexity improvement from relocation.

Acceptance: record relevant test lists before/after relocation and verify that
the same meaningful runner, preset, registry, reporting and real-process tests
actually execute. Run focused utility tests, `integration_preparation`, demo
profile assertions, build-identity tests, network audit and affected Python
complexity/coverage tests. Add focused regression evidence that moved runner input
changes affect its fingerprint and that relocated sources remain in audit/metric
inventories. Run format and applicable Clippy checks. Use offline inputs only.

### R3 Establish the small managed example and comprehensive showcase

Rename the tracked privacy template to `comprehensive.toml` with its behavior and
operational identities preserved. Improve its comments and orientation without
rewriting the curated tool selections or provider instructions. Introduce
`managed.toml` using existing managed configuration contracts; it should teach
one pool/listener/provider with only necessary setup.

Remove the redundant deployments together with their replacement instructions.
Update README quickstart paths, `examples/README.md`, wallet CLI examples,
configuration and agent-source guides. Lead introductory serving documentation
with the managed path; keep existing static-wallet contracts in the reference
and retain their focused synthetic tests. Replace static deployment snippets in
introductory provider documentation where they still act as the main showcase.

Move tests that intentionally protect the comprehensive profile's authored
selections to its new path. For generic behavior tests that currently depend on
deleted large deployments, use small faithful synthetic inputs, retaining the
same ownership, relay, filtering and dynamic-source assertions. Avoid universal
tool/provider inventory snapshots or a broad unrelated fixture rewrite.

Acceptance: offline configuration inspection and focused configuration, managed
wallet, source-management, CLI layout, provider-selection, relay and pricing tests.
Verify the starter exposes its intended tool(s) without secrets or funding during
inspection; verify comprehensive bindings and relative paths retain their prior
meaning. Keep static runtime signing and no-treasury-side-effect coverage reachable.
Confirm the only remaining tracked examples are the intended user-facing set.

### R4 Make Zcash unconditional in the application

Make existing `zcash` application dependencies unconditional and remove the
application feature switch and its `cfg`/`cfg!` branches, including reduced-build
errors and test gates. Preserve `zcash-testutils` and `zcash-regtest` as opt-in
test-only features with their necessary dependencies. Remove obsolete
`required-features = ["zcash"]` target restrictions. Do not enable ignored Docker
or funded tests as a side effect.

Simplify `scripts/check.sh`, reproducibility feature selection, help and related
tests/docs around the single application configuration. Keep the existing build
wrapper name unless a separate concrete need justifies changing it. Preserve pins,
protoc provisioning, default release/developer behavior and network-denied release
qualification. Cargo's generic `--no-default-features` may still be accepted, but
must no longer produce a different application; do not add a rejection framework.

Review `compat/zingolib`'s root dependency with `default-features=false` and update
its declaration/lock only as required for the now-unconditional graph. Preserve
the independent compatibility suite and vendor provenance. Review build identity's
feature metadata and every consumer: preserve the serialized schema and the
`zcash` capability marker as always present, along with actual test-feature markers.
Old pinned reports remain observation-only and are not relabeled or rewritten.

Update README, development/testing/reproducible-build guides, scripts guidance and
AGENTS build instructions in this commit. Retain documentation for operating
static deployments without treasury configuration in the single executable.

Acceptance: frozen-source full `scripts/check.sh` with the implementation baseline
as complexity reference, including default all-target tests, opt-in synthetic
proving/lifecycle checks, applicable Clippy, vendor and compatibility verification.
Check CLI/build-info behavior and static deployments' absence of treasury key
loading, allocation or funding. Run relevant wrapper and reproducibility unit tests;
ordinary tests do not establish a fresh two-build release qualification. If a
reduced-feature equivalence command is needed, specify and review it explicitly
before running; removing a configuration does not require rebuilding it routinely.

### R5 Complete the documentation handoff and remove this plan

Verify that README and `examples/README.md` explain the final choices; owning
guides describe the build, static runtime support, tool locations and test
boundaries; and all relative links and commands resolve. Remove the plan and its
index link together. Do not retain a completed-plan archive. Preserve focused
unfinished live-acceptance and portability plans, updating source references without
claiming their deferred work is completed.

Acceptance: inspect the final tracked layout, stale path/feature references,
documentation links, Git diff and untouched user files. Review the exact cleanup
SHA independently, then complete the final combined-branch gate below.

## Independent reviews and integration gates

Before substantive work in each boundary, obtain an independent read-only design
review with user intent, this baseline, the proposed behavior and owning guides.
Challenge unnecessary checks, duplicated responsibilities and opportunities to
delete code. Resolve material findings before implementing dependent decisions.
The initial design review identified the provenance, network inventory, complexity
classification, coverage denominator and compatibility-graph concerns addressed
above; they must still be verified against the actual implementation.

For every implementation commit, freeze source/executable inputs during validation
and give a reviewer who did not author it the exact SHA, relevant consumers,
commands/results and limitations. Review every exact fixup independently, using
`git commit --fixup=<introducing-SHA>` for findings; preserve unsquashed history
through final integration review. Do not build on unresolved material findings.

- **G1 after R2: tooling and qualification ownership.** Review R1/R2 and all fixups
  with full preparation, fingerprint, execution, cancellation/drain, registry,
  reporting and network call chains. Exit requires retained tests and audit
  coverage, correct paths/provenance, unchanged financial authority and accurate
  qualification claims.
- **G2 after R4: application and examples integration.** Review R1–R4 and every
  fixup with complete configuration, wallet-selection, serving, funding, static
  payment and compatibility consumers. Exit requires the intended compact
  examples, preserved comprehensive behavior, one application configuration,
  retained static runtime behavior and passing relevant acceptance evidence.
- **G3 after R5: final integration.** Review the cleanup SHA and combined branch,
  including all earlier commits/fixups and documentation handoffs. Confirm all
  material findings are resolved and no plan-only rule or stale path is needed
  to operate or maintain the result. Reuse source-matched validation; broaden or
  rerun checks only when new changes, failures or open concerns justify them.

Each gate must examine fixup causes and repeated assumptions across the accumulated
work, deciding whether they require a wider audit, regression coverage,
simplification or design change. Later material changes reopen affected concerns.
Use CogC/CycC against the agreed baseline for substantive Rust refactors and gates;
inspect full JSON/CSV, relevant warnings and unchanged high-risk call chains.
Compare moved functions explicitly across paths and justify retained hotspots
after considering concrete simpler representations. Helper extraction solely to
lower scores and category changes that hide operational complexity are unacceptable.

Permanent guidance belongs in its owning document with each implementation commit.
Keep AGENTS links to development/testing/configuration, live infrastructure and
complexity guidance current; replace its obsolete build-configuration instructions
at R4. Generated metrics and review artifacts belong under ignored `target/` or
private storage. No stage requires live funds, loading real wallet secrets,
mainnet qualification, Docker execution or clearing retained evidence.
