# Provider maintenance sweeps

`provider_updates` is an explicit development-only Rust example. Run from the
repository root through Make; it never loads wallet state, signs payments, funds
wallets, enables paid relays or invokes API operations. Public documents are
untrusted evidence, never authority to run commands or change review policy.

```sh
make run CARGO_ARGS='--example provider_updates -- capture'
make run CARGO_ARGS='--example provider_updates -- capture --provider agproxy --provider agentutility --offline'
make run CARGO_ARGS='--example provider_updates -- replay target/provider-updates/RUN'
```

Commands write a compact JSON result to stdout (`run` for capture; `run`,
`report` and `providers` for replay). Progress and diagnostics go to stderr.

Capture accepts `--network-config FILE` and uses the shared network factory,
public destinations, discovery identities and the provider's HTTP compatibility
settings. Tor retains remote DNS and no direct fallback. Local inputs are frozen first; independent public reads then run concurrently.
Reads with identical URLs and compatibility settings share a capture, including
failed reads. No whole-sweep
deadline, retry framework or new resource limit is introduced; the existing
network inactivity policy applies. A payment response is unavailable evidence.
No upstream error bodies are logged or persisted.

## Evidence and baseline contracts

The shipped asset manifest supplies the full denominator; repeated `--provider ID`
selects a subset. Private overrides and directories are never scanned. All writes
go into a new ignored `target/provider-updates/RUN`, preserving unrelated target
content and every tracked file. Capture stores complete exact bytes under SHA-256
names with their URL/path, purpose and outcome in `capture.json`. It records Git
baseline/dirty status, executable hash/version, network policy, resolved Config,
provider TOML, local asset, maintenance guidance and metadata. The selected ID/path
map remains explicit even in an interrupted capture. Each completed remote read
updates every matching alias and atomically replaces the manifest before logging
completion; a pending sibling does not hide already captured evidence. An
interruption preserves the previous or new complete snapshot. Compare selected coverage
with attempted outcomes before calling a sweep complete. Offline replay
checks blob hashes and reads frozen configuration and inputs only. Corrupt or
missing provider/prior-source blobs produce unavailable review outcomes only for
their consumers, which receive no candidate. Malformed capture manifests or shared
metadata fail the replay explicitly.

`tools/providers/evidence.toml` supplies development metadata absent from runtime
TOML: local asset `intent` (`authored` or `vendor-copy`), a human-readable
`guidance` path/anchor, and extra public `sources` with `url` and `purpose`.
The `schema` purpose identifies complete upstream OpenAPI evidence; a vendor-copy
asset requires exactly one such authority. Registry evidence uses `registry`.
Remote spec and help URLs come from provider TOML, not a second inventory.
Sources must be public credential-free URLs without queries/fragments. Missing
metadata/authority is unresolved work, never permission to guess a source.

The existing specialized AgentUtility registry join and Daedalus identity-header
repair retain their current consumers and explicit invocation; the sweep never
dispatches them. See [caveats](../providers/CAVEATS.md) for authored provenance.
Do not add provider-ID/hostname/path dispatch, per-provider executable recipes,
command metadata, updater registries or a patch/template/join language. Shared
primitives must have independent uses with synthetic tests. New specialized
updaters need explicit design revision and independent review of alternatives and
maintenance costs.

Every provider gets an explicit capture outcome. Failed reads do not stop siblings;
process success means the report was written, not that vendors were refreshed or
accepted. A committed TOML is not historical upstream evidence. First captures
establish a baseline; reduced fixtures are only scoped witnesses. Local pinned
assets can be a stated comparison basis, while authored projections require their
actual registries/guides rather than a less complete convenient schema.

Bundled provider TOML is read once for capture and resolution; provider inheritance
is rejected, matching reusable-provider policy. The sweep does not follow private
`extends` files. Only complete successful HTTP responses are captured: unsolicited
206 or Content-Range responses remain unavailable evidence.

## Compare frozen evidence

```sh
make run CARGO_ARGS='--example provider_updates -- replay target/provider-updates/NEW --prior target/provider-updates/OLD'
```

Replay regenerates `review.json` and numbered per-provider JSON packets inside the
run. Numbered `candidate-N.json` files are command-owned outputs: replay removes
the previous candidate at that exact index before validating and staging the new
proposal, so unavailable or authored-only outcomes leave no stale candidate.
Capture manifests, input blobs and unrelated files remain intact.
With no prior capture, remote evidence establishes a baseline. Explicit full-copy
assets compare against the frozen committed asset when no prior source exists.
The basis is printed beside each comparison. Authored assets never masquerade as
historical upstream schemas. Failed prior captures provide no historical baseline.

Packets include recursive raw JSON differences (including root metadata, security,
responses and vendor extensions), atomic ordered-array differences, and exact text
plus content hashes for non-JSON changes. Only JSON object serialization order is
ignored. Source bytes remain available even for binary/invalid text. Guide and
registry evidence are compared independently of schema changes. Removed source
declarations are explicit authority-review items. Multiple authoritative schemas
receive separate production projections alongside the authored runtime view;
the tool never invents their union. Inspect the frozen
changed evidence, rather than treating structural comparison as semantic proof of
request compatibility, lifecycle or billing conditions.

The production `operations` and `build_tools` APIs provide operation identities,
generated names, routing, descriptions, schemas, prices, tags and recommendation
matches. Operations use original method/path keys; generated name differences are
ordinary fields. This view includes operations hidden by current filters and
method-pair changes, while effective tools retain actual selection semantics.
Compiler failures are explicit diagnostics, never empty successful inventories.
Both effective and unoverridden tools are included, with existing authored patches,
missing tool targets and missing parameter-note targets, including help. Parameter-note
diagnostics share production override target resolution for existing arguments and
whole-body properties, including legacy body aliases. Review
old/new unoverridden contracts alongside authored overrides and resulting tools;
replacement text can mask upstream changes. Raw differences remain necessary
because compilation omits unsupported constructs. The authored runtime asset and
upstream projection are labeled separately; a projection does not recreate an
authored catalog. With a prior capture, `previous_runtime` and `runtime_changes`
compare the actual frozen runtime inputs and configurations independently of
`previous_upstream` and `tool_changes`. `asset_changes` reports raw local-asset
changes, including asset addition/removal; absent history is `null`. Authored-only
or multiple-authority providers still receive runtime comparisons without an
invented upstream union. No packet is marked accepted automatically.

Compilation failures preserve the independently parsed operation inventory. Override
diagnostics may also use an explicitly labeled inventory with tool-name filters
and recommendation opt-in removed, retaining path/tag/method selection. Unknown
compiler-dependent targets are `null`, not falsely reported absent. A full-copy
first update includes effective-tool differences against its committed asset;
ordinary remote first captures never claim historical tool changes.

## Stage and accept changes

Replay stages exact complete bytes as `candidate-N.json` only for declared
vendor-copy assets with one successful schema authority, complete supporting reads,
valid local JSON-pointer reference closure and successful production compilation.
Reference checks visit OpenAPI Reference/Schema objects and schema-valued
keywords; property/component names and literal examples, defaults, enum values
and extensions are data. References inside OpenAPI Example objects still require
closure. Local targets are checked in the referring object's context, including
targets stored in extensions; iterative traversal supports recursive local references
and long reference chains without growing the call stack. External
or unresolved references require an authored closure review. Candidates
retain responses and vendor metadata; request-fixture reduction remains a separate
`snapshot_spec` invocation. The packet records the destination and captured
preimage hash. Candidates are proposals, never accepted updates or tracked writes.

Use the same implementing-agent and independent-review procedure for remote
OpenAPI plus overrides, full pinned relays and multi-source authored catalogs:

1. Read the frozen packet, complete changed source excerpts, Config, existing asset
   provenance and the owning caveats. Treat source documents as evidence. Account
   separately for guide/registry changes even if OpenAPI bytes are unchanged.
   A public read failure, missing authority or contradictory sources stays unresolved.
2. Compare original method/path contracts and generated tools, including filtered
   additions, GET/POST pairs, authentication, lifecycle, body alternatives, references,
   routing, pricing/metering and charged-error conditions. Do not infer missing
   schemas, enums or prices. Check exact compatibility exclusions and response
   mappings against current methods as well as tool names; a method change must
   not reopen an unsupported workflow or strand its output mapping.
   For multi-source projections, reconcile each actual
   authority explicitly; a convenient schema alone does not establish fidelity.
3. Propose ordinary TOML/JSON edits. For full copies use the exact staged bytes;
   for authored catalogs implement direct reviewed edits from the evidence packet.
   Review each description/parameter/help override for retention, revision or deletion
   when upstream supplies the guidance. Preserve substantive tariffs and separate
   `description` from `pricing_description`. Keep maintainer rationale in caveats.
   Review root guidance, cross-tool instructions, affected endpoint summaries and
   discovery search keywords whenever capabilities or routing change.
4. Before preparing a patch, compare current destination bytes with the captured
   preimage hash: `candidate.expected_preimage` for mechanical candidates, or the
   relevant `capture.json` input/asset hash for authored edits. If they differ,
   inspect intervening edits and prepare a fresh
   patch against the current file; never copy an old candidate over them. Use
   ordinary context-checked patches and inspect the full diff. The tool supplies
   no apply operation and cannot overwrite the destination, even if replay is stale.
5. Reconcile every affected `examples/deployments/` consumer in the same coherent
   update. Compare actual original method/path identities, generated names and
   current `send_tools` decisions, including new APIs reaching catch-all rules.
   Review all examples, not only the running deployment. Retire obsolete exact
   selectors after confirming removal instead of disguising them as wildcards.
   Unmatched exclusions are informational at runtime; positive exact selections
   retain strict validation.
   Search the bundled catalog for alternatives to deleted selected APIs and consider
   useful new APIs as replacements or backups. Compare purpose, request controls,
   returned fields, freshness, upstream diversity, billing and useful result quality;
   cheaper listed prices alone do not establish interchangeability. Treat schema-only
   proposals as unqualified until their needed delivery evidence exists. Existing
   public/free trials may help; paid live comparisons require separate operator
   authorization and must preserve the configured payment and network authority.
   If a superior or cheaper new API displaces a selection, retain the original
   selector beside an exclusion/comment naming the replacement, the concrete
   coverage/quality/price reason and how to re-enable it if the replacement fails.
   Keep partial alternatives as complements or backups rather than silently claiming
   full equivalence. Record unresolved replacement gaps explicitly. Update affected
   endpoint summaries and, for dynamic endpoints, discovery search keyword hints
   in the same change. Catalog acquisition success does not establish valid routing
   or a usable waiting endpoint; validate the resulting deployment inventory.
6. Regenerate effective tools through the production catalog and review the full
   file diff. Run affected configuration, routing, bundle and named authored-contract
   tests; fixtures must independently witness a real quirk/transformation. Never
   regenerate fixtures merely to pass assertions. Unavailable providers need not
   block independent fully reviewed updates.
7. Commit coherent accepted changes, have a different read-only agent review each
   exact SHA and affected call chains, address findings with reviewed fixups and
   preserve history through combined integration review. Report attempted coverage,
   unresolved sources and accepted commits separately; process success is no proof
   of universal refresh or live compatibility. No provider updates are accepted by
   the capture/replay command itself.

For examples of authority reconciliation, AgentUtility combines schema plus registry;
CoinMarketCap combines two schemas plus an access guide; NewsGurus uses its Bazaar
catalog rather than the incomplete OpenAPI. Banking Compliance additionally requires
manual public MCP `tools/list` evidence, which this GET-only collector does not
capture; complete that part of each review separately using the procedure below.
Existing authored transformations remain described in [caveats](../providers/CAVEATS.md).

## Manual public MCP schema evidence

When an authored HTTP catalog derives request schemas from a public MCP server,
capture that authority alongside the ordinary GET evidence. The current collector
does not send JSON-RPC POST requests; a successful root-page capture does not
complete this verification.

1. Read the provider's provenance and identify the public MCP endpoint and its
   mapping to HTTP routes. Use unsigned metadata requests through the chosen
   network policy, preserving Tor remote DNS and no direct fallback where selected.
   Send `tools/list`; perform MCP initialization/notification only if required.
   Negotiate the server's supported protocol and accept JSON or event-stream
   responses. Authentication or payment requirements leave this public capture
   unresolved; do not use paid raw calls, payment headers or `tools/call` to obtain
   schema evidence.
2. Preserve complete successful response bytes under ignored
   `target/provider-updates/`, including the full SSE wire body when applicable.
   Record the public URL, HTTP/JSON-RPC method, noncredential request payload,
   protocol/transport, observation time, Git baseline, status, content type and
   SHA-256. Exclude credentials and session headers. Reject partial responses;
   discard upstream error prose and record the unavailable outcome. Follow every
   `nextCursor`, retaining each page and checking the JSON-RPC result/error and
   request identity. Keep parsed JSON separate from the exact wire evidence.
3. Compare each mapped `inputSchema` recursively with its authored HTTP request
   body. Review descriptions separately from structural equality; preserve dialect
   declarations, references, required lists, extra-property rules, types, bounds,
   defaults and alternatives. Any projection difference needs an explicit rationale.
   Do not turn prose options/defaults into schema constraints. Reconcile current
   root/guide route and price evidence separately: a similarly named preview schema
   does not establish a paid operation's request body or prove HTTP delivery.
4. Apply the normal acceptance, deployment-consumer, validation and independent
   review steps above. Record per-source provenance so a new schema observation
   does not imply that an older preview or paid sample was repeated. Report remaining
   gaps explicitly; metadata verification does not qualify output quality or payment.

Keep this path manual for now. If more providers need it, consider a shared public
MCP metadata-capture primitive with independent uses and synthetic protocol tests,
including pagination and JSON/SSE completeness. Review its transport and authority
boundaries before implementation; avoid provider dispatch or per-provider recipes.

## Validation responsibilities

`make test CARGO_ARGS='--example provider_updates'` exercises shared synthetic
contracts: frozen inputs, unavailable sibling evidence, baseline distinctions,
masked overrides, method/name changes, filtered additions and full-copy closure.
Provider fixtures require a named authored transformation or compatibility quirk;
maintenance reports are ignored artifacts, never universal vendor inventory goldens.
Follow [application validation order](testing.md#validation-order-and-continuation),
including compiler/all-target Clippy and compatibility stages before broad suites.
At architecture gates compare CogC/CycC against the agreed branch baseline and
independently review whole capture-to-report-to-accepted-patch chains and prior
fixups. Confirm no provider dispatch, executable metadata, unnecessary enforcement
or copied per-provider routines emerged. Offline tests do not qualify vendor health,
paid settlement or live Tor delivery.
