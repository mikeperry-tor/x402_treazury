# Runtime architecture

Treazury is one Rust process exposing HTTP APIs as MCP tools and handling x402
payments on the caller's behalf. `src/main.rs` selects standalone stdio/HTTP
serving, a multi-listener TOML deployment, or wallet/source administration. There
is no Python server, external wallet daemon or payment logic in the calling agent.
See the [README](../README.md) for commands and configuration examples.

The qualification runner keeps phase prerequisites and outage handling around
explicit round and batch operations. A round owns its rotation controller through
restart and finalization; a batch reserves before dispatch and drains accepted work
before assessing rotation and settlement. Reservation refusal stops that round's
batch loop but still reaches controller finalization. The execution origin is
captured once after preparation; extraction boundaries never renew deadlines or
spending authority.

## Catalog, configuration and execution

Standalone commands separate inspection/routing from server construction. Tags
inventory raw source metadata, including unsupported operations, before base-URL
or tool compatibility validation; ordinary tools/check/route inspection needs no
payment credentials. Serving validates spending policy and credentials before accepting calls. The stdio lifecycle retains one pinned service waiter and the
accepted-work owner through cancellation, drain and cover shutdown.

Deployment catalog inspection and unsigned loading/binding do not read treasury
state. Credentialed serving recovers saved direct-operation ownership before
financial initialization. Binding an inspected deployment performs that recovery
and rechecks effective listener selection before opening its financial runtime;
see [direct swap control](configuration.md#direct-swap-control).

`src/config.rs` composes inline source settings with one level of provider TOML
inheritance. Local values replace inherited values, including lists/maps; omitted
values inherit. Paths belong to the file declaring them. Unknown fields and
unsupported compositions fail explicitly. `config show` resolves composition,
permissions and wallet bindings without loading specs or reading keys.

`src/catalog.rs` loads OpenAPI JSON and builds tool schemas, names, descriptions
and path/query/body routing. Canonical descriptions retain complete fallback pricing
for indexing, inspection and persistence. Runtime-only rendering metadata retains
the pricing-free base and configured character limit. `BoundTool` uses the acquired catalog cache handle and original method/path
template for both description lookup and invocation. Relay and explicit direct-warm
provenance travel with the raw document through binding. Fresh get/list/search results render the applicable hint
without mutating catalogs, ranking or generation-bound cursors. Curated rebinding
updates both the canonical text and retained base; serialized registrations rebuild
metadata on import. Listener selection reuses accepted tools; it projects source
operations for tag matching only when that listener configures tag filters.
Building a tag-free listener inventory needs no schema projection.
Advertised-price matching reads borrowed method/path and configured URL metadata
from the original OpenAPI or digest inventory without projecting request schemas
again. Hidden or unsupported operations still participate in ambiguity checks.
Provider files in `providers/` carry selections, help URLs, pricing settings and
explicitly reviewed response mappings. Curated
local specs are inputs for providers whose public schema needs adaptation; the
fixture utility is `tools/providers/snapshot_spec.rs`. Tests pin generated contracts
against `tests/fixtures/`. Operation validation and stable sorting precede path,
method and tag selection. Exact original `exclude_operations` identities override
operation inclusions without hiding child paths or depending on generated names;
absent exclusions remain valid under independent source filters. Exact same-path
GET/POST pairs retain POST only after unsupported input contracts have been removed; supported POST wins by default (`prefer_post` can
disable this). Suppressed exact GET twins are outside both selected tools and
omissions; disabling preference exposes their compatibility issues. An unsupported
POST cannot suppress a compatible GET. Only surviving operations contribute to
name collisions.
Tool-name filters apply after collision resolution. Recommendation metadata is
advisory until provider/source `use_recommended_tools` opts in. Include, exclude
and recommendation selectors validate against the same compatible named inventory
including help, then a single intersection selects tools. Listeners narrow that
result. Empty active advice fails without
expansion, while unused stale advice only warns. Operator inventories add
recommendation markers separately from `ToolSpec` and MCP publication. Explicit
`catalog tools --all-tools` bypasses only advice after typed configuration resolution,
retains ordinary filters. Digest metadata copying and
OpenAPI operation conversion remain separate. An iterative guard checks physical input before cloning; conversion charges actual
reference expansion and output copying against the shared bounds of depth 64,
500,000 nodes and 64 MiB of string/key traversal and copy work. Repeated
references and local pointer lookup bytes count each time.
OpenAPI paths and opaque operation metadata count for each emitted operation,
including repeated pricing fields. Parameter issue metadata and descriptions
copied from projected schemas also count before allocation; moving already
projected parameters or bodies does not charge their contents a second time.
Used media names, including malformed or unsupported ones, are charged before
parsing on each request-body expansion. Explicit candidate names stay borrowed;
wildcard synthesis and selected media output charge their actual copies before
allocation. Candidate ordering compares borrowed text without cloned sort keys;
ordinary sorting/indexing is not an instruction-count or total-memory bound.
Media candidate parsing/range matching and each inspected parameter-name/value comparison charge the same
node/byte budget before comparison; nested lookups cannot bypass it. Unused definitions are
not hypothetically expanded. Raw-Value operations, naming and tool-building APIs
use the same guard as local, origin, cached and relay catalog acceptance.
Structurally unsafe candidates cannot enter or refresh caches. Each structural-budget
rejection emits a sanitized WARN at the shared guard, including caught errors and
rejected candidates with fallback remaining; no catalog text or URLs are logged. Input schemas use one schema-position-aware projection: only used schema refs
expand. Every descent checks its increasing depth before lookup or output allocation,
so even flat reference chains cannot grow the projection stack past that bound.
Opaque values are charged before cloning, and completed child output moves into its
parent without another traversal or uncharged breadth allocation. Pointer lookup
and exact normalization proofs are charged once for their actual work. Used
refs requiring resolution in an active string-valued `$id` resource receive an
explicit unsupported-operation issue; no URI base is guessed. Target ancestry
carries context for further refs, while exact no-ref targets remain supported.
No-reference IDs and opaque ID data remain unchanged. Used
`$dynamicRef`/`$recursiveRef` cannot retain their resolution semantics after
schema relocation and receive `input_reference`, rather than dangling tool refs.
OpenAPI3.0 `$ref` replacement still ignores unused siblings. Reference-like
keys in opaque data or unused definitions do not trigger this check. Example,
default and unknown annotation payloads remain opaque and unchanged. Parameter and
RequestBody Reference Objects retain target replacement; unresolved input contracts
become operation issues. Dynamic imports retain separate stricter whole-document
reference admission. The direct public sanitizer remains iterative.
Conversion retains operation-level
parameters and the existing reference-resolution rules. Input construction reserves original argument spellings and assigns collision-safe
names across path, query, body and header locations; serialized wire aliases retain
original keys. Required simple object bodies may remain flat; optional bodies and
compositions that cannot be flattened faithfully use one JSON body argument.
Inner schema requirements and HTTP body presence remain separate. Complete body
constraints/annotations stay body-local. Property-bearing allOf and
closed/exclusive alternatives retain their whole body schema; only pure
required-only alternatives on ordinary object roots stay flat. Presence-only alternatives follow
renamed body arguments.
Boolean exclusive bounds normalize only at actual schema positions; tool assembly
does not reinterpret annotations as schemas. Path filters respect segment boundaries.
Declared business header parameters retain their original names and simple-style
serialization in serializable tool routing metadata. Collisions with flattened
query/body arguments use a `_header` argument suffix. OpenAPI's ignored Accept,
Content-Type and Authorization parameters remain ignored. x402 protocol header
declarations also stay out of tool inputs, including required declarations;
the payment runtime owns those headers. Cookies and HTTP transport/framing
declarations are rejected. Runtime-owned headers cannot be supplied as business
headers through persisted or directly constructed routing metadata.
A declared Idempotency-Key is generated once per routed invocation and retained
in the request cloned by existing unsigned and signed attempts. It is not a caller
argument and grants no retry authority. See [header contracts](configuration.md#openapi-header-parameters-and-idempotency).
Catalog construction returns tools plus fixed method/path/reason issues to ordinary
serving and registration. Selected unsupported operations do not hide usable
siblings or suppress same-path GETs. Endpoint omission notices follow configured
instruction overrides; detailed registration/operator inspection retains identities.
Public Vec builders and strict tool/qualification inspection refuse partial results.
Standalone tag inventory is metadata-only; deployment tag inspection retains strict loading.
Operation/path/method/tag exclusions precede compatibility; later tool-name filters
see compatible names only and do not erase source-level omissions.
A serving source with no supported selected API operations retains its issue metadata
and is unavailable to dependent `wait_for_catalogs = true` endpoints; other endpoints
follow their own readiness and serving settings. A help document does not convert
that source into a successful API catalog. Intentionally empty selections remain
configuration errors, and explicit help-only selection remains valid when supported
API operations existed before name filtering. Dynamic registration may reject an
all-unsupported candidate without changing existing registrations. Strict inspection
and qualification still reject any omission.

The existing JSON encoder carries the selected declared application/json or +json
Content-Type and parameters through routing; non-UTF8 or non-JSON-only bodies are
unsupported rather than advertised as JSON. Schema conversion is not a general
OpenAPI evaluator: cookies and unrepresentable parameter serialization are explicit
operation issues. Assertion-bearing OpenAPI3.1/unspecified-version input-schema
reference siblings receive the fixed schema_reference_siblings reason. The seven
JSON Schema2020-12 basic annotations with valid shapes are retained as a separate
allOf branch alongside the target. Their payloads stay opaque and defaults do not
insert request fields; unknown or malformed siblings remain unsupported. An exact unrestricted recursive
six-type JSON union is reduced to its description-only schema after a bounded
proof, while constrained near matches and other cycles remain unsupported. Inspection follows only actual schema positions
and used reference targets before lossy resolution, not unused definitions or
annotations. General Reference Object and stricter importer policies remain separate.
See [input contracts](configuration.md#openapi-inputs-and-request-bodies).

Tool input schemas describe supported caller contracts; argument routing is not
complete JSON Schema validation. Narrowing an enum advertises a supported subset,
not a runtime authorization boundary: nonconforming arguments can still be sent
upstream. Descriptions must distinguish unsupported choices from enforced rejection.
Per-payment authority, destination policy and accepted-work ownership retain their
independent runtime checks.

Schema conversion is not a general-purpose OpenAPI implementation;
for example, file uploads and asynchronous job orchestration are unsupported.

`src/deployment.rs` validates listeners, filters and wallet references and resolves
source overrides, listener defaults and optional automatic assignment. Source
distribution uses a pure catalog-free graph of effective bindings: legacy
listener lists contribute edges, and every non-exclude `send_tools` target contributes
an edge with its authored rule origin. Authored configuration remains available
for inspection and frozen snapshots. The same graph feeds wallet assignment,
bootstrap, relay ownership, catalog dependencies, warming and static name
reservations. Empty current inventories and shadowed rules retain declared edges;
exclusion-only sources have no binding or wallet demand. Wallet identities use
the existing source/listener/profile inputs, never rule positions or catalog data.
Best-effort wallet observation still collects valid known groups when unrelated
bindings are invalid; it does not require strict serving validation.

After ordinary source selection, one ordered `send_tools` decision assigns or excludes
each tool by its generated suffix. Existing listener selection narrows assignments
and feeds publication, listing/search and invocation together. On-demand
`config check --explain-routing` reuses that decision and reports exclusions and
published wallet bindings without retaining explanation state in the runtime.
Frozen qualification snapshots preserve authored rules and effective topology;
reload must not reinterpret derived bindings as legacy attachments. Snapshots
without `send_tools` retain legacy behavior. See
[source tool distribution](configuration.md#source-tool-distribution) for syntax
and diagnostic contracts.

Local catalogs load before ordinary deployment serving; deferred remote catalogs
start after listeners bind. One shared capacity owner spans initial pricing for
local catalogs and deferred catalog, pricing and relay attempts, admitting 2 active
Tor isolation scopes by default (`startup.catalog_concurrency`, 1..64). Requests using the same effective SOCKS
credentials share one slot and proceed in parallel, including when distinct scopes
are queued. Different transport or retry scopes count separately even on one
domain. The last full-body holder releases its slot before retry/handoff delays;
logical loads remain pending. Direct egress, including `catalog warm --direct`,
cache hits, local files and relay serialization/readiness waits hold no slots.
Idle Tor circuits and pooled sockets do not count. Dynamic imports outside this
deployment owner retain their existing scheduling. Each listener defaults to `wait_for_catalogs = true`:
after socket binding, funding/reconciliation and deferred discovery run while HTTP
serving waits for that listener's effectively bound sources. The catalog owner signals
readiness only after publication; no duplicate source-outcome ledger or overall
startup deadline is needed. A required failure logs ERROR and closes only the
dependent waiting endpoints; their tasks await shutdown without stopping peers or
financial owners. Optional remote failures permit startup with unavailable status.
Worker/publication failure fails unresolved readiness signals. Listeners with
`wait_for_catalogs = false` serve available tools while remote catalogs are pending
and omit failed sources regardless of optional status. Strict inspection/qualification
wait for complete catalogs. The
synchronous unsigned-serving loader cancels unfinished loads on required failure
and can omit an explicitly `optional_startup` remote failure. Source
completion order cannot change the final sorted inventory for valid configurations.
Ordinary serving retains successfully generated source catalogs when one endpoint's
selection fails. Initial and deferred selection use the same endpoint-local path:
the failed endpoint publishes no static tools and explains the selection failure;
other destinations retain their valid tools and payers. A waiting endpoint fails
readiness after that snapshot is published. An eager endpoint continues serving
its capabilities and current failure instructions; later catalog publication
recomputes selection. Strict inspection still rejects invalid selection. Reserved
static tool identities reject the newly arriving source while retaining healthy
aliases and existing bindings. Strict inspection rejects these identities without
opening wallets; serving retains the existing discovery-relay preparation and
cleanup order.
Conflicting cover settings for a shared payer/origin remain a shared resource failure and
retain source admission rejection across listeners.
Ordinary deferred
serving retains completed catalogs and omits all owners of a tool name that collides
on a listener after tag selection, before tool-name filters. MCP instructions list
ambiguous names; unique tools and other listeners remain usable. A late collision
withdraws the earlier binding from new calls while captured calls finish with their
original provider and payer. Strict loading rejects duplicates. All declared sources
are loaded, including those unused by listeners. Remote aliases share fetching
and immutable parsed JSON within a load only for the same exact URL, requested
timeout, byte limit, transport policy and discovery identity. Filters, overrides, base URLs and
wallet bindings remain per alias. Local files and later loads do not reuse this
cache. Progress/timings go to stderr. Lazy help fetches on
first invocation; successful content and failures with possible relay charges are
cached. Proven unpaid failures can retry, and simultaneous requests for the same
resource coalesce. These caches are
independent of fresh payment challenges.

`src/http_cache.rs` adds disposable HTTP-aware persistence under an existing
treasury state directory. Complete remote catalogs use the configured local lifetime
and conditional GET validators after expiry. It partitions network/transport identities, bounds storage and
bypasses qualification. See [discovery disk caching](configuration.md#automatic-discovery-disk-cache).

`src/network/acquisition.rs` owns the single unsigned origin sequence, including
fresh Tor scope selection, typed delivery failure and server delays. Static
catalog and Coinbase consumers retain their own content acceptance,
cache/revalidation and publication rules. Runtime imports, help and document probes
use `discovery_relay/document.rs` for complete unsigned acquisition and typed
failures. Consumer validation promotes only accepted origin content and preserves
possible fees through cache/registry publication, qualification and cleanup. `src/discovery_relay/` shares one ordered
method-specific relay traversal with small provider-owned encoders/decoders.
Bootstrap uses complete pinned local schemas rather than another network fetch.
With bootstrap policy, metadata and built-in discovery relay fees use the fixed
bootstrap group. Otherwise static discovery resolves one wallet per source or
an explicit override, and Coinbase uses the authenticated endpoint payer. The relay's ordinary paid client participates
in unsigned recovery while recursive fallback is disabled.

Static relay exchanges serialize per provider across wallets, retaining that
provider's gate through payment, decoding and publication after waiter cancellation.
The shared state mutex covers only cache/gate-map/stopped-state access and
publication; it is released during I/O and waits. Cached results and independent
providers remain available during another exchange. Relay request capacity covers
each HTTP attempt and its body, releasing before retry delays or wallet traversal;
accepted ownership spans those waits. The initial origin-to-relay delay precedes
accepted relay ownership and provider gates. Between-provider delays retain the
logical call owner but release provider gates. This partitions existing
serialization without adding a resource cap. Cache/failure identity includes
adapter/provider, authored position, payer and request scope. Readiness waits
consume no network attempt. There is no separate static request counter/backoff.
Definitive eligible provider/target outcomes may advance through the list once;
uncertain paid failure or an unattributable envelope stops. Static uncertainty
disables that provider for the run without disabling independent providers.
Delivered target errors/truncation may already be billed, and every authorization
retains its liability. No relayed challenge reaches target signing. Relay content never promotes an origin transport.

Configured relays may follow redirects remotely while origin HTTP remains
no-follow. Requested/reported final destinations and every Curl-reported hop pass
applicable public-destination checks. AGProxy/Horizon intermediate hops remain
unknown; missing evidence does not exclude an adapter.
The operator accepts the relay's remote redirect/public-destination enforcement
and plaintext integrity. Relay TLS does not establish target TLS or content
authenticity. Target credentials and payment signatures remain excluded.

Document relay calls retain their acquisition owner and use `provider_metadata`
permission. Their fixed bootstrap payer is independent of API cutover; deployments
without bootstrap policy retain source-bound payment authority. Trusted caller purpose selects `source_discovery`
for built-in directory/Coinbase calls and `api_requests` for actual API calls. The
shared immutable relay context retains all preferences; callers do not replace it
with category-specific clients or caches. Existing help/import
coalescing includes pinned financial authority and effective document policy.
Static help pins the actual payer Arc separately from ordinary API payer replacement;
managed help retains its existing pool Arc across rotation. A possible-paid help
initialization failure is shared with joined waiters, while a proven unpaid failure
retains normal explicit retry behavior. Source-add ownership spans fetching through
validated registry/catalog publication. Revision/removal fences still govern publication.

CLI refresh tries the unsigned origin first. Only a representable eligible relay
request loads the existing bootstrap metadata group under bootstrap policy.
Otherwise it loads the saved registration owner's selected static key or
authenticated existing managed store/pool and Base transport. That narrow owner starts no SDK,
wallet allocation or funding worker; it drains and closes after success or failure.
Missing document payment authority leaves origin retrieval available. These paths
create no target-signing or uncertain-outcome replay authority.

Serving binds listeners before starting initial or
replacement funding, including when a relay payer has no capacity. The dormant
owners start in the serving lifecycle. `deployment/bootstrap.rs` remains the
catalog-free explicit `wallet bootstrap` path. Its initial check and pending
monitor both check completeness of the captured claims.
Reserve vacancies follow their exact current claims; there is no separate initial allocation. Warming starts no funding workers. Origin freshness and relay provenance
remain distinct from ordinary HTTP cache entries. See
[paid discovery relay](configuration.md#relay-fallback).

Deployment initialization checks managed connection settings before opening the
treasury and installs qualification, funding-denial and pause restrictions before
configuring pools. Initialization represents store-only ownership separately from
a treasury paired with its funding runtime, so consumers cannot combine unrelated
optional owners and capabilities. Available treasury serving runs finite startup
ZEC discovery behind bound listeners, then observes only current funding demand or
validated pending source/shielding work. Store-only fallback retains existing Base
capability and its finite startup pass, but does not reconstruct SDK/start ZEC or
source/swap observers until restart restores that capability. Existing owners and
journals implement the [wallet observation policy](wallet-rotation.md#wallet-observation-policy);
publication/reconstruction retries remain separate local work. Shutdown drops dependent Store handles before
closing the matching owner. Reused initialization receives binding-time funding
restrictions before use. Setup and listener failures release relay-held store
handles before waiting for treasury shutdown. Static signers are loaded only for effective selected bindings,
including enabled provider-management/raw-call capabilities and relay wallets. If discovery fails after wallets
are initialized, loading shuts down the retained owner before returning an error.

`src/catalog_state.rs` publishes immutable snapshots. A tool invocation captures
its route, output policy and payer binding so a concurrent catalog replacement
cannot redirect an in-flight call. Search/call uses the readable endpoint-local
tool name, resolving its current visible binding at invocation; restarts do not
invalidate the name. Dynamic source UUID/revision still owns execution isolation,
not an agent-supplied handle. Registration deduplicates canonical specification
URLs under the publication lock, without a separate result-receipt ledger. Accepted
curated sources carry provider identity and effective catalog settings with their
exact document bytes. One catalog path applies canonical provider overrides and
filters before rebinding known tool symbols to `dyn_<registration-name>__<operation>`
in names, descriptions and instructions. Wire paths, schemas and URLs remain
unchanged; reviewed response mappings reach the bound invocation. Legacy generic
records retain their existing default configuration. Restart and saved inventory
inspection use accepted settings without re-resolving the shipped bundle. Explicit
curated refresh validates current bundled settings and newly acquired schema bytes
before replacing the accepted revision atomically. Listener filters constrain
invocation as well as listing. Independent local/remote provider management and
raw-call preferences authorize new operations before identity selection or I/O;
accepted registrations retain existing filters and payer bindings. Restoration
retains the requirement for either management capability; disabling both disables
saved dynamic registrations, including raw-only endpoints. Either
management capability permits operator refresh, while inspection/removal remains
available regardless of preferences. Dynamic document byte limits and current
operator destination/wallet authority govern binding and execution. Bundled dynamic
sources preserve accepted provider read inactivity, then inherit the network default;
generic imports and raw calls use the network default. Reviewed
provider HTTP/TLS permissions reach schema acquisition, refresh, API and help via
the network factory. Generic imports and raw calls remain strict. Accepted catalog
settings do not activate static caches, warming, relays, cover configuration
or recommendation selection. Agent-added sources use the same execution path; see
[agent source management](agent-sources.md) for endpoint-local registration, persistence and refresh.
Remote provider search concurrently owns one workflow per operator-selected
directory and preserves each complete native JSON object under its directory key.
Local failures preserve sibling results and aggregate typed possible-payment
evidence. Accepted paid children outlive waiter cancellation; sibling traffic does
not renew response inactivity. There is no outer replay or whole-search deadline.
Ordinary serving builds the local snapshot before running remote catalog
discovery and retained relay continuations; each endpoint's `wait_for_catalogs`
setting controls when HTTP service exposes it. Late sources publish complete listener
views through the same mutation boundary as dynamic registrations. Instructions
report pending or unavailable sources. Each `tools/list` response also exposes
listener-specific current instructions, catalog generation and publication-stop
status in `_meta["x402_treazury/catalog"]`, alongside tools from that snapshot.
Dynamic registry records retain one process-local immutable unbound catalog result
for their accepted document and candidate settings. Registration validation and
restore produce that result once; unrelated additions and static publications
reuse it. An add chooses its final name before its one catalog construction in the
existing blocking mutation worker. Names, authored guidance and name selectors
therefore share the accepted identity, without a provisional build on the serving
runtime. Validation precedes persistence and transport promotion follows successful
publication. Listener filters, current
payer bindings and compatible help authority are projected anew at publication.
Derived results never enter the durable registry; explicit refresh and restart
revalidate the accepted/current inputs under their existing contracts. Publication
uses local name/help indexes without changing inventory order or collision policy.
Published registration names, exact selectors and complete result metadata share
the same immutable snapshot as tools, omission notices and instructions. Known
registration lookups and source-filtered search capture that one generation without
waiting on registry construction or storage. Durable mutation stays serialized and
persistence precedes the single snapshot publication. Same-registration adds retain one
authenticated-owner/registration-identity mutex from the final duplicate check
through acquisition, validation and publication; completed-fetch eviction and payer
replacement cannot reopen that work. Mutex ownership lasts only for live owners
and waiters; later missing adds remove dead weak entries. Unrelated identities
remain independent. Static bases never retain copied dynamic
metadata; only current source records contribute it to the composed snapshot.
The commit worker retains its registry-identity check for lower-level public
`CatalogState::publish` callers: if they replace the composed registration view
while accepted records remain, adding that identity fails rather than duplicating
stored records. This compatibility fallback is separate from ordinary add
coalescing. `Manager::replace_static` preserves registry composition.
Dynamic source instructions are composed afresh from the immutable static base on
each publication, including only enabled sources with visible tools. Refresh,
removal, restoration and static replacement cannot accumulate or retain stale
provider guidance. Endpoint `instructions_text` replaces authored source guidance
before generated availability/conflict notices are appended, including empty
suppression and deferred rebuilds.
Search-tool instruction fields decorate only exposed wrappers, consistently with
inspection inventory; they do not change tool access or payment authority.
Tool search filters the captured snapshot by listener and source before computing
request-local BM25 over names, descriptions and top-level argument names. Literal
name/description substrings rank first; blank queries retain catalog order. Query
help and result counts expose these semantics without another index, cache or
provider request. Bundled-provider search uses the same shared ranker over its
offline ID/name/description/API-base metadata, with provider-ID tie breaking and
existing page pagination after ranking. Exact provider/base/spec lookups retain
their separate exact-match contracts.
See [local search matching](agent-sources.md#tool-search-matching-and-results).
Tool descriptions omit registered-source state because clients may cache them.
Live search/enumeration reflects the captured listener snapshot; source-name
filtering retains the validated `dyn_<name>__<slug>` contract and final delimiter.
See [endpoint instruction placement](agent-sources.md#endpoint-instruction-placement).
Clients must inspect this metadata; it does not imply a UI notification. Unsigned
dependency waits add no timer or
spending authority. Shutdown drains accepted relay work before financial owners.
Local authored catalogs remain validated before serving; strict inspection and
qualification still require complete discovery. Exact accepted calculations/inputs and prepared/submission journals outlive waiters.
The SDK owns current Zcash funds and chain corrections; local refund credits and
historical source monetary equality do not gate publication. Deposit and shielding
calculations run in a retained blocking task that owns the original offline SDK
client and a Store handle. Its started body accepts inputs, calculates once and
publishes the exact result; caller cancellation or normal runtime teardown does
not discard that work. The treasury joins it before another SDK mutation,
reconstruction or shutdown; [SDK integration](zingo-integration.md) owns the details.
Completed SDK bytes
and their snapshot stay with the existing owner until identical local publication
succeeds; cancelled close waiters do not abandon this drain. The exact serialized
Store worker is supervised in treasury and store-only lifecycles. Its termination
fails dependent operations and stops treasury admission without restarting the
Store or replaying closures. Independent wallets and listeners remain usable.
Store handles retain the original exclusive file lock through accepted work even
after worker death, preventing a second owner from opening the same state.
If that worker dies with a completed unpublished transaction, its blocking owner
retains the original client, exact bytes, snapshot and accepted input ownership
without further Store retries or SDK reconstruction. The caller receives failure;
graceful close remains pending. Explicit forced process termination can lose those
unpublished bytes, leaving the accepted input claim uncertain.
Unhealthy SDK capabilities are reconstructed from the latest committed snapshot
through the same Store owner. Interrupted commands never replay; queued commands
and accepted work remain owned across caller cancellation. Exited funding and
reconciliation tasks reuse existing gates and retry cadences; shutdown stops replacement.
Unavailable shared Store/authentication state may still require restart while
listeners and usable Base payers remain available. Enabled ordinary auto-funding uses actual supported Ironwood funds and existing
quote authority. New wallet transparent-input shielding is unsupported. Generic
pending-source observation and accepted byte/input ownership retain their original
recovery rules; canonical closure never grants replacement authority. Source
selection and actual calculated outputs obey the
[Ironwood-only policy](zingo-integration.md#ironwood-only-policy). These paths have
local SDK fixture coverage without live-funded, consensus or Tor qualification;
see [testing](testing.md#qualification-status).

Single-wallet Base canonical proofs use the wallet identity for headers, exact
contract reads and canonical rechecks across each configured endpoint. Genuine
multi-wallet qualification/library views retain shared RPC-origin headers for one
coherent anchor, while wallet-specific reads retain their actual owners. NEAR
metadata follows the already-known job recipient, as do quotes and status.

## MCP and output boundaries

`src/server.rs` serves tools through `rmcp`. Streamable HTTP listeners are
bearer-authenticated and stateless; stdio reserves stdout for protocol messages.
Accepted calls live in `server/work.rs` independently of response-delivery futures.
Disconnect and stdio EOF cannot abandon them. Graceful shutdown closes admission,
drains accepted calls with periodic progress, then stops financial workers; a second
signal returns typed `ForcedShutdown` from serving, wallet or bootstrap drains.
Each owner registers one `ShutdownSignals` before polling its work and retains
the same streams through cancellation and drain. Linux/macOS use SIGINT and
SIGTERM; Windows uses Ctrl-C. Signals delivered during the handoff remain
observable, without a new subscription or a background watcher. After EOF or
supervisor shutdown, the first subsequent operator signal forces the drain.
Optional cover-task cleanup runs inside that same force-interruptible drain.
Standalone HTTP uses `serve_http_with_signals`. The existing library `serve_http`
API retains its external-trigger contract and registers operator force handling
after that trigger; embedders needing continuous operator-signal handling use
the persistent-signal entry point.
The executable exits immediately without joining blocking workers or releasing
liabilities; library embedders own their process-level force behavior. Stdio input uses a
bounded dedicated reader so an open parent pipe cannot hold Tokio runtime shutdown
after accepted work drains. Logs go to stderr. Current HTTP operation does not promise a server-to-client
list-changed notification channel. Dynamic deployments expose listener-scoped
search/call fallback tools for clients that cache their initial inventory.

`src/payment.rs` owns each identity-bound unsigned challenge and signed attempt. Static
profiles support Base USDC v1/v2 exact and v2 upto signing. Each call pins its
static signer before unsigned I/O, so replacement affects subsequent calls. Managed profiles use
v2 exact EIP-3009 permission checks and durable authorization journals. A paid timeout/body failure
can still have spent money. Only the narrow managed insufficient-funds traversal
may repeat it; see the [wallet rules](wallet-rotation.md#best-effort-wallet-rules).

Direct Rust `PaidClient` calls retain accepted work through response conversion
when their waiter is canceled, and reuse an enclosing MCP owner when present.
Output validation and conditional unsigned transport promotion finish inside
that owner before result delivery, including after the waiter disconnects.
`PaidClient::drain` closes clone-shared admission and waits for accepted work.
`Work` tracks accepted direct tasks and nested managed signing through output
conversion, disconnect and shutdown drain. There is no aggregate foreground count
bound on retained tasks or response memory and no replacement admission gate;
concrete operation-specific I/O/storage guards remain. Qualification case identity and unsigned
restrictions follow the retained task without renewing authority.

The Rust `ManagedPool::pay_with_cover` API now takes an owned optional cover call
and returns `ManagedResponse`, carrying the response, extension warning flag and
cover lease. Raw-body callers retain that lease through output consumption and
mark it complete after success. `pay` preserves its tuple return for callers
without cover; `drain_direct_payments` waits for accepted signing and dispatch.
`PaidClient` retains cover until successful HTTP output collection; attachment
rendering happens later and cannot retry payment.

`src/output.rs` preserves response bytes and MIME, handles ordinary text/JSON,
small supported inline images, configured base64/data-URI mappings and explicit
JSON field omissions. Encoded
media inside ordinary JSON remains text/metadata unless an explicit response
mapping extracts it; no automatic base64 decoding or asset fetch occurs.
Unsupported raw MIME types and multipart request uploads remain separate
transport boundaries. A bounded
or unsupported response produces explicit evidence, never a silently truncated
success. Typed results flow through `PaidClient::execute_response`,
`BoundTool::invoke_output` and `Server::invoke_output`, including dynamic fallback.
A managed target call retains its latest fully validated offer through response
conversion and failures, independently of shared price hints and other calls.
For overlapping calls, a later-finishing call can report `0.009` while a fresh
description shows `0.003` from an offer received later. Completion order does not
replace the newer shared hint or change either call's own observation.
MCP appends one complete JSON text block after body text and images, before
advisories: `{"x402_pricing":{"amount":"0.005","currency":"USDC","basis":"observed_offer"}}`.
The amount uses exact decimal USDC units. It describes the observed offer, not
settlement or a total across authorizations. Body projection and `structuredContent`
remain unchanged. Text-only Rust convenience methods return body text without this
block; clients concatenating all text blocks must handle the adjacent JSON object.
Calls without a validated offer and opaque static SDK signing omit the block.
A final unsigned success, including delivery through a configured API relay after
a payer handoff, still carries the latest target offer validated earlier in the
same logical invocation. It does not describe a new charge for that final response.
Relay fees, cached help acquisition and grouped directory
children and combined source-details acquisitions do not become a scalar price
for the outer result, including failures.

Text-only Rust convenience methods reject attachments. Image mapping and validation
contracts are in the [provider guide](../providers/README.md).
For unsuccessful API responses, the internal error chain retains the upstream body
and the unique payment receipt's string `errorReason`/`errorMessage`, without
copying the full receipt. MCP API errors present a concise failure kind and actual
known provider HTTP status, with one prohibition on automatic retry for potentially
chargeable or uncertain write work. Local argument corrections remain available.
Known outer status survives body-read and output-render failures. Explicit relay
contracts distinguish provider HTTP status from reported destination HTTP status;
arbitrary provider bodies remain unchanged and supply no inferred attribution.
Output/download rejections still identify their setting and rejected content.
Internal causes, qualification categories, liabilities and retry enforcement remain
unchanged; diagnostic chains and arbitrary upstream prose are not agent content.
Logs retain only fixed safe categories. Provider claims
remain unverified and do not authorize replay; see the
[diagnostic boundary](development.md#diagnostic-context).
Accepted tool text, JSON and image metadata remain complete through direct and
wrapper invocation except for configured JSON field omissions. The same response
is parsed once, image extraction runs first, and omission paths remove only
object fields rather than array elements. Result advisories disclose actual
removals and retain transport notices. This local transformation never initiates
another HTTP request or payment; unmapped raw calls retain the full response.
See [JSON response projection](configuration.md#json-response-projection).
There is no completed-output character slicing; provider
`max_description_chars` remains a separate explicit description control. Download
and image bounds still reject oversized resources with explicit errors. Durable
[artifact storage](plans/artifact_storage.md) and
[polling workflows](plans/deferred/polling_tool_support.md) remain future work.

## Financial and network ownership

Named wallet identity determines payer sharing. Two sources can share one wallet,
and the same provider can be bound to different wallets through source aliases.
A single embedded SDK-owned Zcash treasury can fund multiple independently scoped managed
pools. The [wallet architecture](wallet-rotation.md) explains admission, atomic
promotion, background funding, encrypted persistence and recovery boundaries.

Runtime treasury-ID discovery reads the supported schema and instance identity;
static-wallet startup additionally reads managed profile names in the same
transaction. Neither depends on decoding financial reports. Malformed financial
history remains reportable and cannot remove a managed profile's name binding.
These read-only checks do not unlock state or establish payment capacity; managed
opening still authenticates the encrypted snapshot under exclusive ownership.

`src/network.rs` is the shared factory for both direct and optional Tor traffic.
Connection pools are keyed by identity and destination; enabling Tor does not
create a second execution stack. Catalog/help use discovery-origin
identities; payment and recipient-specific funding work use immutable EVM
identities; wallet scans use the treasury UUID. The
[egress inventory](network-egress.md) records callers, SDK integration, credential
encoding, DNS behavior and qualification limits.

Shutdown stops accepting calls and drains accepted work with progress reports for
payment safety. Treasury ownership and durable journals survive cancellation;
an interrupted waiter does not undo accepted preparation or submission. Restart
reconciles existing work rather than inferring non-payment from a missing response.

## Wallet selection and network isolation

An optional deployment `wallet_bootstrap` policy routes explicitly participating
managed groups through one ordinary bootstrap group until the destination has
recorded its first positive initial credit. The trusted API boundary validates
binding, permission and arguments, persists UUID-bound first-use demand, and
captures one effective group before unsigned dispatch. Even free or response-cache
hits activate demand; discovery, catalogs, pricing, registration and help metadata
do not. Activation only wakes the retained funding owner: it allocates no wallet,
performs no swap I/O and never waits for funding in the call path.

The effective group owns that call's payment cap, inventory boundary, signers,
journal, transport and cover state. Credit arriving during an accepted call changes
later captures only. Tiny positive credit counts, including on historical retired
wallets; later depletion, refunds and reorg repair cannot restore bootstrap routing.
There is no cross-group insufficient-funds traversal or fallback after handoff.
Current configuration selects participation; persisted demand and consumed claims
survive rename, restart and removal/re-addition without a deployment-phase ledger.
Qualification retains its reviewed named allocations and does not substitute
bootstrap wallets.

With bootstrap policy enabled, metadata and directory relay fees permanently use
the bootstrap group. This includes static catalogs/pricing and runtime help,
imports and probes. Its cap and inventory apply independently of API cutover,
including when a legacy static relay wallet override is configured. Off-by-default
API/raw fallback retains the API call's captured authority. Non-bootstrap
deployments and qualification preserve their named authority.
Deferred catalog waits subscribe to bootstrap capacity before checking readiness;
ordinary funding runs independently of catalogs. Price refusals remain terminal
cached outcomes under that fixed payer. A started acquisition retains its captured
payer, shared in-flight ownership and possible fees. Document compatibility keys
retain the actual metadata payer and existing policy/limits. Offline document
reads open only the existing bootstrap metadata group, including explicitly named
generated groups, without declaring or activating participants.
The advisory startup funding announcement evaluates effective groups and caps in
one Store projection and never waits for every dormant participant reserve.

Runtime price hints guide selection while wallet-scoped HTTP pooling, cover
ownership and Tor isolation bind each attempt to its payer. With a price hint,
initial selection prefers estimated-affordable RETAINED, then ACTIVE, then READY,
in creation order within each role. Without a hint, healthy positive ACTIVE
precedes healthy RETAINED, then READY; low, zero and unknown estimates remain
fallbacks. Once a challenge is obtained, a suitable eligible bound payer stays
bound even when a retained sibling could cover the offer. Actual challenge terms
and pool/payer authority govern admission. See the [best-effort wallet rules](wallet-rotation.md#best-effort-wallet-rules).

Before the first unsigned request, selection uses a fresh observed offer when
available, otherwise a usable advertised per-call estimate, otherwise unknown
pricing. Advertisements are normalized from provider metadata and stay separate
from observed-offer cache entries and result evidence. Local catalogs supply them
on the first build; remote advertisements follow the configured catalog lifetime
(seven days by default), retaining their acquisition expiry even when persistence
fails. An invocation snapshots its advertised amount without balance or funding
I/O. Advertised prices rank existing identities and do not add admission checks.

`src/payment.rs` binds each unsigned request, signed request and cover owner to the
same EVM address, pinning the acquired `HttpTransport` record and its effective
isolation scope (`cover::registry::Owner::from_transport`) from unsigned challenge
through admission, signing, signed dispatch and response body consumption.
`src/network.rs` uses that canonical identity and effective scope for connection
pooling and SOCKS isolation. Participating API clients (generated MCP tools,
static/managed payers, directory tools and `raw_call`) may try
`1 + network.tor_isolation_retries` Tor isolation scopes (default 3) on unsigned
HTTP `400`, `403`, `408`, `429`, `500`, `502`, `503`, and `504` (honoring
valid `Retry-After` headers on `429`/`503`) with the same pinned payer, and
conditionally promote a winning candidate after valid `402` challenge validation
or completed valid unsigned output. Relay-service unsigned requests use that same
sequence and the broader common transport classifier, without recursive fallback.
Unsigned API GET/HEAD and otherwise-authorized x402 POST use the common transport
classifier for dispatch and successful-response body acquisition. Other methods,
including manual-header writes, recover only from TLS connection-establishment
failures. A TLS failure after dispatch cannot authorize their replay, even before
response headers arrive; their bodies are read only by the final output consumer.
Configured runtime relay handoff uses the
original method's list after one origin sequence, before target signing, and is
suppressed after any earlier target signature. Relay results return directly to
output conversion, never challenge handling, and never promote origin scopes.
The same captured static payer or existing managed pool owns relay fees; no
recursive relay fallback, wallet allocation or funding wait is added. Shared
submission evidence wraps all possible-charge errors; typed response/output facts
keep relay fees distinct from target payment and preserve aggregate uncertainty
through directory parsing or fallible text conversion. Complete invalid output
is terminal. API ranges are legitimate and HEAD returns empty text without
representation-body decoding. If selection changes
after a challenge, obtain another unsigned challenge through the next payer's
transport. The captured inventory and attempted identities bound traversal without
a retry-count ceiling or new allocation.

Only a recognized structured insufficient-funds response to a managed signed
request earns cross-wallet repetition. Each attempt retains its own authorization,
possible charge and canonical outcome, even if a later one succeeds. A successful
signed response, signed transport uncertainty or generic signed error earns no
repetition. Schedule an ordinary advisory balance read after insufficiency without
delaying fallback.
The [wallet observation policy](wallet-rotation.md#wallet-observation-policy)
owns finite startup discovery and requested refresh scheduling; ordinary serving
does not sweep idle managed USDC wallets. Shared scalar acquisition ownership is
separate from payment authorization and canonical qualification.
Qualification retains its existing run permits and charges every attempt.

Remote catalog entries own passive operation estimates. Their existing cache
identity also shares prices across compatible aliases, wallet groups, listeners,
API destination overrides and authentication settings. It never shares a signer
or payer-bound transport. Catalog retention defaults to seven days and
`catalog_cache_ttl_seconds` controls both catalog and hints; there is no price-age
gate. Reads, restart and observations do not renew expiry. Successful unchanged
loads/revalidation preserve prices; HTTP 304 renews the joint entry. Changed bytes
and source configuration updates invalidate both. Per-source revision bookkeeping
runs before shared acquisition and records every alias after acquisition, including
the first cache creation. Revision records disappear with their catalog.

The SQLite cache keeps raw bytes intact, with generation-bound operation amounts
alongside them. Live bindings share in-memory learning immediately; detached,
best-effort writes never delay responses or recreate missing generations. Receipt
order controls in-process updates and conditional versions prevent older writes
from replacing newer ones. Expiry and eviction discard only metadata. No new
background refresh, price probes or financial recovery obligations follow. Local
catalogs, dynamic imports, opted-out sources and deployments without an existing
state directory keep per-call pricing without a shared hint cache. Help retains
its separate authority-bound cache. Hints learn from actual invocation challenges
and guide selection and fresh MCP description rendering. Every
invocation still makes its ordinary unsigned request; authored display prices do
not replace the live challenge. The selected design also creates new identities over time; its reuse objective depends
on the HTTP and cover retention fixes in the [network retention contract](network-egress.md).
Reclamation may require cover requalification and does not guarantee indefinite state retention.
Conversely, a fresh-identity design must account for repeated cover qualification rather
than claiming unchanged total unsigned traffic from API request counts alone.

Retain the small/large reserve design's mixed-cost address-reuse rationale from the
[configuration and sizing guidance](configuration.md).
A uniform large funding size is not an equivalent privacy choice; the selected heuristic
still permits cross-size use and does not guarantee price-class separation.

This is a deliberate design choice, not an unimplemented requirement to add another
selection mode. Revisit it only with an explicit change in priorities, a concrete provider
reuse contract, or evidence that warrants the transport/cover tradeoff. Such a revision
must assess request counts, pooling, owned cleanup, cover qualification and Tor isolation
evidence together; it cannot introduce signed replay.

### Unsigned x402 requests and POST retries

For this project's x402 pathways, protected execution is assumed to be gated on
payment acceptance. An unsigned POST is eligible for an otherwise authorized
unsigned retry on the same basis as GET; the method alone requires no additional
provider idempotency proof or side-effect gate. Business side effects are not an
additional retry-policy constraint for this use case. This is the accepted x402
execution model, not a claim about arbitrary HTTP POST endpoints.

Financial uncertainty remains distinct: a signed request may already have charged
or executed even when its response fails. Generic HTTP errors, timeouts and output
failures do not authorize another chargeable attempt. Preserve the separately
specified structured insufficient-funds traversal and journal-before-disclosure
rules for every authorization. Both payer handoffs and participating Tor unsigned
status retries apply this same GET/POST eligibility. Base read-only RPC uses the
same participating selection with endpoint-first scope recovery and method-specific
validation before promotion; [network policy](network-egress.md) describes its
separate direct behavior, strict evidence fallback and scalar fallback. Transport
scope never becomes financial endpoint provenance or settlement authority.

### Payment extensions and offer metadata

Managed payments remove advertised top-level challenge extensions before
validating and signing a supported ordinary offer. Terminal payment diagnostics
identify known omitted-extension categories and the presence of other extensions,
without logging arbitrary names or contents. Omission implements neither wallet
ownership authentication nor seller-receipt verification or vendor idempotency
keys; a provider may reject it. Static SDK extension handling is separate.
Omission grants no additional replay authority; the
[unsigned request and financial uncertainty rules](#unsigned-x402-requests-and-post-retries)
still apply.

Offer annotations are distinct from top-level extensions. Informational fields,
including vendor annotations and display prices, remain opaque and are preserved
in the accepted offer. `outputSchema` must be an object or boolean; its contents
are not evaluated and references are not fetched.

For v2, `amount` alone determines the atomic amount signed and checked against
the payment cap. A top-level legacy `maxAmountRequired` is preserved as opaque
metadata, without requiring equality or interpreting it as payment authority.
V1 continues to use its required `maxAmountRequired` field.

Admission validates the actual network, asset, payee, atomic amount, transfer
mechanism and USDC domain. Duplicated or conflicting authority fields in `extra`
and unsupported transfer/flow mechanisms are rejected. Display prices and nested annotations
cannot override these checks or the payment cap. SDK serialization must preserve
the selected offer, and the signed authorization uses its atomic amount.
OpenAPI `x-*` pricing/discovery annotations belong to catalog interpretation;
they are not themselves x402 challenge extensions. Local signing fixtures verify
these boundaries without qualifying upstream delivery or live settlement.
Provider-specific observations belong in [provider caveats](../providers/CAVEATS.md#payment-extension-and-offer-boundaries).

## Where to change and verify behavior

Keep provider-specific schema/selection fixes in provider TOML or curated specs
when the generic path can express them. Put general catalog, payment, output and
transport behavior in their shared modules, with regression tests for direct and
fallback MCP invocation where applicable. Do not introduce a second network
constructor or bypass financial checks to accommodate a vendor.
Opted-in [operation URL extensions](configuration.md#operation-url-extensions)
change the execution target while retaining the original OpenAPI path for catalog
identity and output mappings. Bound calls enforce public destinations for these
targets, including static sources, through the existing network factory.

The [testing guide](testing.md) distinguishes offline regressions, coverage,
consensus qualification and explicitly funded live work. Dependency/toolchain
ownership and compatibility patches are documented in
[reproducible builds](reproducible-builds.md) and [vendor/README.md](../vendor/README.md).
