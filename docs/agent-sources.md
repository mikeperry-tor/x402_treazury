# Agent API discovery and source management

For a smaller initial MCP tool list, set `discover_on_demand = true` in a
`[servers.NAME]` table (default `false`). This is independent of provider management.
On-demand listeners advertise `x402_treazury_tools_search` and
`x402_treazury_tool_call`, plus any independently enabled provider-management or raw-call
tools. API tools, including subsequently registered tools, stay out of `tools/list`.

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
requires no provider-management wallet or registry for static listeners. Imported OpenAPI catalogs share default same-path POST preference before naming;
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
`providers_list_local.q` searches bundled provider IDs, display names,
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

`x402_treazury_providers_list_local` enumerates and searches locally bundled,
reviewed provider definitions, including compatibility filters and repairs for
supported API operations. Optional capability keywords in `q` use the same BM25
and literal-match priority as callable tool search across IDs, names, descriptions
and declared API bases. Omitted or blank `q` enumerates in stable provider-ID order;
nonblank results rank before pagination, with provider ID breaking ties. `page`
defaults to 1; `per_page` defaults to 25 and accepts any positive integer.
The response contains `items`, `total`, `page`, `per_page` and `next_page`; items
include `provider_id`, name, description, optional API base and schema provenance.
Listing makes no network requests, selects no payer and never falls back remotely.
Use the exact ID with `providers_add_local` to register its prepared definition.
An absent `api_base` means import resolves the API server from the schema, which
need not share the specification URL's origin. Bundled metadata is independent of
installed tools and does not establish current availability or live qualification.

`x402_treazury_providers_search_remote` starts one workflow for every directory
in the endpoint's `remote_provider_directories`, concurrently. It is exposed only
with remote management enabled and a nonempty directory list. Agents cannot
select directories. Shared inputs are optional capability keywords `q`, `limit`
(default 5, range 1–20 requested from each directory), and numeric `max_price_usd`.
When either Coinbase directory is selected, `q` accepts at most 400 Unicode
characters; x402-list-only endpoints have no Coinbase query bound. Invalid or
retired branch, ranking, pagination and directory-selector arguments fail before I/O.

| Directory key | Native search |
| --- | --- |
| `coinbase` | Coinbase resource search with `curatedOnly=false`, including curated and non-curated resources. |
| `coinbase_curated` | The same Coinbase resource search with `curatedOnly=true`. |
| `x402-list` | x402 List `GET /best`, with `prefer=balanced`, `require_verified=false` and `network=BSE`. |

Coinbase requests retain server-selected Base mainnet USDC/exact constraints.
Price filters concern advertised prices, not spending authority: Coinbase filters
resources, while x402 List filters each service's minimum listed price. These
are different coverage contracts. Advanced x402 List browse/filter operations
remain available through its ordinary provider definition when configured or registered.
Neither remote directory uses the local catalog's BM25 ranking.

Each search returns one JSON dictionary keyed by the configured directory names.
Every successful value is the complete native upstream JSON object, preserving
array order, duplicate resources across directories, unknown fields, missing
metadata, counts, verification/provenance and partial-result indicators. Both
Coinbase selections produce separate responses when enabled together. There is
no merged list, synthetic pagination or total, acquisition wrapper, provider-match
enrichment or invented schema. Empty or limited results do not prove absence.

A failed directory returns a locally generated value such as
`{"local_error":{"kind":"directory_request_failed","payment_may_have_occurred":false}}`.
`local_error` identifies a local failure, never a fabricated empty upstream result;
an available safe HTTP status can accompany it. Non-JSON or invalid
responses fail that directory explicitly. Successful siblings remain complete,
even if every other directory fails. Required x402 List attribution and relay
advisories accompany the output separately, without changing successful JSON.
Cite x402-list.com for its CC BY 4.0 directory data.

One search can authorize more than one directory or relay charge. Each workflow
retains its own origin, Tor, response-inactivity, relay and payment rules. There
is no federation replay after a failure, empty result or partial result, and no
whole-search deadline, combined truncation, discovery cache or background index.
A failing sibling cannot cancel progressing work; accepted paid child work
remains owned through waiter cancellation and shutdown. Typed possible-payment
evidence survives aggregation, parsing and later output errors.

### Coinbase resource search and relay behavior

Coinbase output retains exact URLs, methods, Bazaar metadata, offers, skill links
and missing request contracts. HTTP examples and Bazaar envelope schemas are not
request argument schemas. Documented HTTP requests can use `raw_call` when enabled;
known OpenAPI URLs can use `providers_add_remote` when remote management is enabled.
MCP/unknown protocols remain metadata only. A published usage-document URL can be
read with `provider_details_remote(documentation_url=...)` when available.

Public Coinbase reads are unsigned and use an existing endpoint wallet's
documentation identity without allocation, funding or observation. Missing
identity fails that workflow. Direct mode uses one origin attempt; Tor adds
`network.tor_isolation_retries` eligible fresh-isolation retries (default 2), with
the shared eligibility and `Retry-After` policy, including terminal delay before
fallback. Only complete valid search JSON may promote an origin scope. HTTP 206,
Content-Range and complete invalid JSON fail without paid fallback or replay.
No Coinbase challenge is signed.

With `[relay_fallback]` configured, `source_discovery=true` enables built-in
Coinbase and x402 List fallback by default. `provider_metadata=true` independently
enables schemas, imports/refresh, help, usage documents and URL/spec probes. Actual
API and raw calls require explicit `api_requests=true`; an ordinary call to a
directory URL remains API work. In mixed `provider_details_remote` workflows,
the directory lookup is discovery and later documents/probes are metadata.
Trusted code selects categories; agent arguments, method or hostname cannot.
With no relay section all requests remain origin-only.

Relays use the endpoint's existing wallet regardless of a static relay wallet
override, with ordinary payment admission, journaling and insufficient-funds
traversal. Each authorization may charge; uncertain paid failure grants no replay.
The selected relay sees the query URL and supplies target content/status/headers
under the configured direct/Tor policy and relay trust boundary. Origin challenges
retain ordinary handling for paid directory workflows; relay-supplied target
challenges are terminal. Earlier target signatures suppress generic fallback.
Local input, origin, identity and download-limit failures authorize no relay spending.
See [relay configuration](configuration.md#relay-fallback) for settings and trust.

## Tool documentation

Agent-facing tool descriptions should briefly explain what the tool does, when
to use it and how to use its results. Include request, payment or retry constraints
when they affect the caller's decision or correct use. Describe only available
actions and inputs. Keep implementation details, maintainer rationale and repeated
workflow explanations in the owning guides rather than tool context.

Document every parameter in the same concise, task-focused style: its meaning,
expected value or format, and omission/default behavior and interactions where
they matter. Use schema types, required fields, enums and bounds for structural
constraints; avoid restating those mechanically in prose. Prefer one useful
example over a long explanation when a value would otherwise be ambiguous.

Local-provider help should identify locally bundled definitions and their
compatibility filters and repairs for supported API contracts. This describes
reviewed compatibility work; it does not establish current upstream availability
or universal paid-delivery qualification. Keep claims within the evidence and avoid
repeating generic caveats in each tool and parameter.

Review new and changed help together with its parameter descriptions. Prune
unnecessary internal details and repetition before delivery; preserve information
needed for valid requests and correct interpretation of results.

## Endpoint instruction placement

Each `[servers.NAME]` can customize three instruction surfaces:

| Setting | Placement and semantics |
| --- | --- |
| `instructions_text` | Replaces the composed source guidance in `initialize.result.instructions` and current catalog metadata. Omit to retain source composition; `""` suppresses inherited source guidance. Generated pricing, discovery and availability/conflict notices remain. |
| `discovery_search_tool_instructions_text` | Appends to the standard description of `x402_treazury_tools_search`, exposed by `discover_on_demand` or either provider-management capability. |
| `providers_search_remote_tool_instructions_text` | Appends to `x402_treazury_providers_search_remote`, exposed by remote management with a nonempty directory list. |

These are endpoint settings, not provider/source settings. They do not enable tools
or grant permissions. Absent or empty search text leaves the standard description
unchanged. Both search fields preserve the standard descriptions and input schemas;
they do not alter search matching or routing. Inspection inventory and MCP tool
lookup/listing use the same text. Configured search guidance is available even while
remote catalogs are pending; it describes intended coverage, not current readiness.

When adding or removing capabilities, or changing source/listener filters and
routing, review the affected endpoint summaries and discovery search keywords in
the same change. Cover the capabilities actually selected on that listener with
concise task and domain terms, including useful aliases for newly available
workflows. For endpoints without tool search, keep that coverage in
`instructions_text`. Avoid duplicating the full tool inventory.

Put platform/capability summaries and useful search terms in the tool-search field
when a client does not forward MCP server instructions to its model. Use the
remote-search field for guidance about finding new APIs in configured directories;
`tools_search` searches APIs already available on the listener. To avoid repeating
source guidance, override initialization with concise endpoint instructions.
Keep essential operation requirements in the relevant tool descriptions or include
them in the endpoint search guidance when suppressing inherited instructions.
Prefer brief endpoint guidance even when clients forward server instructions:
some clients repeat initialization guidance alongside each tool. The server sends
`initialize.result.instructions` separately from `tools/list` descriptions. Keep
request quirks and result-coverage limitations with their affected tools so endpoint
overrides do not hide them; keep provider inventories and maintainer rationale out
of initialization text.

```toml
[servers.research]
# Existing listen, wallet, sources and authentication settings also apply.
discover_on_demand = true
instructions_text = "Research endpoint. Discover schemas before invoking tools."
discovery_search_tool_instructions_text = "Search this listener for its configured social platforms; use Twitter for X."
# With remote management and directories enabled, customize remote search:
providers_search_remote_tool_instructions_text = "Prefer documented research APIs with published OpenAPI specifications."
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


## Configuration and authority

Provider management and direct URL calls are independent endpoint capabilities,
disabled by default. Standalone provider serving remains static. The
[managed starter](../examples/deployments/managed.toml) and
[comprehensive toolbox](../examples/deployments/comprehensive.toml) enable all three:

```toml
[servers.research]
wallet = "research" # Existing [wallets.research]; no new pool required.
provider_management_local = true
provider_management_remote = true
remote_provider_directories = ["coinbase", "x402-list"]
# coinbase_curated is the curated-only alternative; coinbase includes both scopes.
raw_url_calls = true
# source_registry_file = "../../state/managed-sources.sqlite" # Optional persistence.
# Retain existing listen, bearer_token_env and sources settings.
```

| Setting | Default | Permitted new operations |
| --- | --- | --- |
| `provider_management_local` | `false` | Offline bundled listing/search and registration by exact bundled ID. |
| `provider_management_remote` | `false` | Configured directory search, remote details and public HTTPS OpenAPI registration. |
| `remote_provider_directories` | `[]` | `coinbase`, `coinbase_curated`, `x402-list`, in any subset. |
| `raw_url_calls` | `false` | Documented public HTTPS calls through `raw_call`. |

Unknown directory identifiers fail configuration parsing; identical repeats select
a directory once, while both Coinbase choices remain distinct. The list grants
no authority when remote management is disabled. An empty list still permits
remote registration and URL-based details, but omits remote search. Directory
selection is not a destination allowlist for APIs or permitted raw calls.
The retired `source_management` and `sources_search_tool_instructions_text`
settings reject rather than alias the new preferences.

Requested operations are checked before identity lookup, upstream I/O or
registration side effects. Tool hiding alone is not authorization. Local-only and
raw-only endpoints do not require remote permission. Either management capability
exposes `tools_search` and `tool_call`; `discover_on_demand` can independently expose
them, while raw permission alone grants no registration authority. Existing accepted
registrations retain their filters and payer bindings when switching between local
and remote management. As before, disabling both management capabilities disables
saved dynamic registrations during restoration; raw permission alone does not
reactivate them. Preferences do not add per-origin or per-registration invocation
rules. Inspection and removal remain available regardless of preferences.
Operator refresh requires either management capability before loading the selected
existing wallet, regardless of whether the saved definition is generic or bundled.

Omit `source_registry_file` for registrations that last only until exit. Static sources
are optional: set `sources = []` for a discovery-only listener. A second listener
can enable provider management with the same shared wallet and its own authenticated
identity; registrations remain separate even when both listeners use one pool.
Static profiles also work here when the operator defines the referenced wallet
with `mode = "static"` and `private_key_env`.

Each enabled endpoint must name an existing wallet with `wallet` and may set its
own `source_registry_file`. No automatic wallet allocation occurs per source.
Managed wallet initialization and funding follow the normal operator-authorized
startup policy. Registration never funds wallets or
probes endpoint prices; paid invocation checks readiness and ordinary payment limits.

A listener's authenticated endpoint ID is the principal. All agents using its bearer
token share authority. Agent/session fields cannot choose an owner. Each registration
is visible only on the endpoint that added it; there are no publication targets,
receiver roles or process-wide grants. Two endpoints adding the same specification
get independent registrations, even if they share a payment wallet. Static TOML
sources cannot be modified by agent tools.

Persistence belongs to the operator. With `source_registry_file`, all additions are saved;
without it, additions disappear on exit. Agents cannot select lifetime, wallet,
timeouts, payment limits or visibility. Existing endpoint tag/tool filters still
constrain imported API tools. Include `dyn_*` when an endpoint has an explicit
`include_tools` allowlist, or use `dyn_netintel__*` to select a named registration.
Filters use source names, not registry IDs; existing ID-based filters are not rewritten.
The `x402_treazury_` and `dyn_` namespaces are reserved in
participating deployments. Management tools follow their respective capability preferences and are
not removed by API filters.

## Agent tools

| Tool | Arguments |
| --- | --- |
| `x402_treazury_providers_list_local` | Local management: optional capability keywords `q`, `page`, `per_page`; list bundled definitions offline. |
| `x402_treazury_providers_add_local` | Local management: required exact `provider_id`, optional registration `name`. |
| `x402_treazury_providers_search_remote` | Remote management and nonempty directory list: optional `q`, `limit`, `max_price_usd`; query every configured directory. |
| `x402_treazury_providers_add_remote` | Remote management: required public HTTPS OpenAPI 3 JSON `spec_url`, optional registration `name`. |
| `x402_treazury_provider_details_remote` | Remote management: exactly one `x402_list_id`, public HTTPS `base_url`, or `documentation_url`. ID mode is available only with `x402-list` selected. |
| `x402_treazury_raw_call` | Raw URL permission: required `method`, public HTTPS `url`; optional `query`, `headers` and JSON `body`. |
| `x402_treazury_tools_search` | Either management capability or on-demand discovery: optional `query`, `source_name`, `cursor`, `limit`. |
| `x402_treazury_tool_call` | Same exposure as tool search: required readable `tool_id` and `arguments`. |

Only enabled tools and permitted follow-ups are advertised. Neither authored
endpoint instruction text nor API tool filters grant management authority.

### Listing and URL discovery

All details modes require remote management. `x402_list_id` is hidden and rejected
before identity lookup or I/O unless `x402-list` is configured; URL modes work with
Coinbase-only or empty directory selections.

Given a resource URL such as `https://api.example.com/weather/v2/forecast?city=London`,
try `provider_details_remote(base_url="https://api.example.com/")` when no API base
is documented. This probes fixed document locations at the supplied root; it does
not call the resource or prove that the root is the API base. Prefer an explicitly
documented base path: root discovery can miss documentation beneath another path
or on another host. Bundled matching remains exact by origin and base path.
`x402_list_id` is the exact native `slug` from x402 List, not a registration ID
or Coinbase resource URL. A Coinbase usage-document URL can be passed directly
as `documentation_url`.

With `documentation_url`, `provider_details_remote` reads an exact caller-supplied public
HTTPS usage-document URL, including its query when present. This is optional;
sufficient documented method, URL and arguments already permit `raw_call`, and a
known OpenAPI URL can go directly to `providers_add_remote`. A Coinbase `skillUrl` is a lead,
not a verified instruction source. The response labels provenance
`caller_supplied` and returns `documentation` with the requested URL, content type
when available, and visible status. `ok` and `empty` include complete `text`;
failures omit text and report safe reasons, HTTP status or the affected byte limit.

The reader uses the shared unsigned origin sequence and, when configured, the
on-demand GET relay list. It performs no HEAD request or linked traversal. The
listener's available documentation identity, transport, public-destination policy,
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

`provider_details_remote` resolves bundled provider definitions before discovering OpenAPI,
x402 discovery and LLM documentation URLs. With a directory `x402_list_id`, it fetches that
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
`providers_add_remote` still performs full import validation. `documentation_found` points to
concise documentation; `directory_json_unverified` is only an unverified lead.
Other statuses expose missing, redirected, blocked, invalid, oversized or unavailable
candidates. Nothing found does not prove the API is unusable.

Probes use a pinned available provider-management wallet isolation identity, including
listener overrides and Tor remote DNS. Without one, the URL's discovery identity
keeps unsigned origin retrieval available. Identity selection does not allocate, fund or
promote a wallet. Directory and probe work remain owned through caller cancellation.
No new discovery cache or registration is created.

If you already know a published OpenAPI URL, pass it directly to `providers_add_remote`;
`provider_details_remote` is optional. Otherwise it can locate a specification or concise
provider documentation with a published link. Do not download large OpenAPI
specifications or full API directories into agent context: import them server-side,
then retrieve relevant tool schemas with `tools_search`. Never invent specification
URLs or paid request arguments. If no specification exists, a documented API can be
called with `raw_call` without registration.

### Direct calls to x402 URLs

Imported OpenAPI tools expose declared business headers and automatically manage
declared `Idempotency-Key` headers under the [shared header contract](configuration.md#openapi-header-parameters-and-idempotency).
This applies equally to direct tool invocation and on-demand `tool_call`; a new
invocation is a new operation, even with identical arguments.

`raw_call` calls any documented public HTTPS x402 API URL. Supply its exact HTTP
`method`, optional `query` parameters and optional JSON `body`. The method is
validated as an HTTP token and forwarded without changing its spelling. Query
values can be strings, numbers, booleans or arrays of those scalars; arrays repeat
the key. They are URL-encoded and appended to any existing URL query. A supplied
body is sent exactly as JSON with `Content-Type: application/json`, including
explicit `null`; omission sends no body. The wrapper adds no method/body policy.

The tool requires `raw_url_calls` and uses the endpoint's existing wallet, ordinary payment
admission, configured direct/Tor transport and response limits. It creates no
registration or wallet. API tool-name filters do not restrict it. Existing public
HTTPS, no-URL-credentials, origin and redirect checks remain server responsibilities;
wallet selectors and payment settings are not agent arguments. Binary
PNG/JPEG/WebP output uses the normal bounded image path; JSON image mappings require
a configured provider and are not inferred by raw calls.

Calls can spend Base USDC and can change provider state. Follow the documented
method and arguments. Directory verification and a payment challenge do not prove
a request schema or guarantee useful output. Unsigned requests retain the
[accepted retry semantics](architecture.md#unsigned-x402-requests-and-post-retries).
A signed, timed-out or ambiguous call may already have charged or acted; a new
invocation is a new logical call and may authorize another payment.

#### Manual raw headers

Optional `headers` is an object of HTTP header names and string values. Supply
only headers documented by the API, including custom `X-*`, explicit
`Authorization` (for example `Bearer example-token`) and a caller-owned
`Idempotency-Key`. No OpenAPI registration is required. Omitted or empty headers
retain ordinary raw-call behavior. Accept, Content-Type, cookies, HTTP
transport/proxy/framing and x402 protocol headers remain reserved. Names and values
are validated before I/O; case-insensitive duplicates and nonstring values fail.
MCP supplies an already parsed JSON object: exact duplicate JSON keys have already
collapsed and cannot be detected here. Case variants remain detectable.

Every nonempty manual set is caller-private: arbitrary names can hold credentials,
including innocuous business names. The call never uses a paid relay and stays on
its initially selected existing payer. A price handoff or recognized insufficient
funds stops the operation before another wallet's I/O, retaining any possible-charge
evidence; it allocates no wallet and waits for no funding. Eligible same-payer
GET/HEAD retries preserve exact values. Manual state-changing requests can execute
without payment, so they recover only from TLS connection-establishment failures,
with no status/body-driven or post-dispatch TLS replay. A failure before response
headers arrive does not prove the write was undelivered. An actual 402 can still
be signed once.

A supplied idempotency key is sent unchanged, never generated, replaced or remembered.
It grants no replay authority, vendor deduplication guarantee or protection against
multiple x402 debits. Reusing a key in a new invocation is a separate caller decision.
Accepted work retains its exact request after waiter disconnect until processing
finishes; no host credential store, cross-call deduplication or recovery journal is added.

Manual secrets appear in agent-authored arguments and may remain in client history.
They do not satisfy a provider's host-only credential contract. Application-owned
diagnostics, route previews and validation errors omit manual values. The
unmodified `rmcp` SDK can log raw tool arguments at DEBUG/TRACE, including
`Authorization` supplied to raw GET calls or any other method, before application
validation. SDK malformed-frame diagnostics, result/error logging and some
HTTP transport-error bodies can also expose caller input or upstream credential
echoes. Avoid verbose SDK logging with secrets, including in embedded consumers
with their own tracing subscribers. Upstream raw responses and HTTP error bodies
are not redacted: an upstream can echo a credential into ordinary output. This
feature cannot clear secrets held by an agent; resending them after rotation can link wallet identities.
Authorization is an invocation-local raw-call exception. Generated tools and
public Rust routed business headers still cannot inject it. Automatic OpenAPI
credential lookup/capture remains a separate, unimplemented mechanism; manual calls
have no access to host-managed credentials. When managed auth is implemented, reject
mixed manual/managed modes before credential lookup or I/O; raw permission cannot
authorize automatic credential access or a cross-payer continuity override.

### Registration and tool invocation

`providers_add_local` accepts an exact `provider_id`; `providers_add_remote` accepts
a public HTTPS OpenAPI 3 JSON `spec_url`. Both accept optional `name` and share
the same import/commit path under their respective operation permissions.
For example, `providers_add_local({"provider_id":"exa"})` registers the bundled Exa definition. A bundled
registration uses its reviewed catalog configuration and acquires its remote or
embedded schema server-side; it preserves base repair, selections, overrides,
help, guidance and response mappings without activating provider recommendations.
No agent-selectable local path or data installation is needed.

A specification URL first returns any existing generic registration for that
endpoint and canonical URL unchanged. Otherwise a unique exact match against
current bundled remote specification URLs selects the bundled definition; multiple
matches require operator resolution, or an available local choice through
`providers_add_local` when local management is enabled. Remote import does not
accept a provider-ID escape hatch. Full canonical specification URL matching
includes significant query components and works even with local management disabled;
API bases, hostnames and inferred resource paths are not specification matches. Other URLs retain generic OpenAPI 3 JSON
import, with the API base from the first root server resolved against the spec URL.
An arbitrary document's `servers` claim never selects a bundled provider.
It returns `name`, `tool_prefix`, filtered `tool_count`, `status` and `next`; disabled registrations also include `reason`. Bundled results and operator inspection also report the accepted `provider_id` and schema provenance.
`next` suggests search then invocation only when tools are visible. A disabled
registration needs operator repair/refresh; repeating its accepted selector with
the corresponding add tool returns that same registration. An enabled registration with zero visible tools retains
`status="available"` and points to source selections and listener filters. Searching
without `source_name` can find other endpoint tools, but does not alter operator
filters or guarantee a callable result.
Registration never funds wallets or probes prices.

Static deployment [source distribution](configuration.md#source-tool-distribution)
does not extend registration authority. Dynamic imports cannot supply `send_tools` and
remain owned by the authenticated endpoint; they cannot target sibling listeners.
Raw-call permissions remain independent of static routing and are not constrained
by its tool selectors.

Names preserve case and allow at most 64 ASCII letters/digits/underscores/hyphens;
`__` is reserved as the source/operation separator. Explicit names must be unique
within the endpoint, including disabled registrations and configured source names.
Configured names are reserved for every effective static binding, including
declared `send_tools` destinations whose catalogs are pending or currently contribute no
tools. Exclusion-only sources reserve no endpoint name.
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

Enabled capabilities determine the management definitions advertised. Eager endpoints also advertise
registered API tools, including restored registrations; `discover_on_demand` keeps
those API definitions out of `tools/list`. HTTP is stateless and does not advertise
list-change notifications, so clients may cache their inventory. Search/call remains
usable with that cached inventory. Search cursors bind the running catalog,
endpoint, query/filter and generation; stale cursors require restarting pagination.
All listeners return the complete advertised inventory in one `tools/list` response,
without `nextCursor`; supplied listing cursors are ignored. Descriptions and
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

There is no tool-count admission quota. A `tools/list` request with more than 500
advertised definitions emits one advisory WARN whose message states the endpoint
name and current tool count and suggests `discover_on_demand = true`. Counts include
filtered static/dynamic API definitions and advertised management wrappers;
on-demand listeners count only wrappers. Each listing may warn again. This observes
advertised inventory, not client/model context, and never rejects registration or
changes access/mode. The operator must explicitly opt in to on-demand discovery.

Generic imports use the shared specification default of 32 MiB, response default
of 16 MiB and help default of 4 MiB. Raw calls and exact usage-document reads use
the same response and help defaults. Bundled imports preserve the provider's
configured byte limits and read-inactivity timeout; an omitted provider timeout
inherits the network default (60 seconds in either transport). Generic imports,
raw calls and exact usage-document reads inherit the network timeout. There are no
provider-management overrides. Spec, help and API responses have no total HTTP
download deadline.

Management requests have a 64 KiB limit; forwarded API calls retain normal API behavior.
Remote imports have no separate per-listener, global-import or active-cache admission
quota; they retain the existing foreground work ownership. Equivalent aliases share
an active fetch keyed by payer authority, document policy and transport settings.
Active fetches cannot be evicted by retention age or completed-cache churn.
Completed fetches retain successes for 300 seconds and failures for one second;
optional completed retention is trimmed on lookup and completion to four entries.
This is a cache-retention allowance,
not a total bound on documents retained by active callers.
At most 16 mutation jobs can be outstanding. Dynamic registration saturation
emits WARN and returns an explicit busy error; static catalog publication emits
DEBUG with limit 16 and waits for the same shared gate. Publication and collision
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
settings where applicable. Rejected mutations, terminal transport failures and rejected destination/reference/server
policies also warn. Retained waits and completed-cache eviction log at DEBUG.
Limits reject visibly rather than trimming documents; warnings never include
submitted names, URLs, schemas or upstream prose.

Agent destinations require public HTTPS without userinfo or fragments. Local/special
addresses, private names, redirects and local files are rejected. Direct connections
validate DNS answers at dial time. Undeclared cross-origin API routes and generic cross-origin help URLs are rejected.
Bundled providers retain their reviewed, authored help origin. An accepted provider's
explicit [operation URL extension](configuration.md#operation-url-extensions) also
permits the public HTTPS targets declared by that extension, including other
origins; generic imports retain same-origin API routing. All production traffic uses the network factory. Dynamic API
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

Dynamic sources use the endpoint's `wallet`. Endpoints referencing the same named
wallet share its payment identity. Registration never creates per-source funding groups.
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
Registry files and sidecars cannot alias configuration or treasury paths. Each
endpoint uses its own registry file; exclusive ownership prevents two endpoints
from opening the same file. Saved records owned by another endpoint fail restoration
without altering the registry.

`config show` inspects configuration without opening registry state or secrets.
`sources inspect --config FILE` returns an object keyed by endpoint ID, with each
persistent endpoint's saved generation and source inventory. It never creates a
missing registry. It does not require listener tokens, wallet keys or catalog requests.
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
x402_treazury sources refresh --config FILE --server research --source SOURCE_ID_OR_NAME
x402_treazury sources remove --config FILE --server research --source SOURCE_ID_OR_NAME
x402_treazury sources remove --config FILE --server research --all
```

`--source` selects an exact saved ID first, then an exact name among sources that
have not been removed. A removed ID fails rather than selecting another source by
name. An ambiguous name fails without changing the registry;
use its ID instead. Disabled sources can also be removed. `remove` requires exactly
one of `--source` or `--all`; `refresh` requires `--source`. Both commands require
`--server` and operate only on that endpoint's registry. `remove --all` atomically
records tombstones for all remaining sources on the endpoint. An empty selection
succeeds without saving the registry. Removal performs no network or wallet work.

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
