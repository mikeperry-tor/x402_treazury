# Best-effort catalog errors and framework integration

## Intent, baseline and status

Prevent remote provider outages and catalog changes from making the hackathon
submission's research integration fail hard. Finish independent catalog work,
report unavailable capabilities accurately, and let authenticated endpoints serve
their remaining tools. Frameworks must distinguish completed preparation,
current MCP availability, catalog coverage and successful paid API execution.

This is an active implementation plan. C1/C2 are implemented and validated;
G1 and G2 are approved; C1/C2/C3 are committed and validated. C4 cleanup remains.
The user has authorized executing this plan. Live paid calls remain outside scope.

- Working branch: `flaky_provider_resiliance`.
- Treasury baseline: `d9c8b8a3d2df281fcfb5b23725fc392e080ca3cc`.
- Consumer checkout: `reference_repos/private-hermes-agent`, baseline
  `3fb188167125ba0874a6ff89353d5060a4c474bf`.
- Its Dockerfile currently pins treasury
  `7ee0f5e89b5413b8ae55e18ee939c9d26df3652d`; the consumer checkout and packaged
  treasury are different baselines. Inspect the exact pin before implementation.
- Preserve unrelated untracked `kilo.json` and `state`, other operator files,
  wallet state and private financial evidence. Do not update reference checkouts
  or dependency forks as a side effect.

## Ownership and maintained guidance

Create **`docs/framework-integration.md`** during implementation. It will own the
public contract for CLI exit status, warming JSON, degraded serving, diagnostics,
framework preparation and MCP activation when this plan is completed. The contract
below is its proposed content, not a second permanent specification.

Keep configuration syntax, defaults, selectors and explicit direct warming in
[configuration](../configuration.md), and acquisition/publication/ownership details
in [architecture](../architecture.md#catalog-configuration-and-execution). Link
those sections to the integration guide instead of copying the framework contract.
Add the guide to [the documentation index](../README.md) and link it from the
startup/catalog guidance in [AGENTS.md](../../AGENTS.md). Update
[agent sources](../agent-sources.md) for affected endpoint instructions, omissions
and search behavior; [testing](../testing.md) and [startup tests](../../tests/STARTUP.md)
own validation boundaries. Update [example guidance](../../examples/README.md)
for the research desk's selected availability policy.

Read those guides and [network policy](../network-egress.md),
[local resource limits](../local-resource-limits.md),
[development diagnostics](../development.md#logging-and-diagnostics) and
[wallet recovery/observation](../wallet-rotation.md) before changing their behavior.
Before consumer edits, read its AGENTS.md, README, architecture, development,
configuration, x402_treazury guide and planning workflow, including relevant plans.

## Current behavior and affected consumers

- `deployment::startup::warm` propagates the first source error and drops unfinished
  buffered loads. It builds with inspection strictness; unsupported selected
  operations fail warming. JSON is printed only on success. Accepted completed
  cache writes survive another source's failure.
- Ordinary deployment serving defers remote catalogs until after binding sockets.
  Default `wait_for_catalogs = true` endpoints close on required source failure;
  peers and the process remain alive, even if every waiting endpoint fails.
  `allow_catalog_failure = true` permits failed-source omission; eager endpoints already
  omit failed remote sources.
- Removed exact provider inclusions/recommendations or `send_tools` names can reject
  an entire source. Removed exact endpoint inclusions can clear its entire static
  selection. Existing unavailable-prefix handling does not cover a missing name
  in an otherwise successfully loaded remote source.
- Optional pricing and cache persistence already have best-effort behavior. An
  unavailable API still advertised by a usable catalog is generally a call-time
  failure, not a startup health check.
- Hermes preparation treats native nonzero exits as failed phases. Its activation
  requires initialize/list on all twelve endpoints and latches two terminal native
  endpoint diagnostic phrases. It does not interpret every ERROR as terminal.
  Its stopped-Tor warm smoke fixture currently expects a nonzero exit.

Relevant paths: `src/deployment/{cache_warm,startup,deferred,readiness}.rs`,
`src/deployment/routing.rs`, `src/deployment.rs`, `src/catalog.rs`,
`src/catalog/diagnostics.rs`, `src/main.rs`, `src/pricing/discovery.rs`, and the
consumer's `components/x402_treazury/{onboarding,render-config}.py`,
`scripts/x402_treazury.py`, image pin and readiness/smoke tests.

## Proposed public contract

### Warming completion, exit status and JSON

`catalog warm` is best-effort acquisition, not strict inspection or qualification.
After valid preflight, consume every independent selected source result. A source
failure must not drop sibling work. Retain existing configured attempts, relay
ordering, capacity ownership and cancellation/draining behavior; add no retries,
whole-work deadline, minimum coverage requirement or network fallback.

| Outcome | Exit status | JSON and diagnostics |
| --- | --- | --- |
| Every selected acquisition and selected operation is usable | 0 | `outcome = "ok"` |
| Attempts finish with unavailable sources, omitted operations or other reported degradation | 0 | `outcome = "degraded"`, affected results and ERROR diagnostics |
| Command cannot execute validly or finish its reporting contract | Nonzero | Safe terminal diagnostic; no completed-success summary |

Zero means the requested warming attempts finished. It does not promise cache
persistence, complete coverage, working help/API endpoints, sufficient funds or
future availability. All remote sources may fail and still produce a completed,
degraded zero exit. Do not add an alternate nonzero code for degradation or a
new strict warming flag; strict inspection already exists.

Preflight remains fatal for malformed TOML/selector syntax, unknown selected source
IDs, invalid local provider configuration or local catalog inputs, selected cache
opt-outs, invalid network configuration, missing required state and unavailable
required wallet/authority initialization. Cancellation, internal worker failure or
inability to write the command's final JSON is not completed warming. Preflight
locally detectable faults before outbound work where practical; a fault found
during execution must not be disguised as a provider outage. Do not classify fault
ownership merely from the presence of a remote spec URL: the existing
`RemoteCatalogUnavailable` wrapper also contains local transport/policy failures.
Define and test this boundary using existing error types before adding new types.

Once source acquisition is validly attempted, exhausted DNS/connect/TLS/HTTP errors,
incomplete or rejected bodies, invalid remote JSON/catalogs, remote generation
failures and remote inventory drift become reported source outcomes. Retain
destination enforcement even when rejection is reported as source unavailability;
never relax validation to produce a usable result. Partial compatible catalogs may
be accepted under the serving generation contract, with their omissions visible.
Strict inspection/qualification still reject partial results. Invalid replacement
documents must not replace or renew accepted cache state.

JSON remains one document on stdout; logs stay on stderr. Preserve `direct` and
`sources`. Return exactly one source result for every selected authored ID in
deterministic order, with this intended shape:

```json
{
  "direct": false,
  "outcome": "degraded",
  "sources": [
    {"source": "healthy", "status": "ready", "catalog_cache": "fresh"},
    {
      "source": "offline",
      "status": "unavailable",
      "failure": {
        "stage": "headers",
        "reason": "connection_failed",
        "message": "Could not connect to the catalog server or configured proxy"
      }
    }
  ]
}
```

Source statuses are `ready`, `partial` (usable tools with reported omissions) and
`unavailable`. Use stable machine-readable failure reasons and stage labels,
authored safe messages and optional numeric HTTP status; messages are for display,
not branching. For partial results expose omission identities/reasons using the
existing safe catalog omission representation. Include missing-selection notices
without upstream prose. Finalize the exact fields and reason vocabulary at D1,
including optional pricing/cache degradation, and publish them with C1. Retain
the existing cache-status vocabulary separately from acquisition status; absence
of storage is not proof acquisition failed. Do not turn cache write refusal into
a new fatal prerequisite. Report pricing failure as unknown pricing, preserving
usable definitions. Avoid a new persistent result ledger or duplicate coverage
inventory simply to build this summary.

### Serving and remote inventory drift

Keep global `wait_for_catalogs` and `allow_catalog_failure` meanings. Set
`allow_catalog_failure = true` on remote research-desk sources, retaining waiting startup
so endpoints publish the settled available inventory after their source attempts.
Required-source configurations elsewhere retain their explicit strict readiness
choice. Frameworks seeking this availability policy must select it explicitly.

Ordinary serving and warming tolerate missing exact inclusions, opted-in
recommendations and distribution selectors when attributable to a known remote
source. Keep the existing compatible selected siblings. Preserve authored filters,
method preference, tags, routing, wallet assignments and recommendation opt-in:
a missing inclusion never falls back to unrestricted exposure. Missing endpoint
selectors must not clear healthy static selection. Source-wide invalidity still
omits that source as a unit.

Malformed selectors, unknown namespaces, ambiguous attribution and genuine local
configuration errors remain strict. Define attribution with resolved prefixes and
effective source bindings, including shared prefixes and local/remote overlap;
do not hide a ready local-source typo behind an unavailable remote source.
Remote typo versus upstream removal is inherently ambiguous: disclose the missing
selection and retain strict inspection as the operator's validation path.

An otherwise valid endpoint emptied by remote outages/removals must still support
authenticated MCP initialize/list and explain missing capabilities. Distinguish
that result from intentionally invalid empty local configuration. Preserve enabled
management/discovery capabilities and endpoint-local instructions/search semantics.
Neither successful MCP initialization nor process liveness proves full coverage.

Terminal source degradation uses ERROR with affected source, safe stage/reason and
impact. It must not use messages claiming the endpoint itself cannot serve if its
remaining or empty valid inventory is available. Reserve terminal endpoint
diagnostics for actual endpoint unavailability. Intermediate attempts with recovery
remaining retain DEBUG/TRACE; avoid duplicate terminal WARN/ERROR reports. Keep
publication-owner failure separate from an individual provider result.

### Contract for Hermes and other frameworks

1. Run warming once under the existing authorized ownership and network policy.
   Zero with `ok` or `degraded` completes the catalog-preparation phase. Display the
   report; do not require source/tool counts, all-ready statuses or fresh cache
   entries. Invalid JSON is a protocol failure, not successful preparation.
2. Preserve financial bootstrap as its own phase. Catalog degradation does not
   invalidate wallet identity, erase successful bootstrap or reopen funding claims.
   Existing preparation records remain historical completion, not current health.
3. Start serving and probe authenticated MCP initialize/list on configured endpoints.
   Hermes retains all twelve probes and registration as one activation operation;
   every valid endpoint can respond even when coverage is degraded. Do not call
   paid APIs as readiness checks. Genuine auth/listener/protocol failures still fail
   the affected activation; Hermes itself remains available.
4. Show source ERROR diagnostics without treating log severity as a process or
   activation result. Prefer native command status and actual MCP availability.
   While Hermes retains its existing terminal-message observer, update producer
   and consumer together so recoverable degradation cannot latch a fatal result.
   Do not introduce a generic event protocol solely to replace two message checks.
5. A failed, interrupted or missing result grants no replay authority. Do not
   automatically rerun bootstrap or potentially paid warming, use direct egress,
   rotate wallets, delete caches/state or loosen payment limits to recover.
   Preserve graceful accepted-work draining and native recovery on the same state.
6. Pin/build a treasury revision implementing this contract. No compatibility shim,
   version-detection branch, parallel readiness ledger or new completion marker is
   needed for this prerelease consumer. Update documentation and tests together.

The framework contract applies equally to other integrations; Hermes's twelve
endpoints, preparation files and container commands are consumer-specific details.

## Preserved boundaries and exclusions

No financial authority, funding policy, accepted-input ownership, exact signed bytes,
Ironwood restrictions, network factory ownership, Tor isolation/remote DNS or
no-direct-fallback behavior changes. No new maintained dependency checkout, fork,
runtime dependency, queue cap, timer or admission gate. Preserve scope retirement,
cache provenance/acceptance and captured-call completion. Registration does not
allocate/fund wallets. Do not use live funded work as implementation validation.

Strict config/tool/tag inspection, frozen qualification and separately authorized
unsigned qualification retain completeness requirements. Standalone single-provider
serving is outside this deployment integration change; document its stricter
startup boundary rather than silently changing it. Dynamic registration failure,
API-call replay and automatic source recovery are outside scope. Failed sources
retain current restart-to-retry behavior; progress may legitimately keep an attempt
pending without an overall deadline.

## Commit stages and review gates

| ID | Coherent implementation boundary | Acceptance and documentation |
| --- | --- | --- |
| D1 | Independently review this design against both exact baselines and the packaged treasury pin. Finalize failure classification, safe JSON fields, pricing/cache reporting and remote-selector attribution before code. | Resolve material findings; commit plan decisions. No new enforcement without a demonstrated contract. |
| C1/C2 | Deliver one coherent native contract: drain warming with zero/degraded JSON and partial-compatible generation; implement narrow remote-selection tolerance, valid empty degraded endpoints and optional research-desk sources. Preserve preflight, cache and accepted-work boundaries. | Multi-source, JSON/exit, cache/cancellation and source/distribution/listener regressions plus authenticated readiness and strict-mode checks. Publish `docs/framework-integration.md` and update configuration, architecture, agent-source, example, startup-test, index and AGENTS guidance. |
| G1 | Independent combined native architecture review of C1/C2 and all fixups, exact SHAs and full acquisition-to-publication/serving call chains. | Compiler/Clippy/compatibility evidence and CogC/CycC review complete; no material findings before consumer integration. |
| C3 | Update private-hermes-agent's image pin, warming display/preparation handling, terminal diagnostic consumer, readiness/smoke expectations and owning docs. | Consumer tests prove degraded preparation activates all twelve endpoints without paid API probes; update stopped-Tor smoke assertions to degraded JSON, preserving forced acquisition. |
| G2 | Independent final combined review of native and consumer exact SHAs, accumulated fixups, complete affected chains and contract documentation. | Focused native and consumer evidence matches the packaged pin; resolve material findings and compelling simplifications. |
| C4 | After implementation gates and user review rounds, remove this completed plan and repair references. | Contract resides in permanent guides; preserve unsquashed plan/review history; independently review exact cleanup SHA and final combined state. |

Consumer changes belong to its repository with explicit file ownership and local
commits; do not accidentally commit its contents into this repository. Verify the
reference-checkout edit policy and execution authorization before C3, using an
authorized writable working checkout if necessary. Record both repository SHAs
and the final packaged treasury pin. Publication/push/merge needs separate authority.

## Validation and concrete failure scenarios

Follow [validation selection and continuation](../testing.md#validation-order-and-continuation)
and the Make entrypoints. Freeze source/executable/features during checks and avoid
competing Cargo/editor builds. First run affected cheap socket-free contracts;
complete applicable design/complexity review, production compiler, all-target
Clippy warning gate and compatibility stages before broad runtime suites. Select
remaining checks by affected behavior, recording exact commands, nonzero test
counts and diagnostics. Request socket-capable execution up front for localhost
fixtures; do not deliberately provoke sandbox failures. Full compiler/provenance or
reproducible-build qualification is unnecessary for this plan-only commit.

Required implementation scenarios:

- One terminal remote failure alongside a gated healthy load: release it and prove
  its accepted cache and tools complete, with every selected result reported.
  Cover DNS/connect failure through deterministic transport fixtures and representative
  HTTP, body, JSON and generation failures without duplicating a provider matrix.
- All remote failures return degraded zero and permit valid empty MCP endpoints;
  malformed local configuration, unknown selected source and cancellation remain
  nonzero. No failed source is mislabeled ready or silently missing from JSON.
- One incompatible operation with healthy siblings yields a partial result; fully
  unusable/invalid remote content is unavailable. Reject invalid replacements
  without replacing/renewing prior accepted bytes, prices or provenance.
- Remove exact names from provider filters, active recommendations, `send_tools`
  and endpoint inclusions. Preserve healthy allowed tools and routing, including
  a remote-emptied endpoint. Assert excluded/unselected tools never appear. Cover
  shared prefixes, unknown namespaces, local/remote overlap, opt-in recommendation
  semantics and continued strict inspection/qualification rejection.
- Optional pricing failure and cache persistence refusal retain usable definitions
  and truthful report fields. Cache absence must not become an activation gate.
- With one source slow but progressing, unrelated tools/loads continue; no whole-work
  timeout appears. Cancellation/restart preserves accepted financial work and
  native liability ownership, with no automatic replay or new funding allocation.
  Use existing payment/relay fixtures for the affected boundaries rather than a
  broad financial rewrite or live funds.
- Source ERROR messages do not match terminal endpoint diagnostics. Actual
  auth/listener/publication failures remain visible and properly scoped. No
  credential-bearing prose leaks into logs/JSON; stdout contains only the summary.
- Consumer tests exercise preparation, degradation display and actual all-twelve
  initialize/list activation with native degraded serving; process replacement,
  observer failure and genuine endpoint failure retain visible failure behavior.
  Preserve bootstrap completion/identity and unrelated MCP registrations. Verify
  the new pinned image rather than substituting a different local executable.

Use small synthetic catalogs and deterministic synchronization. Update
`tests/x402_treazury_smoke.py` and `tests/test_endpoint_readiness.py` in the consumer
and add the missing native multi-source drain regression. No external-provider
probe, funded qualification, real-Tor or cross-platform qualification follows from
offline/localhost results; report unavailable integration checks as missing evidence.

For substantive Rust changes and G1/G2, perform the baseline/final CogC/CycC review
in [COMPLEXITY.md](../../tests/COMPLEXITY.md#review-discipline). Review full affected
chains and existing hotspots, not just changed function scores. Store generated
reports under ignored `target/`; record warning dispositions in this active plan.

## Review evidence and handoff

Independent reviewers are read-only non-authors, starting fresh without inherited
conversation history. Supply exact SHAs/snapshots, required contracts, relevant
consumer/pin, validation failures and limits. Ask for an initial assessment before
author rationale, then reconcile prior findings/fixups. Independently review every
exact committed SHA and each `git commit --fixup=<introducing-SHA>` correction;
preserve unsquashed history. Combined gates examine all accumulated changes and
consumers; per-commit review does not replace them.

Current status: D1 and G1 approved; C1/C2 committed and validated. C3 is
committed in a writable consumer worktree with packaged acceptance; G2 remains
pending. Independent fresh-context,
read-only design review examined this plan against native acquisition, diagnostics,
selection, routing, readiness and pricing, plus consumer preparation, activation,
smoke expectations and the packaged pin. No material findings were reported.
The initial plan review approved exact commit `5a006dbb`. D1 used separate
fresh-context reviewers for warming/ownership and serving/consumer attribution.
Resolved decisions:

- Introduce an explicit Warming purpose; accept compatible partial generation
  without starting Serving's detached pricing and then fetching pricing twice.
- Keep acquisition statuses `ready`, `partial`, `unavailable`; keep pricing separate
  as `not_configured`, `catalog`, `ready`, `unavailable`, `not_attempted`. Pricing ready means valid
  metadata was processed, not that every tool has a known price. Cache refusal or
  unknown external pricing degrades the summary without disabling API tools.
- Preserve typed local acquisition failure at the catalog handoff; request-builder
  and transport-construction faults stay fatal, while download-limit rejection is
  a source outcome even though it cannot authorize paid fallback. Drain sibling
  results before returning a late local failure. Invalid/all-unsupported remote
  replacement does not renew accepted bytes, prices or expiry.
- Keep missing inclusion/recommendation patterns compiled. New endpoint-removal
  tolerance requires exactly one effective resolved-prefix owner, remote and in
  ordinary Serving/Warming; inspect nested prefixes and local overlaps. Preserve
  the existing all-unavailable-owner exception separately. Empty endpoints require
  attributable degradation, never mere presence of a remote source.
- Existing Hermes configurations remain operator-owned: changing the native example
  affects newly seeded configurations only. Document explicit optional-source
  opt-in for preserved configurations. Keep all twelve authenticated probes.
- D1 found warming lacked bootstrap-style retained shutdown ownership. Resolve this
  at C1 by reusing the existing cancellation token, retained supervisor and shutdown
  signal/drain pattern, including relay drain before initialized-wallet shutdown.
  This is necessary accepted-work ownership, not new retry or spending authority.

Local file-link and whitespace checks passed for the plan.
The C1/C2 commit boundary is combined: the shared generation policy, source
selection notices, empty-source admission and warming JSON depend on one another.
Publishing one native contract avoids an intermediate commit whose documented
warming behavior still rejects remote distribution drift. G1 still precedes C3;
scope and acceptance are unchanged.

The user requested a clearer name during implementation. The source setting is
now `allow_catalog_failure`, retaining its false default and readiness semantics;
the old `optional_startup` key is rejected, with no compatibility alias. This is
a configuration rename, not a wallet/state/environment migration. Apply the new
name to consumer guidance and preserved operator configuration explicitly.

Initial independent implementation reviews found empty-selection local errors,
lost selection notices, misleading mixed-cause instructions, unavailable cache
replacement, and local pricing/cache reporting defects. Corrections retain
compiled patterns and accepted cache state, report separate pricing/storage
results, and preserve unavailable-source evidence. The reviewer retracted a
duplicate deferred logging concern after tracing the actual pending/resume path.
Complexity review identified duplicated empty-source cause classification;
endpoint instructions now reuse the existing omission/selection evidence with
one generic empty-source status. The source-matched report is
`target/complexity-runs/20261114T175803Z-072aed27` (512 files, zero exclusions).
Review dispositions retain the distinct publication/selection states in
`build_servers` (CogC 22→31), conservative attribution in
`listener_tool_inventory` (CogC 21→35, CycC 34→43), and pricing generation
ownership in `start` (CogC 24→25). The retained warming supervisor adds
cancellation/draining branches (inclusive CycC 31→42); G1 reviews this ownership
with the full call chain rather than extracting branches merely to move scores.

Validation uses `CARGO_TARGET_DIR=target/improved-errors-build` after a sampled
compiler stack showed no progress reading the existing `target/debug/deps`
directory. Only this task's stalled compiler was terminated; existing artifacts
were preserved. Production compilation and all-target Clippy passed in the isolated directory.
`make fmt-check` passed. Selected passing evidence: 8 remote-selection, 3 catalog
recommendation, 11 warming, 16 deferred-readiness, 21 routing, 7 refresh and 18
pricing unit tests; CLI contracts 27, configuration 17 and startup 19 (one
subprocess helper ignored); fixed `make check-compat` 33 tests. Logs are under
`target/improved-errors-*.log`. The macOS test linker reported an oversized
exception-unwind section; test execution passed. No production/Clippy diagnostics
were suppressed.

Validation exposed two stale baseline research-desk routing assertions. Isolated
fixups `ed89bcf76a4e5dfd16883603f045257752a5b0c6` and
`bdcb946b963a90632c10e7ca2f2275d7414c86b8` correct their expected curation
without changing routing; both exact SHAs were independently approved. Semantic
selection preflight belongs at warm/build boundaries after `--all-tools`, not
general offline configuration inspection. Config show and all-tools behavior are
preserved. Remote name tolerance is scoped to warming and deferred ordinary
serving: synchronous unsigned loading/qualification retains strict authored name
selection. The selected startup/CLI checks cover these boundaries. Earlier failed compiler
attempts exposed a private method and an iterator-future lifetime error, corrected
before continuation. No live qualification is claimed. Record exact-SHA
review and subsequent validation evidence here. At each
handoff report branch, both relevant revisions, outstanding work and missing
validation. Keep the plan until implementation, required reviews and user review
rounds finish; then transfer enduring rules to the named guides and execute C4.


G1 independently approved exact native
`3125dcacca10aee1c988a9ffa088f802025ee77e` and combined baseline-to-SHA including
all accumulated plan commits and both routing fixups, with no material findings.
The reviewer verified final source hashes and retained the warming supervisor's
CycC 42 (own 31): cancellation, accepted-relay drain and wallet shutdown have
distinct result ownership; extracting them would only move that choreography.
All other complexity dispositions above were accepted. No additional financial,
network or resource authority was found.

C3 uses writable existing-repository worktree
`/private/tmp/private-hermes-improved-error-handling`, branch
`codex/improved-error-handling`, preserving the reference checkout. Consumer
onboarding accepts the structured native completion envelope, including degraded
and all-unavailable results, without new count/cache/price gates or persistent
coverage markers. Focused consumer tests passed: onboarding 35, endpoint readiness
12, renderer 4. The packaged fixture adds networkless loopback TLS catalogs and
passed actual all-twelve activation, empty filtered endpoints, source diagnostics
and absence of API calls. Its unique image tag preserves existing local images.
The build uses the consumer Dockerfile's exact native SHA assertion and unchanged
build recipe, substituting a local Git bundle only for unpublished-source
transport. Ordinary upstream image fetch requires publishing the native commit;
this task does not authorize publication. Podman is unavailable, so no native
Podman coverage is claimed. G2 remains pending.


C3 commit `30d74b8` pins native `3125dcacca10aee1c988a9ffa088f802025ee77e`.
The frozen consumer passed all 172 Python tests after standard disposable worktree
initialization and socket-capable execution, effective-model validation,
`compileall`, local file links and whitespace checks. The reviewed-source image
built successfully with the unchanged production recipe and exact native SHA
assertion; the frozen onboarding source was included by a final cached rebuild.
`target/hermes-c3-endpoints-final.log` records all five networkless container
scenarios passing: cancelled pending probe cleanup, delayed startup, genuine
endpoint failure with healthy peer, corrected restart, and degraded catalogs with
all twelve authenticated endpoints (one healthy tool, eleven empty inventories).
Only the three authored catalog paths were requested: no API/payment probe occurred.
The new fixture first exposed two fixture authoring errors (using operationId
instead of native path-based names, and presenting a CA as a server certificate).
It now uses correct names and a separately signed serverAuth/SAN leaf, preserving
TLS verification. Those corrections require final exact-commit review.

The stopped-Tor smoke assertion was updated to degraded zero/JSON and preserved
forced-acquisition/cache assertions, but that full Tor stack scenario was not run.
No live Tor, paid provider, financial settlement, or native Podman qualification
is claimed; these are explicit coverage limits, not passed checks. Final G2 will
review both exact branches, all native fixups, permanent contract ownership and
consumer pin before completed-plan cleanup.


G2 independently approved exact native
`967647d5425f4d072691e79fd2ca7f4476402ca4` with all accumulated baseline changes,
plan commits and unsquashed fixups, and consumer
`30d74b8ab5ee1a797352967788c7beeecb5157b9` including native pin `3125dcac`.
The fresh read-only reviewer traced the complete affected native/consumer chains,
verified every complexity source hash and inspected the recorded compiler,
Clippy, focused native, 172 consumer and five container results. No material
findings or compelling simplifications remain. No tests were rerun by reviewers.
The explicit coverage limits and source-bundle transport difference above remain;
no untested qualification or publication is claimed. Enduring rules reside in
`docs/framework-integration.md`, configuration, architecture and consumer guides,
with the documentation index and AGENTS link updated. C4 now removes this plan,
retaining all evidence in unsquashed Git ancestry, followed by independent exact
cleanup-SHA and final combined review.
