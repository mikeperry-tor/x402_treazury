# Discovery keywords and recommended tools

## Intent, baseline and scope

Add optional endpoint-local YAKE keyword generation for configured and dynamically
registered APIs. Return current coverage with discovery and successful registration
results, and provide an offline comparison against the manually authored discovery
guidance in `examples/deployments/research_desk.toml`. Also let an operator explicitly
advertise recommended API tools beside discovery wrappers, taking recommendations
from provider/source advice or an endpoint override.

This is an implementation plan, not implemented configuration or authorization for
paid calls. The starting baseline is `acc174d8f51c540f185e13f281277d9587ad17b7`.
Preserve unrelated work, including the local untracked `TODO`.

Current behavior:

- `CatalogSnapshot.views` contains each listener's final visible tools and bindings
  after source selection, `send_tools`, listener filters and dynamic publication.
- Tool search captures one snapshot, ranks visible tools with BM25, and returns
  complete schemas with generation-bound pagination. Manual discovery description
  text influences agent tool selection, not BM25 ranking.
- `discover_on_demand = true` hides every API definition from `tools/list`, while
  retaining discovery/call wrappers and independently enabled management tools.
- Provider/source `recommended_tools` is advisory. `use_recommended_tools` is a
  separate, explicit catalog-selection setting. Recommendation inspection markers
  do not currently survive into bound endpoint catalogs.
- HTTP does not advertise or send tool-list changes. Clients can cache descriptions
  and inventories; live results are the reliable surface for changing coverage.

Owning contracts are [agent sources](../agent-sources.md), particularly endpoint
instruction placement, tool search, authority and dynamic publication;
[configuration](../configuration.md), particularly selection and source tool
distribution; [catalog architecture](../architecture.md#catalog-configuration-and-execution);
[provider metadata](../../providers/README.md#bundled-provider-metadata);
[development](../development.md#dependency-vendoring); and
[testing](../testing.md). Follow the independent-review and complexity requirements
in [AGENTS.md](../../AGENTS.md#independent-subagent-workflows).

## Proposed endpoint controls

Proposed names, to finalize at the first implementation design review:

```toml
[servers.science]
discover_on_demand = true
discovery_keywords = "yake" # omitted or "off": no generated response summary
discovery_search_tool_instructions_text = "Search academic research, clinical trials and public health data."
advertise_recommended_tools = true # default false; explicit provider-advice opt-in
# Optional override: actual endpoint-local tool names/selectors.
recommended_tools = ["vaaya_openalex_works", "twos_gov_fda_drug_shortages"]
```

The names in the example illustrate syntax; verify actual generated identifiers
before using it as deployment configuration. These are independent endpoint
settings. They neither enable discovery/management nor change loading, routing,
source ownership, wallet assignment, payment authority or search ranking. Unknown
modes and malformed selector syntax are configuration errors. Expose effective
settings and their origins through ordinary config inspection.

Keep the first implementation small: one YAKE mode with an English extraction
profile evaluated below, no LLM, embeddings, network extraction service, new
persistent index, periodic job or list-change notification machinery. Preserve
manual instructions verbatim. Keywords are optional hints about loaded coverage,
not a claim of provider availability, health, completeness or correctness.

### Static catalogs and authored instruction text

Use the same extractor for both static configured tools and dynamic registrations;
do not infer keywords from a provider-wide inventory before endpoint routing.
An opted-in static-only endpoint can return a generated summary through discovery
without enabling provider management. An eager endpoint may also opt in when it
already exposes search through management. Reject an enabled keyword setting on an
endpoint with no discovery result surface, with a clear configuration error.

Provide an operator preview/evaluation command that builds endpoint-local summaries
from inspection catalogs, even for endpoints whose runtime generation is off. It
prints keywords and a suggested static discovery-instruction fragment; it never
rewrites TOML implicitly. A reviewed fragment can be copied into the existing
`discovery_search_tool_instructions_text`, retaining manual aliases and usage
guidance. This is the static generation path for initial model-visible coverage.

Do not auto-rewrite MCP tool descriptions as catalogs arrive. This avoids a frozen
startup summary becoming stale after deferred loading, refresh or registration.
Manual instructions continue to describe intended coverage before catalogs load;
live summaries describe the currently loaded visible catalog. A live summary can
only help after the agent invokes discovery: it does not solve initial endpoint
selection. Keep concise static guidance for that purpose even when live YAKE is on.

### Direct advertisement of recommendations

Interpret explicit listing as full ordinary API definitions in `tools/list`, beside
the search/call wrappers on an on-demand endpoint. It is not merely a prose list,
an automatic invocation, a claim of best price, or a restriction on discovery.

- `advertise_recommended_tools = false` preserves the current inventory. Merely
  defining endpoint advice does not activate advertisement.
- When enabled, an omitted endpoint `recommended_tools` derives advice from the
  effective provider/source recommendations. An explicit list replaces that advice
  for this endpoint; `[]` explicitly advertises no recommended API tools.
- Resolve advice to the actual public endpoint-local names, intersect it with the
  final visible view, deduplicate, and order deterministically. Existing source
  compatibility/selection restrictions and listener filters always win. Never
  restore an excluded tool or let advice import a new source.
- Reuse established generated-name selector syntax. Unavailable exact override
  names and unmatched patterns produce advisory diagnostics, not startup/readiness
  failures: deferred catalogs and later registrations can make them visible. Scope
  diagnostics to catalog/configuration transitions, not every search request.
  Existing provider/source validation and `use_recommended_tools` semantics remain.
- Source recommendation metadata must be carried deliberately through catalog
  binding and dynamic publication. For curated registrations, map canonical advice
  through the existing name transformation to their actual `dyn_...` IDs; do not
  guess by string prefix replacement. Generic URL imports receive no provider advice.
- Eager endpoints already advertise all visible APIs, so this setting adds no
  duplicate definitions. Both direct invocation and the wrapper resolve the same
  current binding; lookup, listing and invocation must agree.
- Include recommended IDs in live discovery/registration metadata when this feature
  is enabled, so an agent can use the wrapper even if its cached `tools/list` omits
  a newly recommended tool. A recommendation may change after publication; existing
  clients need a tool-list refresh/reconnect to see newly advertised definitions.
  Removed tools remain uncallable even through cached definitions.

Do not reuse `use_recommended_tools` for this feature: it narrows the source catalog,
whereas endpoint advertisement selects a small initial context from tools that
remain discoverable. Avoid duplicating schemas in live recommendation metadata;
return IDs and let search provide complete schemas. Preserve help tools only when
explicitly recommended and actually visible.

## Extraction and result contract

Start with `yake-rust` as the candidate implementation. Its
[upstream documentation](https://github.com/quesurifn/yake-rust) describes statistical
phrase extraction and configurable stopwords; its
[crate API](https://docs.rs/yake-rust/latest/yake_rust/) provides `get_n_best`.
Review a released version, license, transitive dependencies, deterministic behavior,
Unicode handling, panic/error paths and compatibility with the pinned toolchain
before adding it. Prefer the released crate and only needed language features.
No vendoring is proposed. If a patch/vendor checkout becomes necessary, explain
the need, alternatives and maintenance costs before introducing it. Manual-only
guidance remains a supported alternative if the dependency or quality is unsuitable.

Build a stable document from actual visible tool names and concise descriptions.
Normalize generated prefixes and identifier separators for extraction while retaining
acronyms and public names. Keep tool boundaries so phrases cannot span unrelated
tools. Evaluate parameter names as an additional input rather than assuming they
help; omit schema examples/defaults, response data, credentials, raw specifications,
provider instructions and fetched help text. Do not feed manual keywords into the
YAKE baseline being compared against those same keywords.

YAKE is extractive: it cannot invent useful missing synonyms. Assess short technical
descriptions, digit-bearing terms such as x402/2s, acronyms, non-English text, repeated
generic verbs and provider boilerplate. Mark English extraction as such rather than
claiming multilingual coverage. Retain useful branded/acronym terms through a small
auditable normalization policy if evaluation demonstrates losses; do not grow an
implicit domain taxonomy or silently change BM25 tokenization.

Compare concatenated endpoint extraction with per-tool extraction and deterministic
aggregation. Choose the simpler acceptable approach using evidence about dominant
large providers, minority domains, document-order bias and cross-tool phrase noise.
Stable input order and explicit tie-breaking must give identical output for identical
inputs; recompute from the final view rather than accumulate keyword deltas.

The keyword count is an explicit summary display choice, not a limit on tools or
accepted work. Evaluate several phrase counts and lengths, then document the chosen
profile and that the summary is selective. No arbitrary input truncation, catalog
caps, timers, extra readiness gates or extraction work on every page. Store derived
data with its immutable snapshot, or use a snapshot-owned lazy value if measurements
justify it; avoid a separate cache invalidation subsystem. Keep costly derivation
outside publication locks and async reactor work. Do not let advisory extraction
failure undo an accepted registration or hide callable tools: report summary
unavailability explicitly without blocking discovery/publication or returning stale
keywords from an earlier generation.

Proposed additive result shapes:

- `tools_search` gets `capability_summary` when keyword generation is enabled:
  `{status: "available", method: "yake", scope: "endpoint" | "source", source_name: null | string,
  catalog_generation, tool_count, keywords: [...], selective: true}`. An extraction
  failure has `status: "unavailable"` and no stale keywords. An empty catalog is
  `status: "available"` with an empty keyword list, distinct from extraction failure.
- Derive the summary from all visible tools in the selected endpoint/source scope,
  before applying the query or pagination. Capture one snapshot for results,
  counts, recommendations and summary. A zero-match query still returns that scope's
  coverage; an empty scope returns an empty keyword list and zero tool count.
- Return the same scope summary on every page initially; measure response overhead
  before adding another pagination/request option. Expose no scores as confidence
  or quality probabilities. Existing cursors, counts and ranking are unchanged.
- Successful add/refresh results, including idempotent add results, include the
  registering listener's current endpoint summary and catalog generation when
  enabled. Capture it from the published snapshot used to construct the result,
  not a later independent read. Label disabled/zero-visible registrations honestly;
  their endpoint summary may describe other tools, never imply registration success
  or source-specific coverage that is absent. Keep registered `tool_count` distinct
  from summary `tool_count`. Source-scoped search provides the narrower summary.
- Where recommendations are enabled, add `recommended_tool_ids` with an explicit
  endpoint/source scope and generation. It is query-independent, restricted to the
  selected visible scope, and carries no duplicated API schemas. Keyword and
  recommendation options work independently, including when keyword generation is
  off. Management definition descriptions stay stable.

Removal, disablement, restore, deferred static loading, refresh and static
replacement all derive from the same publication path. Remove obsolete terms and
recommendations naturally when their last visible supporting tool disappears.
Endpoint/source isolation applies to extraction statistics as well as output:
hidden tools must not influence which keywords or recommendations appear. No extra
upstream requests, pricing probes, help purchases or wallet allocation are allowed.

## Evaluation and deployment decision

Evaluate before enabling automatic keywords in research_desk. Keep manual entries
as the initial control, with no automatic TOML edits or blanket opt-in.

Use frozen routed catalogs for every research_desk endpoint plus small synthetic
dynamic-tool cases. Retain schema provenance, tool IDs, routing/config revision,
extractor version/settings and reproducible command lines in ignored
`target/discovery-evaluation/`. Do not add universal provider inventory fixtures.
The toolbox's initially empty static catalog must be tested separately from its
registered dynamic tools; a provider directory inventory is not its current coverage.

Compare manual-only guidance, YAKE-only terms, and manual-plus-YAKE suggestions at
similar displayed context sizes. Separate keyword quality from live-result placement
and from BM25, whose algorithm is unchanged. Use a fixed task/query set prepared
before tuning, with a held-out subset, plus human review of proposed terms:

1. Measure useful capability precision, supported-domain coverage, redundant/generic
   phrases, missing aliases, and claims for capabilities absent from the endpoint.
   Existing manual prose is a comparison baseline, not complete ground truth.
2. Check academic work in science, travel/mobility in world, and the legal/government
   split across business, property, finance and world. Include small minority domains
   and providers with many near-duplicate tools. Confirm excluded tools contribute
   neither phrases nor recommendations.
3. Test whether representative tasks select the correct endpoint, then find suitable
   tool IDs after seeing discovery/registration results. Report endpoint selection
   separately from in-endpoint retrieval; extracting terms alone is not evidence of
   better agent behavior. Any model-assisted trial is optional, explicitly accounted
   for, and must not invoke paid provider tools.
4. Measure deterministic output, extraction cost, publication/first-result latency,
   allocation and added response/context size across real catalog sizes. Compare
   before changing resource policy; do not invent performance gates or work caps.
5. Replay add, refresh, removal, disablement, restore, concurrent search/publication
   and pending/failed catalog cases. Confirm one-generation consistency and no stale
   summaries, lost registrations or new network/financial side effects.

Record a concise maintained decision with illustrative successes/failures, without
committing generated report archives. Keep manual terms where they better express
aliases or routing boundaries. Improve research_desk manual entries only when a
suggestion is supported by its actual tools, with comments for material editorial
choices; independently review that exact configuration commit. Enabling live YAKE
can be selective per endpoint (especially toolbox) and is not required by a good
static preview result. Set evaluation acceptance criteria before tuning: no hidden
coverage leakage, reproducible results, no registration/search correctness
regressions, and reviewer-confirmed useful coverage at an acceptable measured
context/latency cost. Do not claim semantic retrieval improvements from term overlap.

## Ordered implementation and review gates

Before implementation, refresh the baseline and have an independent read-only
reviewer challenge this plan, dependency choice, placement and opportunities to
remove settings/state. Resolve material findings before building on those decisions.
The proposed coherent commits are:

1. **D1 — Shared extractor and static preview.** Add the reviewed YAKE dependency,
   deterministic scoped text preparation and extraction, and an operator preview/
   comparison entrypoint using existing catalog inspection machinery. Do not
   implicitly warm catalogs or perform paid fetches; offline evaluation uses frozen
   inputs and any live inspection retains existing explicit authority. Add focused
   synthetic extraction/isolation tests and document the command and dependency
   profile in `docs/agent-sources.md`, `docs/development.md`, `scripts/README.md`
   if a script is added, and `docs/testing.md`. Acceptance: reproducible static and
   dynamic fixture previews, deterministic phrases, no cross-tool phrases/leakage,
   explicit empty/error behavior, and initial manual-vs-YAKE evaluation evidence.
2. **D2 — Optional live endpoint summaries.** Add typed endpoint mode/config
   inspection, final-snapshot derivation and additive search/add/refresh result
   fields together. Wire static-only discovery, restored registrations, idempotent
   registration and all publication consumers. Preserve cursor and binding
   semantics. Focused tests cover filtered/empty scopes, zero matches, pagination,
   concurrent generations, refresh/removal/restore, extraction failure and disabled
   mode compatibility. Update `docs/configuration.md`, `docs/agent-sources.md` and
   `docs/architecture.md` with data ownership, result fields and cached-client limits.
   **Gate G1:** review D1/D2 plus all fixups and full publication/search/registration
   chains; exit only with snapshot isolation, accepted-registration preservation,
   dependency suitability and measured extraction placement established.
3. **D3 — Explicit recommended-tool advertisement.** Carry advice through existing
   catalog/name-binding machinery; implement the endpoint opt-in/override and live
   recommended IDs in the same commit. Check inspection, eager/on-demand listing,
   lookup, direct/wrapper invocation and dynamic canonical-name rewriting together.
   Tests cover opt-in/off, omission vs `[]`, override replacement, help, duplicate/
   stale advice, source/listener exclusions, deferred catalogs, generic imports,
   dynamic refresh/remove/restore and cached removed-tool rejection. Assert identical
   routing/payer identities and discoverability of nonrecommended tools. Update
   `docs/configuration.md`, `docs/agent-sources.md`, `docs/architecture.md` and
   `providers/README.md`; explain source selection versus endpoint advertisement.
4. **D4 — Evaluation and research_desk adoption.** Complete the held-out evaluation,
   document a concise decision in `docs/agent-sources.md`/`docs/testing.md`, and make
   only evidenced endpoint opt-ins/manual keyword edits in research_desk. Leave
   recommendations opt-in and review the selected direct inventory and context cost
   explicitly. Confirm actual names in all permanent examples. Acceptance includes
   complete configuration/routing comparison, generation consistency and the
   context/coverage tradeoffs above; negative YAKE results may justify retaining
   entirely manual guidance. Evaluation reports remain ignored.
5. **D5 — Plan cleanup.** After acceptance, verify permanent documentation and
   applicable AGENTS links, remove completed plan content and update the docs index.
   Retain only concrete unfinished work if any, without an implemented-plan archive.

Have an independent nonauthor review every exact committed SHA and relevant
consumers; fixes use `git commit --fixup=<introducing-SHA>` and receive independent
exact-SHA review. Preserve unsquashed history through integration. At G1 and final
integration after D4, review all prior commits/fixups together with complete call
chains, test wiring, repeated assumptions and uncovered failure paths. Material
later changes reopen affected concerns. Resolve findings before proceeding.

Capture baseline and changed CogC/CycC for substantive Rust changes, including
unchanged high-risk publication/registration/search consumers; use
[complexity review discipline](../../tests/COMPLEXITY.md#review-discipline).
Evaluate simpler ownership/control flow rather than extracting score-only helpers.
Run cheap affected contracts first, then production compiler, all-target Clippy and
compatibility checks before broader suites in the maintained validation order.
Coordinate builds and source freezes; request socket-capable execution up front
for transport fixtures. No real secrets, funded qualification or provider purchases.
Carry forward passed evidence unless invalidated by later changes.

Final integration exits with coherent endpoint semantics, useful evaluated output,
unchanged financial/network authority, resolved material findings and permanent
documentation in place. Independently review the exact D5 cleanup SHA and the final
combined branch after plan removal, including prior fixups and documentation links.
