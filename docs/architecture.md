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
method and tag selection; only selected operations contribute to name collisions.
Tool-name filters apply after collision resolution. Digest metadata copying and
OpenAPI operation conversion remain separate; conversion retains operation-level
parameters and the existing reference-resolution rules. Input construction collects
parameters before merging body properties; renamed body arguments retain their
original wire keys, and presence-only alternatives follow those renamed arguments.
Final sanitization applies to the assembled schema. Header parameters do not grant
agents control over payment headers. Schema conversion is not a general-purpose OpenAPI implementation;
for example, file uploads and asynchronous job orchestration are unsupported.

`src/deployment.rs` validates listeners, filters and wallet references and resolves
source overrides, listener defaults and optional automatic assignment. Static
sources load before serving through a rolling queue, defaulting to 2 active loads
(`startup.catalog_concurrency`, 1..64). Required failures cancel unfinished loads;
serving can omit an explicitly `optional_startup` remote failure with unavailable
status in inventory and MCP instructions. Inspection/qualification remain strict. Source
completion order cannot change the final sorted inventory. All declared sources
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
treasury state directory. Complete remote catalogs use explicit freshness or
conditional GET validators; pricing stores only explicitly fresh derived estimates,
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
configuring pools. Reused initialization receives binding-time funding restrictions before use. Setup
and listener failures release relay-held store handles before waiting for treasury
shutdown. Static signers are loaded only for effective selected bindings,
including source management and relay wallets. If discovery fails after wallets
are initialized, loading shuts down the retained owner before returning an error.

`src/catalog_state.rs` publishes immutable snapshots. A tool invocation captures
its route, output policy and payer binding so a concurrent catalog replacement
cannot redirect an in-flight call. Listener filters constrain invocation as well
as listing. Agent-added sources use the same execution path; see
[agent source management](agent-sources.md) for endpoint-local registration, persistence and refresh.
Enabled-relay serving publishes available tools before running retained relay
catalog continuations and startup pricing. Late sources publish complete listener
views through the same mutation boundary as dynamic registrations. Instructions
report pending or unavailable sources; unsigned dependency waits add no timer or
spending authority. Shutdown drains accepted relay work before financial owners.
Enabled-relay serving also defers initial remote unsigned catalog loading; local
authored catalogs remain validated before serving. Broader independent startup
remains tracked in the
[recovery plan](plans/improved_recovery_robustness.md).

## MCP and output boundaries

`src/server.rs` serves tools through `rmcp`. Streamable HTTP listeners are
bearer-authenticated and stateless; stdio reserves stdout for protocol messages.
Accepted calls live in `server/work.rs` independently of response-delivery futures.
Disconnect and stdio EOF cannot abandon them. Graceful shutdown closes admission,
drains accepted calls with periodic progress, then stops financial workers; a second
signal explicitly forces exit without clearing liabilities. Stdio input uses a
bounded dedicated reader so an open parent pipe cannot hold Tokio runtime shutdown
after accepted work drains. Logs go to stderr. Current HTTP operation does not promise a server-to-client
list-changed notification channel. Dynamic deployments expose listener-scoped
search/call fallback tools for clients that cache their initial inventory.

`src/payment.rs` owns the unsigned challenge and single signed attempt. Static
profiles support Base USDC v1/v2 exact and v2 upto signing. Each call pins its
static signer before unsigned I/O, so replacement affects subsequent calls. Managed profiles use
v2 exact EIP-3009 admission and durable reservations. A paid timeout/body failure
can still have spent money; neither tool dispatch nor output conversion replays it.

Direct Rust `PaidClient` calls retain accepted work through response conversion
when their waiter is canceled, and reuse an enclosing MCP owner when present.
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
success. Display limits and download limits are different controls. Durable
[artifact storage](plans/artifact_storage.md) and
[polling workflows](plans/deferred/polling_tool_support.md) remain future work.

## Financial and network ownership

Named wallet identity determines payer sharing. Two sources can share one wallet,
and the same provider can be bound to different wallets through source aliases.
A single embedded Zcash treasury can fund multiple independently accounted managed
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

Decision: retain runtime price hints with wallet-scoped HTTP pooling, cover ownership
and Tor isolation. Prefer reuse of pooled connections, cover capability/episode state
and Tor circuits within the configured isolation boundary over choosing the exact
smallest fit on every cold or inaccurately predicted call. Reuse is an opportunity,
not a guarantee of circuit lifetime or unlinkability. Preserve the shared
[smallest-sufficient ordering](wallet-rotation.md#capacity-and-admission) before
binding and sufficient-bound-payer stability afterward. Actual challenge terms and
fresh financial evidence remain authoritative for admission; hints never authorize payment.

Late payer selection from the ordinary 402 response is technically feasible. Standard
v2 exact EIP-3009 requirements do not require selecting the signer before that response;
the pinned x402 2.0.2 client parses requirements and then selects/signs. The current
prebinding/re-challenge requirement belongs to this application's network ownership
contract, not to a universal x402 rule. `src/payment.rs` binds the unsigned request's
client and cover owner to the selected EVM address; `src/network.rs` uses that identity
for connection pooling and SOCKS isolation. Do not describe the alternatives as
protocol-impossible, or silently change those ownership boundaries in a future selector fix.

| Alternative | Benefit and request behavior | Decision and reason |
| --- | --- | --- |
| Selected: smallest-sufficient hints before binding, actual-price admission afterward | Normally one unsigned request and one signed submission; preserves wallet-scoped pooling, cover state and isolated Tor reuse. No dedicated price requests. | Keep. Missing/stale/inaccurate hints may produce a less suitable wallet or use the one affordability handoff. Accept that tradeoff without additional prediction machinery. |
| Choose a different signer after the challenge on the original wallet's pooled connection | Exact-price selection with the same two ordinary requests. | Reject. A signature from wallet B on wallet A's reused connection/cover identity can associate B with A's prior activity and violates the wallet-isolation contract. |
| Fresh invocation identity for both unsigned and signed requests; choose payer after the challenge | Exact-price selection with one unsigned request and one signed submission. Could remove runtime hints and affordability re-challenge logic while keeping one transport context for the negotiation. | Not selected. Loses cross-call wallet connection/circuit reuse and changes cover-owner lifetime, cache behavior and qualification evidence. The current bounded HTTP cache would churn; fresh cover owners would consume registry capacity and repeat capability qualification unless explicitly redesigned. Financial journal safeguards would still be required. |
| Fresh unsigned identity, then the selected wallet's transport for the signed request | Exact-price selection with no additional API request; retains pooling for signed wallet traffic. | Not selected. Adds an unsigned transport/cover lifecycle and switches network context between challenge and submission. Core payment fields do not establish provider session or transport portability. A rejection after possible signed submission cannot authorize a fallback payment or replay. |
| Hints plus a utilization-only handoff whenever the actual price reveals a smaller wallet | Improves fit after binding while keeping wallet-specific transports. | Reject. Adds unsigned provider requests merely to improve fit, increasing pressure on providers that rate-limit unsigned traffic. Keep the single affordability handoff only. |
| Extra pricing probes, preflights or hint-refresh requests | Could improve prediction before binding. | Reject. Adds rate-limited traffic and a discovery/refresh lifecycle. Learn runtime hints from ordinary invocation traffic only. |
| Skip the ordinary unsigned request using a cached estimate or previously observed challenge | Could reduce unsigned traffic below the ordinary one-per-invocation flow. | A display estimate is insufficient to sign. Reusable exact requirements would need a concrete provider contract covering terms, request/argument scope and validity; an old observed challenge alone supplies no such contract. This is separate future work, not a fallback here. It also removes the chance to discover a free response. No signed rejection permits automatic replacement or replay. |

Keep three caches/flows distinct. Catalog/help discovery retains its existing caching
and coalescing. Startup pricing remains one-shot and cached, with HTTP-supported disk
freshness for derived display estimates; payment challenges are not persisted as reusable
authority. Runtime hints learn from actual invocation challenges and only guide selection.
The current invocation path still makes an ordinary unsigned request even when a display
estimate is cached. Late selection would use that same response, not inherently add a
pricing request or invalidate discovery caches, and could remove the affordability
re-challenge. Its rejection here rests on transport/cover reuse and lifecycle cost.
The selected design also creates new identities over time; its reuse objective depends
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
