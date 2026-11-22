# Agent instructions for x402_treazury

`x402_treazury` is a Rust executable and library that exposes x402-paid HTTP
APIs as MCP tools, owns payment credentials, and handles payment challenges,
signing, and retries. TOML providers compose into sources and authenticated
listeners. An embedded Zcash treasury funds rotating Base USDC wallets through
NEAR Intents, and optional Tor routes traffic through the same network layer.

Treat [financial, network, and state-compatibility boundaries](#financial-and-network-boundaries)
and [authority limits](#authority-limits) as hard non-negotiables; simplicity,
provider, logging, review, workflow, and writing-style rules serve those
invariants.

## Guides and references

Read the maintained guide for the area you are changing before editing; detailed
contracts and defaults live there rather than in copies here. The
[documentation index](docs/README.md) lists all maintained guides.

| Change area | Read first | Main implementation and tests |
| --- | --- | --- |
| Resource limits, cache lifetimes, readiness gates, or polling | [Local resource limits](docs/local-resource-limits.md), [timer inventory](docs/network-egress.md#remaining-timers-and-budgets), [best-effort wallet rules](docs/wallet-rotation.md#best-effort-wallet-rules), [wallet observation policy](docs/wallet-rotation.md#wallet-observation-policy) | `src/limits.rs`, owning module |
| Payments, selection, funding, or recovery | [Best-effort wallet rules](docs/wallet-rotation.md#best-effort-wallet-rules), [wallet observation policy](docs/wallet-rotation.md#wallet-observation-policy), [wallet ownership](docs/wallet-rotation.md), [reconciliation policy](docs/wallet-rotation.md#recovery-and-reconciliation-policy), [wallet selection and isolation](docs/architecture.md#wallet-selection-and-network-isolation) | `src/payment.rs`, `src/rotation/`, `src/treasury/`; `tests/LIFECYCLE.md` |
| Direct swap control | [Direct swaps](docs/configuration.md#direct-swap-control), [receive ownership](docs/wallet-rotation.md#direct-receive-ownership), [send ownership](docs/wallet-rotation.md#direct-send-ownership), [Ironwood policy](docs/zingo-integration.md#ironwood-only-policy), [SDK receivers](docs/zingo-integration.md#refund-receivers-and-seed-recovery), [network policy](docs/network-egress.md) | `src/direct_swap/`, `src/rotation/store/direct_swap.rs`, `src/treasury/`, `src/server.rs` |
| Wallet CLI, initialization, keys, or state conversion | [Wallet administration](docs/wallet-cli.md), [configuration](docs/configuration.md) | `src/wallet_cli/`, `src/cli.rs` |
| Network clients, HTTP and gRPC timeouts, or Tor | [Network policy](docs/network-egress.md), [resource limits](docs/local-resource-limits.md) | `src/network.rs`, `tests/network_audit.rs`, `tests/TOR.md` |
| Deployment startup, catalog publication, or shutdown | [Runtime architecture](docs/architecture.md), [configuration](docs/configuration.md), [startup tests](tests/STARTUP.md) | `src/deployment/`, `src/server/`, `src/catalog_state.rs` |
| Dynamic sources, bundled providers, directory tools, endpoint instructions, or raw calls | [Agent source management](docs/agent-sources.md) (permissions, registration, matching, listing and URL discovery, raw headers, instruction placement), [bundled provider metadata](providers/README.md#bundled-provider-metadata) | `src/discovery/`, `src/providers.rs`, `src/catalog_state.rs` |
| Deployment source distribution and effective bindings | [Source tool distribution](docs/configuration.md#source-tool-distribution), [catalog and runtime handoff](docs/architecture.md#catalog-configuration-and-execution), [endpoint-local authority](docs/agent-sources.md#configuration-and-authority) | `src/deployment/routing.rs`, `src/deployment/`, `src/rotation/assignment.rs` |
| Provider TOML, OpenAPI schemas, pricing, caching, or relay | [Provider maintenance](docs/provider-updates.md), [configuration](docs/configuration.md), [header and idempotency contracts](docs/configuration.md#openapi-header-parameters-and-idempotency), [catalog architecture](docs/architecture.md#catalog-configuration-and-execution), [provider policy](providers/README.md#provider-policy), [provider caveats](providers/CAVEATS.md), [provider regressions](docs/testing.md#provider-regressions) | `src/catalog.rs`, `src/config.rs`, `src/pricing/`, `src/http_cache.rs`, `src/discovery_relay.rs` |
| MCP images, binary output, or truncation | [Output boundaries](docs/architecture.md#mcp-and-output-boundaries), [download and display limits](docs/configuration.md#download-and-display-limits) | `src/output.rs`, `src/server.rs`, `tests/media.rs` |
| Optional cover traffic | [Cover policy](docs/cover-traffic.md) | `src/cover/` |
| Live integration, financial evidence, or qualification reports | [Runbook](tests/live/INTEGRATION.md), [technical reference](tests/live/INTEGRATION_REFERENCE.md), [qualification status](docs/testing.md#qualification-status) | `tests/live/runner/driver.rs`, `tests/integration_preparation.rs` |
| Builds, dependencies, test infrastructure, or release tooling | [Development](docs/development.md), [testing](docs/testing.md), [scripts](scripts/README.md), [vendor provenance](vendor/README.md), [Zingo integration](docs/zingo-integration.md), [reproducible builds](docs/reproducible-builds.md) | `scripts/`, `tools/`, `vendor/` |

- When changing policy, update its owning guide and linked references. Resource
  policy changes must keep both inventories current. Never infer a runtime
  deadline, external settlement contract, or live qualification from a fixture
  timeout.
- For dependency patches, follow
  [checkout retirement tracking](vendor/README.md#checkout-retirement-tracking).
  Prioritize complete checkout removal; assess every remaining blocker and
  preserve behavior, regression coverage, and provenance before counting a
  retirement.

## Engineering, financial, and runtime contracts

### Financial and network boundaries

Read the
[best-effort wallet rules](docs/wallet-rotation.md#best-effort-wallet-rules),
[wallet observation policy](docs/wallet-rotation.md#wallet-observation-policy),
and [reconciliation policy](docs/wallet-rotation.md#recovery-and-reconciliation-policy)
before changing estimation, polling, payment, funding, or recovery behavior.

- **ZEC is Ironwood only:** Reject Sapling and legacy Orchard transfers,
  historical-pool fallback, transparent wallet receiving or shielding, and
  transparent-address RPC queries. The sole external-output exception is a
  NEAR-issued transparent deposit address paid from Ironwood under its saved
  quote. Never initialize or download legacy proving parameters in application
  code. Keep only SDK-unavoidable types, decoding and build activity, and
  accepted-work ownership; see the
  [Ironwood-only policy](docs/zingo-integration.md#ironwood-only-policy).
- **Reconcile from blockchain and SDK facts:** Derive monetary truth from
  established chain and SDK state instead of parallel local accounting. Repair
  derived state and finish proven work; stale phases, reporting differences, and
  missing optional evidence must not halt authorized progress. Before adding a
  financial check, identify the authority or liability it protects, check
  existing enforcement, and explain why reconciliation cannot resolve it.
- **No double enforcement:** Treasury funding expresses the operator's capital
  limit: spendable funds govern enabled auto-funding, and the funded ZEC wallet
  balance is the ZEC capital limit. Never reintroduce daily or cumulative
  allocation caps, per-transfer ZEC spending budgets, or ZEC caps mirrored from
  USDC reserve targets. Reserve ranges authorize nominal USDC sizing only.
  Per-payment authority, SDK transaction validity, accepted-work ownership, and
  separately authorized qualification API budgets protect distinct things;
  qualification funding has no separate ZEC spending budget (reviewed job
  counts, reserve sizing, and supported treasury funds govern allocations).
  Listener `swap_control.max_send_zec` bounds only per-call debit for direct
  swap tools, never managed funding or other ZEC usage.
- **Advisory balances and insufficient-funds retries:** USDC payment selection
  uses cached balances and price hints as estimates, while payer ownership and
  the payment cap govern signing. After a recognized structured
  insufficient-funds response, try each eligible wallet once with its own
  challenge while balance reconciliation runs independently. Apply the
  [unsigned x402 GET and POST retry semantics](docs/architecture.md#unsigned-x402-requests-and-post-retries)
  for method restrictions, drop the unused error body, and treat each signed
  authorization as potentially chargeable. Other uncertainty grants no automatic
  replay; never create wallets or wait for funding while retrying.
- **Journal before disclosure:** Persist exact accepted inputs, signed bytes,
  original identities, and liabilities across cancellation and restart; missing
  results do not prove nothing happened.
- **Spendability, claims, and refunds:** Blockchain and SDK state own supported
  Ironwood spendability; unsupported pool holdings grant no spending, shielding,
  or fallback authority. Each wallet's initial funding claim is consumed by its
  first positive USDC observation, and low estimates never reopen it or
  authorize top-ups. Managed funding may replace a bound `REFUNDED` swap only
  after confirmed receiver observation and source closure under an unused claim
  (a refund alone grants no replay authority); see the
  [refund recovery policy](docs/wallet-rotation.md#refunds-state-and-recovery).
- **Centralized outbound network policy:** All production outbound HTTP and gRPC
  constructors belong in `src/network.rs`. Preserve identity-bound transports,
  public-destination enforcement, and Tor remote DNS with no direct fallback.
  Never add a vendor-specific bypass.
- **Listener authority and startup:** Listener identity is the authenticated
  source-management principal; agent or session fields cannot choose owners, and
  registration never allocates or funds a wallet. Normal serving binds listener
  sockets before funding. MCP availability follows each endpoint's
  `wait_for_catalogs` setting (default `true`; see
  [catalog startup](docs/configuration.md#catalog-loading-and-listener-startup)
  and the [framework integration contract](docs/framework-integration.md)).
  Ready endpoints stay usable while managed funding is pending or degraded;
  catalog failures disable only waiting endpoints that depend on them and did
  not set `allow_catalog_failure`, emitting `ERROR`. Strict inspection and
  separately authorized qualification keep their completeness and authority
  requirements.
- **State and contract compatibility:** Preserve stable state formats, generated
  `auto_v1_` pool names, Tor token domains, and `X402_*` environment contracts
  when renaming files or product labels.

### Simplicity, portability, and timeouts

- **Simplicity and best-effort execution:** Remove responsibilities before
  adding counters, certificates, recovery ledgers, or workflow frameworks. Check
  component and upstream dependency guarantees before adding wrapper enforcement
  or patches.
- **Platform portability:** Avoid platform-specific code where possible. When
  OS-specific behavior is necessary, isolate it behind a small shared
  abstraction and implement it for both Linux and macOS with equivalent
  correctness and security contracts (never silent no-ops). Follow the
  [portability guidance](docs/development.md#portability).
- **No arbitrary limits or timeouts:** Arbitrary local resource limits and
  network timeouts are reliability hazards. Before adding or tightening a cap,
  queue, readiness gate, timer, or failure switch, identify the concrete
  resource, external contract, or correctness invariant and check existing
  guards. Generic caution and existing limits are not precedent or proof of
  necessity.
- **Distinct timeout contracts:** Let valid work finish while it makes progress.
  Do not stack whole-work deadlines or replace removed timers with huge
  durations. Connection establishment, response inactivity, signed expiry, and
  polling cadence protect separate boundaries; sibling traffic and keepalives
  cannot renew another response or financial authority.
- **Low latency and scoped failures:** Avoid preventable agent-call delays and
  prepare ahead of demand under existing authority where simple (focused
  correctness, race, and side-effect tests suffice for straightforward
  improvements; broader benchmarks belong to resource tuning, complex
  scheduling, or quantitative performance claims). Scope failures to the failing
  operation: unsigned outages do not consume transaction recovery attempts, and
  disconnect, cancellation, or cache eviction must not orphan accepted financial
  work, duplicate active I/O, or report abandonment as success.

### Provider and tool contracts

Follow the [provider policy](providers/README.md#provider-policy),
[shared maintenance boundaries](docs/provider-updates.md),
[selection semantics](docs/configuration.md), and
[test policy](docs/testing.md#provider-regressions).

- Reusable providers expose supported functionality and document concrete
  reasons for compatibility restrictions. Same-path `GET` and `POST` twins
  default to `POST` before naming; inspect actual method contracts and document
  opt-outs. Recommendations remain advisory until opt-in.
- Store provider-specific regression fixtures only for a named quirk, failure,
  or authored transformation; use small faithful inputs and shared synthetic
  tests instead of universal inventory or settings snapshots.
- Keep agent-facing tool descriptions and parameter descriptions concise,
  consistent, and focused on choosing and calling the API successfully. State
  purpose, inputs, useful results, and constraints that affect the caller's next
  action; omit implementation details and boilerplate, and keep maintainer
  rationale in owning guides. Follow the
  [tool documentation policy](docs/agent-sources.md#tool-documentation).
- Review endpoint summaries and discovery search keywords whenever capabilities
  or routing change, including new tools; follow
  [endpoint instruction placement](docs/agent-sources.md#endpoint-instruction-placement).
- Document provider oddities, workarounds, and compatibility exemptions in
  [provider caveats](providers/CAVEATS.md) as current support boundaries and
  open uncertainty, without dated probe or sample journals.

### Logging, diagnostics, and output limits

- **Wire separation and redaction:** All logs go to stderr; stdout is reserved
  for the MCP wire or requested CLI JSON. Never persist or log
  credential-bearing upstream prose. Follow the
  [logging contract](docs/development.md#logging-and-diagnostics).
- **Severity policy:** Choose log severity by the owning capability after
  recovery (see [severity policy](docs/development.md#log-severity)):
  - `ERROR`: required service or endpoint functionality unavailable with no
    recovery remaining in the lifecycle.
  - `WARN`: terminal agent-operation failures or unexpected loss of
    functionality, including omitted selected catalog tools.
  - `INFO`: meaningful user-visible lifecycle, progress, delay, or recovery
    changes.
  - `DEBUG` and `TRACE`: routine outcomes, individual attempts, expected
    negative results, and diagnostic detail.
  Intermediate failures with retry or fallback remaining must not emit `WARN` or
  `ERROR`.
- **Plain-language diagnostics:** `INFO` and higher messages must be readable
  without source code. `WARN` and `ERROR` messages must include enough safe
  context to identify what failed, its impact, and the known cause or
  uncertainty (fields and correlation IDs supplement messages, never replace
  them); see
  [message clarity and diagnostic detail](docs/development.md#message-clarity-and-diagnostic-detail).
  Fix logging-policy violations encountered in touched code (including excess
  `INFO`, unclear messages, and inadequate failure detail), updating owning docs
  and diagnostic tests. Existing call sites are not precedent; unrelated
  repo-wide logging cleanups are not required.
- **Visible rejection and truncation:** Any limit that rejects or truncates
  content must be visible to the consumer; never present a partial document as
  complete. Visibility never justifies adding a limit; see
  [resource-limit behavior](docs/local-resource-limits.md#rejection-and-truncation).

## Independent review and planning

Use independent subagents for design challenge, audits, and implementation
review as a normal part of engineering work, without waiting for a separate
delegation request. Scale review to the change.

### Independent subagent review

- **Applies independently of plans:** Run independent review for non-trivial
  code, financial, network, provider, behavioral, or substantive documentation
  changes, including standalone fixes and single-commit work that need no plan
  file. Only trivial edits (small documentation tweaks, typo fixes, and isolated
  minor edits) skip independent review.
- **Pre-implementation design review:** Before substantive implementation or
  architectural changes, have an independent design reviewer challenge scope,
  approach, and check necessity, verify assumptions against the current
  implementation and pinned dependencies, confirm proposed behavior, tests,
  validation, and documentation agree, identify removable responsibilities, and
  test whether a simpler design suffices. Resolve material findings before
  building on that decision.
- **Fresh, non-author isolation:** Every independent review or audit must run in
  a newly spawned subagent with a fresh conversation and no prior turns or
  inherited history where supported by the agent framework. Give the subagent
  the parent's full tool, Git, MCP, and network capabilities (rather than a
  stripped-down subagent type that blocks Git history or MCP tools), and
  instruct it in its brief to perform a read-only review without modifying files
  or repository state. Never reuse the implementation agent or a prior review
  conversation, and never have a reviewer review any changes they suggest.
- **Coordinator responsibilities and delegation:** The coordinating agent
  integrates findings and owns completion. Supply a self-contained brief with
  user intent, scope, baseline, preserved invariants, owning guides, reviewed
  commit SHAs or snapshot (including new files), validation evidence, known
  gaps, and prior findings, with access to the checkout, plans, and Git history.
  Parallelize independent analysis, coordinate delegated edits with clear file
  ownership, and avoid fixed reviewer counts or duplicate checks without a
  concrete reason.
- **Exact SHAs, fixups, combined reviews, and complexity gates:** Independently
  review every committed SHA (for uncommitted snapshots, identify base SHA and
  exact contents and review the resulting SHA once committed). Address findings
  with `git commit --fixup=<introducing-SHA>`, independently review each fixup,
  and keep history unsquashed through final combined review and user review
  rounds. Before completion and at meaningful architecture gates, independently
  review the combined implementation, all accumulated commits and fixups, and
  complete affected call chains; per-commit approval never replaces this
  combined review (one review may cover both an exact SHA and the combined state
  for small single-commit work when it examines both scopes). For substantive
  Rust refactors and architecture gates, complete the CogC and CycC review in
  [complexity review discipline](tests/COMPLEXITY.md#review-discipline) and use
  hotspots to drive simplification.

### Review method and findings

- **Independent assessment:** Record an initial assessment before consulting the
  author's rationale or prior findings, then reconcile it with all prior
  findings and fixups. Trace surrounding consumers, complete call chains,
  failure paths, unchanged high-risk code (financial ownership, concurrency,
  network policy, documentation handoffs), and test coverage. Verify upstream
  and dependency assumptions against pinned code and reference checkouts
  (checking exact revisions).
- **Simplification and complexity challenge:** Look for simpler alternate
  designs, especially ones that rely on existing components, sub-components, or
  dependency guarantees rather than new state, checks, or wrappers. Flag
  over-engineering, double enforcement, collapsible behaviors, parallel state,
  fragile ownership, speculative machinery, or other awkward, overly complex
  behaviors that can be simplified or removed. Evaluate concrete simplifications
  against the contracts they must preserve without expanding into speculative
  redesign.
- **Concrete failure scenarios and audits:** Assign distinct audit questions and
  have each auditor flag scope or method gaps. Exercise failure scenarios tied
  to the changed contracts (such as concurrent agent calls, competing wallet
  state resolution, cancellation, crash and restart, blockchain reorgs, or
  offline and online transitions), tracing progress, ownership, recovery,
  forbidden side effects, and regression coverage without demanding unrelated
  scenarios.
- **Pattern-sweep prior findings across the branch:** When fixups or multiple
  commits accumulate, each subsequent review pass and the final combined review
  must trace **all prior findings and fixups** through surrounding code for
  recurring causes, shared invalid assumptions, overly complex or fragile design
  patterns that can be simplified or removed, and coverage gaps elsewhere in the
  branch. Add regression coverage for gaps exposed by fixes, and assess whether
  recurring patterns require a wider audit, simplification, or design change.
- **Documentation review:** Review substantive documentation changes (standalone
  or paired with code) for [writing style](#writing-style) compliance, factual
  accuracy against the implementation and pinned dependencies, and preservation
  of current operational procedures, error conditions, security and financial
  boundaries, and qualification limits. Check structure, complexity, scope,
  single ownership, valid local links and anchors, and audience fit
  (human-facing `README.md`, `README_ADVANCED.md`, and `docs/usage/*` vs.
  reference guides and tool descriptions).
- **Verification discipline:** Reviewers verify from the snapshot, source
  inspection, and recorded evidence; they must not re-run already-run Cargo
  builds or test suites unless explicitly asked or to confirm a suspected
  finding with the smallest focused check needed (stating what was not re-run).
- **Resolving findings and honest reporting:** For planned work, check the plan
  for intended scope, acceptance evidence, documented deviations, missing
  portions, unsupported completion claims, and unexplained departures; agreement
  with the plan does not establish correctness. Treat reviewer suggestions as
  proposals rather than automatic scope expansion. Fix material defects before
  dependent work and before declaring completion, evaluate compelling
  simplifications on merit, explain rejected or deferred items, and re-review
  when fixes invalidate earlier evidence. Report concrete findings, examined
  areas with no issues, and review limits (focused checks do not establish
  repository-wide assurance). If subagent tooling or fresh conversations are
  unavailable, state which requirement could not be met, perform the available
  checks with their scope and limits, and never claim independent review
  occurred.

### Planning

- Use a committed plan under `docs/plans/` only when the user requests one, when
  continuing an existing plan, or when staged work genuinely spans multiple
  dependent phases with distinct design decisions, commit boundaries, and review
  gates. Small changes use the independent review workflow above without a plan
  file or extra architecture gates.
- Before creating, revising, executing, or completing a plan, follow
  [docs/planning-workflow.md](docs/planning-workflow.md). It owns plan
  structure, review evidence, handoffs, and removing completed plans from the
  working tree (Git history preserves them).

## Validation and authority

### Checks and test environment

Run Rust builds and checks from the repository root via the
[Makefile entrypoints](scripts/README.md) so pinned Cargo and Cargo-managed
`protoc` are used. Zcash is always compiled in; static-wallet deployments simply
omit treasury configuration. Python is development tooling only, never a runtime
dependency.

| Make command | When to run | Scope and limits |
| --- | --- | --- |
| `make build` or `make build-dev` | Release build (`CARGO_INCREMENTAL=0`) or incremental dev build | Verifies compilation against pinned toolchain and locked dependencies. |
| `make test` | Targeted (`CARGO_ARGS=...`, `TEST_ARGS=...`) or full test runs | Runs Cargo tests through `scripts/test-runner.sh`. Check that filtered runs actually execute tests. |
| `make clippy` | Quick all-target Clippy lint check (`--all-targets -- -D warnings`) | Does not run `zcash-regtest` or cover dependency warnings. |
| `make check-clippy` | Production compiler and all-target `zcash-regtest` warning gate | Pipes builds through `scripts/check_warnings.py`, catching emitted dependency and cached warnings. |
| `make fmt-check` | Formatting check | Verifies `cargo fmt --check`. |
| `make check` | Full application qualification | Runs the ordered `check-*` stages (`check-default-remaining` after `check-compat`), including the synthetic-chain treasury lifecycle. |
| `make toolchain` | Before plain Homebrew Cargo commands | Verifies the pinned Rust toolchain from `rust-toolchain.toml`. |

- **Documentation-only edits:** Verify local links, heading anchors, and
  `git diff --check` without running Cargo suites.
- **Select checks by changed behavior:** Follow
  [selecting validation by change](docs/testing.md#selecting-validation-by-change);
  do not apply full compiler, provenance, or reproducibility qualification to
  every edit. Explicitly requested full checks and named stages keep their fixed
  selections.
- **Efficiency, ordering, and continuation:** Follow
  [testing efficiency](docs/testing.md#testing-efficiency) (reuse valid
  evidence, avoid matrices, consolidate expensive setup, and prefer functional
  assertions and deterministic synchronization) and
  [validation order and continuation](docs/testing.md#validation-order-and-continuation).
  Complete design and complexity review, compiler and Clippy stages,
  compatibility stages (`check-compat`), and cheap contract tests before broad
  runtime suites. After a localized fix, revalidate affected consumers, repeat
  passed stages whose evidence is invalidated, resume unfinished `check-*`
  stages, and record the scope and results carried forward.
- **Compiler diagnostics:** Follow
  [compiler diagnostics and review](docs/testing.md#compiler-diagnostics-and-review):
  when selected, run the production compiler and all-target Clippy warning gate
  (`make check-clippy`) and inspect diagnostics alongside exit status. Remember
  that selected-target `-D warnings` does not cover dependency warnings, and
  vendoring can expose previously suppressed upstream diagnostics.
- **Build coordination and sandbox permissions:** Coordinate editor-triggered
  Cargo checks and separate builds before validation (`Make` serialization does
  not cover external processes), and keep source, executable, feature, and
  profile evidence consistent with `docs/testing.md`. Keep socket-free tests in
  the normal sandbox; when localhost sockets are blocked, request socket-capable
  execution up front for socket fixtures and full suites instead of running once
  to provoke a permission failure (see
  [test sandbox guidance](docs/testing.md#sandbox-permissions)).
- **Opt-in features and test rigor:** Only `zcash-testutils` and `zcash-regtest`
  are opt-in test features; run feature configurations sequentially because CLI
  tests share an executable path (`target/debug/x402_treazury`), and keep
  sources and executables frozen during validation. Verify changed financial
  tests are wired in and actually run (a filtered command can pass with zero
  tests), assert forbidden side effects stay absent, and cover slow progress vs.
  true stalls, cancellation and restart, and unrelated concurrent progress when
  changing failure policy. Never loosen contracts, change toolchain or profile
  pins, weaken the reproducible-build sandbox, or trim documentation to make
  tests pass. Patch dependencies only in reviewed `vendor/` sources, never Cargo
  caches or reference checkouts.
- **Fixture limits:** Tests use temporary state, unfunded deterministic keys,
  and localhost fixtures. Consensus tests require Docker and an explicit
  ignored-test invocation. Offline fixtures never qualify live settlement or
  Tor.

### Authority limits

- **Vendoring and forking:** Obtain informed user approval before adding a
  maintained dependency checkout or fork. General upgrade or audit-cleanup
  authority is not approval to vendor; present actual exposure, mitigations,
  alternatives, and maintenance costs, then wait before implementation. Follow
  the [dependency policy](docs/development.md#dependency-vendoring).
- **Live funds and secrets:** Never load real wallet secrets in tests or inspect
  or print secret values unnecessarily. Live-funded qualification requires
  separate operator authorization.
- **Git remote and destructive actions:** Local commits do not authorize
  pushing, merging, publishing, rewriting history, or destructive changes to
  private state without user instructions. Plan or review recommendations alone
  grant none of these actions.

## Git and workspace care

- **Private and local files:** Keep `.env`, `state/`, `secrets/`, local example
  overrides, and reference checkouts untracked. Preserve unrelated tracked,
  untracked, and ignored user files; leave unexpected edits in unrelated files
  untouched and ask the user if they collide with your task. Do not routinely
  delete `target/`: it can hold private qualification evidence and pinned
  executables as well as build caches.
- **Commits and staging:** Stage only named intended files; never use
  `git add .` or `git add -A`. Follow
  [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) for
  subjects and wrap prose in subjects and bodies to 80 columns (long URLs,
  paths, or hashes do not need artificial line breaks). For changes that fix a
  substantial bug or limitation, use two body paragraphs: describe the prior
  problem in past tense, then describe the fix in present tense (see
  [Writing style](#writing-style)). Follow the `--fixup` and unsquashed-history
  policy in [Independent subagent review](#independent-subagent-review) for
  review findings on branch commits.
- **Handoff:** Report what changed, the validation and reviews performed (with
  omissions and limits), the committed revision or remaining uncommitted work,
  and material next steps.

## Documentation and writing style

### Maintain current documentation

- **Human-facing vs. reference guides:** `README.md`, `README_ADVANCED.md`, and
  `docs/usage/*` are human-facing operator documents and must stay readable
  without implementation context. Keep them task-oriented, short-paragraphed,
  and free of misplaced reference detail, unexplained jargon, internal
  identifiers, agent-directed instructions, and test or probe narratives. Every
  human-facing section serves one purpose: explain what an operator chooses, why
  it matters, and what to do next, and link to the owning reference guide for
  exact contracts and edge cases.
- **Tool rules placement:** Keep tool requirements, usage rules, and recovery
  directives in tool descriptions and agent-facing reference sections. Human
  guides describe those rules; they do not instruct the operator to satisfy them
  manually or write prompts to compensate.
- **Single ownership and in-place updates:** Maintained guides, example and
  deployment READMEs, and provider caveat files describe current behavior,
  support boundaries, and durable limits. Update the owning section in place and
  remove superseded claims. Place each rule, caveat, and detail once in its
  owning section, integrated into the surrounding structure, and link from
  elsewhere (a brief safety reminder linking to the owning section is fine at
  the point of action). Do not bolt unrelated caveats onto the end of a nearby
  paragraph, and keep test notes in verification sections rather than operator
  setup sections.
- **No journals or archives:** Keep changelogs, dated test journals, probe or
  sample narratives, "what was true before" histories, "initially", "later", and
  "retested" sequences, and retired plan phase or gate codes out of permanent
  docs. Git history and ignored `target/` or private external storage carry
  chronology and generated metrics or reports; active plans carry unfinished
  decisions and evidence, and completed plans yield maintained guidance plus
  focused unfinished work. Do not create implemented-plan archives or dated
  handoff docs. Runtime sequencing (such as "first use activates X; a later
  replacement...") and factual dates (data-as-of dates, legal dates, version or
  commit pins, and captured-evidence identifiers) remain valid where they
  explain a current fact.
- **Provider caveats and examples:** `providers/CAVEATS.md` states each
  provider's current support boundary and open uncertainty, not the probes that
  discovered it; shared contracts may use cross-provider sections. Keep
  deployment-specific selections, exclusions, and curated alternatives in the
  example guide with a brief justification linking to the owning caveat rather
  than restating the full contract in both places.
- **Financial evidence during cleanup:** Follow the
  [documentation policy](docs/README.md). Documentation or Git cleanup never
  erases financial evidence or grants replay authority: move qualification
  evidence to its owning private report before removing narrative detail from
  maintained docs.
- **Revising documentation:** Preserve current procedures, recovery steps,
  error conditions, security and financial boundaries, and qualification limits
  while removing superseded or redundant detail. Verify factual claims against
  the implementation and pinned dependencies, and cross-check and update all
  inbound links when renaming, moving, or removing headings.

### Writing style

These rules apply to all prose: documentation, plans, commit messages, PR and
issue drafts, code comments, and handoff notes.

- Write directly in plain English for human readers, and be concrete at the
  reader's level of detail. State the behavior, setting, command, or failure
  mode directly rather than using vague shorthand or invented jargon (for
  example, "the affected operation", "captured policy eligibility", or
  "historical completion authority"). Use file paths, function names, and
  implementation details in commit bodies, code comments, and maintainer docs
  where they clarify the change, without turning prose into pseudocode.
- State what a feature does rather than piling up defensive negative disclaimers
  about what it does not do or does not prove. Only use caveats that prevent a
  real misunderstanding or safety mistake.
- Keep internal process terms (qualification runs, replay branches, cleanup
  passes, or plan phases) out of user-facing docs, commit messages, and
  upstream PR and issue drafts.
- Omit filler qualifiers such as "selected", "ordinary", "applicable",
  "configured", "disposable", "harmless", "affected", "actual", "native",
  "existing", "matching", "deliberate", "explicit", "saved", "current", or
  "relevant" unless the adjective distinguishes two real cases in context.
- Do not stack nouns or modifiers into dense noun-chains (such as "managed
  wallet funding recovery qualification check" or "deferred catalog startup
  policy enforcement"); break them into clear subject-verb clauses.
- Do not pack multiple concepts into slash-joined compounds (such as
  "balance/quote/swap/claim reconciliation", "policy/credential/storage
  validation", or "refund/retry", "source/listener", "outage/recovery"). Spell
  out separate items with commas and conjunctions, or split them into bullets.
  Reserve slashes for file paths, URLs, `owner/repo` names, and standard
  protocol or identifier forms such as `HTTP(S)`, `UID/GID`, or `IPv4/IPv6`.
- Keep paragraphs short (generally 1-4 sentences, one idea per paragraph). Break
  multi-part details, options, and edge cases into headings, tables, or
  bulleted lists with clear lead-ins rather than dense walls of text.
- Always use the Oxford (serial) comma (`a, b, and c`).
- Use plain ASCII punctuation and symbols instead of Unicode special characters.
  Avoid curly quotes, em and en dashes, ellipses, arrows, multiplication signs,
  box-drawing characters, and decorative symbols; use ASCII equivalents such as
  `"`, `'`, `-`, `--`, `...`, `->`, `x`, and ASCII diagrams instead. Exceptions
  are non-English text, English loanwords that keep their diacritics (such as
  "protégé"), and verbatim quotes of external text or UI literals.
- Enforce clear tense boundaries:
  - Maintained docs, READMEs, and code comments describe **current** behavior
    in present tense. Do not narrate prior code states, how the code evolved, or
    investigation chronology (concrete upgrade or migration steps and factual
    provenance such as version pins or submission dates remain valid where they
    help readers).
  - Commit messages and PR descriptions state the prior bug or limitation in
    **past tense** and the new behavior or fix in **present tense**. Never
    describe both the old broken state and the new state in present tense.

### Target writing style examples

- Avoid: "`wallet preview` compares pure validated assignments with one
  read-only public metadata snapshot, including while serving. It grants no
  authority and does not authenticate encrypted contents."
  Prefer: "`wallet preview` compares your config's wallet assignments against
  the wallet's public metadata without modifying state or unlocking encrypted
  keys, and works while serving."
- Avoid: "Configured policy applies on restart; rename itself grants no
  allocation or funding. With no managed profiles startup skips
  `configure_profiles`, preserving saved group enablement."
  Prefer: "`wallet rename` updates the group name on disk without allocating or
  funding wallets; the server picks up the new name on restart."
- Avoid: "Ordinary USDC payment selection treats balances/price hints as
  advisory; current payer ownership and explicit amount authority govern signing
  without triggering allocation/funding traversal."
  Prefer: "Payment selection uses cached balances and price hints as estimates,
  while payer ownership and the payment cap govern signing; insufficient funds
  never trigger wallet creation or funding."
