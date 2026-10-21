# Agent API discovery and source management

For a smaller initial MCP tool list, set `discover_on_demand = true` in a
`[servers.NAME]` table. This is independent of `source_management` and defaults to
`false`. Static listeners advertise just `x402_treazury_tools_search` and
`x402_treazury_tool_call`; listeners with source management enabled advertise all
six management tools. API tools, including subsequently registered tools, stay
out of `tools/list`.

Agents search the listener's catalog with `x402_treazury_tools_search`, receive
complete descriptions and input schemas, and invoke a result through
`x402_treazury_tool_call` using its readable `tool_id` and `arguments`. On-demand
search defaults to five results per page; `query`, `limit` (1–100), and `cursor`
support targeted discovery and pagination. A broad search can still return large
schemas. By default, provider instructions remain available in the initialization response.

This setting changes tool advertising, not loading or access: all configured
catalogs still load, with remote catalogs published after listeners bind during
ordinary deployment serving. Filters and wallet bindings still apply, and existing direct
tool calls remain supported. It grants no source registration permissions and
requires no source-management wallet or registry for static listeners. Imported OpenAPI catalogs share default same-path POST preference before naming;
registration exposes no preference argument and raw calls retain their explicit method.
Tool IDs are exact endpoint-local names, reusable across turns, reconnects and restarts while
the tool remains available. Calls resolve the current definition and wallet binding,
including after a source refresh or re-addition with the same tool name. Inspection and
qualification retain the complete inventory and validate full schemas. Restart the
server and reconnect clients after changing the setting to refresh cached lists.


Static provider/source `recommended_tools` metadata narrows catalogs only with
explicit `use_recommended_tools = true`; listener filters still apply. Advice
uses each source's actual generated names without prefix rewriting. Operator
`catalog tools --all-tools` bypasses advice selection for inspection only and
retains ordinary filters. Recommendation markers do not change MCP descriptions,
source owners, payer bindings or invocation identity. Static advice is not inherited
by agent-imported specifications, and raw calls retain their separate authority.
See [catalog selection](configuration.md) for composition and stale-advice behavior.

## Tool search matching and results

`x402_treazury_tools_search.query` performs lexical BM25 keyword retrieval over
visible tool names, descriptions and top-level input parameter names. For example,
`yelp reviews` finds a description such as "List public reviews for a Yelp business"
without requiring those words to be adjacent. Query terms have OR semantics:
tools matching one term remain candidates, and BM25 ranks their relevance.

Names and descriptions containing the complete trimmed query as a lowercase
substring rank first, followed by BM25 score and then tool name for deterministic
ties. Literal priority favors precise identifiers, identifier fragments and phrases;
it is a relevance rule, not a legacy query mode. Terms are lowercased Unicode
alphanumeric runs; punctuation, underscores and hyphens separate terms, and repeated
BM25 query terms count once. There is no stemming, stopword removal, camel-case splitting,
semantic/fuzzy matching, wildcard expansion or Boolean/quoted-phrase operator syntax.
A nonblank query without alphanumeric terms can only match a literal substring;
it never silently becomes a full-catalog enumeration.

Omit `query`, or pass empty/whitespace text, to enumerate every tool in the selected
scope in catalog order. Omit `source_name` to search configured and registered APIs together. Set it to
a registration name to restrict the search to that source. Results
are paginated: pass the returned `next_cursor` with the same query and source filter
until it is null. On-demand discovery defaults to five results per page; ordinary
source-management search defaults to twenty. `limit` accepts 1–100 in either mode.

Each response includes:

| Field | Meaning |
| --- | --- |
| `items` | This page's matching tool IDs, descriptions and complete schemas. |
| `available` | Visible tools after endpoint and optional source filtering, before query matching. |
| `total` | All query matches before pagination, independent of page size. |
| `next_cursor` | Opaque continuation, or null on the last page. |
| `catalog_generation` | Snapshot generation; a publication invalidates old cursors. |
| `hint` | Present for zero matches from a nonblank query when available tools exist; suggests different keywords or enumeration. |

Both counts refer to the selected scope. `available > 0` with `total = 0` means
search found no match; `available = 0` means no tools are currently visible there,
which can reflect pending discovery or filtering. Remove a source filter to inspect
the endpoint as a whole. Neither case proves a configured provider has no APIs.
No speculative near-miss list is produced: OR-term retrieval already returns tools
matching some of the query, and enumeration is the explicit fallback.

Ranking uses only the captured endpoint/source-filtered snapshot, so inaccessible
or unrelated tools cannot influence relevance or counts. It builds no persistent
index, cache or background job and makes no provider requests. BM25 uses `k1=1.2`,
`b=0.75` and positive IDF `ln(1 + (N - df + 0.5)/(df + 0.5))`, as documented by
[Lucene BM25Similarity](https://lucene.apache.org/core/9_12_3/core/org/apache/lucene/search/similarities/BM25Similarity.html).
Ranking preserves complete descriptions, schemas and payment bindings.

## External directory matching

`x402_treazury_sources_search` searches x402 List rather than the endpoint's loaded
tools. Matching is performed upstream: `mode=browse` (default) uses case-insensitive
substring matching across service name, description, category and base URL, with
pagination. `mode=best` uses x402 List's relevance and quality ranking, matching
service metadata plus AI-derived capability tags and summaries. Neither mode uses
the local tool catalog's BM25 ranking or literal-match priority. A nonadjacent
multiword query that works in `tools_search` may therefore need fewer keywords in
directory browse mode. Each invocation dispatches one selected directory request;
it does not crawl the directory or build a local index. Directory reads share an
IP-level free quota and can require payment through normal payment admission.

## Endpoint instruction placement

Each `[servers.NAME]` can customize three instruction surfaces:

| Setting | Placement and semantics |
| --- | --- |
| `instructions_text` | Replaces the composed source guidance in `initialize.result.instructions` and current catalog metadata. Omit to retain source composition; `""` suppresses inherited source guidance. Generated pricing, discovery and availability/conflict notices remain. |
| `discovery_search_tool_instructions_text` | Appends to the standard description of `x402_treazury_tools_search`, exposed by either `discover_on_demand` or `source_management`. |
| `sources_search_tool_instructions_text` | Appends to the standard description of `x402_treazury_sources_search`, exposed by `source_management`. |

These are endpoint settings, not provider/source settings. They do not enable tools
or grant permissions. Absent or empty search text leaves the standard description
unchanged. Both search fields preserve the standard descriptions and input schemas;
they do not alter search matching or routing. Inspection inventory and MCP tool
lookup/listing use the same text. Configured search guidance is available even while
remote catalogs are pending; it describes intended coverage, not current readiness.

Put platform/capability summaries and useful search terms in the tool-search field
when a client does not forward MCP server instructions to its model. Use the
source-search field for guidance about finding new APIs in the external directory;
`tools_search` searches APIs already available on the listener. To avoid repeating
source guidance, override initialization with concise endpoint instructions.
Keep essential operation requirements in the relevant tool descriptions or include
them in the endpoint search guidance when suppressing inherited instructions.

```toml
[servers.research]
# Existing listen, wallet, sources and authentication settings also apply.
discover_on_demand = true
instructions_text = "Research endpoint. Discover schemas before invoking tools."
discovery_search_tool_instructions_text = "Search this listener for its configured social platforms; use Twitter for X."
# When source_management=true, this customizes external directory search:
sources_search_tool_instructions_text = "Prefer documented research APIs with published OpenAPI specifications."
```

Clients may cache tool descriptions. Restart/reconnect or refresh tools after
configuration changes; description changes do not promise a live model-context
update. Detailed source instructions are not automatically copied to search tools.

With source management enabled, `tools_search` help also lists registered source
names that have searchable tools on this endpoint in the description's catalog
snapshot (`none` when there are no such sources). Names retain their original case,
hyphens and underscores and are sorted and deduplicated. Disabled, removed and
fully filtered registrations do not appear, nor do other listeners' registrations.
The summary is generated on each tool lookup/list request from the same immutable
snapshot as the tool list, including after registry restore or static publication.
It adds no registry read, provider request or cache. Inspection inventory describes
configured tools, without loading the runtime registry.
The help warns that the list is not updated dynamically in the client's context
and may be cached; search results reflect current availability. The external
`sources_search` description does not receive this installed-source list.


Enable `source_management = true` on an HTTP deployment endpoint to let its agents
search the built-in x402 List directory, add public OpenAPI APIs, inspect tool signatures and
invoke them. It defaults to false. Standalone provider serving remains static.
See the [README workflow](../README.md#agent-api-auto-discovery-and-tool-addition)
and the [comprehensive example's toolbox endpoint](../examples/deployments/comprehensive.toml).

## Configuration and authority

The [managed starter](../examples/deployments/managed.toml) already enables source
management using its research pool. Its settings are:

```toml
[source_management]
wallet = "research" # Existing [wallets.research]; no new pool required.
# registry_file = "../../state/managed-sources.sqlite" # Optional persistence.

[servers.research]
source_management = true
# Retain its existing listen, bearer_token_env, wallet and sources settings.
```

Omit `registry_file` for registrations that last only until exit. Static sources
are optional: set `sources = []` for a discovery-only listener. A second listener
can enable source management with the same shared wallet and its own authenticated
identity; registrations remain separate even when both listeners use one pool.
Static profiles also work here when the operator defines the referenced wallet
with `mode = "static"` and `private_key_env`.

The shared block names an existing wallet and optionally a separate registry file.
A server's existing `wallet` setting overrides the shared dynamic wallet. No automatic
wallet allocation occurs per source. Managed wallet initialization and funding follow
the normal operator-authorized startup policy. Registration never funds wallets or
probes endpoint prices; paid invocation checks readiness and ordinary payment limits.

A listener's authenticated endpoint ID is the principal. All agents using its bearer
token share authority. Agent/session fields cannot choose an owner. Each registration
is visible only on the endpoint that added it; there are no publication targets,
receiver roles or process-wide grants. Two endpoints adding the same specification
get independent registrations, even if they share a payment wallet. Static TOML
sources cannot be modified by agent tools.

Persistence belongs to the operator. With `registry_file`, all additions are saved;
without it, additions disappear on exit. Agents cannot select lifetime, wallet,
timeouts, payment limits or visibility. Existing endpoint tag/tool filters still
constrain imported API tools. Include `dyn_*` when an endpoint has an explicit
`include_tools` allowlist, or use `dyn_netintel__*` to select a named registration.
Filters use source names, not registry IDs; existing ID-based filters are not rewritten.
The `x402_treazury_` and `dyn_` namespaces are reserved in
participating deployments. Management tools have their own enable switch and are
not removed by API filters.

## Agent tools

| Tool | Arguments |
| --- | --- |
| `x402_treazury_sources_search` | Optional `mode`: `browse` (default) or `best`, with upstream query/filter/ranking arguments. Server sets `network=BSE`. |
| `x402_treazury_source_details` | Exactly one directory `slug` or public HTTPS `base_url`. Returns listing metadata and discovers working specification/documentation URLs. |
| `x402_treazury_source_add` | Required public OpenAPI 3 JSON `spec_url`; optional readable `name`. |
| `x402_treazury_raw_call` | Required `method` and public HTTPS `url`; optional `query` object and JSON `body`. |
| `x402_treazury_tools_search` | Optional `query`, registered `source_name`, `cursor`, `limit`. |
| `x402_treazury_tool_call` | Required readable `tool_id` and `arguments`. |

Directory search uses embedded reviewed schemas and the live x402 List API. It
requires no static directory source or startup catalog fetch. Browse dispatches to
`GET /services`, with `page`, `per_page` and directory filters. Best dispatches to
`GET /best`, with `prefer` (`balanced`, `cheapest`, `fastest`, `most_reliable`),
`max_price_usd`, `require_verified`, `limit` (1–20) and
`include_facilitator_context`. For example:
`{"mode":"best","q":"weather","prefer":"cheapest","limit":3}`.
The price filter controls recommendations, not payment admission. The local mode
selector is stripped; invalid or mixed-mode fields fail before I/O. Each search
makes one directory request. Reads share an IP-level free quota and may charge
through the endpoint's source-management wallet and ordinary payment checks.
Cite x402-list.com for its CC BY 4.0 directory data. Results are discovery leads,
not executable schemas or delivery/payment guarantees.

### Listing and URL discovery

`source_details` helps discover working OpenAPI, x402 discovery and LLM
documentation URLs. With a directory `slug`, it fetches that service's listing once,
then probes common URLs under its API base, origin root and allowed website origin.
With `base_url`, it skips the directory lookup. A slug is the external directory's
service identifier, not a local registration name or UUID.

The response contains `listing` (the exact parsed directory response, or null for a
base URL) and `url_discovery`. Discovery includes an overall `status`, per-URL
`results`, and available base/website metadata. A received directory response is
preserved when discovery cannot proceed, including missing/rejected API bases or an
unavailable documentation identity. Non-JSON directory output is preserved as text
with `directory_response_not_json`; the tool does not automatically repeat a paid
lookup. Directory request failures propagate through ordinary response handling.

Provider probes use unsigned HEAD followed by bounded GET for successful or
unsupported HEAD responses. They never pay a provider's 402 and never follow
redirects. Candidate URLs are fixed and deduplicated, with four probes in flight.
Existing spec/help byte and response-read-inactivity limits apply. Results include
URL, candidate kind, HTTP statuses, content type and outcome; document contents stay
server-side. `verified_openapi` means structural OpenAPI 3 JSON checks passed;
`source_add` still performs full import validation. `documentation_found` points to
concise documentation; `directory_json_unverified` is only an unverified lead.
Other statuses expose missing, redirected, blocked, invalid, oversized or unavailable
candidates. Nothing found does not prove the API is unusable.

Probes share a pinned existing source-management wallet isolation identity, including
listener overrides and Tor remote DNS. Identity selection does not allocate, fund or
promote a wallet. Directory and probe work remain owned through caller cancellation.
No new discovery cache or registration is created.

If you already know a published OpenAPI URL, pass it directly to `source_add`;
`source_details` is optional. Otherwise it can locate a specification or concise
provider documentation with a published link. Do not download large OpenAPI
specifications or full API directories into agent context: import them server-side,
then retrieve relevant tool schemas with `tools_search`. Never invent specification
URLs or paid request arguments. If no specification exists, a documented API can be
called with `raw_call` without registration.

### Direct calls to x402 URLs

`raw_call` calls any documented public HTTPS x402 API URL. Supply its exact HTTP
`method`, optional `query` parameters and optional JSON `body`. The method is
validated as an HTTP token and forwarded without changing its spelling. Query
values can be strings, numbers, booleans or arrays of those scalars; arrays repeat
the key. They are URL-encoded and appended to any existing URL query. A supplied
body is sent exactly as JSON with `Content-Type: application/json`, including
explicit `null`; omission sends no body. The wrapper adds no method/body policy.

The tool requires source management and uses its resolved wallet, ordinary payment
admission, configured direct/Tor transport and response limits. It creates no
registration or wallet. API tool-name filters do not restrict it. Existing public
HTTPS, no-URL-credentials, origin and redirect checks remain server responsibilities;
headers, wallet selectors and payment settings are not agent arguments. Binary
PNG/JPEG/WebP output uses the normal bounded image path; JSON image mappings require
a configured provider and are not inferred by raw calls.

Calls can spend Base USDC and can change provider state. Follow the documented
method and arguments. Directory verification and a payment challenge do not prove
a request schema or guarantee useful output. Unsigned requests retain the
[accepted retry semantics](architecture.md#unsigned-x402-requests-and-post-retries).
A signed, timed-out or ambiguous call may already have charged or acted; a new
invocation is a new logical call and may authorize another payment.

### Registration and tool invocation

`source_add` fetches and fully validates a public HTTPS OpenAPI 3 JSON document.
The API base comes from the first root OpenAPI server, resolving relative URLs
against the specification URL. It returns only `name`, `tool_prefix`, filtered
`tool_count`, `status` and `next`; disabled registrations also include `reason`.
Registration never funds wallets or probes prices.

Names preserve case and allow at most 64 ASCII letters/digits/underscores/hyphens;
`__` is reserved as the source/operation separator. Explicit names must be unique
within the endpoint, including disabled registrations and configured source names.
When omitted, a readable hostname-derived name is generated (for example
`api-example-com`), with a numeric suffix such as `-2` if occupied. Name allocation
and duplicate-URL checks share the existing publication lock. Choose `netintel` to
produce `dyn_netintel__<operation>` tools; operation suffixes use normalized OpenAPI
paths, adding method/numeric suffixes where necessary.

The canonical specification URL identifies an existing live registration within the
endpoint. Repeated additions return it without fetching again, regardless of a new
requested label. Concurrent additions recheck under the publication lock. Busy
errors can be retried with the same URL. Persistence follows operator configuration.
Durable source records, UUIDs, revisions, hashes and tombstones remain internal and
available through operator inspection; no registration-result receipt ledger is
needed. Old registry receipt fields are ignored on read and omitted on later saves.
These were registration bookkeeping, not payment or financial journals.

Search with optional `source_name` and a targeted query, or omit the source filter
to search all available endpoint tools. Results contain readable `tool_id`, complete
`description` and `input_schema`. Invoke `tool_call` with that ID and matching
`arguments`. The server resolves the currently visible bound tool and captures its
route, payer and internal source identity before executing. Names are local to the
selected MCP endpoint; the same name on another endpoint resolves that endpoint's
tool. Removed, hidden or filtered tools fail before provider I/O. Management tools
cannot be recursively invoked through this API-tool lookup.

Tool IDs are reusable across turns and restarts while present. Refreshing or
re-adding a source with the same tool name intentionally selects its current
operation; search again when you need the current schema. Accepted calls retain
their captured binding even during publication/removal. Existing direct MCP tool
calls use the same name-based semantics. API responses and payment admission remain
unchanged.

Source management contributes six definitions. Eager endpoints also advertise
registered API tools, including restored registrations; `discover_on_demand` keeps
those API definitions out of `tools/list`. HTTP is stateless and does not advertise
list-change notifications, so clients may cache their inventory. Search/call remains
usable with that cached inventory. Search cursors bind the running catalog,
endpoint, query/filter and generation; stale cursors require restarting pagination.
Tool-list pages contain at most 100 entries on enabled endpoints. Descriptions and
schemas are not silently truncated.

## Limits and network policy

The default dynamic-tool quota is 512 tools per endpoint, controlled by
`[source_management].max_tools_per_server`. Endpoints with
`discover_on_demand = true` are exempt because API tools are omitted from their
advertised inventory. The exemption applies to registration, startup restoration,
static catalog publication and operator refresh. Eager endpoints count dynamic
tools after endpoint filters; static tools do not consume this quota.
There is no registration-count or per-source tool-count quota. The removed
`max_sources` and `max_tools_per_source` settings are rejected as unknown fields;
remove them from existing configuration.

Specifications default to 32 MiB. Other advanced overrides in
`[source_management]` are `max_spec_bytes`, `max_response_bytes`, `max_help_bytes`,
`read_timeout_seconds`, and `allowed_origins`. The optional read-inactivity
override replaces the network default (60 seconds in either transport). Spec and
API responses have no total HTTP download deadline. `fetch_timeout_seconds` is a
legacy alias.

The endpoint tool quota accepts 1–10,000 and specifications up to 64 MiB.
Management requests have a 64 KiB limit; forwarded API calls retain normal API behavior. Import
concurrency is four process-wide and one per endpoint; at most 16 mutation jobs can
be outstanding. Aliases share an active fetch without each consuming a permit.
Active fetches cannot be evicted by retention age or completed-cache churn; a full
active cache returns busy. Success retention is 300 seconds from completion,
transient failure backoff one second. Completed entries remain byte/count bounded.
Saturation returns an explicit busy error. Publication and quota
checks are serialized, so concurrent additions cannot publish partial sources.

JSON/reference traversal remains bounded to depth 64, 500,000 expanded nodes and
64 MiB of expanded string/key content. Import rejects over 10,000 operations,
generated schemas above 256 KiB, names above 512 bytes, external/unresolved/cyclic
references, and per-path/operation server overrides. Accepted live documents total
at most 128 MiB; serialized registry state is at most 256 MiB. Stored registration
records, including removed records, are governed by that same serialized byte bound
on save and load. Record/receipt counts introduce no separate admission or reload
gate. Resource-limit
rejections emit WARN diagnostics with safe counts, thresholds and adjustment
settings where applicable. Import saturation, transport timeouts and rejected destination/reference/server
policies also warn.
Limits reject visibly rather than trimming documents; warnings never include
submitted names, URLs, schemas or upstream prose.

Agent destinations require public HTTPS without userinfo or fragments. Local/special
addresses, private names, redirects and local files are rejected. Direct connections
validate DNS answers at dial time. Optional `allowed_origins` narrows specification
and API destinations to exact canonical HTTPS origins. Undeclared cross-origin API
routes are rejected. All production traffic uses the network factory; paid dynamic
clients retain public-destination restrictions.

Tor uses authenticated remote-DNS SOCKS with no direct fallback. Private DNS-answer
rejection depends on Tor's `ClientRejectInternalAddresses`; the application does not
resolve destinations locally or inspect proxy-hidden answers. Keep that protection
enabled. Vendor descriptions are untrusted data and grant no additional authority.

## Trusted invocation and payment scope

Every direct or fallback invocation captures one immutable catalog snapshot and
its trusted `BoundTool`. Runtime price hints are scoped by the effective wallet
group, source identity/revision, HTTP method and normalized operation path. Agent
arguments, query values and request bodies cannot choose that scope, payer, reserve
claim or funding authority. Removing or refreshing a source cannot rebind an
already captured call; removal does not erase its financial obligations.

Dynamic sources share `source_management.wallet` unless an explicit listener wallet
override splits that group. Registration never creates per-source funding groups.
Hints contain amounts, not reusable payment challenges or signatures. Every call
uses a fresh request/challenge and current payment admission. See the
[selection and isolation decision](architecture.md#wallet-selection-and-network-isolation)
for connection, cover-state and Tor reuse limitations.

## Persistence and operator maintenance

The separate owner-only SQLite registry holds accepted specification bytes, hashes,
source UUIDs, endpoint ownership, revisions and tombstones. It contains no
wallet keys, but URLs and documents can reveal operator interests. Back it up while
serving is stopped. Never delete or replace its ownership sidecar while serving.
Registry files and sidecars cannot alias configuration or treasury paths.

`config show` inspects configuration without opening registry state or secrets.
`sources inspect --config FILE` reads saved registrations and never creates a missing
registry. It does not require listener tokens, wallet keys or catalog requests.
Each source includes `tools`, an array of generated `name`, HTTP `method` and
`path` entries rebuilt from its accepted specification and saved selection.
Disabled sources retain their saved inventory; removed sources have an empty list
because removal clears the specification. An invalid saved catalog reports
`tools: null` and `tools_error: "saved_catalog_invalid"` without hiding other
sources or exposing specification/error prose. Existing registration metadata is
preserved. Inspection can run while serving and does not acquire registry ownership.

This is the saved registration inventory, before current listener filters; it does
not assert that a tool is presently available or funded. Use
`x402_treazury_tools_search` on the owning listener for its loaded tools, complete
schemas and callable tool IDs.

Stop serving before operator maintenance; exclusive registry ownership is enforced:

```sh
x402_treazury sources refresh --config FILE --server research --source-id SOURCE_ID
x402_treazury sources remove --config FILE --server research --source-id SOURCE_ID
```

Refresh fetches and validates new bytes through the configured network policy without
wallet credentials or pricing probes. Removal records a tombstone. Neither command
changes wallets, payment reservations, funding/refund records or reconciliation.
Temporary registrations need no registry maintenance; they disappear on exit.

Restart rebuilds from accepted bytes without vendor requests. Disabled endpoints,
nonlocal saved bindings, invalid formats and quota conflicts cannot regain access
silently. Corrupt/unsupported registries fail startup. Persistent additions commit
before publication; cancelling an accepted commit's waiter cannot undo the write.
