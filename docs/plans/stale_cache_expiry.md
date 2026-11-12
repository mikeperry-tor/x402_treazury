# Stale cache expiry

## Intent and baseline

Keep catalog snapshots usable indefinitely by default, and expire/re-download only
the affected source when a concrete operation-removed or schema-changed signal
appears. Preserve the original call outcome and never replay an API call. Catalog
and pricing hints share one lifetime: expiring or invalidating a catalog also
discards its advertised estimates and cached challenge-derived price data, and
re-acquires any configured pricing hint document before wallet selection. A live
challenge mismatch updates the observed price; it does not independently evict
pricing data.

Decision baseline: `a6630bfc`; inspect intervening changes before starting. Source
baseline for the original investigation:
`4476090b6d43d7016f1d1ee36eb2040d9910b5cd`. The shared cache contract lives in
[configuration](../configuration.md#automatic-discovery-disk-cache),
[architecture](../architecture.md#wallet-selection-and-network-isolation) and the
[resource inventory](../local-resource-limits.md).

Current `payment::HttpFailure` and `payment::diagnostics::ProviderFailure` consumers
recognize settlement failure, invalid arguments, temporary unavailability and
provider data-refresh failure. None establishes that an operation was removed or
its schema changed. Dynamic-source refresh is explicit maintenance.

## Evaluated indicators

The 2026-11-11 public sweeps captured 69 of 70 bundled providers; `glassnode` help
remained unavailable at capture time. `ax1` and `coinmarketcap` became available
after final-URL and identifying-User-Agent fixes. `agproxy` and `curl` were
semantically unchanged; `horizonpulse` produced the only real vendor-copy candidate.

| Indicator | Evidence | Decision |
| --- | --- | --- |
| Operation identity drift (added/removed method/path, generated-name change, GET/POST pair change) | Provider-update projection; only Horizon Pulse showed a real candidate | **High-specificity hard trigger** for explicit maintenance or refresh validation |
| Override and parameter-note target drift | Provider-update override diagnostics | **High-specificity hard trigger** when an authored target no longer resolves |
| Explicit operation removal (`410 Gone` or a structured provider removal response) on a previously published operation | Runtime operation outcome | **High-specificity hard trigger** |
| A `404`/`405` final response from a provider | Runtime operation outcomes after the request's existing retry/relay path completes | **One observation triggers.** The provider had its configured chances; no cross-call counter or strike state is kept |
| Response validators (`ETag`, `Last-Modified`, `Date`) | Inconsistent vendor support already observed | **Rejected as a trigger.** No automatic conditional revalidation; manual warm is unconditional |
| Source generation metadata (`info.version`, `openapi`, vendor timestamps) | `2s` `generatedAt` and `brazilayer` `lastUpdated` churn without structural change | **Rejected as a trigger** |
| Pricing relocation/shape change | Advisory pricing metadata moved without catalog changes | **Rejected as a catalog trigger.** Update pricing metadata deliberately; never expire the catalog |
| Cache age, `disk_hit`, `revalidate`, `revalidated`, `requires_revalidation`, `InlineRefreshFailed` | Cache mechanics, not contract evidence | **Rejected as a trigger.** No timer, probe or periodic scheduler |
| Live challenge shape drift | The challenge is authoritative for the current payment | **Not a cache-expiry trigger.** Update the observed price; do not invalidate the catalog |
| Generic HTTP `400` or `422` | Cannot distinguish removal, routing, arguments or provider policy | **Rejected** |

The same-day comparison observed 49 no-change providers, seven authored no-change
providers, three semantically unchanged vendor assets, five authored baselines, one
unavailable source, and reviewable differences limited to local docs, timestamp
churn, live directory statistics, a dynamic guide re-render and an array reorder
with no slug or field change. Those are the false-positive cases this decision
avoids.

## Decisions

### Indefinite catalog and pricing lifetime

- `catalog_cache_ttl_seconds` becomes **indefinite by default**. Preserve `0` as
  "disk caching disabled" and retain an explicit positive-seconds override for
  operators. Use a distinct lifetime representation rather than overloading `0`.
- Catalog and pricing share the entry, lifetime and invalidation. There is no
  independent pricing TTL and no independent eviction from a challenge mismatch.
- A catalog refresh/expiry discards its generation-bound advertised estimates and
  cached challenge-derived price data for the affected source. Accepted in-flight
  work keeps the offer it already validated.
- On expiry/invalidation, re-download the configured pricing hint document when
  the provider declares one, so wallet selection has fresh initial hints before
  any new challenge. If no hint is configured, selection uses unknown pricing until
  a challenge is observed.
- A live challenge updates the observed price for that operation. It does not
  expire the catalog.
- Add the setting and an operator comment to the example deployments so the
  indefinite default is visible and overridable. Changing the default changes the
  cache key; document the one-time miss/re-download after upgrade.

### Hard expiry only, per source

A source becomes eligible for expiry only when one of these holds:

1. One high-specificity signal: operation identity drift, an unresolved authored
   override target, or an explicit operation removal such as `410`.
2. One `404`/`405` final response from any previously published operation of the
   provider. The request's existing retry and relay policy has already run; this
   plan adds no extra 404/405 retry, probe or multilateral vantage-point rotation.
   The trigger is per source, not per operation, so a failure on any of its calls
   can expire the shared catalog.

Current transport policy does not include `404`/`405` in the Tor fresh-origin retry
set or runtime relay blocking statuses. This plan does not change that network
policy: it observes the final outcome the caller already receives. Adding
404/405 retries or paid-relay handoff is a separate decision with its own request
and fee exposure.

Do not use validator, generation, pricing-relocation, age, cache-status, generic
`400/422` or challenge-drift signals. No soft revalidation path, timer, probe or
periodic scheduler is added.

Provider-update operation drift is offline maintenance evidence. It does not
automatically signal a running server in this work; it drives an explicit
`catalog warm --force` or provider edit. Cross-process stale markers are out of
scope.

### Refresh behavior and agent messaging

- Expire only the affected source, then coalesce through the existing catalog
  acquisition/publication path. Never refresh unrelated sources, duplicate an
  active refresh, allocate wallets, replay a request, sign against the expired
  catalog or fund during the refresh.
- Mark the source as refreshing; calls that would use it fail fast. Do not hold the
  call open and do not automatically retry it.
- For now, synthetic refresh messaging belongs only to dynamic discovery endpoints.
  A dynamic search/call may report that the provider's cached definitions are being
  refreshed and that the caller should retry later. Static tool calls receive the
  actual catalog/upstream error; no permanent synthetic message is introduced.
- After refresh completes or fails, return the actual result/error. If the
  operation is still absent, do not keep reporting refresh. A later independent
  trigger may try again.
- Accepted financial work keeps its original catalog and ownership. Cancellation
  or refresh failure must not turn accepted work into success or trigger a replay.
- Re-download pricing hints as part of the same source refresh and clear
  generation-bound advertised/cached-challenge estimates only for that source.

### Tool publication and client visibility

Refreshing a live snapshot is not sufficient for static clients that cache tool
definitions. `rmcp` 3.9 supports `notifications/tools/list_changed`, but client
support is not guaranteed and must not become a correctness dependency.

- Dynamic discovery endpoints remain the supported path for seeing refreshed
  definitions.
- A best-effort `tools/list_changed` notification may be emitted after a successful
  publication if the available peer/session path supports it. Verify current rmcp
  behavior and add compatibility tests; do not add a peer registry solely for
  notifications and do not assume clients re-list.
- Static clients may require reconnect/reload after a schema-changing refresh.
  Document this limitation rather than promising that a retry always picks up new
  definitions.

### Manual warm versus serving

- `catalog warm` must force an unconditional re-download of every selected catalog
  and its configured pricing hint, even when a fresh cache entry exists and even
  when validators are present. Validator-based `304` reuse is not a manual refresh.
- `serve` and normal startup must never force. With the indefinite default they
  reuse the disk entry until a hard trigger or explicit warm changes it.
- A warm process updates the shared SQLite cache, not a running server's in-memory
  snapshot. Applying a forced warm to an already-serving process requires the
  runtime refresh path or a restart; document this boundary.
- Failed refreshes retain the previous complete snapshot; they do not publish a
  partial catalog or destroy the old entry.

## Commit boundaries

### SC1 — Indefinite lifetime and forced warm

Introduce the indefinite default while preserving `0` and positive overrides. Make
`catalog warm` unconditionally refresh selected catalogs and pricing hints, and keep
`serve` on the normal freshness path. Bind pricing invalidation to catalog
invalidation. Update example deployment comments, configuration/architecture
documentation and cache-status reporting. Tests cover indefinite reuse across
restart, forced warm's one unconditional fetch, pricing re-download, `serve` not
forcing, failed-refresh retention, cache-key migration and direct-warm provenance.

### SC2 — Per-source hard expiry and refresh gate

Add operation-specific `410` and single-observation provider-level `404`/`405` detection, source-scoped
coalesced refresh, fail-fast behavior, dynamic-discovery retry messaging and actual
post-refresh errors. Add optional best-effort tool-list notification only after
compatibility tests. Do not add soft revalidation, timers, probes or a scheduler.
Tests cover single-observation `404`/`405` triggers, per-source rather than
per-operation scope, no added retries or probes, source isolation, no duplicate refresh, no signing/funding/replay, preserved
accepted work, refresh failure, operation still absent, dynamic messaging, static
actual errors and notification compatibility.

### SC3 — Documentation handoff and cleanup

Verify the permanent cache, refresh, pricing and client-visibility contracts in
their owning guides, update the index and remove this plan without an archive.
Independently review that exact cleanup SHA and the final combined branch.

## Review gates

Before SC1, an independent read-only design review must challenge the indefinite
default, the forced-warm semantics, the pricing/catalog coupling and the
client-visibility limitation. Independently review each exact commit SHA and each
introducing-SHA fixup, preserving unsquashed history. The final integration gate
must examine all commits and fixups with complete acquisition, binding, observation,
description/selection, challenge and journal/signing call chains, including aliases,
source refresh, relay consumers and direct-warm provenance. Resolve recurring
causes and coverage gaps; compare source-matched CogC/CycC against the starting
baseline and review relevant unchanged hotspots.

Follow [AGENTS.md](../../AGENTS.md) and the
[validation order](../testing.md#validation-order-and-continuation). This plan
authorizes no live paid calls.

## Implementation status and review evidence

Working branch: `stale_cache_expiry`. Implementation starting baseline:
`837ea6a6ada7f0d87541f01b4146012d173ea4af`. The plan was already committed
before implementation (`6cb2e2ea`); intervening changes from the decision baseline
adjusted this plan, example configuration and the unsigned discovery header profile.
Unrelated local files are preserved. No live paid calls are authorized or used.

Independent fresh-conversation design review examined the starting SHA and
acquisition, pricing, static/dynamic publication and payment consumers. A distinct
runtime design analysis examined refresh ownership and final response attribution.
Material findings accepted before implementation:

- Represent indefinite explicitly and avoid translating its persistence sentinel
  into a platform clock deadline.
- Stage forced cache replacement through complete source generation and routing
  validation; failed downloads and invalid definitions retain old complete bytes.
- Preserve optional asynchronous startup pricing. Runtime refresh awaits configured
  pricing acquisition before publication; failure leaves unknown prices. This
  clarifies “before wallet selection” without adding a universal readiness gate.
- Add a separate source-generation admission binding; registered `source` identity
  retains its existing semantics. Observe final response facts inside accepted
  payment work, including decoded relay destination statuses and read failures.
- Retain failed snapshots for inspection, but block new execution through expired
  definitions and return the actual refresh failure. An independent already
  admitted outcome may trigger another attempt; failure alone never schedules one.
- Retire obsolete generations so late outcomes cannot refresh their replacements.
  Runtime metadata refresh uses existing authority without allocation or funding.
- Keep dynamic discovery authoritative. No peer registry or notification broadcast
  is introduced; static clients may need reconnect/reload.

SC1 and SC2 implementation and focused regression coverage are in progress.
Compiler, Clippy, source-matched complexity, exact-SHA reviews and combined review
remain pending. SC3 cleanup remains pending until all required gates pass.

SC1 committed as `0e9754d2`; independent exact-SHA review found two material
retention gaps. Strict warm completeness was checked after staged publication;
the single completeness check now precedes it, with a mixed supported/unsupported
operation regression preserving old bytes, generation and observed price. Forced
relay reads also retained proven completed unpaid failures; retained failure
metadata now preserves the acquisition's existing definitive fact so force can
retry completed failure while uncertain accepted work remains protected. Focused
fixup and its exact-SHA review are pending. The prior compiler/all-target Clippy
snapshot passed; final changed-source revalidation remains pending. Complexity
attempts invalidated by source edits are recorded as failed, not passes.

SC1 fixups `51030b41` and `a802de66` received independent source review;
the two reported findings are resolved. SC2 is `0f9c9fbb`. Independent combined
review covered all four commits, complete affected consumers and source-matched
complexity report `target/complexity-runs/20261112T011346Z-625e9e12` (506 files,
no exclusions, zero source-hash mismatches). The retained Slot::write transaction
and serving shutdown coordination were accepted; unchanged payment, relay and
selection hotspots were examined. Compiler/all-target feature Clippy passed that
source. Config tests passed 17/17; initial refresh tests passed 4/4, including
cancelled waiter ownership. These results are scoped evidence, not a full check.

The combined review found later static retention and runtime diagnostic gaps:
cache commit must follow shared admission/publication, and already-ready waiting
listeners need runtime ERROR reporting when refreshed definitions invalidate exact
selection. Corrections publish the complete new source before best-effort disk
commit and optional observed-hint attachment; this brief advisory-hint window
avoids new mutable generation registries or transactional publication machinery.
Price invalidation is terminal within its cell so late optional enrichment cannot
revive obsolete estimates or restore baked-in generated price descriptions.
Regression coverage and independent correction review are pending.

SC2 correction `2c84fb2c` received independent exact-SHA source review with no
remaining findings in that correction. Review identified a recurring SC1
replacement gap across cache provenance: older indefinite direct/origin/relay
variants could supersede new data or reappear after eviction. The focused fix
indexes each exact catalog policy/URL/lifetime family in disposable SQLite and
retires sibling variants atomically on accepted replacement, preserving distinct
provenance. It also retires old variants on accepted uncacheable responses while
failed staging retains them. No provider handles, wallets or cross-process stale
markers are needed. Tests cover both direct/ordinary directions, relay variants,
eviction, observations and successful uncacheable acquisition. The initial local
key version advances for indexed families; documented one-time upgrade miss still
applies. A small separate SC2 test fix adds the required isolated log-capture child.
Exact correction reviews and final runtime evidence remain pending.

Independent exact-SHA and combined review cleared cache-family fix `2c61c800`
and log-isolation fix `6f356c5c`. The complete 507-file complexity artifact matched
that SHA with no exclusions. It identified a compelling simplification in
Runtime::complete_refresh (CogC 45): failed acquisition now leaves deployment state
untouched, and one early rollback path handles combined validation/publication.
This removes the optional acquired-document state and duplicate rollback work.
A shared terminal diagnostic now also covers first-publication failure. Optional
hint attachment remains a separate post-success responsibility. Slot::write's
atomic family replacement/retirement/eviction responsibilities were accepted.

The focused HTTP cache suite initially reported 36 passes and two old-contract
assertions: expired 304 revalidation formerly retained observations, and oversized
refresh formerly deleted old snapshots. Assertions now enforce the planned
contracts: expiry discards observations even on 304, and failed acquisition retains
complete expired bytes without renewing them. Error/status, bounds, privacy and
request-count assertions remain intact. Focused revalidation is pending.

The full default library run completed with 1,426 passes, four failures and 24
ignored helper/opt-in tests. Independent baseline source inspection established
three inherited failures (no historical executable was run): two relay tests
index absent comprehensive-example source names, and readFetch has ten unresolved
root `$defs` references while definitions are operation-local. The relay tests
are repaired to current corresponding payer groups so affected ownership assertions
execute. The acquisition-age regression now explicitly covers a finite override
and indefinite lifetime. Focused continuation of those three tests is pending;
unchanged passing library results remain applicable. The unchanged readFetch
import failure is retained as an inherited schema-reference defect; no provider is
omitted and reference validation is not weakened. This limits full-library
assurance and does not establish a cache regression.

The final source-matched complexity report through `87ee849f` is complete (508
files, no exclusions, independently verified zero source-hash mismatches).
Independent review accepted every exact implementation/fixup SHA through that
revision and the combined implementation. Further changes here are test/document
corrections only. Selected production compiler and all-target feature Clippy gates
passed; default debug test linking emits a macOS unwind-table-size warning, with
no compiler warning in the selected production/Clippy gate. Compatibility passed
31 tests. Focused cache/refresh/deferred/warm tests passed; affected integration
and focused library continuation remain pending.

## Final implementation gate

SC1 and SC2 are implemented and source-reviewed through `ac298359`, including all
accumulated exact commits and unsquashed fixups. The three library continuations
now pass one test each: acquisition age (`29f8f843`), relay sharing and wallet
selection (`ac298359`). The small synthetic relay fixture removes the repeated
comprehensive-example naming/routing dependency while preserving every ownership
assertion. No production behavior changed in these last test corrections.

Validation on the fixed source, using pinned Make entrypoints and socket-capable
execution for local fixtures:

- Production `make check-clippy` compiler gate passed; subsequent test-only edits
  retain that production evidence. Final all-target feature Clippy passed via
  `make clippy CARGO_ARGS='--all-targets --features zcash-regtest'`.
- `make check-compat` passed 31 tests. Its unchanged production/executable inputs
  remain applicable after test-only fixture corrections.
- Config tests passed 17. Focused cache tests passed 38; source refresh/signed
  preservation passed six; dynamic refresh passed six; deferred publication passed
  29 (three isolated helpers ignored); warm tests passed five.
- Full default library run plus the three focused continuations establishes 1,429
  passes, one inherited readFetch failure and 24 ignored helpers/opt-in tests.
  This is a failed full-library check with focused continuations, not a full pass.
  ReadFetch source attribution and required correction are in the focused deferred
  plan. Its invalid reference asset and validators were not changed or bypassed.
- Nine affected integration targets (`cache_warm`, `catalog_selection`,
  `deployment`, `foreground_capacity`, `pricing`, `pricing_inspection`,
  `relay_shutdown`, `shutdown`, `startup`) passed 53 tests, one opt-in test ignored.
- Final source-matched complexity artifact is
  `target/complexity-runs/20261112T014202Z-c64276c5`: 508 files, no exclusions.
  Production metrics and earlier independent dispositions remain unchanged;
  complete_refresh is 2/5 and optional hint attachment 9/6. Retained Slot::write
  transaction and serving shutdown coordination warnings were accepted. Reports
  and execution logs remain in ignored `target/`.
- Default debug test linking reported a macOS unwind-table-size warning. Tests ran;
  selected production/compiler Clippy diagnostics were clear. No pin, profile,
  warning gate, proof boundary or test contract was weakened.

No live paid calls, consensus/Tor qualification, full `make check`, push or merge
is claimed. No new dependency checkout or monetary authority was introduced.
Permanent cache/refresh/pricing/client contracts and example comments are updated;
endpoint descriptions/search keywords were reviewed with no new tool or routing
capability requiring new discovery terms. Runtime refresh guidance stays in
search/call results and the owning guides. The known inherited readFetch repair is
explicitly deferred with concrete acceptance evidence. Final independent runtime
and evidence review, then SC3 plan/index cleanup and exact cleanup review, remain.
