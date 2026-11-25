# Provider maintenance sweeps

`provider_updates` is a development-only Rust example run through Make from the
repository root. It never loads wallet state, signs payments, funds wallets,
enables paid relays, or invokes API operations.

```sh
make run CARGO_ARGS='--example provider_updates -- capture'
make run CARGO_ARGS='--example provider_updates -- capture --provider agproxy --provider agentutility --offline'
make run CARGO_ARGS='--example provider_updates -- replay target/provider-updates/RUN'
```

Commands print one compact JSON summary to stdout (`run` for capture; `run`,
`report`, and `providers` for replay) and send progress and diagnostics to
stderr.

Capture accepts `--network-config FILE` and uses `src/network.rs` with public
destinations, discovery identities, and each provider's HTTP compatibility
flags (preserving Tor remote DNS with no direct fallback). Reads send no
`User-Agent` and follow no redirects (declare redirecting authorities at their
final public URL; hosts like CoinMarketCap that return HTTP `403` without a
`User-Agent` record unavailable evidence). Identical URL and transport pairs
share one capture. Capture records HTTP `402`, `206`, and `Content-Range`
responses as unavailable evidence without logging upstream error bodies.

## Evidence and baseline contracts

- **Scope and storage:** The bundled manifest (`src/providers/assets.rs`)
  supplies the full provider list; repeat `--provider ID` for a subset. Private
  local overrides are ignored. Capture writes only to a new ignored
  `target/provider-updates/RUN` directory, storing raw bytes under SHA-256
  names with URL, purpose, outcome, Git baseline, binary hash, frozen config,
  and safe response validators (`etag`, `last_modified`, `date`) in
  `capture.json`.
- **Incremental durability:** Each completed read updates matching aliases and
  atomically replaces the manifest so an interrupted capture preserves all
  earlier snapshots. Offline replay verifies blob hashes and isolates missing
  or corrupt provider blobs to their consumers while failing on malformed
  manifests.
- **Metadata (`tools/providers/evidence.toml`):** Declares local asset `intent`
  (`authored` or `vendor-copy`), `guidance` path and anchor in
  `providers/CAVEATS.md`, and extra public `sources` (`schema`, `registry`,
  `guide`, `discovery`, `source`, or `pricing`). Vendor-copy assets require
  exactly one `schema` authority. Remote spec and help URLs come from each
  provider TOML. URLs must be public, credential-free, query-free, and
  fragment-free; missing sources stay unresolved rather than guessed.
- **Validator metadata:** Replay prints `response` and `previous_response`
  (`etag`, `last_modified`, `date`) so consecutive sweeps distinguish body
  changes from validator changes. These are comparison data only, not runtime
  cache triggers (see [runtime source refresh](configuration.md#runtime-source-refresh)).
- **No per-provider dispatch:** Specialized offline tools (AgentUtility
  registry join, Daedalus identity-header repair) stay separate and explicitly
  invoked. Never add provider-ID or hostname dispatch, executable recipes, or a
  patch or template language to `provider_updates`.

## Compare frozen evidence

```sh
make run CARGO_ARGS='--example provider_updates -- replay target/provider-updates/NEW --prior target/provider-updates/OLD'
```

Replay writes `review.json` and numbered per-provider packets inside the run
directory, removing any prior `candidate-N.json` at that index before staging a
new candidate so stale proposals never linger.

- **Comparison basis:** Without `--prior`, remote sources establish a baseline
  (full-copy assets compare against their committed file). Failed prior reads
  never form a baseline.
- **Raw and compiled diffs:** Packets contain recursive JSON diffs (ignoring
  only object key order), atomic array diffs, exact text and hash diffs for
  non-JSON files, and production catalog projections. The comparison uses the
  same compiled inventory for unoverridden and effective tools. Incomplete
  catalogs report a compiler error in both views, with operation omissions and
  available targets shown in the diagnostic inventory. The comparison reports
  malformed override tables even if the catalog cannot compile. Parameter
  diagnostics include the target and the selected path when legacy spellings
  overlap.
- **Runtime vs. upstream drift:** `previous_runtime` and `runtime_changes`
  track local TOML, guidance, and asset changes separately from
  `previous_upstream` and `tool_changes`. Local edits alone report
  `no upstream evidence change`. A vendor-copy download identical in JSON
  semantics to the committed file produces `committed asset unchanged; no candidate`.

## Pricing evidence and variation review

Pricing hints are advisory display and selection metadata; live x402 challenges
always govern payment. See
[pricing metadata and runtime hints](configuration.md#pricing-metadata-and-runtime-hints).

### Pricing capture

- Freezes `[pricing]` and `pricing_key` from provider TOML and captures
  configured external `[pricing] source` URLs plus `purpose = "pricing"` entries
  from `tools/providers/evidence.toml` as separate advisory reads.
- Pricing capture failures never block schema candidates or alter schema
  capture outcomes. External pricing URLs with query strings work at runtime
  while recording unavailable capture evidence (do not rewrite working TOML for
  that). No paid calls are made, and offline capture marks pricing not
  attempted.

### Pricing in packets

- Reports effective `catalog_key`, `pricing_metadata_operations`,
  `total_operations`, generated pricing text, `pricing.evidence` (root shapes,
  statuses, and diffs), `pricing.relocation` (empty catalog key or changed
  external source), and `pricing.signals` (bounded sorted JSON pointers whose
  keys match pricing terms across remote documents, excluding local files and
  values).
- Vendor generation timestamps (`generatedAt`, `lastUpdated`) can change on
  every read; inspect structural and amount fields separately. Pricing evidence
  never stages a schema candidate.

### Pricing review steps

1. Verify the pricing authority and adapter (`payment-info`, `x402`,
   `x402-resources`, or `x402-discovery`) match the observed document shape.
2. Check `key`, `unit`, `usd_as_usdc`, asset and network, fixed vs. dynamic or
   metered semantics, and the vendor's per-call billing contract. Do not turn
   prose examples into schema constraints.
3. Inspect `pricing.relocation`, `pricing.signals`, and raw diffs for moved or
   renamed fields, then verify the raw document directly.
4. Confirm external document mapping leaves ambiguous or unmatched operations
   unknown rather than guessing or summing alternatives.
5. Keep hints advisory: unknown is not free, and stale hints never block
   authorized calls.
6. Update `[pricing]`, `evidence.toml`, `pricing_description`, and
   [provider caveats](../providers/CAVEATS.md) together.
7. For full-manifest sweeps (omitting `--provider`), report per-provider
   outcomes (unchanged, updated, relocation unresolved, no authority captured,
   missing metadata) separately from accepted edits.

### Pricing coverage triage and adapter gaps

Compare `pricing_metadata_operations` against `total_operations` and inspect
generated pricing text: a key may exist on every operation while rendering
`Cost: unknown.` for an unsupported shape. Use `pricing.signals` to locate
candidate fields and inspect the raw object.

Add a new format adapter only when it serves multiple independent providers
(after independent design review); keep one-off shapes in `pricing_description`
and `providers/CAVEATS.md`. Open adapter gaps are tracked in the
[custom pricing adapters plan](plans/custom-pricing-adapters.md).

### Review outcomes and manual review

| Packet outcome | Meaning |
| --- | --- |
| `source unavailable` | At least one required public read failed (for example, CoinMarketCap `403` without `User-Agent`). Retry once for transient errors, then leave unresolved. |
| `baseline established` or `authored baseline established` | No valid prior comparison exists (or a newly added or recovered read). Review current evidence for consistency. |
| `no upstream evidence change` or `authored no upstream evidence change` | Comparable upstream source, pricing, and asset evidence is unchanged (local edits still show in `runtime_changes`). |
| `authored comparison ready; human review pending` | Upstream or asset difference exists for a locally authored `spec`; apply reviewed edits manually. |
| `candidate ready; human review pending` | Vendor-copy asset with complete reads and valid local references is staged as `candidate-N.json`. |
| `committed asset unchanged; no candidate` | Current vendor document matches the committed asset semantically. |

For authored providers (reconciling multi-source registries, guides, or local
projections):

1. Confirm every declared schema, registry, discovery, and guide read succeeded.
2. Compare operation identities and prices per authority without merging
   conflicting alternatives by convenience.
3. Where no vendor OpenAPI exists, complete any required
   [manual public MCP capture](#manual-public-mcp-schema-evidence).
4. Edit local assets, TOML, and caveats only for concrete differences (ignore
   timestamp-only or array-order-only churn).
5. Revalidate dependent deployments with `config show` and `config check`.

## Stage and accept changes

Replay stages `candidate-N.json` only for `vendor-copy` assets with one
successful `schema` authority, complete supporting reads, valid iterative local
JSON-pointer reference closure (including OpenAPI Example objects and local
extension targets), and a complete catalog. Staging checks override value
types and edits to compiled tools. The comparison packet identifies absent
override targets for review. Candidates keep full vendor documents
(`snapshot_spec` reduces request-only fixtures separately) and record
`expected_preimage` hashes; the tool never writes tracked files.

Acceptance workflow:

1. **Read evidence:** Inspect the frozen packet, changed source excerpts,
   resolved config, asset provenance, and owning caveats. Review guide and
   registry changes even when OpenAPI bytes did not change.
2. **Audit contracts:** Compare original `METHOD path` contracts, generated
   names, filtered additions, `GET` and `POST` pairs, auth, lifecycle, body
   unions, pricing, exclusions, and `response_mappings` so method changes never
   reopen unsupported workflows or strand mappings.
3. **Edit files:** Copy exact staged bytes for full vendor copies or apply
   reviewed edits for authored catalogs. Review description and parameter
   overrides, keep `description` separate from `pricing_description`, and
   update `instructions_text`, endpoint summaries, and discovery search keywords
   when capabilities or routing change.
4. **Check preimage hashes:** Verify current file bytes match
   `candidate.expected_preimage` (or `capture.json` hashes) before patching so
   intervening edits are never overwritten.
5. **Reconcile deployment consumers:** Update every dependent deployment under
   `examples/deployments/`, checking catch-all `send_tools` rules and retiring
   removed exact selectors instead of masking them with wildcards. Follow the
   [deployment tool selection workflow](#compare-and-select-deployment-tools)
   when adding or displacing tools.
6. **Verify:** Run matching config, routing, bundle, and authored-contract
   tests (never regenerating fixtures solely to pass assertions).
7. **Commit and review:** Commit coherent changes, have a fresh independent
   reviewer check each SHA and complete call chains, and address findings with
   `--fixup` commits. Report attempted coverage, unresolved sources, and
   accepted commits separately.

Multi-authority examples: AgentUtility joins schema and registry; CoinMarketCap
combines two schemas and a guide; News Gurus uses its Bazaar catalog; Banking
Compliance also needs manual public MCP `tools/list` capture below.

### Compare and select deployment tools

1. **Find the current coverage.** Inspect the destination's effective catalog
   and search bundled providers for alternatives. Compare supported operations,
   filters, response fields, source coverage, and freshness. Use the configured
   toolbox to register and inspect remote candidates on demand.
2. **Compare matched calls when authorized.** Use the same query, filters,
   result budget, and period where the APIs allow it. Compare stable IDs,
   ordering, fields, timestamps, partial results, and downstream handoffs.
   Record observed offers with their input shape, keeping fixed, variable, and
   metered pricing distinct. Compare coverage and quality before price.
3. **Route the selected tools.** Put new reusable catalogs, pricing, and
   compatibility guidance in [provider definitions](../providers/README.md#provider-policy).
   Import them with `extends` and keep deployment selections in
   [`send_tools`](configuration.md#source-tool-distribution). Add distinct
   coverage or a useful alternate and keep tools that serve a separate task.
   When replacing a provider tool in a deployment such as Research Desk,
   leave its exact route commented beside the active rules. Name the
   replacement and explain the specific reason for the switch, such as a
   lower price for comparable results. Say when the old route is worth
   restoring. Scope the reason to the modes and cases you compared.
4. **Check discovery and routing.** Inspect `config check --explain-routing`
   for the changed sources and test representative search queries against
   the destination's inventory. Follow the
   [endpoint search hint guidance](agent-sources.md#endpoint-instruction-placement)
   when adding terms for new capabilities.

## Manual public MCP schema evidence

When an authored HTTP catalog derives request schemas from a public MCP server
(such as Banking Compliance), capture `tools/list` alongside normal GET
evidence:

1. Use unsigned metadata requests through the configured network policy (no
   payment headers, raw paid calls, or `tools/call`). Negotiate MCP protocol,
   initialize if required, and follow every `nextCursor` page. Auth or payment
   prompts leave the capture unresolved.
2. Save complete wire bytes (including full SSE bodies) under ignored
   `target/provider-updates/` with URL, method, payload, transport, timestamp,
   Git baseline, status, content type, and SHA-256 (excluding credentials and
   session headers). Keep parsed JSON separate from wire evidence.
3. Recursively compare each mapped `inputSchema` against the authored HTTP
   request body (preserving dialect, references, `required`, extra-property
   rules, bounds, defaults, and alternatives), and reconcile route and price
   docs separately.
4. Follow the standard acceptance, deployment-consumer, validation, and
   independent-review steps above, recording per-source provenance and open
   gaps.

## Validation responsibilities

`make test CARGO_ARGS='--example provider_updates'` exercises frozen inputs,
response validators, unavailable siblings, baseline states, unchanged
candidates, masked overrides, method and name drift, local reference closure,
external pricing isolation, relocation flags, advisory pricing signals, and
corrupt blob handling. Follow
[validation order](testing.md#validation-order-and-continuation), verify no
provider-ID dispatch or executable recipes were introduced, and remember that
offline tests do not qualify vendor health, paid settlement, or Tor delivery.
