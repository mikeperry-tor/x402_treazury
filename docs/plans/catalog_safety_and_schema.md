# Catalog expansion and OpenAPI correctness (A1/R8)

## Intent, baseline and preserved contracts

Make every catalog ingestion path safe to expand and make generated tool inputs
faithful to their HTTP contract. Baseline `1bd810eb`; original review baseline
`3ceb88f2e41038c4f6ef6959494c2f501373f1cd`. Plans do not authorize upstream calls.
Read [catalog architecture](../architecture.md#catalog-configuration-and-execution),
[configuration/header contracts](../configuration.md#openapi-header-parameters-and-idempotency),
[agent sources](../agent-sources.md), [resource limits](../local-resource-limits.md),
[network inventory](../network-egress.md#remaining-timers-and-budgets),
[provider policy](../../providers/README.md#provider-policy),
[caveats](../../providers/CAVEATS.md) and [provider tests](../testing.md#provider-regressions).
Preserve endpoint-local ownership, selection, help/pricing metadata, original raw
header contracts, HTTPS/full validation and no paid replay. Recommend nothing
without explicit opt-in; concise tool/parameter descriptions stay agent-facing.

The refreshed baseline includes `main` at `a626b998`: randomized nominal reserve
ranges and saved unsigned choices, removal of configurable per-transfer ZEC caps,
and qualification's running USDC API budget. Preserve current fee and accepted
operation authority; do not restore removed ZEC budgets. Historical qualification
registries remain read-only under the version-2 authority contract.

## Current gaps and proposed behavior

C1 and its independent G1 are accepted through `a50d4f7a`: shared structural and
actual expansion accounting now protects authored, import, cache/304, relay and
direct-Value construction, with iterative traversal. Maintained contracts live in
the architecture/configuration/source guides and both resource inventories.
C2 below is still unfinished; its schema projection changes must reopen affected
C1 call chains at combined G2. The following gap description records the starting
behavior, not a claim that C1 remains unimplemented.

Dynamic import recursively inspects references with existing depth/node/expanded-byte
guards. Static local/remote/cached and relay catalog construction lacks a shared
pre-expansion guard; `resolve` recursively clones referenced values. Public
`build_tools`/`operations` also accept Values directly, bypassing byte parsing. JSON parse
limits alone do not bound reference expansion. Reuse or replace the existing
inspection at every common construction/expansion boundary, including direct Value
APIs, counting actual expanded work;
protect traversal itself from stack exhaustion and use checked arithmetic. Do not
add independent limits per loader or silently truncate tools. Shared safety checks
must not impose unrelated dynamic-import policy (such as public-only destinations
or format restrictions) on authored catalogs. This protects stack
and allocation during authored expansion; any changed threshold requires the
resource-policy justification and both maintained inventories.

Parameter collection still overwrites equal path/query names and routes cookies as
query. Composed-body support already exists in parts of normalization and required
alternatives: R8's claim of no fields must be checked rather than implemented twice.
Inspect request-body media selection: unsupported non-JSON contracts must not be
advertised as JSON. Favor explicit unsupported-operation errors or faithful existing
encoding over a generic new serializer framework. Preserve usable sibling endpoints
and make any rejected/omitted operation visible to the affected consumer.

## Ordered commits and gates

1. **C1 — Enforce expansion safety at the shared construction boundary.** Audit
   parse/build entrypoints, relay cache acceptance, file loading and discovery import.
   Consolidate existing inspection; eliminate redundant import-only traversals when
   shared enforcement covers them. Add small synthetic chains, diamonds, cycles,
   missing/external refs and exponential compositions. Test every ingestion route and direct Value entrypoint,
   checked accounting and boundary acceptance/rejection without crashing the process.
   Update `docs/architecture.md`, `docs/configuration.md`, `docs/agent-sources.md`
   and both resource/timer inventories if guard ownership or behavior changes.
2. **C2 — Preserve parameter identity and body semantics.** Give distinct locations
   collision-safe tool names with explicit original-name routing; retain header
   casing/idempotency and inherited overrides. Reject unsupported cookies explicitly.
   Verify allOf/oneOf/anyOf bodies and required alternatives before fixing actual
   gaps: preserve property-bearing compositions as whole bodies, and retain
   only ordinary object roots with pure required-only alternatives as flat inputs
   or explicitly reject unsupported shapes. Select supported content types truthfully; reject unsupported bodies with
   useful diagnostics. Test serialized outbound path/query/body/header bytes and
   negative cases with zero paid disclosure, including suffix collisions, inherited
   overrides, same-path POST preference and filters. Update
   configuration/architecture and agent-source guidance, review endpoint summaries
   and discovery keywords, and document only named provider quirks in caveats.

**G1 after C1:** independent ingestion/expansion review of every loader and all
prior fixups. Exit: no bypass, stack-safe accounting, consumer-visible rejection,
and no duplicated arbitrary limits. **G2 after C2:** combined catalog-to-tool-to-
request review across all commits/fixups, provider selection, descriptions and
fixtures. Exit: faithful wire behavior and no lost supported catalog capabilities;
add focused regressions for any newly exposed normalization assumptions.

## Review, validation and handoff

Before substantive implementation, capture the current exact baseline and have an
independent read-only reviewer challenge the design, necessity of checks and
opportunities to remove responsibilities. Resolve material findings before dependent
work. The baseline above describes the proposal, not completed validation.

For each ordered commit, freeze sources, run affected cheap contract tests, then
applicable production compiler/all-target Clippy and compatibility checks before
broader runtime suites, following [testing](../testing.md#validation-order-and-continuation).
Use pinned Make entrypoints and public unfunded keys; record actual nonzero tests,
commands, source identities and limitations under ignored `target/`. Serialize
builds. Offline fixtures establish no live-funded or Tor qualification.
Independently review every exact committed SHA and its consumers; the reviewer
must not author it. Fix findings with `git commit --fixup=<introducing-SHA>`,
independently review each fixup, and retain unsquashed history through integration.

Each named gate examines all preceding plan commits and fixups alongside complete
call chains, tests and documentation. Resolve material findings before exit;
examine fixup causes, recurring assumptions and missing coverage, and decide
whether they require regression tests, a wider audit, simplification or a design
change. Earlier approvals do not exempt code; later material changes reopen gates.
For substantive Rust refactors and architecture gates compare CogC/CycC with the
confirmed baseline using [complexity discipline](../../tests/COMPLEXITY.md#review-discipline).
Review warnings, changed chains and unchanged high-risk hotspots; assess simpler
ownership/control flow rather than extract helpers solely to lower scores.

Update the named permanent guides with each implementation commit. Keep the
applicable [AGENTS.md](../../AGENTS.md) links pointing to those guides; add an
owning-guide link there if a new enduring rule needs one. Before removing this
completed plan, verify guide/reference handoff, remove its documentation-index
link, and independently review both the exact cleanup SHA and the final combined
branch after removal. Keep concrete unfinished work, no completed-plan archive;
documentation cleanup never deletes financial evidence or grants replay authority.
