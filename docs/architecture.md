# Runtime architecture

`x402_treazury` exposes x402-paid HTTP APIs as MCP tools, manages Zcash-funded
rotating Base USDC wallets, and routes traffic through a centralized network
layer supporting both direct connections and Tor.

## Catalog configuration and execution

Configuration, catalog loading, payment execution, and MCP serving are divided
across these modules:

| Module | Responsibility |
| --- | --- |
| `src/config.rs` | Loads standalone TOML providers and deployment files, resolves relative paths, runs one-level `extends` inheritance, enforces strict schema validation, and validates `send_tools` rules, `response_mappings`, and `operation_url_extension`. |
| `src/catalog.rs` | Parses OpenAPI JSON into typed tools, resolves local `$ref` targets within a structural work budget, generates deterministic names, flattens simple request bodies, extracts `in: header` parameters, and manages automatic `Idempotency-Key` headers. |
| `src/discovery/` | Fetches remote specs and lazy help documents, manages endpoint-local dynamic providers, runs local BM25 tool and provider search, and handles remote directory queries. |
| `src/pricing/` | Parses embedded (`payment-info`, `x402`) and external (`x402-resources`, `x402-discovery`) pricing metadata into per-operation hints and formats description suffixes. |
| `src/deployment/` | Orchestrates multi-listener startup, `catalog_concurrency` Tor scope gating, source distribution (`routing.rs`), deferred catalog publication, and graceful shutdown. |
| `src/server.rs`, `src/server/` | Serves stateless MCP Streamable HTTP and stdio, enforces bearer authentication (`auth.rs`), Host validation (`host.rs`), accepted-work draining (`work.rs`), and MCP tool inventories. |
| `src/catalog_state.rs` | Owns live source snapshots, runtime publication, tool-name ambiguity suppression, and on-demand search cursors. |
| `src/catalog_refresh.rs`, `src/discovery/refresh.rs` | Coalesces automatic catalog refreshes triggered by final unsigned HTTP `404`, `405`, or `410` responses for static and dynamic sources. |
| `src/payment.rs` | Handles x402 challenges, payment caps, wallet selection, EIP-3009 and Permit2 signing, recognized insufficient-funds wallet traversal, and optional HTTP/2 cover traffic. |
| `src/http_cache.rs` | Implements the private owner-only SQLite discovery cache (`http-cache/discovery-v1.sqlite`) for remote catalogs, external pricing, and observed offers. |
| `src/discovery_relay.rs` | Executes ordered paid relay fallback for catalogs, help documents, directory searches, and opt-in API requests. |
| `src/providers.rs` | Embeds bundled provider metadata, curated schemas, and offline search support. |
| `src/direct_swap/` | Implements optional native shielded ZEC and NEAR foreign-asset receive and send tools. |

### OpenAPI schema projection and structural safety

`src/catalog.rs` parses every OpenAPI document (local, remote, cached, or
relayed) under a shared structural work budget (`document_complexity_limit`):
maximum nesting depth of 64, 500,000 visited nodes, and 64 MiB of string and
key traversal and copy work, including repeated `$ref` lookups. Exceeding the
budget rejects the document as a whole without caching a partial catalog.

Catalog construction returns `CatalogBuild`, separating supported tools from
unsupported operations rather than silently dropping constraints:

- **Method selection (`prefer_post`):** After path, operation, and tag filters,
  exact-path `GET` and `POST` pairs keep `POST` by default before collision
  naming. An unsupported `POST` never hides a compatible `GET`.
- **Schema projection:** Required simple object bodies flatten into top-level
  tool arguments; optional bodies and complex root constraints (`allOf` with
  properties, exclusive alternatives, unfamiliar root keywords) stay wrapped
  under a `body` argument with a closed outer schema. OpenAPI 3.1 `$ref`
  siblings preserve the seven JSON Schema 2020-12 basic annotations via `allOf`
  and reject assertion-bearing or unknown siblings as
  `schema_reference_siblings`. Used `$ref` targets under string `$id` scopes,
  `$dynamicRef`, and `$recursiveRef` are omitted as `input_reference`.
- **Header parameters and idempotency:** Business `in: header` parameters
  become tool arguments supporting OpenAPI `simple` serialization. Protocol
  headers (`Accept`, `Content-Type`, `Authorization`, and x402 payment headers)
  are ignored, and cookies and framing headers are rejected. `Idempotency-Key`
  is managed automatically per call using a fresh UUID string.
- **Omissions and strictness:** Serving and `catalog warm` keep supported
  sibling tools and disclose omissions in MCP instructions and JSON output.
  Strict inspection (`catalog tools`, `config check`), qualification, and the
  public `Vec` builder reject partial catalogs.
- **Schema trust boundary:** Tool input schemas describe supported caller
  contracts rather than complete runtime JSON Schema validation. Narrowed enums
  advertise supported subsets rather than enforcing an authorization boundary,
  so nonconforming arguments can still reach upstream.

### Multi-listener startup and deferred catalog loading

Deployment startup validates configuration, bearer tokens, and local catalogs,
then binds every listener socket before starting background remote catalog
loads and wallet funding.

- **Tor concurrency gate (`[startup].catalog_concurrency`):** Defaults to 2
  concurrent active Tor SOCKS isolation scopes (range 1..=64). Requests sharing
  credentials share one slot; slots are held through response-body collection
  and released before retry delays or wallet waits. Cache hits, local files,
  and direct egress consume no slots.
- **Endpoint readiness (`wait_for_catalogs`):** Defaults to `true`. Each
  endpoint accepts MCP HTTP requests only after its bound sources finish
  loading. Set `wait_for_catalogs = false` to serve available tools immediately
  and publish remote catalogs as they arrive.
- **Catalog failures:** A required remote catalog failure closes its waiting
  endpoints and logs `ERROR`, leaving unrelated endpoints running. Setting
  `sources.NAME.allow_catalog_failure = true` (or `wait_for_catalogs = false`)
  omits the failed source as a unit and lets the endpoint serve healthy
  siblings with coverage notices in MCP instructions. Local configuration and
  parsing errors always fail startup.
- **Ambiguous tool names:** If two sources on the same listener publish the
  same tool name after tag selection and before tool-name filters, serving
  omits every binding of that name and reports the collision in MCP
  instructions. Calls already in flight finish with their active binding.
  Strict inspection rejects duplicates.
- **Advisory startup announcement:** Once every endpoint is listening with its
  catalogs settled and every bound managed pool has at least one eligible payer
  whose saved USDC estimate covers a finite `max_api_payment_usdc`, startup
  logs `All MCP endpoints listening, funded, and ready` at `INFO`. Static
  payers, unlimited caps, or required catalog failures suppress this message.

### Discovery cache, pricing enrichment, and runtime refresh

Deployments with an existing `treasury.state_dir` cache remote catalogs in
`http-cache/discovery-v1.sqlite` (owner-only, up to 4,096 entries, 128 MiB of
payload and metadata, and a 144 MiB file cap). Keys separate exact URL, network
and isolation policy, transport flags, timeouts, limits, and resource kind.

- **Lifetime:** `catalog_cache_ttl_seconds` defaults to `"indefinite"`. Local
  TTLs govern retention regardless of upstream `Cache-Control` headers, and
  reads or price observations do not extend expiry. Validated bytes are staged
  and committed only after generation and `send_tools` validation succeed.
- **Asynchronous pricing enrichment:** External pricing (`x402-resources` and
  `x402-discovery`) loads asynchronously without delaying tool publication.
  Parsed prices inherit the parent catalog's expiry. Fresh observed offers from
  live calls take precedence over advertised hints.
- **Runtime source refresh:** A final unsigned target HTTP `404`, `405`, or
  `410` triggers a coalesced background refresh for that deployment or dynamic
  source. Existing tools stay callable while refresh runs; if refresh fails,
  the previous generation stays active. When a replacement generation
  publishes, old handles and cached price hints retire before the replacement's
  asynchronous pricing task starts, preventing retired tasks from overwriting
  new metadata.

### Direct swap control and dynamic sources

- **Endpoint-local dynamic sources (`src/discovery/`):** Enabled independently
  per authenticated listener via `provider_management_local`,
  `provider_management_remote`, and `raw_url_calls`. Registrations live in
  `src/catalog_state.rs` (and optionally in a per-listener SQLite registry) and
  use the listener's explicit wallet without allocating or funding pools. See
  [agent source management](agent-sources.md).
- **Direct swap control (`src/direct_swap/`):** Enabled per authenticated
  listener via `[servers.NAME.swap_control]`. Native shielded ZEC transfers
  use the embedded Ironwood treasury directly; foreign-asset receives and sends
  use NEAR 1Click quotes and share the treasury's durable SQLite journal.
  Persisted operations remain inspectable via `x402_treazury_swap_status` even
  after new sends or receives are disabled. See
  [direct swap control](configuration.md#direct-swap-control).

## MCP and output boundaries

`src/server/` serves stateless Streamable HTTP at `/mcp` (no session header or
sticky session required) and stdio for standalone providers.

- **Incoming bounds and authentication:** MCP HTTP requests have a 4 MiB body
  limit (returning HTTP `413` with a stderr warning). Bearer authentication
  runs before MCP dispatch using constant-time token comparison, returning HTTP
  `401` and rate-limiting warning logs to once per 30 seconds per listener and
  failure category. Host header validation defaults to `localhost`,
  `127.0.0.1`, and `::1`; disabling both authentication and Host validation is
  rejected.
- **Inventory and instructions:** `tools/list` returns the complete advertised
  list in one response (logging an advisory `WARN` when more than 500
  definitions are advertised) and attaches `_meta["x402_treazury/catalog"]`
  carrying listener instructions, catalog generation, and publication-stop
  status. With `discover_on_demand = true`, `tools/list` advertises only the
  search and call wrappers plus enabled provider-management tools and
  `x402_treazury_raw_call` (when `raw_url_calls = true`). Enabled direct-swap
  tools are omitted from `tools/list` and remain discoverable via
  `x402_treazury_tools_search` and callable via `x402_treazury_tool_call`.
  `instructions_text` overrides composed source guidance while preserving
  pricing caveats and operational coverage notices.
- **Images and binary output (`src/output.rs`):** Small PNG, JPEG, and WebP
  responses become MCP image attachments. Provider `response_mappings` can also
  extract base64 or data-URI images from JSON responses while preserving
  metadata. Oversized responses and images are rejected whole with errors
  naming the configured limit (`max_response_bytes`, `max_help_bytes`,
  `max_spec_bytes`).
- **JSON response projection:** Provider `response_mappings` can omit verbose
  JSON object fields locally after the response arrives (never deleting array
  elements). When a mapping both extracts images and omits fields, image
  extraction runs first on the same parsed response. Result advisories disclose
  every omitted path and removal count.
- **Pricing metadata block:** Managed API results include a second MCP JSON
  text block carrying the latest validated offer for that call:

  ```json
  {"x402_pricing":{"amount":"0.005","currency":"USDC","basis":"observed_offer"}}
  ```

  This block is appended after body text and images and before advisories
  without changing body projection or `structuredContent`. It appears on
  successes, post-validation errors, and final unsigned successes after an
  earlier payer handoff (reporting the latest target offer validated in that
  call without implying a new charge for the final response), stays separate
  from the shared hint cache so concurrent calls retain their own offer, and
  reflects the latest offer rather than settlement or a sum of authorizations.
  Relay fees and cached help never produce a target `x402_pricing` block, and
  body-only library helpers skip this block.
- **Error formatting:** Unsuccessful MCP API errors return a concise failure
  kind, the known HTTP status (distinguishing relay status from reported
  destination status), and a single warning not to auto-retry potentially
  chargeable or uncertain write calls. Internal error chains retain sanitized
  upstream context and payment receipt error fields (`errorReason` and
  `errorMessage`), while logs emit only fixed safe categories without upstream
  prose.

## Financial and network ownership

Named wallet identities determine payer sharing. Two sources can share one
wallet, and the same provider can bind to separate wallets through source
aliases. One embedded SDK-owned Zcash treasury can fund multiple managed pools;
see [wallet rotation](wallet-rotation.md) for admission, promotion, encrypted
persistence, and recovery.

Read-only startup checks (treasury-ID discovery and managed profile name
lookup) read schema and identity metadata without unlocking keys or parsing
financial reports. Managed serving still authenticates the encrypted snapshot
under an exclusive lock.

`src/network.rs` is the sole constructor for production outbound HTTP and gRPC
clients in both direct and Tor modes. Connection pools are keyed by identity
and destination: discovery traffic uses origin identities, payments and
recipient-specific funding use immutable EVM addresses, and treasury scans use
the treasury UUID. See [network egress](network-egress.md).

On `SIGINT` or `SIGTERM`, serving stops accepting new calls and drains accepted
work with 10-second stderr progress reports and no fixed drain deadline. A
second signal forces exit with an unknown-outcome warning while preserving
reservations and journals. Client disconnects and stdio EOF stop response
delivery without abandoning accepted calls.

## Wallet selection and network isolation

### Bootstrap routing

When `[wallet_bootstrap]` is configured, participating managed groups pay
through the bootstrap group until each destination group records its first
positive initial USDC credit.

- The trusted API boundary validates arguments and permissions, records durable
  UUID-bound first-use demand (even on free calls or cache hits), and captures
  one effective wallet group before unsigned dispatch. Discovery, catalogs,
  pricing, registration, and help do not activate demand.
- Activation wakes the background funding owner to arm an independent 30-120
  second funding opportunity without blocking the API call.
- Once a destination group records any positive initial credit (even partial or
  on a now-retired wallet), it switches permanently to its own wallet and cap
  across restarts; depletion or refunds never revert it to bootstrap.
- Paid metadata and directory relay fees always use the bootstrap group while
  `[wallet_bootstrap]` is active, overriding `relay_fallback.wallet`.

### Payer selection and HTTP pooling

Before the first unsigned request, managed selection chooses a price hint in
order: fresh observed offer, valid advertised per-call USDC estimate, or
unknown.

- **With a price hint:** Prefers estimated-affordable `RETAINED` wallets (oldest
  first), then `ACTIVE`, then `READY`.
- **Without a price hint:** Prefers healthy positive `ACTIVE`, then `RETAINED`,
  then `READY`.
- **Fallback:** Low, zero, and unknown estimates remain eligible fallbacks.
  Once a challenge arrives, an eligible bound payer stays bound rather than
  handing off just to spend a retained wallet.

`src/payment.rs` binds the unsigned request, challenge validation, signing,
signed dispatch, response body read, and optional cover traffic
(`cover::registry::Owner::from_transport`) to the same pinned `HttpTransport`
and EVM address. `src/network.rs` keys both HTTP/2 connection pools and Tor
SOCKS isolation on that identity, so rotating wallets always rotates circuits
and TLS connections.

### Unsigned x402 requests and POST retries

Protected x402 `POST` endpoints gate execution on payment acceptance rather
than executing on the initial unsigned probe, so unsigned x402 `POST` probes
follow the same retry eligibility as unsigned `GET` and `HEAD` (this is a
property of payment-gated x402 execution, not a general claim about HTTP
`POST` idempotency).

Participating API clients (generated tools, static and managed payers,
directory tools, and `raw_call`) may try `1 + network.tor_isolation_retries`
Tor isolation scopes (default 3) for unsigned requests on eligible transport
failures and HTTP `400`, `403`, `408`, `429`, `500`, `502`, `503`, and `504`
(honoring valid `Retry-After` on `429` and `503`), promoting a winning scope
only after a valid `402` challenge or completed unsigned response.

Method retry boundaries are enforced strictly before any retry or relay:

- **Unsigned `GET`, `HEAD`, and eligible x402 `POST`:** Use the full unsigned
  transport classifier across request dispatch and successful-response body
  collection. Unlike complete-document discovery and catalog reads, API
  responses allow HTTP `206` range responses, and `HEAD` returns empty text
  without body decoding.
- **Other methods and manual-header writes:** Recover only from TLS
  connection-establishment failures before dispatch. A TLS failure after
  dispatch does not authorize replay even before response headers arrive.
- **Relay fallback:** Runs once after the origin sequence using the original
  method's relay list, before any target signing. Any prior signed target
  attempt suppresses relay fallback. Relayed `402` responses are terminal and
  never signed.
- **Recognized insufficient-funds traversal (managed profiles only):** After a
  signed attempt receives a recognized structured insufficient-funds `402`
  response, selection may visit each remaining eligible wallet in the pool's
  current inventory once, fetching a fresh unsigned challenge through each
  wallet's own transport while scheduling an asynchronous balance read. Each
  signed authorization remains potentially chargeable; successful signed
  responses, signed transport errors, and generic signed errors never trigger
  another attempt, and selection never creates wallets or waits for funding.

### Shared price-hint cache

Remote catalog cache entries own passive operation price hints, shared across
compatible aliases, listeners, and wallet groups without sharing signers or
transports. Cache entries and hints share the catalog's
`catalog_cache_ttl_seconds` lifetime (default indefinite) and invalidate
together on expiry, forced refresh, changed bytes, or source configuration
updates.

When a runtime refresh publishes a new generation, its pricing task attaches
before publication and starts only after the old generation retires. Retired
generations cannot commit late observations or overwrite the new generation's
hints.

## Payment extensions and offer metadata

`src/payment.rs` negotiates x402 payment requirements using `x402-rs` over
canonical Base mainnet USDC (`0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`).

- **Supported schemes and mechanism selectors:** Static profiles support x402
  v1 and v2 `exact` and v2 `upto` (Permit2 `upto` requires an externally
  provisioned allowance and never auto-approves one). Managed profiles support
  v1 and v2 `exact` EIP-3009 only, requiring v2 `extra.assetTransferMethod` to
  be absent or `"eip3009"` (rejecting Permit2 and unknown transfer selectors).
- **Top-level challenge extensions:** Managed payments strip top-level
  challenge extensions before validation and signing so unrecognized or
  optional extensions cannot silently alter signing behavior. Stripping
  extensions provides neither wallet-ownership authentication (SIWX) nor seller
  receipts or vendor idempotency keys, so providers requiring them may reject
  payment; see
  [payment extension boundaries](../providers/CAVEATS.md#payment-extension-and-offer-boundaries).
- **Opaque offer metadata and amount fields:** `accepts[].outputSchema` and
  informational `extra` fields are treated as opaque metadata and echoed
  unchanged without affecting payment authority. On v2 challenges, `amount`
  alone governs the atomic payment amount (any legacy `maxAmountRequired` is
  opaque); v1 challenges use `maxAmountRequired`. Per-payment
  `max_api_payment_usdc` is checked in exact atomic micro-USDC before signing,
  and v2 challenge descriptions cap at 500 characters for facilitator
  compatibility.
- **Offer metadata:** Validated v2 offers populate both the call's
  `x402_pricing` MCP block and, for managed calls backed by a remote catalog
  cache entry, the shared runtime price-hint cache. Advertised spec prices and
  relay fees remain separate from observed target offers.
- **Cover traffic:** `ManagedPool::pay_with_cover` accepts an optional cover
  profile bound to the same payer transport and origin. See
  [cover traffic](cover-traffic.md).
