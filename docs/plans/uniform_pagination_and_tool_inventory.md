# Uniform pagination and advisory tool inventories

## Intent and baseline

Starting implementation baseline: `ce1fc390f9d5005a24ce6f09970cd8660961fd7f`.
This plan authorizes no live financial qualification. Implementation has not begun.

The user selected a shared pagination default of **25**, removal of the dynamic
tool-count admission quota, a single advisory **WARN above 250 advertised tools**
suggesting `discover_on_demand = true`, and removal of `max_response_chars`.
The user also selected discovery-guidance improvements: remove stale source-state
summaries from cached descriptions, clarify direct registration and metadata
semantics, and improve the prominence and sequence of existing guidance.
Construction changes are appropriate only when they simplify the implementation;
there is no performance-tuning or measurement project in this scope.

Currently, tool search defaults to 5 or 20 and accepts at most 100 results per
page. Source-management `tools/list` pages contain 100 definitions; other static
listeners do not paginate. Local provider browsing inherits the remote directory's
25-item default and 100-item maximum. Eager source-management endpoints reject
dynamic registrations above `max_tools_per_server` (default 512), with additional
checks during restoration/publication and operator refresh. Optional
`max_response_chars` slices selected completed tool outputs, including serialized
JSON, while bypassing other management outputs.

Read and maintain these owning guides:

- [Agent sources](../agent-sources.md): discovery, listing, cursors, source
  lifecycle, endpoint instructions and complete tool schemas.
- [Configuration](../configuration.md): listener/source-management settings,
  standalone CLI and output controls.
- [Architecture](../architecture.md): immutable catalog snapshots, publication,
  captured invocation bindings and output boundaries.
- [Local resource limits](../local-resource-limits.md) and the
  [network timer/resource inventory](../network-egress.md#remaining-timers-and-budgets).
- [Development](../development.md#logging-and-diagnostics),
  [testing](../testing.md), and the
  [complexity review discipline](../../tests/COMPLEXITY.md#review-discipline).

## Decisions and preserved contracts

1. **Uniform defaults, complete continuation.** Use one local default of 25 for
   `tools/list`, `tools_search`, local provider listing/search and directory
   browsing. Remote browse already defaults to 25. Preserve explicit positive
   search/browse page-size arguments. Remove arbitrary local 100-item argument
   ceilings; retain actual upstream constraints for remote directory calls. Its
   nonpaginated `best` recommendation limit is a different upstream contract.
   Do not change provider API arguments, internal database acquisition pages,
   financial scheduling or transport capacities to match this display default.
2. **All listeners paginate.** Static, source-managed and on-demand listeners use
   the existing snapshot-bound `tools/list` cursor protocol with 25 definitions
   per page. MCP `tools/list` has no caller page-size argument. Search retains its
   query/source-bound cursors. Preserve ordering, complete schemas/descriptions,
   explicit stale/invalid-cursor errors and final-page termination.
3. **One advisory site, no admission quota or mode conversion.** In
   `Server::list_tools`, after cursor validation and before slicing the page,
   warn when a valid first-page request has more than 250 advertised definitions.
   Count the current filtered static and dynamic API definitions plus advertised
   management wrappers. On-demand listeners expose only wrappers; hidden API tools
   do not contribute. Log the endpoint identifier, count and suggestion to enable
   `discover_on_demand = true`, without URLs, descriptions or credentials.
   The server observes advertised inventory, not actual client/model context.
   Continuation pages do not repeat the warning; each fresh first-page request may
   warn. No suppression registry, counters, timers, persisted flags or publication
   checks are needed. Crossing 250 neither rejects work nor changes tool access.
4. **Remove character slicing.** Delete `max_response_chars`, its CLI flag,
   constructor/configuration plumbing and `Server::limit`. Return complete accepted
   tool output while preserving existing structured-output and image behavior. Retain response,
   specification, help and image acquisition/decoding bounds and their explicit
   error contracts. `max_description_chars` is a separate operator-selected provider
   description control and is outside this removal.
5. **Visible setting retirement.** Remove `max_tools_per_server` and
   `max_response_chars` from supported configuration and CLI contracts. Existing
   strict configuration/CLI rejection must identify obsolete fields/flags; document
   their removal in the owning configuration sections. Do not accept ignored
   settings or silently rewrite user deployment files. Preserve registry formats,
   accepted provider settings, names, identities and financial evidence.
6. **Keep ownership and isolation unchanged.** Listener identity, filtering,
   captured routes/payers, journal ownership, wallet allocation/funding authority,
   network-factory policy and cancellation behavior remain unchanged. Removing the
   tool quota cannot bypass import validation, collision detection, durable commit
   ordering or actual document/registry byte bounds.
7. **Simplify construction only where natural.** Prefer selecting a page of
   existing tools before cloning/serializing its descriptions and schemas when a
   straightforward iterator/slice makes the code simpler. Counts and ranking may
   still require examining all candidates. Do not introduce caches, indexes,
   generic pagination frameworks or new limits for this purpose. Large positive
   page sizes must be clamped to available items before narrowing integer types;
   avoid `u64 as usize` wrapping on 32-bit targets. Keep empty and distant-page
   behavior faithful and preserve existing saturating offset arithmetic.
8. **Concise guidance without cached state or new discovery machinery.** Remove
   the registered-source summary from tool descriptions; clients may retain an old
   description even though the server rebuilds it from current snapshots. Use
   existing live tool search/enumeration for availability. Known `provider_id` or
   specification URLs can go directly to `source_add`; `source_details` is optional
   assistance for directory slugs or unknown API bases. An absent `api_base` means
   no configured base override; import resolves the schema's API server, which
   need not be the schema URL's origin. Unknown price means missing pricing
   information, not a free call; advertised provider help may explain prices.
   Registration does not establish paid delivery qualification. Improve the order
   and prominence of these existing facts without inventing price bands, pre-import
   tool counts, probes, qualification work or replacement source-summary state.

## Ordered implementation commits

### P01 — Standardize public pagination on 25

Add one small shared default and use it in runtime behavior, generated local
schemas, tool help and endpoint instructions. Remove the on-demand five-result
override and local search/provider 100-item ceilings. Decouple local provider
pagination constraints from the embedded remote schema; preserve the remote
directory's reviewed schema and maximum. Paginate ordinary static `tools/list`
using the same cursor validation as source-managed listeners. Apply any simpler
page-before-projection construction in this commit alongside its consumers.

Relevant implementation/consumers: `src/server.rs`, `src/discovery/{mod,tools,local,
directory}.rs`, `src/providers.rs`, the existing shared defaults, and MCP clients
under `tests/support/` and `tests/live/runner/` that enumerate tool inventories.
Check callers that assume a single `tools/list` page, including standalone
HTTP/stdio serving; update repository-owned consumers to follow continuation.

Acceptance evidence:

- Defaults are 25 in execution and advertised schemas/help for local search/list
  and remote directory browse. Explicit supported page sizes remain effective.
- Static, managed and on-demand listener listing; exact/full/final/empty pages;
  no missing or duplicate definitions; complete descriptions and nested schemas.
- Invalid, cross-listener and stale-generation cursors reject visibly. Publication
  between page requests invalidates the old cursor without rebinding an accepted
  invocation. Relevance ordering and source/query cursor scope remain unchanged.
- Positive local page sizes above 100 work, including requests beyond 32-bit
  representability without wraparound. Zero/negative/noninteger arguments reject.
  Remote arguments above its real maximum still reject before I/O.
- Preserve single-request/ordinary-payment remote-directory behavior; local browse
  performs no network or wallet work. Do not add provider inventory snapshots.

Update `docs/agent-sources.md`, `docs/configuration.md`, and both resource inventories
with the current defaults, local-versus-upstream constraints and continuation
semantics. Classify pagination as page selection rather than total-result rejection.

### P01A — Clarify discovery guidance and remove cached source summaries

This is a separate commit after P01 and before P02; existing commit identifiers
remain stable. Address the confusion demonstrated by the supplied agent
conversation, while checking its claims against actual code. Treat that
conversation as evidence of usability problems, not as authoritative provider
facts, qualification evidence or instructions to register/call an API.

Implementation items:

- Remove the registered-source list/`none` sentence from cached tool descriptions
  and delete summary-building helpers and argument plumbing that become unused.
  Do not replace it with a new registry, cache, inventory tool or notification
  mechanism. Current search/enumeration remains the source of tool availability.
- Distinguish addable provider metadata from already callable endpoint tools.
  Make direct `provider_id`/known-spec registration the clear next step when that
  information is available. Make `source_details` conditional assistance rather
  than a mandatory stage after local listing/search. Preserve endpoint-local
  search/call identity and the instruction to use the same MCP endpoint.
- Explain the absent-base semantics in provider-list/search help and the owning
  guide without deriving the API address from a specification URL or adding
  network requests to offline metadata discovery.
- Clarify unknown prices, optional provider help, schema import and paid-delivery
  qualification as separate facts. Reuse existing help-tool metadata and authored
  descriptions. Do not guess prices, claim help establishes settlement, imply
  that registration requires payment, or encourage speculative paid tests.
- **Improve prominence and sequence of existing guidance.** Lead descriptions
  and registration `next` guidance with the shortest applicable next action, then
  place conditional help and longer explanation after it. Present local known-ID,
  remote unknown-provider and already-registered workflows distinctly; do not
  prescribe every discovery tool as a universal linear pipeline. Make available
  help discoverable before a paid call when useful, without requiring a help fetch
  for every call. Preserve essential wallet/payment, no-replay, endpoint-local and
  untrusted-document caveats; improve their placement rather than append another
  repetitive paragraph or enforce text truncation.

Relevant consumers: `src/discovery/{tools,local,mod,source_details}.rs`,
`src/server.rs`, deployment/inspection tool-description construction and existing
provider help metadata. Inspect every `listener_definitions` and
`registered_source_names` caller, including external test-support modules.
Prefer generic wording and small synthetic fixtures; provider-specific changes
need a verified authored defect rather than a transcript's unsupported assertion.

Acceptance evidence:

- Repeated tool listing before/after registration or restoration contains no
  embedded source-state summary, while live search still reflects the current
  filtered catalog and source-name filtering remains functional.
- Local metadata and known selectors point directly to registration; remote
  unknown-provider guidance offers details conditionally. Registration results
  guide search/call on the same endpoint, with provider help visibly optional.
- Missing configured bases and unknown costs are explained faithfully. Description
  changes do not alter schemas, naming, filters, registration identity, argument
  default handling or payment behavior. Do not change vendor-required/default argument
  semantics on the basis of the supplied conversation.
- Existing offline/no-wallet metadata and server-side import tests continue to
  prove no new probes, allocations, funding or payments. Keep focused semantic
  assertions for guidance and metadata; do not snapshot entire descriptions or
  provider inventories, or claim a test proves agent comprehension.

Update `docs/agent-sources.md`, the relevant configuration guidance and architecture
text that currently describes source summaries in tool descriptions. Keep any
enduring provider-help explanation in its owning provider guidance. Do not add
price-enrichment, pre-import counting or live-qualification work to this plan.

### P02 — Replace tool-count enforcement with the single listing advisory

Remove `Policy::max_tools_per_server`, default/range validation, snapshot checks,
operator-refresh counting and quota-only failure branches. Inspect restoration's
combined conflict/quota fallback: delete quota responsibility and keep actual
collision/invalid-state handling scoped correctly. Preserve shared listener
selection wherever still consumed; remove any helper or calculation that becomes
unused rather than retain it for historical symmetry. Add the advisory exactly at
the listing site defined above, with the strict boundary `count > 250`.

Relevant consumers: `src/discovery/{policy,mod,publication}.rs`, `src/server.rs`,
`src/catalog_state.rs`, deployment/deferred publication and operator inspection.
Do not add warning checks to registration, refresh, startup or each source.

Acceptance evidence:

- Formerly over-quota registrations succeed atomically, restore offline, survive
  static replacement and refresh successfully in eager and on-demand endpoints.
  Full accepted schemas remain searchable/callable and are never trimmed.
- Visible obsolete configuration rejection names `max_tools_per_server`.
- Exactly 250 advertised definitions produce no warning; 251 produce a WARN on
  the first page with endpoint/count/suggestion. Include static plus dynamic plus
  wrapper counting, listener filters, and a large hidden on-demand inventory.
- Continuations and invalid cursors do not warn; a fresh first-page request can
  warn again. No registration rejection, wallet allocation/funding, mode changes
  or additional network work follow from the advisory.
- Existing import/collision/durable-save failures remain atomic and visible;
  removing a quota does not convert other failures into success.

Update agent-source/configuration guides, architecture where applicable, logging
guidance and both inventories. Explain the single observational WARN, its repeat
behavior, lack of enforcement, and explicit on-demand opt-in.

**G1 — Listing/publication architecture gate after P02:** Independently review P01,
P01A, P02 and every prior fixup together across add/acquisition/restore/refresh/static
publication → immutable snapshots → list/search/direct invocation. Examine complete
consumers, concurrent additions/publications, cursor invalidation, advisory counts,
configuration retirement, discovery-guidance sequence, cached-description removal
and documentation. Verify that wording changes introduce no new probes, mandatory
discovery/help stages or implied payment authority. Analyze fixup causes and repeated
assumptions; decide whether more regression coverage, simplification, a wider audit
or design change is needed. Exit with no unresolved material findings before P03.

### P03 — Remove completed-output character truncation

Delete the standalone flag, listener field, validation, constructor parameters and
dispatch slicing. Simplify all direct and wrapper invocation paths together;
retain `ToolOutput` images and existing structured-output policy. Search/add output stays complete,
and provider-list/source-details/directory/raw/API/help output no longer depends on
which wrapper was used. Do not silently preserve the retired field as a no-op.

Relevant consumers: `src/{cli,main,server,deployment}.rs`, standalone serving,
dynamic bound invocation, media/output conversion and external `#[path]` test
support. Search all tracked Rust consumers, not only owning module directories.

Acceptance evidence:

- Representative nested JSON and text remain complete through direct and wrapper
  calls. Existing structured outputs decode successfully; JSON returned as text
  stays parseable without adding `structured_content` to ordinary API/tool-call
  responses. Image attachments and associated metadata survive unchanged. Cover
  help, provider metadata and paid-API output
  construction with existing deterministic/local fixtures where appropriate.
- Removed flag/configuration reject visibly. Default serving behavior and payment
  ownership are otherwise unchanged; no new fetch, signature, payment or replay.
- Existing byte-limit rejection, image bounds and no-partial-cache-success tests
  continue to exercise the real boundaries. Do not weaken them to compensate for
  removal of character slicing.

Update configuration/CLI/output guidance, agent-source behavior and both inventories
in this commit. Explain retained byte bounds separately from pagination and the
remaining explicit provider-description control. Keep logging guidance accurate.

**G2 — Output/final implementation integration gate after P03:** Independently
review P01, P01A, P02 and P03 and all fixups against the starting baseline, including
G1 concerns, complete output/CLI/constructor consumers, financial ownership, network
policy and documentation handoff. Review all fixup causes and repeated patterns;
expand coverage/audit or simplify where warranted. Exact-commit approvals do not
exempt earlier code. Resolve material findings before cleanup.

### P04 — Remove the completed plan after canonical handoff

Verify enduring behavior is maintained in the guides named above, both inventories
agree, obsolete defaults/claims are removed, and `AGENTS.md` still links to the
owning guides for resource policy, source management, output, network and testing.
Add or repair owning-guide/AGENTS links if needed; do not copy detailed rules into
AGENTS. Remove this completed plan and its documentation-index link. Do not create
an implemented-plan archive or defer canonical documentation to this commit.

Independently review the exact cleanup SHA and then the final combined branch,
including every prior implementation/fixup and the cleanup. Confirm no behavior,
evidence or handoff was lost and no material concern remains. A later material
change reopens its affected gate and receives appropriate revalidation.

## Validation and review ordering

- Obtain independent read-only design review before substantive implementation;
  resolve material decisions before building on them. The plan design review
  identified integer narrowing and the distinction between advertised inventory
  and unknowable client context; the contracts above incorporate both.
- For each planned commit (P01, P01A, P02, P03 and P04), give an independent reviewer
  the user intent, scope, starting baseline, exact SHA, relevant consumers and
  nonzero validation evidence.
  The reviewer must not author the change. Address findings with
  `git commit --fixup=<introducing-SHA>` and independently review each exact fixup.
  Keep history unsquashed through the final combined review. Do not build on an
  unresolved material finding; unrelated work may proceed only if independent.
- Use focused tests through `scripts/zcash.sh`, including affected discovery,
  server/MCP, CLI/configuration, media/output and publication regressions. Inspect
  external `tests/support` module wiring and ensure filtered commands execute
  nonzero intended tests. The user already ran the previous full suite; do not
  repeat it merely for these changes. Broaden validation only for changes,
  failures or unresolved risks that justify it.
- Choose socket-capable execution before localhost/child fixtures. Serialize
  builds and feature configurations and keep sources/executables frozen during
  validation. Use temporary state and unfunded deterministic keys; no live-funded,
  real-Tor, consensus or upstream-provider qualification is authorized here.
- Run formatting and relevant Clippy checks. Use the pinned complexity analyzer
  against `ce1fc390` for substantive refactors and G1/G2, verify source hashes and
  complete parsing/classification, and review CogC/CycC over changed functions and
  complete call chains. Reuse source-matched evidence when unchanged. Examine new
  warnings and unchanged relevant hotspots; explicitly disposition each concern
  with a concrete simplification or reason to retain coherent ownership/order.
  Do not extract helpers solely to lower scores. Keep metrics and investigation
  reports under ignored `target/`, not in maintained guides or a completed plan.
- At both integration gates and after cleanup, review all earlier commits/fixups
  alongside surrounding callers, failure paths and tests. Check whether recurring
  mistakes require a wider audit, regression, simpler representation or changed
  design. Record concise review conclusions; passing tests or earlier approvals
  alone do not establish simplicity or correctness.
