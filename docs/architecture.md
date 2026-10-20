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

Standalone commands separate inspection/routing from server construction. Tags can
return before base-URL validation; ordinary tools/check/route inspection needs no
payment credentials. Serving validates spending policy and credentials before
pricing discovery. The stdio lifecycle retains one pinned service waiter and the
accepted-work owner through cancellation, drain and cover shutdown.

`src/config.rs` composes inline source settings with one level of provider TOML
inheritance. Local values replace inherited values, including lists/maps; omitted
values inherit. Paths belong to the file declaring them. Unknown fields and
unsupported compositions fail explicitly. `config show` resolves composition,
permissions and wallet bindings without loading specs or reading keys.

`src/catalog.rs` loads OpenAPI JSON and builds tool schemas, names, descriptions
and path/query/body routing. Provider files in `providers/` carry selections,
help URLs, pricing settings and explicitly reviewed response mappings. Curated
local specs are inputs for providers whose public schema needs adaptation; the
fixture utility is `examples/snapshot_spec.rs`. Tests pin generated contracts
against `tests/fixtures/`. Operation validation and stable sorting precede path,
method and tag selection; exact same-path GET/POST pairs then retain POST by default (`prefer_post` can
disable this). Only surviving operations contribute to name collisions.
Tool-name filters apply after collision resolution. Recommendation metadata is
advisory until provider/source `use_recommended_tools` opts in. Include, exclude
and recommendation selectors validate against the same compatible named inventory
including help, then a single intersection selects tools. Listeners narrow that
result; pricing rebuilds use the same selection. Empty active advice fails without
expansion, while unused stale advice only warns. Operator inventories add
recommendation markers separately from `ToolSpec` and MCP publication. Explicit
`catalog tools --all-tools` bypasses only advice after typed configuration resolution,
retains ordinary filters, and cannot run pricing discovery. Digest metadata copying and
OpenAPI operation conversion remain separate; conversion retains operation-level
parameters and the existing reference-resolution rules. Input construction collects
parameters before merging body properties; renamed body arguments retain their
original wire keys, and presence-only alternatives follow those renamed arguments.
Final sanitization applies to the assembled schema. Path filters respect segment
boundaries; OpenAPI boolean exclusive bounds normalize to numeric bounds.
Header parameters do not grant
agents control over payment headers. Schema conversion is not a general-purpose OpenAPI implementation;
for example, file uploads and asynchronous job orchestration are unsupported.

`src/deployment.rs` validates listeners, filters and wallet references and resolves
source overrides, listener defaults and optional automatic assignment. Local
catalogs load before ordinary deployment serving. Remote catalogs share capacity
for 2 active downloads by default (`startup.catalog_concurrency`, 1..64), after
listeners bind. Server-directed retry waits release download capacity while their
logical loads remain pending. Available tools remain callable while remote catalogs and
pricing are pending; failed remote sources remain unavailable with status in MCP
instructions. Strict inspection/qualification wait for complete catalogs. The
synchronous unsigned-serving loader cancels unfinished loads on required failure
and can omit an explicitly `optional_startup` remote failure. Source
completion order cannot change the final sorted inventory for valid configurations.
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
cache. Progress/timings go to stderr. Startup pricing discovery runs after tool
selection, uses unsigned eligible GETs and caches both success and failure for
the process lifetime. Empty price results reuse the original tools instead of
regenerating identical definitions. TTL expiry never initiates a refresh. Lazy help fetches on
first invocation; successful content is cached, failed initialization can retry,
and simultaneous requests for the same resource coalesce. These caches are
independent of fresh payment challenges.

`src/http_cache.rs` adds disposable HTTP-aware persistence under an existing
treasury state directory. Complete remote catalogs use the configured local lifetime
and conditional GET validators after expiry; pricing stores only explicitly fresh derived estimates,
never challenges. It partitions network/transport identities, bounds storage and
bypasses qualification. See [discovery disk caching](configuration.md#automatic-discovery-disk-cache).

`src/discovery_relay.rs` implements an explicitly configured paid Curl fallback.
It resolves one assigned wallet per source (or an explicit override), uses the
ordinary paid client, serializes requests across wallets and disables
further relay spending across all wallets after a possibly submitted payment fails
or returns an invalid envelope. Waiter cancellation retains serialized ownership
through terminal outcome publication. Proven pre-submission failures retain bounded retry/backoff and
do not disable unrelated targets. Local bootstrap metadata avoids
a catalog dependency cycle. Serving binds listeners before starting initial or
replacement funding, including when a relay payer has no capacity. The dormant
owners start in the serving lifecycle. `deployment/bootstrap.rs` remains the
catalog-free explicit `wallet bootstrap` path with captured-claim completeness.
Reserve vacancies follow their exact current claims; there is no separate initial allocation. Warming starts no funding workers. Origin freshness and relay provenance
remain distinct from ordinary HTTP cache entries. See
[paid discovery relay](configuration.md#paid-discovery-relay).

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
including source management and relay wallets. If discovery fails after wallets
are initialized, loading shuts down the retained owner before returning an error.

`src/catalog_state.rs` publishes immutable snapshots. A tool invocation captures
its route, output policy and payer binding so a concurrent catalog replacement
cannot redirect an in-flight call. Search/call uses the readable endpoint-local
tool name, resolving its current visible binding at invocation; restarts do not
invalidate the name. Dynamic source UUID/revision still owns execution isolation,
not an agent-supplied handle. Registration deduplicates canonical specification
URLs under the publication lock, without a separate result-receipt ledger. Listener filters constrain invocation as well
as listing. Agent-added sources use the same execution path; see
[agent source management](agent-sources.md) for endpoint-local registration, persistence and refresh.
Ordinary serving publishes available local tools before running remote catalog
discovery, retained relay continuations and startup pricing. Late sources publish complete listener
views through the same mutation boundary as dynamic registrations. Instructions
report pending or unavailable sources. Each `tools/list` response also exposes
listener-specific current instructions, catalog generation and publication-stop
status in `_meta["x402_treazury/catalog"]`, alongside tools from that snapshot.
Endpoint `instructions_text` replaces authored source guidance before generated
availability/conflict notices are appended, including during deferred rebuilds.
Search-tool instruction fields decorate only exposed wrappers, consistently with
inspection inventory; they do not change tool access or payment authority.
Tool search filters the captured snapshot by listener and source before computing
request-local BM25 over names, descriptions and top-level argument names. Literal
name/description substrings rank first; blank queries retain catalog order. Query
help and result counts expose these semantics without another index, cache or
provider request. See [tool search matching](agent-sources.md#tool-search-matching-and-results).
Source-management tool-search descriptions derive their registered-source summary
from visible dynamic tool names in the captured listener snapshot, without another
registry read or cache. These names follow the validated `dyn_<name>__<slug>`
contract; splitting the final delimiter preserves trailing source underscores.
Client-cached descriptions are not live inventories. See
[endpoint instruction placement](agent-sources.md#endpoint-instruction-placement).
Clients must inspect this metadata; it does not imply a UI notification. Unsigned
dependency waits add no timer or
spending authority. Shutdown drains accepted relay work before financial owners.
Local authored catalogs remain validated before serving; strict inspection and
qualification still require complete discovery. Exact accepted calculations/inputs and prepared/submission journals outlive waiters.
The SDK owns current Zcash funds and chain corrections; local refund credits and
historical source monetary equality do not gate publication. Completed SDK bytes
and their snapshot stay with the existing owner until identical local publication
succeeds; cancelled close waiters do not abandon this drain. Irrecoverable storage
failure or forced process death can lose unpublished bytes, leaving the accepted
input claim uncertain. Unhealthy SDK capabilities are reconstructed from the latest committed snapshot
through the same Store owner. Interrupted commands never replay; queued commands
and accepted work remain owned across caller cancellation. Exited funding and
reconciliation tasks reuse existing gates and retry cadences; shutdown stops replacement.
Unavailable shared Store/authentication state may still require restart while
listeners and usable Base payers remain available. Enabled ordinary auto-funding
also attempts per-address shielding after successful persisted sync, including
unattributed owned transparent returns. Pending inputs and bytes take priority;
uncertain broadcasts never replay automatically. SDK-selected confirmed spendable
inputs, actual fee safeguards and decrypted owned shielded outputs govern new
calculation. A proposal must recover positive shielded value before accepting
inputs; a fee-only proposal cannot strand an input owner. Qualification job permits
do not authorize jobless shielding. These paths have local SDK fixture coverage,
without live-funded, consensus or Tor qualification; see [testing](testing.md#qualification-status).

## MCP and output boundaries

`src/server.rs` serves tools through `rmcp`. Streamable HTTP listeners are
bearer-authenticated and stateless; stdio reserves stdout for protocol messages.
Accepted calls live in `server/work.rs` independently of response-delivery futures.
Disconnect and stdio EOF cannot abandon them. Graceful shutdown closes admission,
drains accepted calls with periodic progress, then stops financial workers; a second
signal returns typed `ForcedShutdown` from serving, wallet or bootstrap drains.
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
The existing foreground bound limits independently retained direct tasks; nested
managed signing reuses that permit. Qualification case identity and unsigned
restrictions follow the retained task without renewing authority.

The Rust `ManagedPool::pay_with_cover` API now takes an owned optional cover call
and returns `ManagedResponse`, carrying the response, extension warning flag and
cover lease. Raw-body callers retain that lease through output consumption and
mark it complete after success. `pay` preserves its tuple return for callers
without cover; `drain_direct_payments` waits for accepted signing and dispatch.
`PaidClient` retains cover until successful HTTP output collection; attachment
rendering happens later and cannot retry payment.

`src/output.rs` preserves response bytes and MIME, handles ordinary text/JSON,
small supported inline images and configured base64/data-URI mappings. A bounded
or unsupported response produces explicit evidence, never a silently truncated
success. Typed results flow through `PaidClient::execute_response`,
`BoundTool::invoke_output` and `Server::invoke_output`, including dynamic fallback.
Text-only Rust convenience methods reject attachments. Image mapping and validation
contracts are in the [provider guide](../providers/README.md).
Display limits and download limits are different controls. Durable
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
create a second execution stack. Catalog/help/pricing use discovery-origin
identities; payment and recipient-specific funding work use immutable EVM
identities; wallet scans use the treasury UUID. The
[egress inventory](network-egress.md) records callers, SDK integration, credential
encoding, DNS behavior and qualification limits.

Shutdown stops accepting calls and drains accepted work with progress reports for
payment safety. Treasury ownership and durable journals survive cancellation;
an interrupted waiter does not undo accepted preparation or submission. Restart
reconciles existing work rather than inferring non-payment from a missing response.

## Wallet selection and network isolation

Runtime price hints guide selection while wallet-scoped HTTP pooling, cover
ownership and Tor isolation bind each attempt to its payer. Prefer a suitable bound
payer, then existing eligible wallets whose estimates suggest affordability; unknown
or low estimates remain fallbacks. Actual challenge terms and pool/payer authority
govern admission. See the [best-effort wallet rules](wallet-rotation.md#best-effort-wallet-rules).

`src/payment.rs` binds each unsigned request, signed request and cover owner to the
same EVM address, pinning the acquired `HttpTransport` record and its effective
isolation scope (`cover::registry::Owner::from_transport`) from unsigned challenge
through admission, signing, signed dispatch and response body consumption.
`src/network.rs` uses that canonical identity and effective scope for connection
pooling and SOCKS isolation. Participating API clients (generated MCP tools,
static/managed payers, directory tools and `raw_call`) may try up to 3
Tor isolation scopes on unsigned HTTP `400`, `403`, `429`, and `503` (honoring
valid `Retry-After` headers on `429`/`503`) with the same pinned payer, and
conditionally promote a winning candidate after valid `402` challenge validation
or completed valid unsigned output. Internal discovery relay calls remain on
`SelectionPolicy::Legacy` (single unsigned attempt). If selection changes
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

Keep three caches/flows distinct. Catalog/help discovery retains its existing caching
and coalescing. Startup pricing remains one-shot and cached, with HTTP-supported disk
freshness for derived display estimates; payment challenges are not persisted as reusable
authority. Runtime hints learn from actual invocation challenges and only guide selection.
The current invocation path still makes an ordinary unsigned request even when a display
estimate is cached. The selected design also creates new identities over time; its reuse objective depends
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

## Where to change and verify behavior

Keep provider-specific schema/selection fixes in provider TOML or curated specs
when the generic path can express them. Put general catalog, payment, output and
transport behavior in their shared modules, with regression tests for direct and
fallback MCP invocation where applicable. Do not introduce a second network
constructor or bypass financial checks to accommodate a vendor.

The [testing guide](testing.md) distinguishes offline regressions, coverage,
consensus qualification and explicitly funded live work. Dependency/toolchain
ownership and compatibility patches are documented in
[reproducible builds](reproducible-builds.md) and [vendor/README.md](../vendor/README.md).
