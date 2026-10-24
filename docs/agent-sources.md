# Agent API discovery and source management

For a smaller initial MCP tool list, set `discover_on_demand = true` in a
`[servers.NAME]` table. This is independent of `source_management` and defaults to
`false`. Static listeners advertise just `x402_treazury_tools_search` and
`x402_treazury_tool_call`; listeners with source management enabled advertise all
seven management tools. API tools, including subsequently registered tools, stay
out of `tools/list`.

Agents search the listener's catalog with `x402_treazury_tools_search`, receive
complete descriptions and input schemas, and invoke a result through
`x402_treazury_tool_call` using its readable `tool_id` and `arguments`. On-demand
search defaults to 25 results per page; `query`, positive integer `limit`, and `cursor`
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
source owners, payer bindings or invocation identity. Generic specification imports
do not inherit provider advice. Accepted bundled
definitions retain recommendation metadata without enabling recommendation selection;
raw calls retain their separate authority.
See [catalog selection](configuration.md) for composition and stale-advice behavior.

## Tool search matching and results

Both local search surfaces use the same lexical BM25 ranker: `tools_search.query`
searches visible tool names, descriptions and top-level input parameter names;
`sources_search(local_only=true).q` searches bundled provider IDs, display names,
capability descriptions and declared API bases. For example,
`yelp reviews` finds a description such as "List public reviews for a Yelp business"
without requiring those words to be adjacent. Query terms have OR semantics:
candidates matching one term remain eligible, and BM25 ranks their relevance.

Names and descriptions containing the complete trimmed query as a lowercase
substring rank first, followed by BM25 score and then tool name for deterministic
ties. Provider search applies the same literal priority to its space-joined metadata
and breaks score ties by provider ID. Blank searches retain enumeration order.
Literal priority favors precise identifiers, identifier fragments and phrases;
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
until it is null. All tool search defaults to 25 results per page. `limit` accepts any positive integer.

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

Tool ranking uses only the captured endpoint/source-filtered snapshot, so
inaccessible or unrelated tools cannot influence relevance or counts. Provider
ranking uses only the shipped metadata collection. The shared ranker builds no
persistent index, cache or background job and makes no provider requests.
BM25 uses `k1=1.2`,
`b=0.75` and positive IDF `ln(1 + (N - df + 0.5)/(df + 0.5))`, as documented by
[Lucene BM25Similarity](https://lucene.apache.org/core/9_12_3/core/org/apache/lucene/search/similarities/BM25Similarity.html).
Ranking preserves complete descriptions, schemas and payment bindings.

## External directory matching

`x402_treazury_sources_search(local_only=true)` searches only bundled provider
metadata using the same BM25 and literal-match priority as callable tool search.
Multiword queries match any of their terms and rank by relevance across IDs,
display names, capability descriptions and declared API bases. Omitted or blank
`q` enumerates the bundle in stable provider-ID order. Nonblank results rank before
pagination and use provider ID to break ties. `page` defaults to 1;
`per_page` defaults to 25
and accepts any positive integer. Only omitted or `browse` mode is allowed; directory filters,
ranking arguments and `best` fail before I/O. `x402_treazury_providers_list` accepts
only the same pagination fields and shares this projection with blank local search.
Both return `items`, `total`, `page`, `per_page` and `next_page`; each item includes
`provider_id`, name, description, optional API base and schema provenance. Results
are metadata leads, independent of installed tools, with no wallet identity lookup,
network requests, registration or automatic remote fallback. Bundled means shipped
configuration knowledge, not current reachability, funding or live qualification.
Pass a known `provider_id` or specification URL directly to `source_add`.
`source_details` is optional assistance for an unknown API base, x402 List slug or
published usage-document URL.
An absent `api_base` means no configured base override; import resolves the API
server from the schema, which need not share the specification URL's origin.
Unknown cost means missing price information, not a free call. After import,
optional advertised provider help may explain prices and request contracts before
calling; neither registration nor help establishes paid-delivery qualification.

With `local_only` false or omitted and `directory` omitted or `x402_list`,
`x402_treazury_sources_search` searches x402 List
rather than the endpoint's loaded tools. The local selector is stripped before
remote dispatch. Matching is performed upstream: `mode=browse` (default) uses
case-insensitive
substring matching across service name, description, category and base URL, with
pagination (default 25, upstream maximum 100 per page). `mode=best` uses x402 List's relevance and quality ranking, matching
service metadata plus AI-derived capability tags and summaries. Neither mode uses
the local tool catalog's BM25 ranking or literal-match priority. A nonadjacent
multiword query that works in `tools_search` may therefore need fewer keywords in
directory browse mode. Each invocation dispatches one selected directory request;
it does not crawl the directory or build a local index. Directory reads share an
IP-level free quota and can require payment through normal payment admission.

### Coinbase curated resource search

The tool description leads with collection choices; parameter help carries branch
constraints. Transport and relay mechanics belong in this guide and the operator
configuration, rather than repeated agent-facing descriptions.

Select `directory="coinbase"` for ranked resource search, optionally using `q`.
Omitted mode or `mode="best"` selects this search; `browse`, page/per-page/cursor
arguments and x402 List-only filters reject before I/O. Mode defaults are
branch-specific: local and x402 List omission still means browse. There is no
search federation or automatic directory fallback. Search always uses curated
resources. Its shared inputs are `q`, `limit` and numeric `max_price_usd`, just as
for x402 List ranked search. `limit` defaults to 5 and accepts 1–20; Coinbase
queries accept at most 400 Unicode characters. Price filters are advisory, not
payment caps or evidence of useful delivery. Provider-specific filters and a
non-curated mode are not exposed. The server supplies Base mainnet USDC/exact
payment constraints; all returned offers remain intact.

The response contains `directory="coinbase"`, `acquisition.via` (`origin`, `curl`, `agproxy` or `horizonpulse`), `response` (the complete upstream object) and guidance. It preserves
exact URLs, methods, Bazaar metadata, offers, skill links, unknown fields and
missing contracts without normalizing resources into services. `partialResults`
remains upstream metadata, with no invented totals or continuations. Empty or
partial results do not prove absence and never trigger a paid fallback. HTTP
206 responses and responses marked with Content-Range fail the complete-response
contract on both origin and relay paths, without fallback or automatic paid replay. HTTP
examples and a Bazaar envelope schema are not invented request argument schemas.
Documented HTTP requests may go straight to `raw_call`; known OpenAPI URLs go to
`source_add`. MCP/unknown protocols remain metadata only.

Public Coinbase reads are unsigned, using an existing endpoint wallet's
documentation identity without allocation, funding or observation. Missing
identity fails that request. Under Tor they use one origin attempt plus
`network.tor_isolation_retries` eligible fresh-isolation retries (default 2), with the shared eligibility and `Retry-After`
policy, including the terminal delay before fallback. Direct mode uses one
origin attempt. Only complete valid search JSON may promote an origin scope.

When its category is enabled, runtime fallback also applies to API/directory/raw GET and HEAD, and
otherwise-authorized unsigned x402 POST when a listed adapter represents the exact
request. The original method selects its list. Origin challenges retain ordinary
payment handling; relay-supplied target challenges are terminal. Any earlier target
signature suppresses generic fallback. Complete invalid directory JSON/output and
terminal HTTP statuses do not buy another read. API ranges remain legitimate;
raw HEAD succeeds with empty text. Typed possible-charge facts survive directory
processing and output conversion, separately from relay advisories and JSON text.

With `[relay_fallback]` configured, `source_discovery=true` enables built-in
Coinbase and x402 List fallback by default. `provider_metadata=true` independently
enables schemas, imports/refresh, help, explicit usage documents and URL/spec probes.
Actual API and raw calls require explicit `api_requests=true`; an ordinary call to
a directory URL remains API work. In mixed `source_details` workflows, the directory
lookup is discovery and each later provider document/probe is metadata. Categories
are selected by trusted code, never agent arguments, method or hostname. With no
relay section all requests remain origin-only. Each search fetches fresh results;
it inherits no catalog URL cache,
sticky failure, readiness queue or shared breaker. The relay uses this listener's
source-management payer, regardless of the static relay wallet override; ordinary
payment admission, journaling and insufficient-funds traversal still apply.
Each authorization may charge, and uncertain paid failure grants no automatic
replay. The relay connection retains configured direct/Tor policy. The selected service
sees the query URL, reaches Coinbase from its own network and supplies target
content/status/headers. Origin failures and relay failures remain request-scoped.
See [relay configuration](configuration.md#relay-fallback) for settings,
precedence and trust. Local input/origin/identity/download-limit failures
never authorize relay spending. No Coinbase challenge is signed.

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
source-search field for guidance about finding new APIs locally or in the external directory;
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

Dynamic source guidance is recomposed from the immutable static instruction base
on every publication. Only enabled registrations with visible tools contribute;
refresh, removal, restore and static republishing replace that contribution without
duplication or stale guidance. Listener `instructions_text` replacement, including
empty suppression, applies to dynamic guidance too. Known canonical tool references
in provider help descriptions and instructions use their public dynamic names;
URLs, wire fields and input schemas retain their accepted values.

Clients may cache tool descriptions. Restart/reconnect or refresh tools after
configuration changes; description changes do not promise a live model-context
update. Detailed source instructions are not automatically copied to search tools.

Tool descriptions contain no registered-source inventory because clients may cache
it. Use live tool search/enumeration for current endpoint-local availability and
`source_name` to narrow to a registration.


Enable `source_management = true` on an HTTP deployment endpoint to let its agents
search bundled providers, x402 List or Coinbase resources, register bundled or
public OpenAPI APIs, inspect tool signatures and
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
| `x402_treazury_sources_search` | `local_only=true`: optional `q`, `page`, `per_page` and `mode=browse`. Otherwise `directory=x402_list` (default) selects browse/best; `directory=coinbase` selects ranked resource search. Each branch has its own arguments and defaults. |
| `x402_treazury_source_details` | Exactly one x402 List `slug`, public HTTPS API `base_url` or `documentation_url`. Returns listing/base discovery outcomes, or complete accepted usage text. |
| `x402_treazury_source_add` | Exactly one bundled `provider_id` or public OpenAPI 3 JSON `spec_url`, plus optional readable `name`. |
| `x402_treazury_providers_list` | Optional `page` and `per_page`; enumerates bundled provider metadata offline. |
| `x402_treazury_raw_call` | Required `method` and public HTTPS `url`; optional `query` object and JSON `body`. |
| `x402_treazury_tools_search` | Optional `query`, registered `source_name`, `cursor`, `limit`. |
| `x402_treazury_tool_call` | Required readable `tool_id` and `arguments`. |

With `directory` omitted or `x402_list`, remote search uses embedded reviewed schemas
and the live x402 List API; the server sets `network=BSE`. It
requires no static directory source or startup catalog fetch. Browse dispatches to
`GET /services`, with `page`, `per_page` and directory filters. Best dispatches to
`GET /best`, with `prefer` (`balanced`, `cheapest`, `fastest`, `most_reliable`),
`max_price_usd`, `require_verified`, `limit` (1–20) and
`include_facilitator_context`. For example:
`{"mode":"best","q":"weather","prefer":"cheapest","limit":3}`.
The price filter controls recommendations, not payment admission. The local mode
selector is stripped; invalid or mixed-mode fields fail before I/O. Each x402 List search
makes one directory workflow. Reads share an IP-level free quota and may charge
through the endpoint's source-management wallet and ordinary payment checks.
Cite x402-list.com for its CC BY 4.0 directory data. Results are discovery leads,
not registered tools or delivery/payment guarantees. Coinbase has a separate
[resource-search contract](#coinbase-curated-resource-search), including unsigned
origin attempts and optional paid fallback.

### Listing and URL discovery

With `documentation_url`, `source_details` reads an exact caller-supplied public
HTTPS usage-document URL, including its query when present. This is optional;
sufficient documented method, URL and arguments already permit `raw_call`, and a
known OpenAPI URL can go directly to `source_add`. A Coinbase `skillUrl` is a lead,
not a verified instruction source. The response labels provenance
`caller_supplied` and returns `documentation` with the requested URL, content type
when available, and visible status. `ok` and `empty` include complete `text`;
failures omit text and report safe reasons, HTTP status or the affected byte limit.

The reader uses the shared unsigned origin sequence and, when configured, the
on-demand GET relay list. It performs no HEAD request or linked traversal. The
listener's available documentation identity, transport, `allowed_origins`,
`max_help_bytes` and response-read inactivity policy apply. Missing wallet identity
leaves origin retrieval available under the URL's discovery identity. Origin HTTP
remains no-follow; configured relays may follow redirects under the
[documented trust boundary](configuration.md#relay-fallback). A separate skill host must independently satisfy the origin policy.
Accepted text is complete UTF-8 without NUL, with `text/plain`, `text/markdown`,
`text/x-markdown` (parameters allowed), or no content type. HTML MIME and leading
HTML/doctype markup are rejected, as are other MIME types and invalid UTF-8.
Unrequested partial responses (HTTP 206 or Content-Range) fail without text.
Returned text remains untrusted content; reading it never executes instructions,
registers a source or extends payment authority.

`source_details` resolves bundled provider definitions before discovering OpenAPI,
x402 discovery and LLM documentation URLs. With a directory `slug`, it fetches that
service's listing once. After validating the API base, exact normalized origin/base
path matches return selectable `provider_matches` with provider IDs and schema
provenance. Companion definitions sharing a base remain separate candidates; they
are not combined. `url_discovery.status = "bundled_provider_found"` and empty
`results` report that speculative probes were skipped. These offline matches make
no reachability or qualification claim. Unknown bases retain common-URL probes
under the API base, origin root and allowed website origin.
With `base_url`, it skips the directory lookup. A slug is the external directory's
service identifier, not a local registration name or UUID.

The response contains `listing` (the exact parsed directory response, or null for a
base URL), `provider_matches` (empty when none) and `url_discovery`. Discovery includes an overall `status`, per-URL
`results`, and available base/website metadata. A received directory response is
preserved when discovery cannot proceed, including missing/rejected API bases or an
unavailable optional discovery work. Non-JSON directory output is preserved as text
with `directory_response_not_json`; the tool does not automatically repeat a paid
lookup. Directory request failures propagate through ordinary response handling.

Provider probes use one validated GET per fixed, deduplicated candidate, with four
probes in flight. They share origin retries and configured on-demand relay fallback;
blocked retrieval can charge, while terminal `401`/`404`/`410`, target `402`, complete
invalid content and partial-document rejection do not buy another read. Results
include URL, candidate kind, GET status, content type and outcome; no synthetic HEAD
status is reported. Document contents stay server-side. Existing spec/help byte
and response-read-inactivity limits apply. `verified_openapi` means structural OpenAPI 3 JSON checks passed;
`source_add` still performs full import validation. `documentation_found` points to
concise documentation; `directory_json_unverified` is only an unverified lead.
Other statuses expose missing, redirected, blocked, invalid, oversized or unavailable
candidates. Nothing found does not prove the API is unusable.

Probes use a pinned available source-management wallet isolation identity, including
listener overrides and Tor remote DNS. Without one, the URL's discovery identity
keeps unsigned origin retrieval available. Identity selection does not allocate, fund or
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

`source_add` accepts exactly one `provider_id` or `spec_url`, plus optional `name`.
For example, `{"provider_id":"exa"}` registers the bundled Exa definition. A bundled
registration uses its reviewed catalog configuration and acquires its remote or
embedded schema server-side; it preserves base repair, selections, overrides,
help, guidance and response mappings without activating provider recommendations.
No agent-selectable local path or data installation is needed.

A specification URL first returns any existing generic registration for that
endpoint and canonical URL unchanged. Otherwise a unique exact match against
current bundled remote specification URLs selects the bundled definition; multiple
matches require an explicit provider ID. Other URLs retain generic OpenAPI 3 JSON
import, with the API base from the first root server resolved against the spec URL.
An arbitrary document's `servers` claim never selects a bundled provider.
It returns `name`, `tool_prefix`, filtered `tool_count`, `status` and `next`; disabled registrations also include `reason`. Bundled results and operator inspection also report the accepted `provider_id` and schema provenance.
`next` suggests search then invocation only when tools are visible. A disabled
registration needs operator repair/refresh; repeating `source_add` returns that
same registration. An enabled registration with zero visible tools retains
`status="available"` and points to source selections and listener filters. Searching
without `source_name` can find other endpoint tools, but does not alter operator
filters or guarantee a callable result.
Registration never funds wallets or probes prices.

Names preserve case and allow at most 64 ASCII letters/digits/underscores/hyphens;
`__` is reserved as the source/operation separator. Explicit names must be unique
within the endpoint, including disabled registrations and configured source names.
When omitted, bundled sources use their short stable provider ID and generic
sources use a readable hostname-derived name (for example `api-example-com`), with a numeric suffix such as `-2` if occupied. Name allocation
and duplicate-URL checks share the existing publication lock. Choose `netintel` to
produce `dyn_netintel__<operation>` tools; operation suffixes use normalized OpenAPI
paths, adding method/numeric suffixes where necessary.

Curated registrations deduplicate by authenticated owner and provider ID; generic
registrations retain owner plus canonical specification URL. Repeating a provider
ID returns its accepted registration before consulting the current bundle, without
implicit refresh or a new label. Concurrent provider-ID and current known-spec
additions converge under the publication lock. A legacy generic URL registration
remains generic when its URL is repeated; explicitly selecting its provider ID can
create a separate bundled source, subject to existing name/tool collision checks.
Busy errors can be retried with the same selector. Persistence follows operator
configuration.
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

Source management contributes seven definitions. Eager endpoints also advertise
registered API tools, including restored registrations; `discover_on_demand` keeps
those API definitions out of `tools/list`. HTTP is stateless and does not advertise
list-change notifications, so clients may cache their inventory. Search/call remains
usable with that cached inventory. Search cursors bind the running catalog,
endpoint, query/filter and generation; stale cursors require restarting pagination.
All listeners return at most 25 definitions per `tools/list` page. Follow `nextCursor` until absent; cursors bind the listener and snapshot, and catalog publication invalidates them. Descriptions and
schemas and accepted tool outputs are returned complete. The removed listener
`max_response_chars` field and CLI flag reject visibly; byte acquisition and image
bounds still reject oversized content explicitly.

Help and imported specifications share the common complete-document acquisition
path. Only validated complete content can promote an origin transport. Complete
invalid schemas/text, unsolicited `206`/`Content-Range`, local bounds and destination
rejections are terminal; blocking HTTP/transport failures can use configured
on-demand fallback. A relayed target `402` is never signed. Relayed content retains
requested/reported-destination checks and the configured relay trust boundary.

Help caches and active import coalescing include retained payer authority and
effective document policy. A static help binding pins the exact payer independently
of later ordinary API payer replacement; managed bindings retain their existing
pool authority. Accepted help/add/refresh work survives waiter cancellation through
validation and cache/source publication. Shared failures preserve typed bounds and
possible-charge facts. Possible-paid help failures remain cached so joined waiters
cannot sequentially purchase the same failed initialization; proven unpaid help
failures retain normal explicit retry behavior. These readers allocate no wallets,
start no funding and create no new replay authority.

## Limits and network policy

There is no tool-count admission quota. A valid first-page `tools/list` request
with more than 250 advertised definitions emits one advisory WARN naming the
listener and count and suggesting `discover_on_demand = true`. Counts include
filtered static/dynamic API definitions and advertised management wrappers;
on-demand listeners count only wrappers. Continuations and invalid cursors do not
warn; a fresh first-page request may warn again. This observes advertised inventory,
not client/model context, and never rejects registration or changes access/mode.
The operator must explicitly opt in to on-demand discovery.

Removed `max_tools_per_server`, `max_sources` and `max_tools_per_source` settings
are rejected as unknown fields; remove them from existing configuration.

Specifications default to 32 MiB. Other advanced overrides in
`[source_management]` are `max_spec_bytes`, `max_response_bytes`, `max_help_bytes`,
`read_timeout_seconds`, and `allowed_origins`. The optional read-inactivity
override takes precedence over an accepted bundled provider value, which otherwise
inherits the network default (60 seconds in either transport). Generic imports and
raw calls inherit the network default when the source-management value is omitted.
Source-management response, help and specification byte limits govern dynamic
operations rather than the provider's static byte settings. Spec, help and API
responses have no total HTTP download deadline. `fetch_timeout_seconds` is a
legacy alias.

The specification byte override accepts up to 64 MiB.
Management requests have a 64 KiB limit; forwarded API calls retain normal API behavior.
Remote imports have no separate per-listener, global-import or active-cache admission
quota; they retain the existing foreground work ownership. Equivalent aliases share
an active fetch keyed by payer authority, document policy and transport settings.
Active fetches cannot be evicted by retention age or completed-cache churn.
Completed fetches retain successes for 300 seconds and failures for one second;
optional completed retention is trimmed on lookup and completion to the smaller of
eight entries or `128 MiB / max_spec_bytes`. This is a cache-retention allowance,
not a total bound on documents retained by active callers.
At most 16 mutation jobs can be outstanding. Dynamic registration saturation
emits WARN and returns an explicit busy error; static catalog publication emits
WARN with limit 16 and waits for the same shared gate. Publication and collision
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
settings where applicable. Mutation saturation, transport timeouts and rejected destination/reference/server
policies also warn.
Limits reject visibly rather than trimming documents; warnings never include
submitted names, URLs, schemas or upstream prose.

Agent destinations require public HTTPS without userinfo or fragments. Local/special
addresses, private names, redirects and local files are rejected. Direct connections
validate DNS answers at dial time. Optional `allowed_origins` narrows specification
and API destinations to exact canonical HTTPS origins. Undeclared cross-origin API
routes and help URLs are rejected; a cross-origin help URL requires its origin in
`allowed_origins`. All production traffic uses the network factory. Dynamic API
clients and lazy help retain public-destination restrictions at execution, including
after binding, while help uses its own discovery-origin isolation identity. Accepted
bundled protocol compatibility applies to schema acquisition, refresh, API and help
through that factory; generic imports and raw calls retain strict defaults.

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
source UUIDs, endpoint ownership, revisions and tombstones. A bundled source also
saves its provider identity and effective catalog settings as one accepted
definition. Catalog construction applies canonical provider names and name-keyed
overrides before rebinding tools to `dyn_<registration-name>__<operation>`; the
provider prefix is not repeated. Accepted settings preserve base repairs,
compatibility filters, descriptions, help, instructions, pricing metadata and image
mappings. They do not activate static caches, warming, relays,
cover configuration or recommendation selection, and grant no wallet/payment
authority. Legacy generic records retain their existing settings and identity.

The registry contains no wallet keys, but URLs and documents can reveal operator interests. Back it up while
serving is stopped. Never delete or replace its ownership sidecar while serving.
Registry files and sidecars cannot alias configuration or treasury paths.

`config show` inspects configuration without opening registry state or secrets.
`sources inspect --config FILE` reads saved registrations and never creates a missing
registry. It does not require listener tokens, wallet keys or catalog requests.
Each source includes `tools`, an array of generated `name`, HTTP `method` and
`path` entries rebuilt from its accepted specification and saved catalog settings
or legacy generic selection. Inspection does not resolve the current bundle.
Disabled sources retain their saved inventory; removed sources have an empty list
because removal clears the specification and accepted provider settings. An invalid
saved catalog reports
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

Generic refresh fetches and validates new bytes through the configured network
policy without pricing probes. Origin retrieval needs no wallet credentials. Only
when an eligible relay acquisition is necessary does refresh load the saved owner's
selected existing payer. Static mode reads only that profile's key; managed mode
opens the authenticated existing store and named pool with Base transport, without
SDK reconstruction, pool/wallet allocation or funding workers. Unrepresentable
relay requests do not load a key or store. The owner remains alive through registry
publication and closes on success or failure; errors retain possible charge facts. Curated refresh resolves the
current bundled provider definition and acquires its remote or embedded schema,
then atomically replaces settings, provenance and document together while retaining
owner, name and source identity and advancing the revision. A missing provider or
failed refresh preserves the accepted revision. Removal records a tombstone. Neither
command
changes wallets, payment reservations, funding/refund records or reconciliation.
Temporary registrations need no registry maintenance; they disappear on exit.

Restart rebuilds from accepted bytes and saved settings without vendor requests
or bundle re-resolution; a binary upgrade alone cannot change accepted provider
semantics. Saved inventory, restoration, publication and refresh use the same
catalog construction path. Runtime publication applies the listener's tag and
tool-name projection after full catalog validation; operator refresh validates the
full catalog without tool-count admission checks. Saved inventory inspection remains
unfiltered. A listener may filter every tool
without invalidating the accepted source. Disabled endpoints,
nonlocal saved bindings, invalid formats and tool-name conflicts cannot regain access
silently. Corrupt/unsupported registries fail startup. Persistent additions commit
before publication; cancelling an accepted commit's waiter cannot undo the write.
