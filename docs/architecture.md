# Runtime architecture

Treazury is one Rust process exposing HTTP APIs as MCP tools and handling x402
payments on the caller's behalf. `src/main.rs` selects standalone stdio/HTTP
serving, a multi-listener TOML deployment, or wallet/source administration. There
is no Python server, external wallet daemon or payment logic in the calling agent.
See the [README](../README.md) for commands and configuration examples.

## Catalog, configuration and execution

`src/config.rs` composes inline source settings with one level of provider TOML
inheritance. Local values replace inherited values, including lists/maps; omitted
values inherit. Paths belong to the file declaring them. Unknown fields and
unsupported compositions fail explicitly. `--show-config` resolves composition,
permissions and wallet bindings without loading specs or reading keys.

`src/catalog.rs` loads OpenAPI JSON and builds tool schemas, names, descriptions
and path/query/body routing. Provider files in `providers/` carry selections,
help URLs, pricing settings and explicitly reviewed response mappings. Curated
local specs are inputs for providers whose public schema needs adaptation; the
fixture utility is `examples/snapshot_spec.rs`. Tests pin generated contracts
against `tests/fixtures/`. Header parameters do not grant agents control over
payment headers. Schema conversion is not a general-purpose OpenAPI implementation;
for example, file uploads and asynchronous job orchestration are unsupported.

`src/deployment.rs` validates listeners, filters and wallet references and resolves
source overrides, listener defaults and optional automatic assignment. Static
sources load before serving through a rolling queue, defaulting to 2 active loads
(`startup.catalog_concurrency`, 1..64). Any failure cancels unfinished loads; source
completion order cannot change the final sorted inventory. All declared sources
are loaded, including those unused by listeners. Remote aliases share fetching
and immutable parsed JSON within a load only for the same exact URL, requested
timeout, byte limit and discovery identity. Filters, overrides, base URLs and
wallet bindings remain per alias. Local files and later loads do not reuse this
cache. Progress/timings go to stderr. Startup pricing discovery runs after tool
selection, uses unsigned eligible GETs and caches both success and failure for
the process lifetime. Empty price results reuse the original tools instead of
regenerating identical definitions. TTL expiry never initiates a refresh. Lazy help fetches on
first invocation; successful content is cached, failed initialization can retry,
and simultaneous requests for the same resource coalesce. These caches are
independent of fresh payment challenges.

`src/catalog_state.rs` publishes immutable snapshots. A tool invocation captures
its route, output policy and payer binding so a concurrent catalog replacement
cannot redirect an in-flight call. Listener filters constrain invocation as well
as listing. Agent-added sources use the same execution path; see
[agent source management](agent-sources.md) for grants, persistence and refresh.
Independent background startup of static sources is still
[deferred](plans/deferred/independent_provider_startup.md).

## MCP and output boundaries

`src/server.rs` serves tools through `rmcp`. Streamable HTTP listeners are
bearer-authenticated and stateless; stdio reserves stdout for protocol messages.
Logs go to stderr. Current HTTP operation does not promise a server-to-client
list-changed notification channel. Dynamic deployments expose listener-scoped
search/call fallback tools for clients that cache their initial inventory.

`src/payment.rs` owns the unsigned challenge and single signed attempt. Static
profiles support Base USDC v1/v2 exact and v2 upto signing. Managed profiles use
v2 exact EIP-3009 admission and durable reservations. A paid timeout/body failure
can still have spent money; neither tool dispatch nor output conversion replays it.

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

`src/network.rs` is the shared factory for both direct and optional Tor traffic.
Connection pools are keyed by identity and destination; enabling Tor does not
create a second execution stack. Catalog/help/pricing use discovery-origin
identities; payment and recipient-specific funding work use immutable EVM
identities; wallet scans use the treasury UUID. The
[egress inventory](network-egress.md) records callers, SDK integration, credential
encoding, DNS behavior and qualification limits.

Shutdown stops accepting calls and announces a deliberate bounded drain for
payment safety. Treasury ownership and durable journals survive cancellation;
an interrupted waiter does not undo accepted preparation or submission. Restart
reconciles existing work rather than inferring non-payment from a missing response.

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
