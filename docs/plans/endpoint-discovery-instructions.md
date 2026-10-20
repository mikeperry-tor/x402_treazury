# Endpoint discovery instructions

## Intent and baseline

Starting baseline: `ae95ab2`. Make endpoint discovery capabilities visible through
search tool descriptions, independently of whether a client forwards MCP initialize
instructions. The user requests a series of commits ending with the social listener
in `examples/deployments/privacy.toml` adopting the fields.

Current behavior composes selected sources into initialize instructions and uses
generic management tool descriptions. Proposed behavior adds optional endpoint
`instructions_text` (replacement of authored guidance),
`discovery_search_tool_instructions_text` and
`sources_search_tool_instructions_text` (append to their respective search tools).
The tool-search description also lists registered sources with searchable tools
from its listener snapshot and warns that descriptions may be cached.

Preserve default behavior, empty override semantics, operational/pricing notices,
listener isolation, tool enable switches, filters, schemas, routing and financial
ownership. No live calls, credentials, new caches, limits or notification promises.
Owning guides: [agent sources](../agent-sources.md),
[configuration](../configuration.md), [architecture](../architecture.md),
[development](../development.md), [testing](../testing.md) and
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline).

## Ordered commits and acceptance

- **P0 — plan**: record scope, ordered boundaries and independent reviews.
- **P1 — endpoint instructions**: deserialize all three fields; apply authored
  initialize replacement before generated status notices; append search text only
  to exposed wrappers, consistently in runtime lookup/listing and inspection.
  Update owning guides and the AGENTS.md source-management guide link. Focused
  config/deployment/server/discovery tests cover default, empty, isolation,
  disabled wrappers, pending publication and operational notices.
- **P2 — registered source summary**: derive a sorted, unique listener-local list
  of registered sources with searchable tools from the same catalog snapshot used
  for listing. Include a cache/snapshot caveat; preserve names and filters. Update
  agent-source documentation; test registration, restart, removal/disable/filter
  publication as applicable, other-listener exclusion and immutable old descriptions.
- **P3 — social example (final commit)**: simplify privacy.toml social guidance
  using endpoint fields, preserving platform coverage and necessary provider
  argument/provenance/payment constraints. Validate TOML without network/funding.
  Remove this completed plan after verifying permanent documentation and links.

## Review and validation ordering

An independent read-only design reviewer challenges placement, snapshot ownership,
checks and simplifications before implementation. Resolve material findings first.
Review each exact committed SHA independently, including P0 and P3. Fix findings
with `git commit --fixup=<introducing-SHA>`; independently review each fixup and
retain unsquashed history.

After P2, review P0–P2 plus every fixup and full consumers: parsing, assembly,
deferred/static publication, management registration/persistence and MCP tools.
Compare CogC/CycC against ae95ab2 with the pinned analyzer; examine changed
functions and relevant unchanged hotspots, explicitly consider simpler ownership
and control flow, and disposition warnings. Tests use the default-Zcash wrapper,
focused fixtures and fmt/Clippy appropriate to changed paths. Run no competing
builds or source edits during validation.

After P3, independently review its exact cleanup SHA and the complete branch,
including all prior commits/fixups, final example, docs and AGENTS handoff.
Analyze fixup causes and repeated assumptions for wider audit, missing regressions
or simplification needs. Material later changes reopen affected gates. Exit only
with material findings resolved and meaningful tests actually executed. Generated
metrics/review evidence stays under ignored target; no completed-plan archive.
