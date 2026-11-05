# AGENTS.md

Guidance for code agents working on **x402_treazury**, a Rust executable and library.
The server exposes x402-paid HTTP APIs as ordinary MCP tools, owns payment
credentials and handles challenge/sign/retry. TOML providers compose into sources
and authenticated listeners. An embedded Zcash treasury can fund rotating Base
USDC wallets through NEAR Intents; optional Tor uses the same runtime machinery.

## Engineering preferences

- Avoid platform-specific code where possible. When OS-specific behavior is
  necessary, isolate it behind a small shared abstraction and implement it for
  Linux and macOS. Preserve equivalent correctness and security contracts;
  do not substitute silent no-ops. Follow the [portability guidance](docs/development.md#portability).
- Before vendoring a new package, tell the user why it is needed and describe
  alternatives and maintenance costs. Logging or diagnostic changes alone are
  generally insufficient justification for vendoring. Follow the
  [dependency policy](docs/development.md#dependency-vendoring).
- Favor simplicity and best-effort execution. Remove responsibilities before
  adding counters, certificates, recovery ledgers or workflow frameworks.
- Avoid additional local resource limits and network timeouts. Arbitrary limits
  are largely reliability hazards. Before adding or tightening a cap, queue,
  readiness gate, timer or failure switch, identify its concrete resource, external
  contract or correctness invariant and check existing guards. Generic caution is
  insufficient. Existing limits are not precedent or proof of necessity.
- Let valid work finish while it makes progress. Do not stack whole-work deadlines
  or replace removed timers with enormous durations. Connection establishment,
  response inactivity, signed expiry and polling cadence have separate meanings.
  Sibling traffic and keepalives cannot renew another response or financial authority.
- Avoid preventable agent-call delays. Prepare ahead of demand under existing
  authority where simple. Focused correctness, race and side-effect tests suffice
  for straightforward improvements; broader measurements belong to resource tuning,
  complex scheduling or quantitative performance claims.
- Scope failures to affected operations. Unsigned outages do not consume transaction
  recovery attempts. Disconnect, cancellation or cache eviction must not orphan
  accepted financial work, duplicate active I/O or report abandonment as success.

## Provider guidance

- Reusable providers expose supported functionality, with concrete reasons for
  compatibility restrictions.
- Same-path GET/POST twins default to POST before naming; review actual method
  contracts and document necessary opt-outs.
- Recommendations remain advisory until explicit opt-in.
- Provider-specific regression test and fixture storage needs a named quirk,
  failure or authored transformation; use small faithful inputs and shared
  synthetic tests instead of universal inventory/settings snapshots in
  fixtures and tests.
- Keep agent-facing tool descriptions and every parameter description concise,
  consistent and focused on selecting and using the API successfully. Explain
  purpose, inputs, useful results and constraints that affect the caller's next
  action; omit internal implementation details and repeated boilerplate. Keep
  maintainer rationale in owning guides. Follow the
  [tool documentation policy](docs/agent-sources.md#tool-documentation).
- Review endpoint summaries and discovery search keywords whenever capabilities or
  routing change, including new tools; follow
  [endpoint instruction placement](docs/agent-sources.md#endpoint-instruction-placement).

Follow the [provider policy](providers/README.md#provider-policy),
[shared maintenance boundaries](docs/provider-updates.md),
[selection semantics](docs/configuration.md) and
[test policy](docs/testing.md#provider-regressions). Provider oddities,
quirks, workarounds, and compatibility exemptions are documented in
[provider caveats](providers/CAVEATS.md).

## Financial and execution boundaries

- Reconcile from established blockchain/SDK facts instead of parallel local
  monetary tracking. Repair derived state and finish proven work; stale phases,
  reporting differences and missing optional evidence must not halt otherwise
  authorized progress. Follow the [reconciliation policy](docs/wallet-rotation.md#recovery-and-reconciliation-policy).
- Before adding a financial check, identify the actual authority or liability it
  protects, check existing enforcement, and explain why reconciliation cannot
  resolve the issue.
- Needless double-enforcement is forbidden. Treasury funding expresses the
  operator's capital limit: actual spendable funds govern ordinary enabled
  auto-funding, and the ZEC wallet balance the operator funds is the ZEC capital
  limit. Do not reintroduce daily/cumulative allocation, per-transfer ZEC spending
  budgets, or mirror a USDC reserve target into a ZEC cap. Reserve ranges authorize
  nominal USDC sizing only. Per-payment authority, actual network-fee caps,
  accepted-work ownership and separately authorized qualification API budgets
  protect distinct things. Qualification funding has no separate ZEC spending budget;
  reviewed job counts, reserve sizing and actual treasury funds govern new allocations.
- Ordinary USDC payment selection treats balances and price hints as advisory;
  current payer ownership and amount authority govern signing.
- After recognized structured insufficient funds, try each existing eligible wallet
  once with its own challenge while balance reconciliation proceeds independently.
  Apply the [unsigned x402 GET/POST retry semantics](docs/architecture.md#unsigned-x402-requests-and-post-retries)
  when assessing method restrictions. Drop the unused error body. Each authorization
  remains potentially chargeable. Other uncertainty grants no automatic replay;
  never allocate wallets or wait for funding during this traversal.
- Journal before disclosure. Preserve exact accepted inputs, signed bytes, original
  identities and liabilities through cancellation/restart; missing results do not
  prove nothing happened.
- SDK/blockchain state owns current ZEC spendability. Automatic shielding can recover
  owned transparent funds without refund attribution; refunds do not authorize
  another funding deposit.
- Each wallet's initial funding claim is consumed by its first positive USDC
  observation; low estimates never reopen it or authorize top-ups.
- All production outbound HTTP/gRPC constructors belong in `src/network.rs`.
  Preserve identity-bound transports, public-destination enforcement and Tor's remote
  DNS/no-direct-fallback policy. Do not introduce a vendor-specific bypass.
- Listener identity is the authenticated source-management principal; agent/session
  fields cannot select owners. Registration never allocates or funds a wallet.
- Ordinary serving binds listener sockets before funding. MCP availability follows
  each endpoint's `wait_for_catalogs` setting (default true); see
  [catalog startup](docs/configuration.md#multiple-mcp-ports-from-one-configuration).
  Ready endpoints remain usable while managed funding is pending or degraded;
  catalog failures disable only dependent waiting endpoints and emit ERROR.
  Strict inspection and separately
  authorized qualification retain their completeness and authority requirements.

Read the
[best-effort wallet rules](docs/wallet-rotation.md#best-effort-wallet-rules),
[wallet observation policy](docs/wallet-rotation.md#wallet-observation-policy),
and [reconciliation policy](docs/wallet-rotation.md#recovery-and-reconciliation-policy)
before changing estimation, polling, payment, funding or recovery behavior.

## Independent subagent workflows

Use independent subagents for design, audits, and review as a normal part of the
workflow; do not wait for the user to request delegation each time.

### Independence and coordination

Independent reviewers and auditors must not be authors of the work being
reviewed and start in fresh conversations without inherited history where
supported by the agent framework. These requirements apply to all design
reviews, audits and implementation reviews below, including those required by
the planning workflow.

Give reviewers self-contained briefs with the user intent, scope, baseline,
preserved invariants, owning guides, reviewed SHAs or snapshot, and validation
evidence, plus access to relevant repository state, planning documents and Git
history. Ask them to verify claims, question unnecessary enforcement, and look
for simpler approaches, not merely confirm the author's approach.

- Keep review/audit agents read-only by default. Parallelize independent analysis;
  coordinate delegated edits with explicit file ownership and avoid competing
  builds or source changes during validation. Do not impose a fixed reviewer count
  or duplicate identical checks without a concrete reason.
- The coordinating agent integrates findings and owns completion. If subagent
  tooling or fresh conversations are unavailable, the coordinating agent must tell
  the user which requirement could not be met when reporting review results.
  Perform the available checks, identify their scope and limitations, and do not
  claim that the required independent review occurred.

### Required reviews

- Before substantive implementation, arrange an independent design review.
  Resolve material design findings before building on the affected decision.
- For audits, assign distinct concerns that can be investigated independently.
- Independently review every exact committed SHA and its relevant consumers. The
  reviewer must not be the author of the change. Identify uncommitted review
  snapshots by base SHA and exact contents, including new files, and review the
  resulting exact SHA when committed.
- Address findings with `git commit --fixup=<introducing-SHA>` and independently
  review each fixup. Preserve unsquashed history through final integration review
  and subsequent user review rounds.
- Before completion and at meaningful architecture gates, independently review
  the combined implementation, all accumulated commits and fixups, and complete
  affected call chains. Per-commit approval does not replace this combined review.
  Scale review to the change; one review may cover both an exact SHA and the final
  combined state when it examines both scopes.
- For substantive Rust refactors and architecture gates, complete the CogC/CycC
  review required by the
  [complexity review discipline](tests/COMPLEXITY.md#review-discipline). Use
  complexity hotspots as focus points for simplification review.
- Resolve material findings and evaluate compelling simplifications before
  declaring completion.

### Review method

- For design reviews, challenge the scope, approach, necessity of checks, and
  opportunities to remove responsibilities, or otherwise simplify the deisgn.
  Verify assumptions against the current implementation and relevant pinned
  dependencies; check that proposed behavior, implementation detail, tests,
  validation and permanent documentation agree. Verify the applicability of
  reference checkouts before relying on them. Look for redundant enforcement or
  responsibilities already owned by components or dependencies for further
  opportunities to simplify the design.
- Inspect surrounding consumers, complete affected call chains, failure paths,
  test coverage and relevant unchanged high-risk code. Examine financial ownership,
  concurrency, network policy and documentation handoffs where affected. Each audit
  agent should assess its assigned scope and method and flag gaps.
- Choose concrete failure scenarios relevant to the affected contracts, such as
  concurrent agent calls, competing wallet state resolution, cancellation, crash
  and restart, blockchain reorgs or offline/online transitions. Trace progress,
  ownership, recovery and forbidden side effects and assess regression coverage.
  Do not require scenarios unrelated to the change.
- For implementation reviews, record an initial assessment before consulting the
  author's rationale and prior findings, then reconcile it with all prior findings
  and fixups. Supply required contracts and security context, known limitations
  and validation failures from the outset.
- For planned work, use the retained plan to check intended scope, implementation,
  acceptance evidence and documented deviations. Flag missing implemented portions,
  unsupported completion claims and unexplained departures. Assess correctness
  independently; agreement with the plan does not establish correctness.
- Use fixup and prior finding history to investigate recurring causes, repeated
  assumptions, hazards and testing gaps elsewhere in the affected call chains.
  Add regression coverage for gaps exposed by fixes and assess whether
  recurring patterns require a wider audit, simplification, or design change.
- A compelling simplification removes demonstrated fragility, redundant enforcement
  or unnecessary state and responsibilities while preserving required contracts.
  Investigate actual dependency contracts and recurring causes where relevant,
  rather than expanding into speculative redesign.
- Report concrete findings and evidence, the examined areas where no issue was
  found, and review limits. Focused checks do not establish repository-wide assurance.

### Implementation plan structure

Use a plan when requested or when staged work needs explicit decisions, commit
boundaries and review gates. Small changes can use the independent review
workflow above without requiring a plan file, a separate review record, or
architecture gates.

Before creating or revising an implementation plan, executing planned work or
removing a completed plan, read and follow the
[planning workflow](docs/planning-workflow.md). It owns plan structure, review
evidence, handoffs and completion. Commit and evolve active plans; remove completed
plans from the current tree while preserving them in Git history. Small changes
still use the independent review requirements above without requiring a plan or
redundant review stages.

## Read the relevant guide before editing

Load the documents for the area being changed; the detailed contracts and defaults
belong there rather than in another copy of this file. The [documentation index](docs/README.md)
links other maintained guides.

| Change area | Read first | Main implementation / tests |
| --- | --- | --- |
| Resource limits, cache lifetimes, readiness gates or polling | [Local resource limits](docs/local-resource-limits.md), [timer inventory](docs/network-egress.md#remaining-timers-and-budgets), [best-effort wallet rules](docs/wallet-rotation.md#best-effort-wallet-rules), [wallet observation policy](docs/wallet-rotation.md#wallet-observation-policy) | `src/limits.rs`, owning module |
| Payments, selection, funding or recovery | [Best-effort wallet rules](docs/wallet-rotation.md#best-effort-wallet-rules), [wallet observation policy](docs/wallet-rotation.md#wallet-observation-policy), relevant sections of [wallet ownership](docs/wallet-rotation.md), [selection/isolation design](docs/architecture.md#wallet-selection-and-network-isolation) | `src/payment.rs`, `src/rotation/`, `src/treasury/`; `tests/LIFECYCLE.md` |
| Wallet CLI, initialization, keys or state conversion | [Wallet administration](docs/wallet-cli.md), [configuration](docs/configuration.md) | `src/wallet_cli/`, `src/cli.rs` |
| Network clients, HTTP/gRPC timeouts or Tor | [Network policy](docs/network-egress.md), [resource limits](docs/local-resource-limits.md) | `src/network.rs`, `tests/network_audit.rs`, `tests/TOR.md` |
| Deployment startup, catalog publication or shutdown | [Runtime architecture](docs/architecture.md), [configuration](docs/configuration.md), [startup tests](tests/STARTUP.md) | `src/deployment/`, `src/server/`, `src/catalog_state.rs` |
| Dynamic sources, bundled providers, directory tools, endpoint instructions or raw calls | [Agent source management](docs/agent-sources.md), [manual raw headers](docs/agent-sources.md#manual-raw-headers), [bundled provider metadata](providers/README.md#bundled-provider-metadata), [provider-management permissions](docs/agent-sources.md#configuration-and-authority), [registration and published-source contracts](docs/agent-sources.md#registration-and-tool-invocation), [local and remote directory matching](docs/agent-sources.md#external-directory-matching), [tool search matching](docs/agent-sources.md#tool-search-matching-and-results), [remote provider details and URL discovery](docs/agent-sources.md#listing-and-url-discovery), [endpoint instruction placement](docs/agent-sources.md#endpoint-instruction-placement) | `src/discovery/`, `src/providers.rs`, `src/catalog_state.rs` |
| Deployment source distribution and effective bindings | [Source tool distribution](docs/configuration.md#source-tool-distribution), [catalog and runtime handoff](docs/architecture.md#catalog-configuration-and-execution), [endpoint-local authority](docs/agent-sources.md#configuration-and-authority) | `src/deployment/routing.rs`, `src/deployment/`, `src/rotation/assignment.rs` |
| Provider TOML, OpenAPI schemas, pricing, caching or relay | [Provider maintenance](docs/provider-updates.md), [Configuration](docs/configuration.md), [header and idempotency contracts](docs/configuration.md#openapi-header-parameters-and-idempotency), [catalog architecture](docs/architecture.md#catalog-configuration-and-execution), [provider policy](providers/README.md#provider-policy), [provider caveats](providers/CAVEATS.md), [provider regressions](docs/testing.md#provider-regressions) | `src/catalog.rs`, `src/config.rs`, `src/pricing/`, `src/http_cache.rs`, `src/discovery_relay.rs`; focused provider regressions |
| MCP images, binary output or truncation | [Output boundaries](docs/architecture.md#mcp-and-output-boundaries), [download/display limits](docs/configuration.md#download-and-display-limits) | `src/output.rs`, `src/server.rs`, `tests/media.rs` |
| Optional cover traffic | [Cover policy](docs/cover-traffic.md) | `src/cover/` |
| Live integration, financial evidence or qualification reports | [Runbook](tests/live/INTEGRATION.md), [technical reference](tests/live/INTEGRATION_REFERENCE.md), [qualification status](docs/testing.md#qualification-status) | `tests/live/runner/driver.rs`, `tests/integration_preparation.rs` |
| Builds, dependencies, test infrastructure or release tooling | [Development](docs/development.md), [testing](docs/testing.md), [scripts](scripts/README.md), relevant [vendor provenance](vendor/README.md) / [Zingo integration](docs/zingo-integration.md) | `scripts/`, `tools/`, `vendor/`; [reproducible builds](docs/reproducible-builds.md) |

When changing policy, update its owning guide and affected references. Resource
policy changes must keep both inventories current. Do not infer a runtime deadline,
external settlement contract or live qualification from a fixture timeout.

For dependency patches, follow [checkout retirement tracking](vendor/README.md#checkout-retirement-tracking).
Prioritize complete checkout removal; assess every remaining blocker and preserve
required behavior, regression coverage and provenance before counting a retirement.

## Commands and test exceptions

Run commands from the repository root. Zcash is unconditional in the application
build; static wallet deployments simply omit treasury configuration. Use the
[Makefile entrypoints](scripts/README.md) for pinned Cargo and Cargo-managed protoc. `make build` defaults to release without incremental caches;
`make build-dev` opts into an incremental development build.

```sh
make build
make test
make clippy
make fmt-check
```

- Follow [testing efficiency](docs/testing.md#testing-efficiency): avoid validation
  matrices, reuse valid evidence, consolidate expensive setup and prefer functional
  assertions and deterministic synchronization.
- Follow [validation order and continuation](docs/testing.md#validation-order-and-continuation):
  complete applicable design/complexity review and full all-target Clippy and
  compatibility checks before broad runtime suites; run affected cheap contract
  tests first. After a localized fix, revalidate affected consumers and resume
  unfinished named `check-*` stages. Repeat passed stages only when their evidence
  is invalidated, and record the scope and results carried forward. `make check`
  remains the full application check, including the synthetic-chain treasury lifecycle.
- Follow [compiler diagnostics and review](docs/testing.md#compiler-diagnostics-and-review):
  run the production compiler and all-target Clippy warning gate; inspect diagnostics
  alongside exit status. Selected-target `-D warnings` does not cover dependency
  warnings, and vendoring can expose previously suppressed upstream diagnostics.
- Coordinate editor-triggered Cargo checks and separate builds before validation;
  Make's serialization does not cover other processes. Keep source, executable,
  feature and profile evidence consistent with the testing guide.
- Choose sandbox permissions before running tests. Keep socket-free tests in the
  ordinary sandbox. When localhost sockets are known to be blocked, request
  socket-capable execution through the runner up front for socket fixtures and
  full suites; do not deliberately run them once to obtain a permission failure.
  Follow the [test sandbox guidance](docs/testing.md#sandbox-permissions).
- Only `zcash-testutils` and `zcash-regtest` are opt-in test features. Run requested
  feature configurations sequentially: CLI tests share an executable path. Keep
  sources/executables frozen during their validation.
- Tests use temporary state, public unfunded deterministic keys and localhost
  fixtures. Never load real wallet secrets. Live-funded qualification requires
  separate operator authorization; consensus tests require Docker and an explicit
  ignored-test invocation. Offline fixtures do not qualify live settlement or Tor.
- Verify changed financial tests are actually wired into their modules and execute;
  a filtered command can pass with zero tests. Assert forbidden side effects stay
  absent. Cover slow progress versus true stalls, cancellation/restart and unrelated
  concurrent progress when changing failure policy. Do not loosen contracts or trim
  documentation to make tests pass.
- Run `make toolchain` before plain Homebrew Cargo commands. Do not
  change toolchain/profile pins or weaken the reproducible-build sandbox to pass a
  check. Patch dependencies in reviewed `vendor/` sources, never Cargo caches or
  reference checkouts. Python is development tooling, never a runtime dependency.

## Local files, logs and documentation

- Keep `.env`, `state/`, `secrets/`, local example overrides and reference checkouts
  untracked.
- Preserve unrelated user files and local changes, whether tracked, untracked or
  ignored.
- Do not inspect or print secret values unnecessarily.
- Do not routinely delete `target/`: it may hold private qualification evidence and
  pinned executables as well as disposable build caches.
- All logs go to stderr; stdout is the MCP wire or requested CLI JSON. Never persist
  or log credential-bearing upstream prose. Follow the [logging contract](docs/development.md#logging-and-diagnostics)
  when changing diagnostics.
- Choose log severity by the owning capability or operation after applicable
  recovery: ERROR for required service/endpoint functionality unavailable with no
  recovery remaining in the current lifecycle; WARN for terminal agent-operation
  failures or unexpected loss of functionality, including omitted selected catalog
  tools; INFO for meaningful user-visible lifecycle, progress, delay or recovery
  changes; DEBUG/TRACE for routine outcomes, individual attempts and diagnostic
  detail. Intermediate failures with retry or fallback remaining must not emit
  WARN/ERROR. Expected negative results and routine successes normally belong at
  DEBUG/TRACE. Follow the [severity policy](docs/development.md#log-severity)
  for scope and examples.
- INFO and higher need plain-language message strings a user can understand
  without reading source code. WARN/ERROR also need enough safe diagnostic context
  to identify what failed, its impact and the known cause or uncertainty. Follow
  [message clarity and diagnostic detail](docs/development.md#message-clarity-and-diagnostic-detail);
  fields and correlation IDs supplement meaningful messages, never replace them.
- Correct logging-policy violations encountered during development, including
  excess INFO, unclear messages and inadequate failure detail, and update their
  owning documentation and affected diagnostic tests.
  Existing call sites are not precedent; a repository-wide logging cleanup is not
  required for unrelated work.
- Limits that reject or truncate content must be visible to the affected consumer;
  never present a partial document as complete. Visibility requirements do not
  justify adding a limit; see [resource-limit behavior](docs/local-resource-limits.md#rejection-and-truncation).
- Maintain current rules in place. Keep detailed contracts in their owning guides,
  not appended migration notes or duplicate checklists.
- Follow the [documentation policy](docs/README.md): generated metrics/reports go
  under ignored `target/` or private external storage, and completed plans yield
  maintained guidance plus focused unfinished work. Do not add implemented-plan
  archives or dated handoffs.
- Documentation/Git cleanup never erases financial evidence or grants replay authority.
- Preserve stable state formats, generated `auto_v1_` pool names, Tor token domains
  and `X402_*` environment contracts when renaming files or product labels.
