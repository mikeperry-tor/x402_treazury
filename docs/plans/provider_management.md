# Provider management and remote directory search

## Intent and baseline

Give operators independent control over bundled-provider management, remote
provider management and direct URL calls. Agents enumerate bundled definitions
separately from remote discovery, and one remote search queries every directory
selected by the operator. Preserve native directory responses and the existing
registration, payment and network ownership contracts.

Starting implementation baseline: `4d287f58266828c1234d36f0876d927605252d1c`.
Reconcile intervening changes before implementation. The working tree contains an
untracked user-owned `TODO`; preserve it and all unrelated files. This plan
describes unfinished work, with no authorization for live purchases or funded
qualification.

Current implementation uses `source_management` to grant local discovery,
arbitrary specification imports, remote directory searches, details and raw calls
together. `sources_search` combines local browsing with x402 List browse/best and
curated Coinbase search. Coinbase search currently hardcodes `curatedOnly=true`.
`source_details` accepts `slug`, `base_url` or `documentation_url`.

No compatibility aliases or migration layer are required for replaced settings,
tool names or arguments. This does not authorize changing persisted registration
formats, accepted identities, signed bytes, financial evidence, generated pool
names, Tor token domains or `X402_*` environment contracts.

## Owning guides

Read [agent source management](../agent-sources.md),
[configuration](../configuration.md),
[runtime architecture](../architecture.md),
[bundled provider metadata](../../providers/README.md#bundled-provider-metadata),
[provider policy](../../providers/README.md#provider-policy),
[provider caveats](../../providers/CAVEATS.md) and
[provider regression policy](../testing.md#provider-regressions).
Network and accepted-work changes also require
[network policy](../network-egress.md),
[resource limits](../local-resource-limits.md),
[best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules) and
[wallet observation policy](../wallet-rotation.md#wallet-observation-policy).
Use [development](../development.md), [test permissions](../testing.md#sandbox-permissions),
[Make entrypoints](../../scripts/README.md) and
[complexity review discipline](../../tests/COMPLEXITY.md#review-discipline)
for implementation evidence.

## Endpoint preferences and authority

Replace `source_management` with these fields directly under `[servers.NAME]`:

| Field | Default | Authority |
| --- | --- | --- |
| `provider_management_local` | `false` | Enumerate/search bundled definitions and register by bundled ID. |
| `provider_management_remote` | `false` | Search configured remote directories, inspect remote provider details and register public HTTPS OpenAPI specifications. |
| `remote_provider_directories` | `[]` | Directory choices for remote search: `coinbase`, `coinbase_curated`, `x402-list`. |
| `raw_url_calls` | `false` | Invoke documented public HTTPS URLs through the existing raw-call tool. |

```toml
[servers.research]
provider_management_local = true
provider_management_remote = true
# coinbase_curated searches only Coinbase's curated subset; coinbase includes both.
remote_provider_directories = ["coinbase", "x402-list"]
raw_url_calls = true
```

The directory list permits none, all three or any subset. It grants no authority
when remote management is disabled. Validate directory identifiers during
configuration parsing. Repeated identical entries designate the same directory
once; the two Coinbase choices remain distinct. Do not add a separate concurrency
preference or resource limit.

Remote management with an empty directory list still permits remote registration
and URL-based details; omit the remote-search tool. Directory selection controls
built-in directory discovery, not a general destination allowlist for registered
APIs or otherwise permitted raw calls.

Each enabled management/raw capability uses the endpoint's explicitly configured
existing wallet, retaining current wallet validation and payment admission. Local
listing itself acquires no payer identity and makes no network requests. Bundled
registration may fetch a remote schema. Raw-only and local-only endpoints must
work without remote-management permission. Enabling capabilities does not itself
allocate or fund a wallet; normal serving startup/funding remains unchanged.

Use one simple endpoint policy derived from these preferences for advertising and
operation checks. A shared manager/client helper must not require an unrelated
capability. Check the requested operation before identity selection, upstream I/O
or registration side effects. Avoid treating tool hiding as the authority check,
or adding a second mutable permission registry.

## Agent tools

Retain the `x402_treazury_` namespace; names below omit that common prefix.

| Tool | Exposure and input |
| --- | --- |
| `providers_list_local` | Local management. Optional `q`, `page`, `per_page`; omitted/blank query enumerates bundled providers using current pagination and metadata. Nonblank queries retain existing BM25 matching. |
| `providers_add_local` | Local management. Required `provider_id`, optional `name`. |
| `providers_search_remote` | Remote management and nonempty directory list. Optional `q`, `limit`, `max_price_usd`; `limit` means results requested from each directory, default 5, range 1–20 from the ranked upstream contracts. |
| `providers_add_remote` | Remote management. Required `spec_url`, optional `name`; accepts public HTTPS OpenAPI 3 JSON specifications. |
| `provider_details_remote` | Remote management. Exactly one `x402_list_id`, `base_url` or `documentation_url`; expose the ID input only when `x402-list` is configured. |
| `raw_call` | `raw_url_calls`. Preserve its current method/URL/query/body contract. |
| `tools_search`, `tool_call` | Preserve existing catalog behavior. Expose wrappers when either management capability is enabled or `discover_on_demand=true`; raw calls alone grant no registration authority. |

Remove old tool names and the old `local_only`, directory-selector, browse-mode
and ranking/filter arguments from remote search. Do not advertise unavailable
tools, parameters or follow-up actions in standard descriptions or result
guidance. Operator-authored instruction text remains authored text, not a new
permission source. Keep the existing `discovery_search_tool_instructions_text`
setting for endpoint tool search; replace
`sources_search_tool_instructions_text` with
`providers_search_remote_tool_instructions_text` for the renamed remote search.
Update parsing, configuration display, inspection and serving together.

Apply the maintained [tool documentation policy](../agent-sources.md#tool-documentation)
to every tool above and every advertised parameter. Keep help concise and focused
on choosing an API, supplying valid inputs and using the result. Omit internal
dispatch, transport, storage, ranking-algorithm and payment-plumbing details unless
they change the caller's decision or required action. Preserve necessary price,
possible-charge, retry and result-coverage guidance without repeating boilerplate
across descriptions. Give every parameter a similarly concise explanation of its
purpose, expected value and meaningful omission/default or interaction behavior.

Both `providers_*_local` descriptions must identify the definitions as local,
bundled and reviewed, with compatibility filters and repairs for known compatible
API operations. Listing is offline; adding registers the prepared definition.
This is the benefit of using those definitions, not a promise that every upstream
is currently available or has passed live paid qualification. Do not put a
repeated qualification disclaimer in the tool help. Suitable concise descriptions:

- `providers_list_local`: "Find locally bundled provider definitions, reviewed,
  filtered and repaired for compatible API use. Use a provider_id to add one."
- `providers_add_local`: "Add a locally bundled provider using its reviewed,
  filtered and repaired API definition. Find its callable tools with tools_search."

Tailor follow-up guidance to exposed tools. Parameter help should explain `q` as
capability keywords, `provider_id` as the exact local definition ID, and `name` as
an optional registration name; keep detailed ranking and import mechanics in
maintained documentation. Review all descriptions and parameters together to
remove unnecessary internal detail before final integration.

Do not introduce another general search tool or expose the remote directory list
as an agent-selectable argument. Advanced x402 List browse/filter operations remain
available through its ordinary provider definition when explicitly configured or
registered; leave that reusable provider's supported operations intact.

## Remote search and response contract

Start the selected directory workflows concurrently without a new queue or gate.
Validate shared input before starting any workflow. When either Coinbase choice
is selected, advertise and enforce its existing 400-character query bound; do not
apply that Coinbase-specific bound to x402-list-only endpoints.

| Directory key | Upstream request |
| --- | --- |
| `coinbase` | Coinbase resource search with `curatedOnly=false`. |
| `coinbase_curated` | Coinbase resource search with `curatedOnly=true`. |
| `x402-list` | x402 List `/best`, `prefer=balanced`, `require_verified=false`. |

Map the common query, limit and price filter to the native request fields.
Preserve current server-selected Base mainnet USDC/exact constraints for Coinbase
and Base selection for x402 List. Price filters are advertised-price discovery
filters, not spending authority: x402 List filters a service's minimum listed
price while Coinbase filters resources. Do not imply identical pricing coverage.

Return one JSON dictionary keyed by the configured directory identifiers. Each
successful value is the complete upstream JSON object. Preserve arrays/order,
unknown fields, missing metadata, native counts, partial-result indicators,
verification fields and provenance. JSON whitespace need not be byte-identical.
Do not add Coinbase fields, acquisition wrappers, curation flags, provider matches,
shared scores, deduplication, merged lists, global totals or synthetic pagination.
When both Coinbase choices are configured, query both and retain both responses.

For a failed directory, return a safe, explicitly local error value under that
directory key; do not invent an empty upstream result or discard sibling results.
Distinguish local failure values from successful upstream objects in the documented
contract. Preserve ordinary safe diagnostics, possible-charge information and
required attribution without mutating successful values: reuse `ToolOutput`
advisories and its aggregate typed payment evidence. Review whether any adapter's
fallible JSON/output conversion must carry forward a successful sibling's possible
payment. Non-JSON/invalid upstream responses are explicit per-directory failures,
without credential-bearing upstream prose in errors or logs.

No automatic federation replay follows a failure, empty result or partial result.
Retain existing per-workflow origin/Tor/relay retry rules and accepted financial
ownership. An outer waiter disconnect cannot cancel accepted paid child work;
one failing directory cannot cancel a progressing sibling. Add no whole-search
deadline, combined response truncation, discovery cache or background index.
Existing per-response bounds and network inactivity policies continue to apply.
Document that one search can authorize more than one directory/relay charge.

## Remote provider details

Rename `source_details` to `provider_details_remote` and rename its `slug` input
to `x402_list_id`. Describe the value as the exact `slug` returned by x402 List;
the upstream API still receives its native slug. It is neither a registration ID
nor a Coinbase resource URL. Remove the old input and tool name, with no aliases.

Under the final preferences, require remote management for every mode. The
`x402_list_id` mode also requires `x402-list` in the directory list; remove its
schema branch and reject manually submitted IDs before identity lookup/I/O when
that directory is disabled. URL-based modes remain usable with Coinbase-only or
empty directory selections. Retain exact public-HTTPS validation, complete
plain/Markdown documentation reading, current fixed-path probes, relay disclosure
categories and accepted-work ownership. Add no `resource_url` argument.

Recommend trying a resource URL's origin root as `base_url` when no explicit API
base is available. For example, given
`https://api.example.com/weather/v2/forecast?city=London`, try
`base_url="https://api.example.com/"`. This searches the supplied root using the
existing mechanism; it does not call the resource operation. Explain that it is a
best-effort discovery location, not proof of the provider's API base. An explicitly
documented base path is preferable, and root discovery can miss documentation
hosted beneath another path or on a separate host.

Keep exact bundled API-base matching; trying a root must not broaden it into
hostname, path-prefix or inferred-operation matching. A Coinbase usage-document
URL can use `documentation_url` directly when supplied. Preserve Coinbase search
output unchanged, and never invent request schemas from resource examples.

## Registration and existing consumers

Reuse one internal import/commit path for both add tools; the tool's input and
operation permission distinguish registration routes. Before fetching, a unique
canonical full specification URL match in `providers_add_remote` uses the bundled
definition, including its settings and embedded schema where applicable, even
when local management is disabled. Do not match an API base, hostname or guessed
resource path as a specification. Query components remain significant.

Preserve reuse of an accepted generic URL registration before consulting current
bundle matches, provider-ID registration reuse, endpoint ownership, stable names,
persistence and atomic catalog publication. Ambiguous matches must not select
arbitrarily or recommend a disabled local tool: return available local choices
when local management permits them, otherwise safe operator-resolution guidance.
Do not add a remote `provider_id` escape hatch or new selector.

Audit persisted restoration and operator refresh/remove/inspect call chains when
replacing broad `enabled` predicates. Existing accepted registrations and evidence
must not be rewritten or erased to migrate preference names. Preserve their
existing visibility/filtering semantics and current payer bindings; new-operation
permissions are not a new per-origin or per-registration invocation policy.
Operator refresh of any accepted registration requires either local or remote
provider management; raw-call permission alone is insufficient. Do not infer
refresh authority from `candidate.provider`: a remote specification match can
produce a bundled candidate, and saved records have no import-route provenance.
Resolve the operation's authority before loading its existing selected wallet;
`ExistingRead` must support either management capability for authorized refresh.
Keep operator inspection/removal available for saved records regardless of these
preferences. Document these administration semantics in the owning guide.

## Ordered commits and acceptance

### P1 Rename remote details and explain root discovery

Rename the details tool and its ID parameter, with no legacy aliases. Update
`src/discovery/source_details.rs`, its directory constants/dispatch/schema/help,
registration guidance, and all relevant MCP, inspection and test consumers.
Keep existing `source_management` authority for this self-contained commit; P2
replaces it atomically. Do not expose final preferences before they are implemented.

Update permanent details and URL-discovery guidance in `docs/agent-sources.md`
and `docs/configuration.md`, plus existing workflow references. Include root-base
advice, limitations and examples in both agent-facing parameter help and the owning
guide. Audit AGENTS.md links if a maintained heading changes; prefer stable headings.

Acceptance evidence: focused details input/schema/dispatch tests; old tool and
`slug` input rejection; native upstream slug translation; root candidate URLs and
exact base matching; Coinbase documentation reading; no business-endpoint request;
retained details cancellation, relay/payment evidence and invalid-document tests.
Use small existing faithful fixtures, not inventories or live providers.

After tests and formatting, commit P1 and independently review that exact SHA and
relevant consumers. Gate G1 reviews its complete details-to-network/document and
registration-guidance chains, root/base semantics and CogC/CycC evidence against
the starting baseline. Resolve material findings before P2.

### P2 Replace permissions and implement grouped remote search

Replace the endpoint preference, registration/search tool surface and operation
checks together, including P1 details capability/schema gating. Implement both
Coinbase scopes, fixed balanced x402 List search and complete grouped output in
the same commit. Removal of the old behavior ships with its replacement.

Review `src/deployment.rs`, `src/deployment/existing_read.rs`, initialization and
deferred startup, `src/server.rs`, `src/discovery/{mod,policy,tools,local,directory,
coinbase,raw,source_details}.rs`, provider URL matching, registry publication,
operator source maintenance and the output/payment evidence consumers. This is a
consumer checklist, not a requirement to rename every internal module or type.
Prefer deleting combined schemas and redundant dispatch branches over adding
capability frameworks. Preserve startup binding, current wallet funding policy,
catalog readiness and unrelated static serving.

Update permanent docs with this commit: `docs/agent-sources.md` owns tool contracts,
permissions, grouped failures and details behavior; `docs/configuration.md` owns
defaults, endpoint fields and instruction placement; `providers/README.md` owns
bundle lookup semantics. Update the README, managed/comprehensive deployment
examples, relevant architecture/network/resource inventories, tests documentation
and affected unfinished plans/references. Keep both timer/resource inventories
current even where the only change is a renamed operation. AGENTS.md should link
the maintained provider-management and remote-details contracts from its dynamic
sources guide row; keep detailed rules in those owning guides.

In `examples/deployments/managed.toml` on `servers.research` and
`examples/deployments/comprehensive.toml` on its existing toolbox listener, enable
`provider_management_local`, `provider_management_remote` and `raw_url_calls`.
Set `remote_provider_directories = ["coinbase", "x402-list"]` in both examples.
Describe `coinbase_curated` as the curated-only alternative in a comment, but do
not include it in either configured directory list. Preserve the examples'
existing wallets, static sources, listener identities and persistence choices.

Acceptance evidence must cover:

- Preference parsing/display, strict rejection of retired `source_management`,
  instruction field and old tools/arguments; default-disabled behavior and all
  eight directory subsets, including `[]` and both Coinbase entries.
- Managed and comprehensive example validation confirms all three capability
  booleans are enabled on their intended listeners, with only `coinbase` and
  `x402-list` configured and the curated-only alternative explained in a comment.
- The local/remote/raw permission matrix: advertised schemas and manually invoked
  disabled operations agree; failures precede I/O, identity lookup, registration
  and wallet side effects. Raw-only, local-only, remote-only, empty-source and
  independent `discover_on_demand` endpoints work. Static serving remains usable.
- Native request translation for both Coinbase scopes and fixed x402 ranked
  settings; common filters, unknown/mixed legacy parameters and endpoint-specific
  query bounds. Keep native unknown/partial response fields and cross-source
  duplicates unchanged, with no Coinbase acquisition/guidance enrichment.
- Per-directory safe errors, all-directory failure, partial success, non-JSON
  responses, empty results, native limited results and attributable x402 output.
  Assert each configured directory starts once; no hidden fallback from one
  directory to another and no outer paid replay.
- Slow progress versus true response stalls, a failed sibling beside progressing
  work, waiter cancellation/shutdown and accepted paid/relay ownership. Assert
  original possible-payment evidence survives aggregation, parsing and later
  output errors; independent origins retain their own inactivity rules.
- Unique bundled specification matching with local permission disabled, ambiguous
  matches, existing generic-registration reuse, persistence/restart, endpoint
  isolation, listener filters and refresh/read authority, including remote-only
  refresh of a matched bundled definition, local-only refresh of an existing
  generic registration and raw-only refresh rejection. Verify rejected work
  does not allocate/fund wallets, publish registrations or request another signer.
- P1 details ID hiding/rejection without x402-list, enabled ID lookup with it, and
  URL details remaining available with Coinbase-only or empty directory lists.
  Tool descriptions and result hints recommend only permitted follow-ups.
- Every advertised tool parameter has useful, concise help in a consistent style;
  local tool descriptions identify bundled definitions and compatibility repairs
  and filters. Review the complete help surface for unnecessary implementation
  detail and repetition. Test meaningful contract omissions or misleading claims
  without freezing complete prose snapshots or requiring internal terminology.

Use existing discovery, registration, relay and deployment regressions, plus
focused tests in `tests/{config,managed_config,deployment,cli_contracts,
supervision}.rs`, `tests/support/` and `tests/live/runner/preparation_catalogs.rs`
where they consume renamed settings/tools. Update authored local fixtures and
live runner configuration consumers without performing live qualification.

Commit after focused validation and independently review the exact P2 SHA and its
consumers. Gate G2 is the final implementation integration review: examine P1,
P2, every prior fixup and complete config -> advertising/dispatch -> identity ->
HTTP/payment -> output/publication/shutdown chains. Verify the claims above,
documentation handoffs and complexity dispositions. Per-commit approval does not
substitute for this combined review.

### P3 Remove the completed plan

After G2 has no material findings and maintained guides contain all enduring
contracts, verify AGENTS.md links, examples, unfinished-plan references and the
documentation index. Remove this plan and its index link in a separate cleanup
commit; do not create an implemented-plan archive. Independently review the exact
cleanup SHA and the final combined branch, including all fixups. Reopen affected
gate concerns and validation if cleanup changes behavior or permanent guidance.

## Validation and independent review procedure

Before substantive implementation, the coordinating agent gives an independent
read-only design reviewer the user intent, this plan, exact baseline and consumer
map. Resolve material findings before building on the affected decisions.
Reviewers challenge extra enforcement, hidden work and opportunities to remove
responsibilities. Coordinate source edits and builds; keep sources/executables
frozen during each validation run.

For each implementation commit and fixup, provide its exact SHA, original baseline,
relevant consumers, commands and nonzero executed test counts. Address findings
with `git commit --fixup=<introducing-SHA>`, independently review each exact fixup
SHA, and retain unsquashed history through G2 and final cleanup review. At each
gate inspect recurring fixup causes, repeated assumptions, hazards and coverage
gaps across all earlier work. Decide whether they require a targeted wider audit,
new regression, simplification or a changed design; resolve material findings.

Run focused tests through Make. For example, select discovery library tests with
`make test CARGO_ARGS='--lib' TEST_ARGS='discovery:: --test-threads=1'`; derive more
focused filters from actual module wiring and check the executed counts. Use
`make test CARGO_ARGS='--test NAME'` for affected integration targets. Choose
socket-capable execution up front for localhost HTTP/SOCKS/MCP fixtures; keep
socket-free selections in the ordinary sandbox. Use public deterministic unfunded
keys and temporary state, never `.env` or real wallet secrets.

Run `make fmt-check` and appropriate Clippy validation on each final source state;
at G2 run `make clippy` and the combined affected discovery/deployment/config/CLI/
network-policy regression selection. Broaden to a full suite only if the changed
consumers or unresolved concerns warrant it. No Docker, funded, ignored live or
new feature-matrix qualification is required for this work.

For substantive Rust changes and G1/G2, run
`python3 scripts/complexity.py --base 4d287f58266828c1234d36f0876d927605252d1c`.
Retain generated reports under ignored `target/`; inspect full JSON/CSV, source
hashes, unmatched renames, relevant warnings and unchanged high-risk consumers.
Compare CogC/CycC for dispatch/schema assembly, manager authorization/import and
details/fanout ownership chains with the same pinned analyzer. Evaluate deleting
redundant state/branches before explaining retained hotspots; do not extract
helpers merely to lower a score. Gate reviewers examine these dispositions and
concrete regression evidence. Reuse source-matched evidence after doc-only cleanup.
