# Agent API discovery and source management

For a smaller initial MCP tool list, set `discover_on_demand = true` in a
`[servers.NAME]` table. This is independent of `source_management` and defaults to
`false`. Static listeners advertise just `x402_treazury_tools_search` and
`x402_treazury_tool_call`; listeners with source management enabled advertise all
eight management tools. API tools, including subsequently registered tools, stay
out of `tools/list`.

Agents search the listener's catalog with `x402_treazury_tools_search`, receive
complete descriptions and input schemas, and invoke a result through
`x402_treazury_tool_call` using its opaque `tool_ref` and `arguments`. On-demand
search defaults to five results per page; `query`, `limit` (1–100), and `cursor`
support targeted discovery and pagination. A broad search can still return large
schemas. By default, provider instructions remain available in the initialization response.

This setting changes tool advertising, not loading or access: all configured
catalogs still load, with remote catalogs published after listeners bind during
ordinary deployment serving. Filters and wallet bindings still apply, and existing direct
tool calls remain supported. It grants no source registration permissions and
requires no source-management wallet or registry for static listeners. References
are bound to the listener and process, and dynamic references also bind the source
revision; search again after a restart or stale-reference error. Inspection and
qualification retain the complete inventory and validate full schemas. Restart the
server and reconnect clients after changing the setting to refresh cached lists.


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
scope in catalog order. Omit both `source_name` and `source_id` to search configured
and registered APIs together. Set one to restrict registered-source search. Results
are paginated: pass the returned `next_cursor` with the same query and source filter
until it is null. On-demand discovery defaults to five results per page; ordinary
source-management search defaults to twenty. `limit` accepts 1–100 in either mode.

Each response includes:

| Field | Meaning |
| --- | --- |
| `items` | This page's matching tool references, descriptions and complete schemas. |
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
Complete descriptions, schemas, references and payment bindings are preserved;
only discovery matching/order and result diagnostics change.

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
See the [README workflow](../README.md#agent-api-auto-discovery-and-tool-addition),
[static-wallet example](../examples/deployments/agent-sources.toml) and
[managed-wallet example](../examples/deployments/agent-sources-managed.toml).

## Configuration and authority

```toml
[source_management]
wallet = "agent_shared"
registry_file = "../../state/agent-sources.sqlite" # optional persistence

[servers.research]
source_management = true
# Existing listen, bearer_token_env and other server settings go here.
# Static sources are optional.
```

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
| `x402_treazury_sources_search` | Optional `mode`: `browse` (default) or `best`. Both accept `q`, `category`; the server always sets `network=BSE`. Browse supports pagination and directory filters; best supports ranking preferences and a result limit. |
| `x402_treazury_source_details` | Required directory `slug` from search. Returns advertised endpoints, pricing and reliability; this is not a registered `source_id`. |
| `x402_treazury_source_url_info` | Exactly one directory `slug` or public HTTPS `base_url`; checks common specification and documentation URLs. |
| `x402_treazury_raw_get` | Required public HTTPS `url`; optional `query` object. Direct GET, including automatic x402 payment when required. |
| `x402_treazury_raw_post` | Required public HTTPS `url` and JSON `body`; optional `query` object. Direct POST, including automatic x402 payment when required. |
| `x402_treazury_source_add` | Required `spec_url`; optional `name`. |
| `x402_treazury_tools_search` | Optional `query`, `source_name` (or `source_id`, mutually exclusive), `cursor`, `limit`. |
| `x402_treazury_tool_call` | Required `tool_ref` and `arguments`. |

Directory search and service details are provided automatically, using embedded reviewed schemas
and the live x402 List API. It needs no static source or tool selector, and startup
does not fetch its catalog or probe its prices. Both tools use the endpoint's
source-management wallet (including its explicit listener override), normal response
handling and payment admission, and the configured direct/Tor network policy. The
bundled directory can charge after its shared-IP quota. Results are leads, not
payment/delivery guarantees, and may lack an OpenAPI URL. Specification discovery
and validation belong to `source_url_info` and `source_add`, not the agent's web
or raw-request tools.

Browse dispatches to `GET /services`, with `page` and `per_page`. Best dispatches
to `GET /best`, with `prefer` (`balanced`, `cheapest`, `fastest`, or
`most_reliable`), `max_price_usd`, `require_verified`, `limit` (1–20), and
`include_facilitator_context`. For example:
`{"mode":"best","q":"weather","prefer":"cheapest","limit":3}`.
The price filter is a directory recommendation filter, not a payment cap.
The built-in search hides the network parameter and always sends `network=BSE` in both modes.
Agent-supplied `network` arguments are rejected before I/O.
The local `mode` selector is never sent upstream; unsupported or mixed-mode arguments
fail before network I/O. Each call selects exactly one directory operation and preserves
its response, including ranking information. Details dispatches to
`GET /services/{slug}`; use `{"slug":"<slug from search>"}` to inspect a candidate
before locating its published specification and adding it. `source_details` returns
listing metadata, not the provider's OpenAPI document. Use `source_url_info` with
its slug or known API base to check common locations, then pass a returned
`verified_openapi` URL directly to `source_add`. The server downloads and validates
the specification; the agent must not fetch, read or verify large OpenAPI documents
or full API directory JSON using web/raw tools.

If common locations yield nothing, concise provider documentation or `llms.txt`
may supply a published specification link. Pass that URL directly to `source_add`
for server-side validation without opening the specification. Never invent URLs
or derive request schemas from prose. If no published spec can be found, use
`raw_get` or `raw_post` only with a documented endpoint, method and arguments.
After registration, optionally restrict `tools_search` with `source_name` set to
the returned `name` and a targeted query. Omit both `source_name` and `source_id`
to search all available API tools on the endpoint, including configured APIs and
all added sources. Results are paginated; the filter is optional in both ordinary
and on-demand listing modes. Invoke the chosen `tool_ref` through `tool_call`.

Use `x402_treazury_source_url_info` to locate documents without manual URL guessing:

```json
{"slug":"vendor-slug-from-search"}
```

Or bypass the directory lookup when you already know the API base:

```json
{"base_url":"https://api.example.com/v1"}
```

Exactly one input is required. Slugs resolve through the ordinary x402 List details
call, including its payment policy and attribution. Base URLs require public HTTPS
without credentials, fragments or queries. The tool checks these relative paths
under the API base, its origin root, and an allowed website origin from directory
details: `openapi.json`, `api/openapi.json`, `.well-known/openapi.json`, `swagger.json`,
`.well-known/x402`, `discovery/resources`, `llms.txt`, and `llms-full.txt`.
Candidates are deduplicated: at most 24 documents, with four requests in flight per
lookup. These bounds limit speculative requests and simultaneous document buffers;
there is no separate whole-lookup deadline. This is not an exhaustive crawler.

HEAD successes and HEAD 405/501 responses get an unsigned GET. HEAD or GET 402s
are reported without signing or paying, and redirects are not followed. GET bodies
use `max_spec_bytes` for JSON candidates and `max_help_bytes` for documentation,
with the existing source-management read-inactivity policy. Checks share one pinned
existing wallet isolation identity, including listener overrides; Tor uses that
wallet's SOCKS identity with remote DNS. Managed identity selection does not
promote wallets, allocate funds, or require a new spending reservation. It fails
explicitly if no eligible existing identity is available.

Each result includes URL, candidate kind, available HEAD/GET statuses, content type,
and an outcome. `verified_openapi` means OpenAPI 3 JSON structure was checked;
`source_add` still performs full import validation. `documentation_found` means
nonempty Markdown-style text was found. `directory_json_unverified` means JSON was
found at a directory candidate, without verifying its protocol schema. Other
outcomes preserve unverified content, invalid documents, HTTP failures, blocked
access, redirects, missing documents, transport failures and explicit byte-limit
rejections. Disallowed website origins are reported as skipped. No document prose
is returned or treated as authority. Nothing found does not mean the API is unusable.

Direct GET/POST tools are available only when this listener enables
`source_management`; `discover_on_demand` alone does not enable them. They use the
same resolved source-management wallet, including a listener wallet override, with
normal payment admission, cancellation ownership, public-destination checks,
`allowed_origins`, direct/Tor transport (including participating Tor unsigned
status retries and conditional scope promotion shared with generated MCP tools,
dynamic `tool_call` and directory tools) and source-management response limits.
`source_url_info` and `source_add` keep their existing single-scope discovery
and import transport policies.
They create no registrations or per-endpoint wallets. API tool-name filters do not
restrict these management tools: enabling source management permits direct calls
to documented public HTTPS endpoints within the configured origin policy.

Use direct calls for a one-off request or a documented API without a published
OpenAPI document; prefer registration for reusable tool schemas. Query values can
be strings, numbers, booleans or arrays of those values (arrays repeat the key).
They are URL-encoded and appended to any existing URL query. POST sends the exact
JSON value supplied as `body` with `Content-Type: application/json`; GET accepts
no body. Headers, credentials, wallet selectors and payment settings are not
agent arguments. Redirects are not followed. Raw binary PNG/JPEG/WebP responses
use the normal bounded image output path; JSON image mappings require a configured
provider and are not inferred by raw calls.

Both tools can spend Base USDC. Directory verification and a valid payment
challenge do not establish the request schema or guarantee useful output.
Unsigned x402 POST uses the same
[accepted retry semantics](architecture.md#unsigned-x402-requests-and-post-retries)
as GET; POST alone does not prohibit an authorized unsigned retry. A signed,
timed-out or ambiguous paid attempt may already have charged: do not automatically
repeat the logical call, because a new invocation may authorize another payment.
Business side effects add no separate retry gate for this use case.

Source management contributes eight definitions; searches return directory leads
and selected tool signatures on demand. Registered tools, including persisted
registrations restored at startup, are also advertised by `tools/list`, so clients
that include their entire tool inventory may expand the prompt. The stable search/call pair also works with clients that cache their initial
inventory. The built-in directory is not exposed as additional `x402_list_*` tools.
An explicit x402-list provider remains available for its broader API surface.

Add accepts a public HTTPS OpenAPI 3 JSON document. The API address comes from the
first root OpenAPI server, resolving relative addresses against the specification
URL. Optional names are ASCII labels of at most 64 letters/digits/underscores/hyphens;
`__` is reserved as the source/operation delimiter and is rejected within names.
Names are preserved exactly, are case-sensitive and must be unique within an endpoint,
including disabled registrations. Omission produces a stable URL-derived label.
Choose a readable name such as `netintel`: its tools use `dyn_netintel__<operation>`.
Operation suffixes still come from normalized OpenAPI paths, with method or numerical
suffixes where needed to distinguish operations within one source.
Unsupported saved names remain disabled on restore while unrelated registrations
remain available. A name containing the reserved `__` separator produces an explicit error and a WARN diagnostic when rejected, including
during restore and offline inspection. Unknown fields fail. There is no
preview, scope, lifetime, selection or mutation-key argument.

Add validates before publication and returns `name`, `tool_prefix` (for example
`dyn_netintel__`), a source ID, revision and filtered tool count. Agents can search
with the name and call the returned reference without using the source ID.
The canonical specification URL identifies an existing registration within the
endpoint. A repeated add returns that registration without fetching again, regardless
of the supplied label. Concurrent additions recheck duplicates under the publication
lock. Busy errors can be retried with the same URL. An operator can remove a source
and subsequently allow it to be added again with a new source UUID.

Tool search returns complete descriptions and schemas plus an opaque `tool_ref`.
Pass that reference unchanged to tool call with arguments matching its schema.
References bind the endpoint, running catalog instance, tool name and source revision;
a stale or foreign reference fails before HTTP. Search again after restart. Tools
cannot use this path to recursively invoke management commands.

HTTP is stateless and does not advertise list-change notifications. Stable search/call
tools work with clients that cache their initial tool list. Clients that refresh
`tools/list` can also invoke normal `dyn_<name>__*` tools. Each call retains its captured
route and payer for its entire lifetime.

Search pages default to 20 and allow at most 100 entries. MCP tool-list pages also
contain at most 100 entries on enabled endpoints. Cursors bind the running catalog,
endpoint, query and generation; restart a search when its cursor becomes stale.
Descriptions and schemas are not silently truncated.

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
at most 128 MiB; serialized registry state is at most 256 MiB. Retained mutation
receipts and stored registration records (including removed records) are bounded to
10,000 on registry load; new additions also check receipt capacity. Resource-limit
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
source UUIDs, endpoint ownership, revisions, receipts and tombstones. It contains no
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
schemas and callable references.

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
