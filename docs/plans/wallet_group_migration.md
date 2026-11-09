# Wallet group rename and configuration preview

## Intent, baseline and status

Provide a general migration mechanism for managed wallet group names and endpoint
assignments: preserve selected existing groups through explicit renames, and explain
how a deployment configuration relates to saved wallet state. Documentation must use
generic examples, not prescribe a migration for a particular deployment revision.
Reusing an existing group for a different purpose is an operator choice; it preserves
its payer identities and historical correlation.

Starting branch: `main`. Source baseline:
`6f3e1e6fd6738c5fece86dc796c5ef0686653552`.
Status: implementation in progress on `wallet_group_migration`; no operator state migration performed. Preserve the
unrelated untracked `kilo.json` and `state`; do not inspect real wallet secrets or
perform a migration against operator state for validation.

Owning contracts: [wallet administration](../wallet-cli.md),
[configuration](../configuration.md),
[wallet ownership](../wallet-rotation.md#binding-and-ownership),
[source ownership](../agent-sources.md#persistence-and-operator-maintenance),
[development](../development.md), [testing](../testing.md),
[complexity review](../../tests/COMPLEXITY.md#review-discipline), and the
[planning workflow](../planning-workflow.md). Verify section links when updating
maintained documentation. The existing [sweeping plan](wallet_merge_sweeping.md)
owns transfers and consolidation, which this work does not require or implement.

## Current behavior and preserved boundaries

- One treasury database owns all managed groups. `pools.name` resolves a durable
  random UUID; wallets, claims and financial work reference that UUID. EVM key
  authentication binds treasury/network/group/wallet identities, not the group name.
- Editing a configuration group name alone selects a new group. Managed startup
  creates missing groups and their empty claims, disables omitted groups, and retains
  keys and accepted work. Allocation/funding remains separately governed by existing
  authority. Explicit unreferenced managed profiles still request groups.
- Source wallet overrides take precedence over listener defaults and then automatic
  assignment. Many endpoints may reuse one group. Automatic scope-derived names can
  change with endpoint/source names; template edits alone preserve those names.
- `config show` resolves bindings without opening wallet state. `wallet status`
  reads coherent saved state without remote observation. Existing read-only SQLite
  coordination side effects are permitted; preview must not modify logical state or
  recover original journals in place.
- Preserve exact accepted work, original identities, signed bytes, initial-funding
  claim consumption, and existing schema/security contracts. No new monetary ledger,
  readiness gate, timeout, resource cap, dependency or general migration framework.

## Proposed command contracts

### Rename

`wallet rename OLD NEW [--config FILE | --state-dir DIR ...] [--json]` uses the
existing wallet location and optional treasury-identity conventions. With `--config`,
resolve location/identity only: neither name needs to be present in that file. Do not
apply profiles, declare groups, load catalogs, resolve unrelated credentials, decode
the SDK wallet, start workers or contact the network as part of rename.

Require stopped serving through the existing exclusive authenticated store owner.
In one transaction, verify the source exists and destination is unoccupied, then
change only `pools.name` semantically. Existing authenticated store opening can run
its supported additive initializers; do not introduce extra migration writes or
promise byte-identical SQLite files. Preserve the group UUID and every other group field, key,
wallet role, claim, generation, financial record and liability. Pending work and a
disabled source are valid: no financial quiescence prerequisite is needed for a
lookup-name change. An existing source renamed to itself succeeds as a no-op; a
missing source fails even if OLD equals NEW. Follow existing store name validation.
Allow supported `auto_v1_` names for explicit migration to/from generated identities;
the restriction against declaring such names in explicit TOML profiles remains.

Reject occupied destinations without overwriting, merging or inferring intent,
including empty groups created by an earlier startup. Failure must leave names and
financial state unchanged. Output the original/new names and unchanged group UUID;
JSON follows existing CLI conventions. After success, the operator updates config
references and reopens serving. Rename does not edit TOML or grant funding authority;
restart applies the selected group's current configured policy as today. Lost CLI
acknowledgment is resolved by inspecting state, never by creating another identity.

### Preview

`wallet preview --config FILE [--json]` compares the pure validated deployment wallet
resolution with a single coherent read-only saved-state snapshot. Require deployment
configuration, honoring `X402_TREAZURY_CONFIG`; standalone location-only mode cannot
describe a configuration comparison. Read while serving where existing inspection
permits it. Read identity validation and inventory/policy data in the same inspection
transaction, following `static_ownership` rather than separate identity/status reads;
compare any configured treasury-ID assertion there. Preserve existing schema/preflight
checks. This is unauthenticated saved-metadata inspection, not cryptographic verification
or spending authority. Do not require the encryption key or SDK decoding merely to compare public
group metadata, and do not construct a Treasury owner or mutate logical state.
Missing initialized state is an actionable initialization error, not implicit init.

Report deterministically in human and versioned JSON output:

- Existing matching managed groups and stable UUIDs, distinguishing reuse,
  reactivation, and changes to saved versus requested payment/reserve policy.
- Configured managed names missing from state, which normal startup would declare.
- Saved groups omitted from the managed configuration, distinguishing already
  disabled groups from groups startup would disable. Their holdings and history
  remain owned; there is no automatic transfer, deletion or sweeping.
- Managed/static name conflicts that prevent startup. Valid unrelated static profiles
  require no key loading or state creation. Handle static-only configurations without
  claiming they disable state: startup without managed groups skips
  `configure_profiles`, while saved managed names still block static reuse.
  Separately required direct-swap admission/recovery may still open the treasury.
- Effective source/listener bindings and explicit/generated origins, plus configured
  reserve sizing and auto-funding policy using existing resolution. Clearly distinguish
  nominal settings from accepted funding terms and actual spendable capacity.
- Saved inventory and pending-work summaries sufficient to identify useful or occupied
  groups, reusing existing status projections. Saved balances are advisory observations,
  not fresh reads or promises of available funds; expose no secret material.

Do not infer rename matches, execute changes, contact providers, fetch catalogs,
require listener/static-key/Base/NEAR credentials, or forecast actual transaction
costs or guaranteed funding. Report valid differences successfully; configuration,
identity, storage failures and startup-blocking mode conflicts return nonzero with
clear safe diagnostics. Do not change `config show` into a stateful command or create
a saved executable migration plan. Preview is a snapshot, not an authority token;
rename rechecks names under ownership and startup rechecks current configuration.

## Ordered commits and acceptance evidence

### WG1 — Offline group rename

Add the store operation and CLI path together with generic wallet CLI guidance,
configuration identity/rename guidance, and focused regressions. Inspect
`src/wallet_cli.rs`, its resolution/output consumers, `src/rotation/store.rs`,
`store/capacity.rs`, `store/schema.sql`, and funding consumers selecting configuration
by the current joined pool name. Use the existing store owner rather than an SDK
Treasury open solely to change a name. No schema conversion is expected.

Acceptance: rename active and disabled groups with an unchanged UUID; reopen and
decrypt their keys; preserve claims, authorizations, exact pending funding bindings
and stored bytes; resume the renamed group's relevant startup/funding lookup path
under synthetic authority. Cover missing source, self-rename, invalid names,
occupied target and transaction failure without partial updates. Cover ownership
refusal while serving, generated-name migration, location-only config resolution,
and absence of allocation, signing, network or SDK sync side effects. Use existing
synthetic fixtures; do not build a new financial workflow harness.

### WG2 — Configuration-versus-state preview

Add the read-only comparison and human/JSON CLI together with maintained generic
usage and output contracts. Reuse `rotation/assignment.rs`, existing pure deployment
validation and saved store reporting; prefer `MetaConfig` validation/assignment
resolution rather than `Deployment::show_config`, which resolves provider files and
registry paths. Include every resolved managed profile, even unbound declarations;
disabled saved names still conflict with static names. Do not duplicate wallet resolution or startup
configuration logic. Update CLI help and owning `docs/wallet-cli.md`,
`docs/configuration.md`, and `docs/wallet-rotation.md` where behavior belongs.

Acceptance: synthetic inventories with fewer/more groups, reactivation, policy
changes, sharing/source overrides, generated scope changes, static conflicts and
static-only serving semantics. Verify coherent reads while an owner exists, safe
handling of missing/unsupported state and mismatched treasury assertions, deterministic
human/JSON output, and no dependence on unrelated credentials or the key. Preserve
all logical state, accepted bytes and keys; preview neither allocates nor funds nor
opens catalogs/network clients. Exercise renamed groups through preview and normal
startup, including a destination created before rename. Tests must execute nonzero
intended cases and assert forbidden effects, rather than mirror report construction.

### WG3 — Documentation handoff and cleanup

After the final implementation review and applicable user review rounds, verify
that generalized guidance covers backup/stopped rename, config reference edits,
preview, restart, different group counts, name collisions and historical linkability.
Explain that listener principal changes for registries/direct payments are separate
and that automatic wallet assignment does not create listener definitions. No
example-specific runbook, split/merge command, registry owner reassignment or transfer
is included. Existing AGENTS.md links to these owning guides suffice; update them
only if a destination moves. Remove this plan and repair its index link in a cleanup
commit, preserving the full ancestry and review evidence in Git history.

## Review and validation gates

- **D1, before implementation:** independent fresh-conversation design review of
  both commands, actual dependency/store contracts, complete relevant consumers,
  scope and simpler alternatives. Resolve material findings and commit plan changes.
- Independently review each exact WG1/WG2 SHA and affected unchanged consumers.
  Record an initial assessment before consulting rationale/prior findings. Fix
  findings using `git commit --fixup=<introducing-SHA>`; independently review every
  fixup and preserve unsquashed history. Inspect name-selected funding configuration,
  UUID-bound encryption/ownership, assignment resolution and preview read consistency.
- Use source-matched CogC/CycC evidence and independent simplification review under
  `tests/COMPLEXITY.md` for substantive Rust refactors/architecture changes before
  broad runtime suites. Review affected CLI resolution/reporting hotspots; no
  blanket numerical complexity gate or speculative refactor.
- Select validation by changed behavior: cheap contract checks, then the production
  compiler and all-target Clippy gate (`make check-clippy`), affected compatibility
  coverage and focused store/CLI/deployment tests. Use Make's pinned toolchain and
  test entrypoints, inspect diagnostics and counts, and run socket-free fixtures
  normally; request socket-capable execution up front for any selected socket tests.
  Broaden only for affected boundaries; no full qualification, live funds, Docker,
  Tor run or dependency provenance rebuild is required solely by these commands.
- **I1, after WG2 and all fixes:** independent combined review of all accumulated
  SHAs/fixups, complete affected rename/preview/startup/funding call chains, maintained
  documentation and actual validation. One appropriately scoped review may satisfy
  exact-SHA and combined obligations. Exit only with no unresolved material findings
  or compelling simplifications and all selected evidence present. Repeat affected
  review after changes; reuse evidence whose source/build inputs remain valid.
- **I2, after WG3:** independently review the exact cleanup SHA and final combined
  branch, including prior commits, fixups, documentation and remaining plans. No
  push, merge, state migration or funded execution is authorized by planning.

## Evidence and remaining work

- Baseline investigation: current code and documentation examined; independent
  read-only audit confirmed stable UUID/name separation and existing count mismatch
  behavior. No wallet state, secrets or live behavior inspected. This is design
  evidence, not compiler or runtime validation.
- D1: independent fresh-conversation reviewer `/root/wallet_plan_design` examined
  the proposed contracts and complete relevant source consumers, then reviewed the
  plan snapshot SHA-256 `f74e00ce5af81d401b185bd3129334312052af54967ecc89275b38d0bec85a64`
  and index snapshot `b5815c7ea58cc514c527f62345266edac32359fbeedbaa812ff1cfccafcf6c4d`
  against the baseline. No unresolved material design findings. Incorporated
  recommendations: preserve supported store-open additive initialization; validate
  identity and inventory in one read transaction; describe metadata inspection
  without authentication claims; reuse pure resolution and include unbound groups;
  distinguish skipping profile configuration from direct-only treasury opening.
  Exact committed-SHA review follows the plan commit. Review was read-only; no
  compiler/runtime assurance is claimed.
- Planning checks: `git diff --check` and local Markdown target/section-anchor
  validation passed. WG1/WG2/WG3: not implemented. I1/I2: pending.
- Record exact plan and implementation SHAs, design findings/dispositions, fixups,
  source/build evidence, executed checks and missing coverage here as work proceeds.

- Implementation D1: fresh read-only reviewer `/root/design` reviewed exact plan
  commit `53478e522a8ba9b3b815b49900b73db5a9fd8598` and actual CLI, store,
  assignment, startup and funding consumers. No material blockers. Reuse shared
  location validation for rename, coherent identity/status inspection for preview,
  and persisted bounds for policy comparison; historical sizing mode and auto_fund
  are not persisted and must not be reconstructed. No new dependency or framework.
- WG1 implemented: authenticated exclusive Store rename, shared name validation,
  one immediate name-only transaction, CLI location-only resolution, generic guides
  and exact logical-state/key/pending-byte regressions. Pre-commit read-only review
  `/root/design` found inherited network installation could read unrelated CA files;
  rename now skips that setup and its CLI fixture contains a missing CA file.
  The test fixture's missing accepted preparation/revision was corrected before
  commit. Store rename tests executed 2/2 successfully (`/tmp/wg1-tests.log`);
  macOS linker emitted its existing large unwind-section warning, no runtime failure.
  First `make check-clippy` passed (`/tmp/wg1-clippy.log`); later CLI resolution/test
  changes require the final compiler gate. CLI execution, complexity and exact-SHA
  review pending. Operator state and unrelated files remain untouched.
- WG1 exact implementation: `680a3248`. Executable rename regression initially
  failed because its assertion expected prose containing "own" rather than the
  existing `state_in_use` category. Corrected only that assertion; rerun executed
  1/1 successfully (`/tmp/wg1-cli-fix.log`), including missing custom CA and exclusive
  ownership. The prior 2/2 store-test evidence remains valid. Fixup review pending.
- WG1 exact-SHA review: `/root/design` independently reviewed `680a3248` and
  evidence commit `3d00a0bb`, then fixup `585f1d67`; no unresolved production
  findings. Added coverage in the existing concrete funding-backend fixture:
  accept original quote, rename, resolve the same job by UUID, select current-name
  sizing, then continue its original preparation/submission/recovery path. No new
  harness or financial authority; execution and exact fixup review pending.
- WG2 implemented in the working tree: pure validated resolution, coherent
  identity/status inspection, deterministic versioned comparison and human output,
  CLI routing before generic state/key setup, and generic maintained guides.
  Fresh read-only combined snapshot review `/root/design` found no material
  correctness issues. `make check-clippy` passed (`/tmp/wg2-clippy.log`). Initial
  preview unit run executed 5 cases but failed because the shared synthetic routing
  fixture used two unconditional rules; corrected it to the existing single-rule
  multiple-destination contract. No production relaxation. Runtime rerun pending.
- Complexity: complete pinned baseline comparison against `6f3e1e6f`, 490 files,
  zero exclusions, artifact `target/complexity-runs/20261109T051227Z-ef499928`.
  Independent review retained the explicit `run_args` dispatcher (CogC 14→22,
  CycC 35→45): early returns expose offline/SDK ownership boundaries; extracting
  dispatch only moves complexity. Selected simplification replaces rename's dynamic
  JSON/three unwraps with a typed result. Preview report (11/24) and human formatter
  (13/12), shared status rendering, startup and funding consumers were reviewed.
  Final source-matched refresh and compiler gate are pending after these changes.
