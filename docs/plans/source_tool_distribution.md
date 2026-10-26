# Source tool distribution

## Intent and starting baseline

Replace repeated provider-specific listener exclusions with ordered `send` rules
on deployment sources. Each source assigns or excludes groups of generated tools;
listeners describe endpoint properties, instructions, capabilities and wallets.
New tools follow an explicit final destination instead of requiring allowlist
maintenance. Reusable provider files continue to expose supported APIs.

Starting baseline: `1632fa16c062f17c95065754a73b8f5a93848d44`.
Reconcile subsequent commits and preserve unrelated working-tree changes before
implementation. At plan authoring the only unrelated working-tree item was an
untracked `TODO`; do not overwrite or remove it. This plan is unfinished work,
not implemented configuration syntax or authorization for funded tests, wallet
administration, deployment startup or external messages.

The deployment to migrate is
[`examples/deployments/onyx_categorized.toml`](../../examples/deployments/onyx_categorized.toml).
Preserve its eight endpoints and their current tool availability. Endpoint
recategorization, including a property endpoint or merging markets and crypto,
was discussed but is not part of this implementation. Preserve all provider IDs,
tool names, source IDs, wallet profiles, payment caps and funding settings.
In particular, `financials` has a shared $1.00 per-payment cap; web is $0.25 and
research is $0.50. Do not create replacement wallet groups.

Read the owning guides before implementing:

- [Configuration and selection](../configuration.md), especially source/provider
  composition, multiple listeners, tags and tool-name selectors.
- [Catalog and runtime architecture](../architecture.md#catalog-configuration-and-execution)
  and [wallet selection and isolation](../architecture.md#wallet-selection-and-network-isolation).
- [Agent sources](../agent-sources.md), especially endpoint instruction placement,
  endpoint-local registration, raw calls and provider-management authority.
- [Provider policy](../../providers/README.md#provider-policy).
- [Best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules) and
  [observation policy](../wallet-rotation.md#wallet-observation-policy).
- [Startup acceptance](../../tests/STARTUP.md), [testing](../testing.md),
  [complexity review](../../tests/COMPLEXITY.md#review-discipline),
  [development](../development.md) and [Make entrypoints](../../scripts/README.md).

## Current implementation

`MetaConfig.sources` and `SourceConfig` in `src/deployment.rs` hold deployment
sources. `SourceConfig.provider` is a flattened TOML table resolved by
`src/config.rs`; reusable provider files cannot extend other files. Lists and
override maps replace rather than concatenate. `ListenerConfig.sources` supplies
the source-to-listener graph today, and listener tags/name filters narrow it.

Source path/tag/operation selection, POST preference, collision naming and source
name/recommendation filters run during catalog construction. Listener selection
is centralized in `listener_tool_inventory` in `src/deployment.rs`. The same
selected tools feed inspection, publication, discovery and execution. Do not add
a separate dispatch-time interpretation of `send`.

Source attachments also determine wallet resolution, startup dependencies and
reserved source names. They are not merely display metadata. Important consumers
include:

| Area | Files and responsibilities |
| --- | --- |
| Configuration | `src/deployment.rs`: parsing, validation, `show_config`, strict and ordinary loading, inventory, qualification snapshots, binding and publication |
| Catalog loading | `src/deployment/startup.rs`: compatible download sharing, independent source catalogs and failures; `src/deployment/deferred.rs` and its submodules: publication and endpoint readiness |
| Selection diagnostics | `src/deployment/selection_diagnostics.rs`: pending-source suppression and explanatory selector warnings |
| Wallet authority | `src/rotation/assignment.rs`: explicit wallet precedence, deterministic automatic identities and best-effort observation names |
| Funding and document reads | `src/deployment/bootstrap.rs`, `cache_warm.rs`, `existing_read.rs`, initialization and relay-wallet resolution |
| Dynamic sources | `src/discovery/mod.rs`: endpoint-local targets, explicit and generated name reservations; `src/catalog_state.rs`: published callable views |
| Wallet observation | `src/wallet_cli/sync.rs`: uses `assignment::managed_names` without turning observation into strict serving validation |
| CLI and qualification | `src/cli.rs`, `src/deployment/catalog_evidence.rs`, frozen deployment/catalog snapshot consumers under `tests/live/runner/` |

Audit direct `.sources` accesses in these call chains; the table is a starting
map, not permission to leave an unexamined consumer. Compatible remote aliases
already share downloads under policy-compatible keys; preserve that behavior.

## Configuration contract

Add an optional deployment-only `send` field to `SourceConfig`, alongside
`wallet` and `optional_startup`, outside the flattened provider table. Do not add
it to `catalog::Config`, reusable provider definitions, dynamic import arguments
or raw-call arguments. Keep deployment `version = 1`; this is additive syntax.

Example, with abbreviated exclusions and assignments:

```toml
[sources.vaaya]
extends = "../../providers/vaaya.toml"
send = [
  { to = "exclude", tools = ["supersearch", "consult", "root"] },
  { to = "company", tools = ["edgar_*", "theirstack_*", "propublica_*"] },
  { to = "social", tools = ["tomba_*"] },
  { to = "web" },
]

[sources.locus]
extends = "../../providers/locus/provider.toml"
send = [
  { to = "exclude", tools = ["help", "well_known_*"] },
  { to = "company" },
]

[servers.company]
listen = "127.0.0.1:3012"
bearer_token_env = "X402_TREAZURY_MCP_TOKEN"
wallet = "financials"
discover_on_demand = true
instructions_text = "Company and public-record research."
```

These examples explain syntax, not the complete Onyx migration or production
source exclusions. Actual migration must preserve the baseline inventory.

### Rules and matching

1. Omitted `send` preserves the existing listener `sources` contract exactly.
   A present `send` must be a nonempty ordered array of rules.
2. A rule has required `to` and optional `tools`. Reject unknown rule fields.
   `to` is one endpoint string, a nonempty array of distinct endpoint strings,
   or the scalar string `"exclude"`. Mixed exclusion/destination arrays are
   invalid. All endpoint names must exist.
3. Reserve the endpoint name `exclude` when any source uses `send`, since it
   would be ambiguous with the exclusion action. Legacy deployments without
   `send` retain their existing identifier behavior.
4. `tools` is a nonempty array of selectors. Omission denotes an unconditional
   catch-all. Require exactly one unconditional rule, in the final position.
   A final exclusion catch-all is allowed as an explicit choice. Reject `tools=[]`
   rather than accidentally interpreting it as a catch-all.
5. Reuse the existing case-sensitive `*` and `?` matcher. Patterns match the
   generated name after removing exactly the resolved provider `prefix + "_"`
   once. Do not strip the deployment source ID unless it actually supplies the
   resolved prefix. Do not strip additional repeated prefix text.
6. Thus `vaaya_edgar_filings` matches `edgar_*`, `locus_locus_zoning` matches
   `locus_zoning`, and `locus_list` matches `list`. Help has suffix `help`.
   Preserve existing method/collision suffixes and the full externally callable
   names. Never alter source `include` anchors to implement distribution.
7. Compile provider compatibility and ordinary source selection as today, then
   evaluate `send` against that stable generated source inventory. Rules cannot
   restore anything removed by those earlier filters. No path/tag routing or
   argument-based routing is part of the initial feature.
8. Evaluate each tool against the ordered rules. The first matching rule consumes
   it. An exclusion publishes it nowhere; an assignment publishes it to every
   endpoint in `to`. No later rule can revive or further redistribute it.
9. Sharing is explicit inside a single destination array. Rule order does not
   resolve collisions between different sources. Preserve existing duplicate-name
   rejection in strict inspection and omission/error behavior in ordinary serving.
10. Resolve rule selectors against the full available source inventory, before
    consumption. Missing exact selectors are errors, as with current selectors;
    matching a known tool consumed by an earlier rule is not an unknown-tool
    error. Unmatched wildcards and fully shadowed rules receive advisory warnings,
    not additional fatal policy. Suppress inventory-dependent diagnostics while
    that source is pending/unavailable; do not hide another source's errors.

For a routed source, reject listing that same source in any authored
`servers.NAME.sources`; otherwise legacy attachment could bypass its rules.
Unrouted and routed sources may coexist in one deployment and on one listener.
Existing listener filters remain supported as a final narrowing step, including
for routed tools. They cannot restore excluded or differently routed tools.
Do not remove legacy configuration fields or rewrite other deployments.

### Effective bindings before catalog acquisition

Introduce one pure, catalog-free resolution of the effective source-to-listener
graph and its origins. Every non-exclude destination mentioned anywhere in a
source's rules establishes a potential binding, even if that rule is currently
shadowed or matches no tools. Deduplicate graph edges while retaining rule-origin
information for inspection. Legacy edges still come from listener source lists.

Resolve this graph before wallet assignment, listener validation, bootstrap,
relay ownership, catalog dependency construction or dynamic name reservation.
Do not fetch catalogs to decide financial identities or startup dependencies.
The conservative declared graph intentionally preserves a destination's binding
for future tools even when its current selected inventory is empty.

Use one resolved graph/view for all consumers. Prefer a small pure binding
representation plus the existing listener/tool inventory machinery; do not add
persisted routing state, a second source registry, copied provider definitions,
new source identities or repeated catalog compilation per endpoint. Keep authored
configuration available for inspection and snapshots rather than destructively
turning `send` into undocumented listener lists. Finalize the exact representation
with the R1 design reviewer before editing its consumers.

An exclusion-only source has no effective destination bindings, wallet demand or
endpoint readiness dependencies. Retain the existing metadata-loading policy for
unused sources; eliminating their metadata loads is outside this change.

### Authority and lifecycle invariants

- Preserve source-wallet override, then listener wallet, then configured automatic
  assignment precedence. An explicit fanout may use different existing wallets
  on different endpoints, exactly as shared sources do today.
- Preserve `auto_v1_` identity inputs and names. Rule position, selector order,
  filenames and catalogue contents must not enter wallet identity generation.
  Reordering equivalent rules must not allocate or fund another wallet.
- `assignment::managed_names` must continue best-effort observation of known
  explicit and valid automatic groups when an unrelated binding is invalid.
  Do not replace observation with full strict serving validation.
- `config show`, catalog checking and routing explanation load no secrets, state,
  signers or funding workers. Bootstrap remains catalog-free; registration remains
  unable to allocate/fund. No new monetary counters or authority checks are needed.
- Ready endpoints stay usable while unrelated catalogs or funding are pending.
  Potential bindings determine affected dependencies for `wait_for_catalogs`.
  Routing/selector failure for one source follows existing dependent-endpoint
  failure handling; strict inspection still reports incomplete catalog failures.
- Dynamic registrations remain owned by their authenticated endpoint. They cannot
  submit `send` or reach sibling listeners. Reserve names for potential static
  bindings even while a catalog is pending or currently routes no tools there.
- Raw-call permissions, provider-management permissions, request transport,
  Tor isolation, API origins, signed-work ownership and financial recovery remain
  unchanged. Routing selection is not a destination allowlist for raw calls.
- Locus's free gateway and mode-based tools remain indivisible tool definitions.
  Do not dispatch or authorize by inspecting their business arguments.

## Inspection and diagnostics

Keep `config show` offline. Include the authored `send` rules and derived effective
source bindings with origins such as `sources.vaaya.send[1].to`; retain wallet
origin information separately. Do not claim catalog-dependent matches are known
without loading a catalog. Preserve existing fields for legacy configurations.

Add `config check --explain-routing` as an explicit operator inspection option.
Its JSON should retain the normal check result and add per-source tool decisions:
full name, suffix, original method/path, zero-based winning rule index, matching
selector or catch-all, assignment/exclusion, routed targets, effective targets
after listener filters, and wallet bindings for published targets. Include excluded
tools from the source inventory so an operator can explain their absence. Earlier
source-filtered tools stay governed by existing selection diagnostics; do not
rebuild an unrestricted catalog or fetch additional pricing merely to explain them.

Produce explanation on demand from the same pure selection decision used for
publication. Keep it out of normal MCP tool schemas/descriptions, financial journals
and always-retained runtime state. Existing `catalog tools` remains the ordinary
inventory interface. Keep stdout JSON-only and diagnostics on stderr; follow the
existing logging severity and safe-cause contracts. Rule shadowing or zero matches
must not become a new readiness gate, timeout or resource cap.

Include authored rules and sufficient effective topology in qualification snapshots
and frozen-reload paths. A frozen snapshot must reproduce the original endpoint
tool/payer assignments without reinterpreting a generated listener list as a legacy
bypass. Test old snapshots without `send` as well as new routed snapshots.

## Onyx migration

Keep the current endpoint taxonomy. Convert the deployment, not reusable provider
files. Move editorial source exclusions to leading `to="exclude"` rules where
that simplifies the source; leave inherited compatibility restrictions and ordinary
source filters intact. Preserve the rationale comments and alternative-tool notes.

Use these primary catch-all destinations as the starting policy. Verify them with
the inventory review before migration; any revision needs a concrete rationale and
independent review, with no unapproved wallet/cap changes.

| Default endpoint | Sources |
| --- | --- |
| web | exa, google_trends, stableenrich, vaaya, concordance |
| social | socialfetch, pdl, botsmith, hirescrape |
| company | deepline, agosint, genuinegood, locus |
| markets | x402stock, lonestar, otto, agentfund, brazilayer |
| crypto | arkham, kronos, regimeshift, claw402 |
| security | relayshield, relayshield_phone, netintel, ipintel |
| world | straits |

Before each final catch-all, group the currently enabled tools sent elsewhere by
meaningful families and exact exceptions. Use destination arrays for current sharing.
In particular, do not assume every RootData tool belongs on company or every
OpenWebNinja tool belongs on web. Preserve source-global exclusions ahead of broad
assignment families. Preserve any deliberate current duplicate availability.

Remove migrated static sources from listener `sources` and remove their old
provider-specific listener selectors after their assignments are represented.
Keep endpoint properties, management/raw capabilities, authentication, discovery
mode, instructions, source-registry paths and wallet references unchanged.
Update search keywords only where needed to keep capability guidance accurate;
the migration itself must not add or drop current tools.

Capture metadata before editing the deployment and use the identical frozen
catalog bytes for old/new comparison. Remote vendors can change during the work;
two live enumerations are not an equivalence test. Existing qualification snapshot
support and local catalog overrides can provide temporary frozen inputs under
ignored `target/source-distribution/`. Do not commit vendor inventory snapshots or
depend on ignored artifacts from the plan-authoring conversation. Obtain needed
public metadata without paid relay fallback; no real keys or wallets are required.

Compare each endpoint's exact `(source, tool name, method, original path, input
schema, description, price metadata, wallet identity/cap)` tuples. Also compare the
potential binding graph and complete wallet resolution, not just tool counts.
Inspect baseline edges that currently expose no selected tools. Preserve intentional
declared dependencies with a documented purpose; do not silently drop them or add
unexplained dead/shadowed rules just to satisfy the comparison. If a topology
reduction is necessary, separate it from the equivalence migration and resolve its
wallet/readiness effects through explicit scope and independent review first.
Retain a private generated explanation of each migrated exclusion and assignment.
For each large source, inject synthetic future tools into a temporary local copy
of the frozen document: one matching a routed family, one matching an exclusion
family, and one unmatched. Verify intentional family placement and final fallback.
New unmatched tools going to one primary endpoint rather than every old attachment
is the intentional future behavior change; current inventory must remain equal.

## Ordered implementation commits

Before substantive implementation, obtain a fresh independent design review of
the intended R1 representation and tests. Supply this intent, baseline, invariants
and current consumer map. Resolve material findings before building on a decision.

### R1 Source rules and runtime integration

Implement the deployment schema, pure binding resolution, suffix matching,
ordered consumption and on-demand explanation as one coherent behavior change.
Integrate all static configuration entrypoints and complete consumers before
introducing an example that depends on it. Keep legacy paths fully supported.
This boundary must not ship parsed `send` settings that silently fail to constrain
publication, wallet binding, readiness or calls.

Suggested owning module is `src/deployment/routing.rs`; keep responsibilities small
and reuse current pattern compilation and source catalog construction. Update
source/listener struct-literal consumers, serde/inspection output, qualification
snapshots, dynamic reservations, observation names and CLI handling as necessary.
Do not introduce an abstraction solely to lower a complexity score.

Permanent documentation in this commit: `docs/configuration.md` for syntax,
ordering, compatibility, bindings and inspection; `docs/architecture.md` for the
selection/publication and wallet handoff; `docs/agent-sources.md` for endpoint-local
management and source reservations; `providers/README.md` for deployment-only
distribution. Update affected `tests/STARTUP.md` and CLI reference material.
Add or refine the relevant `AGENTS.md` guide-table link so source distribution
points to its maintained configuration/architecture contract.

Acceptance: the synthetic contract and consumer tests below pass; production
compiler/all-target Clippy and compatibility checks complete before broad runtime
suites. Commit R1, independently review its exact SHA and relevant consumers, and
resolve findings with separately reviewed fixup commits before Gate A.

### Gate A Binding and publication architecture

Independently review all R1 commits/fixups with the complete call chains from
parse through effective bindings, wallet resolution, catalog acquisition, deferred
publication, listener selection, discovery/direct invocation, bootstrap, observation,
warming and qualification reload. Check unchanged high-risk consumers as well as
new code. Verify dynamic registrations cannot use the new syntax to widen scope.

Compare CogC/CycC against the agreed starting baseline for changed functions and
their full call chains, including deployment selection and assignment hotspots.
Review warnings and concrete opportunities to remove duplicated graph/selection
responsibilities. Do not approve merely because tests pass or scores are lower.
Exit only after material findings, missing regression coverage and unnecessary
enforcement are resolved. The gate must approve R1 before migration relies on it.

### R2 Onyx migration and equivalence evidence

Migrate only `examples/deployments/onyx_categorized.toml` to source `send` rules.
Preserve comments, all current endpoint tool sets, source/payer bindings and all
financial/network settings. Keep the eight endpoint definitions focused on their
properties. Do not combine this commit with provider refreshes, tool restoration,
category changes, new wallets or live comparisons.

Acceptance: frozen pre/post inventories and bindings match exactly, every excluded
tool has an explanatory rule/comment, every rule has a clear purpose, future-tool
probes demonstrate fallback behavior, and the new explanation agrees with actual
selection. Add a small synthetic migration-shaped regression if this exposes an
uncovered semantic case; do not check in Onyx's full vendor inventory as a fixture.
Update maintained example documentation to show the compact source-first model.
Commit R2 and independently review its exact SHA plus relevant consumers/fixups.

### Gate B Combined integration

Review R1/R2 and every fixup together, using the complete consumer and authority
chains from Gate A. Re-examine fixup causes for repeated assumptions, hazards,
unnecessary responsibilities and testing gaps; require wider audit, simplification
or design changes where warranted. Check actual frozen migration evidence and
uncategorized/fallback placement, not only tests or counts. Carry forward prior
checks only when later changes do not invalidate their evidence.

Exit criteria: current Onyx behavior and wallet identities preserved, legacy
configurations still valid, new rules callable only on intended endpoints,
pending/unavailable sources affect only dependent endpoints, management/raw scope
unchanged, required validation stages complete, and no material findings remain.

### R3 Documentation handoff and plan removal

Verify enduring behavior lives in its owning guides and the `AGENTS.md` links point
there. Update `docs/README.md`, remove this completed plan and its incoming plan
link, and retain only concrete unfinished work if any remains. Do not create a
completed-plan archive. Independently review the exact cleanup SHA and the final
combined branch after removal, including all earlier fixups and guide links.

Every committed implementation or fixup SHA needs independent review by someone
other than its author. Use `git commit --fixup=<introducing-SHA>` for findings and
preserve unsquashed history through final integration review. Give reviewers the
exact baseline, intent, scope and actual validation evidence; do not describe an
unexecuted check as passed. If subagents are unavailable, disclose that limitation
and perform available checks without claiming independent review.

## Required regression evidence

Use shared synthetic catalogs, temporary files/state, localhost fixtures and public
unfunded deterministic keys. No production state, secrets, funded serving or paid
provider calls are needed.

| Concern | Acceptance cases |
| --- | --- |
| Syntax | Scalar/array targets, unknown fields/targets, reserved name, empty arrays, required final catch-all, early catch-all, no send and mixed legacy/routed sources; reusable provider/dynamic import reject send |
| Matching | Exact names, case sensitivity, `*`/`?`, prefix different from source ID, repeated Locus prefix, help, method/collision suffixes, ordinary source filters applied first, no tool renaming |
| Ordered assignment | Exclusion first, overlapping families, first match wins, explicit fanout, shadow warnings, final assignment/exclusion, all-excluded source, synthetic future arrivals |
| Bound topology | All declared targets exist before catalogs; current zero-match/shadowed destinations retain bindings; exclusion-only sources add no binding/demand; legacy graph unchanged |
| Wallets | Source/listener/automatic precedence; all four auto-assignment scopes; stable auto names under equivalent rule ordering; no new allocation/funding/signers during inspection or registration |
| Observation | Best-effort explicit/automatic managed names with unrelated invalid bindings; no state mutation or strict serving prerequisite for observation |
| Publication and calls | Inventory, ordinary listing, search, direct invocation and wrapper calls agree; excluded/routed-away tools uncallable; same-prefix source conflicts retain existing strict/serving behavior |
| Startup | Pending catalogs, optional and required failures, reversed completion order, wait_for_catalogs true/false, unaffected ready endpoints, shared downloads, cancellation and accepted relay work remain correct |
| Management | Explicit/generated static source-name collisions rejected for derived bindings, including pending/empty current inventories; dynamic additions stay endpoint-local; raw permission unchanged |
| Inspection | Offline show retains authored rules and derived origins without secrets/network; explain reports winning rule/exclusions/final narrowing and matches actual published wallets; old/new frozen snapshots reload correctly |
| Migration | Exact current per-endpoint tuples and binding/wallet resolution equal on frozen inputs; every new unknown tool takes its documented fallback; no vendor inventory snapshot fixtures |

Tests should live with the owning synthetic selection, deployment, startup,
assignment, discovery and CLI tests. Extend existing fixtures only for an exposed
quirk or transformation; do not add tests that merely freeze TOML text or mirror
the implementation. Verify filters actually execute the intended tests.

## Validation order and completion

Before implementation, record the exact base SHA and run the existing complexity
tool against that baseline, preserving reports under ignored `target/`. At Gate A
and final integration, compare changed functions and complete call chains using
the same baseline/analyzer. Follow `tests/COMPLEXITY.md`; inspect advisory warnings
and unchanged hotspots rather than treating scores as automatic pass/fail.

Coordinate existing Cargo/editor processes, use pinned Make entrypoints and keep
sources/executables/features frozen during their validation. Run applicable cheap
contract tests first. Run `make check-clippy` and `make check-compat` before broad
runtime stages, then complete the remaining required `make check` stages in the
documented order. `make fmt-check` and the applicable static/tooling checks must
pass. Observe compiler diagnostics as well as exit codes. After a localized fix,
rerun affected consumers and resume unfinished named stages; do not repeat passed
stages without an invalidating change.

Choose socket permissions up front: pure matching/config/assignment tests stay in
the ordinary sandbox; known socket-based deployment/startup/discovery fixtures
need socket-capable execution. Do not deliberately run them once to obtain a
sandbox failure. No Docker consensus or live-funded qualification is required.

Before declaring completion, record actual commands/results, exact reviewed SHAs
and fixups, carried-forward evidence, complexity findings, migration equivalence,
documentation handoff and any remaining limitations in the implementation report.
Keep generated reports and frozen provider documents ignored. Do not erase wallet
or financial evidence as part of documentation cleanup.
