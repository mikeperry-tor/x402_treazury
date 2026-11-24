# Runtime network egress and timeout policy

`src/network.rs` is the sole constructor for production outbound HTTP and gRPC
clients in both direct and Tor modes. Direct and Tor traffic share the same
factory, callers, and payment and funding state machines.

See
[wallet selection and network isolation](architecture.md#wallet-selection-and-network-isolation)
for pooling, cover-state, and Tor reuse tradeoffs, and
[local resource limits](local-resource-limits.md) for memory, cache, and queue
bounds.

## Network policy and client pooling

- **Policy resolution:** Commands use the deployment's `[network]` table unless
  `--network-config FILE` replaces it whole (omitted fields take network
  defaults). If `--network-config` also contains `[relay_fallback]`, it
  replaces the deployment's relay table; otherwise the deployment's relay table
  is preserved. Standalone provider, wallet, and diagnostic commands accept
  `--network-config` without loading relay providers. Supervised qualification
  rejects `--network-config` and requires policy inside its pinned deployment.
- **Base RPC privacy in direct mode:** Direct Base RPC reads expose your public
  IP alongside queried wallet addresses, so direct serving logs a warning. Tor
  mode suppresses that warning (RPC providers still see queried wallet
  addresses). `config show` reports `base_rpc_policy` in both modes.
- **Local listener DNS:** Listener bind hostnames resolve once via system DNS
  at bind time and never affect outbound routing or Tor remote DNS.
- **No User-Agent and no redirects:** Runtime and evidence-capture clients send
  no `User-Agent` header and disable HTTP redirects.
- **HTTP and gRPC transport caches (`HttpTransport` and `GrpcIndexer`):** Each
  `NetworkContext` keeps separate 256-entry LRU caches for outbound HTTP and
  gRPC clients, keyed by `SelectionPolicy` (`Legacy` vs. `Participating`),
  protocol flags (`allow_http1`, `allow_tls12`), public-destination
  enforcement, Tokio runtime, canonical identity, origin, and exact
  read-inactivity timeout. The mutex guards only lookup and winner publication,
  never client construction. Eviction drops only cache ownership; active
  handles finish in-flight work.
- **Participating Tor scope retries:** In Tor mode, `http_participating` misses
  and `http_candidate` retries derive fresh owner-bound `tor-scope` isolation
  credentials (`IsolationId::fresh_scope`) with separate connection pools.
  `promote_http` replaces a canonical slot only if it still holds the exact
  starting `Arc<HttpTransport>` (preventing late losers from overwriting
  winners or resurrecting evicted entries).

### Outbound caller identity inventory

| Caller | Transport and isolation identity |
| --- | --- |
| `main.rs`, `deployment.rs`, `catalog.rs` | Spec fetch, participating discovery origin (promotes on valid JSON or `304`) |
| `catalog_state.rs` | Lazy help reader, help URL discovery origin, source-bound relay |
| `discovery/coinbase.rs` | Unsigned resource search, endpoint wallet documentation identity, participating origin retries, optional paid relay |
| `discovery/import.rs` | Complete OpenAPI reader, URL discovery origin, public-only destinations |
| `discovery/source_details.rs` | Usage-document GET or fixed GET probes, endpoint documentation or URL discovery identity |
| `payment.rs` | Unsigned challenge, signed retry, and cover traffic pinned to the selected EVM payer |
| `rotation/base.rs` | Participating read-only RPC: address reads and single-owner headers by EVM owner; multi-owner views by RPC discovery origin |
| `rotation/near.rs` | Metadata, quotes, and status by funding recipient EVM address; direct swaps by treasury UUID and operation UUID |
| `treasury/birthday.rs` | Mainnet tip and capability lookup, per-invocation random bootstrap identity |
| `treasury/mod.rs`, `server.rs`, `send.rs` | Sync scans, witnesses, proposals, and capability checks by treasury UUID |
| `treasury/submission.rs`, `expiry.rs`, `refunds.rs` | Shared scans by treasury UUID; per-operation lookup and submission by persisted operation or recipient identity |
| `wallet_cli.rs` | Same factory as serving; local inspection stays offline |
| `tools/providers/snapshot_spec.rs`, `tools/diagnostics/quote_near.rs`, `tools/providers/provider_updates.rs` | Same factory; discovery or public-only policy with `--network-config` support |

Identity rules:

- **Base clients:** Single-owner views (including retirement pages) use the
  queried EVM owner for headers, canonical rechecks, and fallback views.
  Multi-owner pool views keep RPC-origin headers while still keying each
  contract read to its wallet. Base clients never use paid relays.
- **Prepared Zcash transactions:** Resolve transport identity from persisted
  funding job, recovery, or `operation_network` bindings (never guessing from
  current pool roles). Direct operations use their persisted operation
  identity.
- **Suppressed transparent queries:** Generated, imported, reopened, and
  reconstructed wallets disable transparent known-address RPCs and gap scans so
  refund receivers disclosed to NEAR cannot be correlated at the Zcash indexer.
  See [refund receivers](zingo-integration.md#refund-receivers-and-seed-recovery).

### Transport failure classification

Participating unsigned retry recovery (`1 + network.tor_isolation_retries`
scopes in Tor mode, default 3; 1 attempt direct) applies to catalogs,
Coinbase search, document reads, relay-service unsigned requests, Base reads,
and unsigned API `GET`, `HEAD`, and eligible x402 `POST`:

- **Eligible for a fresh Tor scope:** Transient SOCKS target errors, remote DNS
  failures, connection resets, TLS failures (keeping certificate, hostname, and
  protocol checks enabled), read and connect timeouts, and HTTP `400`, `403`,
  `408`, `429`, `500`, `502`, `503`, and `504`. Valid `Retry-After` on `429`
  and `503` delays before the next attempt or relay handoff (unrepresentably
  long delays remain pending and cancellable rather than firing immediately).
- **Terminal (never retried on another scope):** Proven local SOCKS connection
  refusal (preserved across the vendored reqwest SOCKS boundary), SOCKS auth
  rejection, HTTP parse, construction, and redirect errors, other `5xx` (such
  as `501`, `505`, and `520`), local destination, size, and policy rejections,
  and any signed request failure.
- **Other API methods and manual-header writes:** Recover only from TLS
  connection-establishment failures before dispatch; post-dispatch TLS failures
  never authorize replay.

## Relay fallback

Static catalogs, Coinbase searches, runtime imports, help, usage documents,
probes, and opt-in API calls share `network::acquisition` for origin attempts
followed by ordered paid relay fallback (`[relay_fallback.relays]` `GET`,
`HEAD`, and `POST` lists).

- **Categories:** `source_discovery` (default `true`), `provider_metadata`
  (default `true`), and `api_requests` (default `false`). Mixed workflows
  classify each request by trusted purpose rather than method or hostname.
- **Origin handoff:** Direct mode hands off after 1 origin attempt, Tor mode
  hands off after exhausting its configured scopes on retryable failures, and
  HTTP `501`, `505`, and `520` hand off immediately without fresh Tor retries.
  Local policy, destination, and size errors, unsolicited `206` on complete
  catalogs, and terminal `4xx` never pay a relay. Built-in directory reads
  require [complete results](agent-sources.md#external-directory-matching).
- **Traversal and liability:** Each listed relay runs at most once using its
  own `relay_format` (`curl`, `agproxy`, `horizonpulse`, `readfetch`) and
  unsigned Tor retry count, with recursive fallback disabled. Adapters skip
  requests whose headers or body they cannot preserve before paying. A relay
  service error, relay `402`, or malformed envelope advances to the next
  provider while preserving possible charges; a decoded **target** `402` is
  terminal and never signed. Any prior signed target attempt suppresses API
  relay fallback.
- **Locking and concurrency:** Static relay requests serialize per relay
  provider across wallets while holding shared cache and gate locks only
  briefly (never across I/O or waits). Tor catalog and relay requests count
  against `startup.catalog_concurrency` through response-body download and
  release slots before retry delays or wallet waits.
- **Diagnostics:** `INFO` logs safe advancement across remaining relays with
  possible-payment context; `WARN` reports the final exhausted or stopped relay
  using `failure_location` (distinguishing relay service, reported target
  response, target delivery, validation, and local policy) without URLs or
  upstream prose.
- **Trust boundary:** All relay connections use `src/network.rs` (Tor never
  falls back to direct). Relays see target URLs, forwarded headers, and
  plaintext responses and may follow redirects remotely. Final URLs and Curl
  hops are checked against the public-destination policy; AGProxy, Horizon
  Pulse, and ReadFetch report only the final URL. Nonempty manual raw headers
  disable paid relay fallback.

## Explicit direct discovery warming

`catalog warm --config FILE --source ID --direct` fetches selected remote
catalogs and external pricing documents unsigned over direct networking without
constructing a serving `Deployment`, payer, or treasury.

Entries store separate direct-warm provenance bound to the deployment's
original network policy. A later Tor `serve` reuses fresh entries (logging a
warning) without sending directly fetched validators over Tor or refreshing
directly after expiry. Live qualification rejects warming and bypasses disk
caches. See
[catalog warming](configuration.md#warming-a-selected-source-including-directly-for-a-tor-deployment).

## Timeout budgets and concurrency

Omitting timeout settings selects defaults rather than disabling timeouts.

- **HTTP read inactivity (`read_timeout_seconds`):** Defaults to `60` seconds
  (range `1..=86400` at network level; providers may override with positive
  floats up to `86400`, with a `1ms` floor in standalone mode; curated dynamic
  imports keep provider values; generic imports and `raw_call` inherit network
  default). Renews on every nonempty response DATA
  read with no whole-download deadline. The vendored reqwest patch starts the
  header timer only after DNS, SOCKS, and TLS connect complete (post-connect
  header wait still includes stream-capacity waiting and request upload); no
  sibling request or keepalive can renew it.
- **Connection establishment (`connect_timeout_seconds`):** Defaults to `15`
  seconds direct and `120` seconds Tor (range `1..=300`), independent of read
  inactivity.
- **Base RPC reads:** Use connection and read-inactivity timeouts with no total
  view deadline. Direct mode allows 2 same-client transport attempts (200 ms
  backoff); Tor mode uses `1 + tor_isolation_retries` scopes per endpoint
  before failing over to the next configured endpoint (restarting the whole
  read-only view, discarding partial evidence). Canonical proofs require the
  provider's `finalized` anchor (no depth-only downgrade) and cap response
  bodies at 16 MiB.
- **Treasury gRPC:** Uses network connection and read-inactivity timeouts,
  stripping legacy `grpc-timeout` metadata and measuring per-response
  inactivity below protobuf decoding (`grpc_headers_inactivity`,
  `grpc_body_inactivity`, and `grpc_dispatch_inactivity` for channel and stream
  readiness at `DEBUG`). Cancelled or timed-out queued calls cancel their
  request bodies, and the vendored Hyper and h2 patches discard unopened
  streams without sending `HEADERS` or `RST_STREAM`.
  - **Accepted limitation:** Because the gRPC transport has no per-request
    HTTP/2 upload-progress hook, the header timer starts when the request body
    is first polled and includes request upload. A slow upload can time out
    before headers arrive; timeout never proves non-submission or authorizes
    replay.
- **External pricing enrichment:** Uses the same transport policy, Tor
  `catalog_concurrency` slots, and metadata relay rules as its parent catalog,
  runs asynchronously without delaying tool publication, and inherits the
  parent catalog's expiry.
- **MCP HTTP streaming:** Stateless SSE responses emit keepalive comments every
  2 seconds to keep client connections alive without extending remote read
  timeouts or payment authority. The pinned `rmcp` teardown drain (2s cancel
  and 5s transport close) bounds only response delivery; accepted calls survive
  in the application `Work` owner and log `mcp_response_cancelled` and
  `mcp_work_finished_after_cancellation`.
- **Foreground work and shutdown:** Signed HTTP and advisory Base reads run
  outside the serialized Store worker. `Work` tracks accepted MCP, directory,
  and relay calls through output conversion and drains without a fixed cutoff
  on first `SIGINT` or `SIGTERM`; a second signal exits immediately while
  keeping reservations and journals intact.

## Retry ownership

Automatic transport retries are disabled on application HTTP clients: reqwest
uses `retry(never())`, and the vendored reqwest patch disables Hyper's
automatic retry of unstarted pooled requests (both to prevent hidden retries
and to stop a replacement Tor connection from running inside an already-started
header timer). Application owners decide every retry explicitly:

| Activity | Retry and observation owner |
| --- | --- |
| Catalogs, help, and imports | Shared unsigned acquisition (`network::acquisition`) owns origin Tor retries and ordered relay handoff; consumers own validation and caching. |
| Unsigned x402 `GET`, `HEAD`, and `POST` | `PaidClient` owns unsigned Tor scope retries and opt-in API relay under the [x402 retry contract](architecture.md#unsigned-x402-requests-and-post-retries). |
| Signed x402 payments | Payment owner keeps each authorization. Generic errors and timeouts never retry; only recognized structured insufficient-funds `402` responses traverse remaining eligible wallets once. |
| Base balance and receipt RPCs | `rotation/base.rs` owns per-endpoint transport retries and whole-view provider failover; reconciliation owns recurring schedules. |
| NEAR metadata, quotes, and status | Funding and direct-swap owners run independent status polling (including archived requests, logged at `DEBUG` while unresolved); polling never authorizes another deposit. |
| Zcash sync, lookup, and submission | Tonic gRPC (outside reqwest). Pepper owns read retries; treasury reconciliation and explicit `wallet reconcile --rebroadcast` own source work without replacing uncertain submissions. |

## Stable credential encoding

Tor SOCKS isolation tokens are lowercase hex SHA-256 of
`x402_treazury-tor-isolation-v1`, a zero byte, 4-byte big-endian namespace
length, UTF-8 namespace bytes, and 4-byte big-endian length-prefixed identity
fields (including the kind field):

- **EVM:** `evm`, `eip155:8453`, normalized 20-byte address.
- **Treasury:** `treasury`, persisted treasury ID string bytes.
- **Discovery:** `discovery`, canonical scheme, canonical host, 2-byte
  big-endian effective port (no path, userinfo, or query).
- **Bootstrap:** `bootstrap`, 16 random UUID bytes per invocation.
- **Participating Tor scope:** `tor-scope`, the complete owner encoding above,
  16 random UUID bytes per transport record.

Pinned by `tests/network.rs`. Contains no signing or encryption secrets; format
changes require a new versioned domain separator.

## Dependency boundaries

- **x402 clients:** Sign locally; the `upto` client uses provider type `()` so
  it never opens SDK-owned RPC connections.
- **Zingo and Pepper:** `LightClient` stays offline; `network::GrpcIndexer`
  wraps the generated gRPC client over `src/network.rs` with no SDK online
  constructors, URI setters, or nym and price workers. Build-time Sapling
  parameter downloads remain an upstream `zingolib` build-script exception
  outside runtime Tor routing; see
  [Ironwood-only policy](zingo-integration.md#ironwood-only-policy).
- **reqwest and tonic:** Use `no_proxy`, explicit `socks5h` in Tor mode (no
  unauthenticated SOCKS downgrade), no redirects, and normal TLS and SNI
  verification. `tests/network_audit.rs` rejects direct network constructors
  outside `src/network.rs` (the sole exception is the Docker regtest harness).
- **Qualification scope observer:** Opt-in live qualification installs a
  private pre-SOCKS scope observer to audit circuit separation; normal serving
  installs no observer and retains no scope history.

## Agent-selected destinations

Dynamic imports, refreshes, lazy help, `documentation_url` reads, `base_url`
probes (4 concurrent GETs), and `raw_call` all use the factory's `public_only`
pool key:

- **Public HTTPS only:** Rejects non-HTTPS URLs, embedded credentials,
  fragments, non-public literal IPs, non-public DNS answers (a custom direct
  resolver rejects the whole lookup if any answer is non-public), and HTTP
  redirects. Generic help URLs must match the API base origin; bundled
  providers keep their reviewed help origin.
- **Tor internal-address guard:** Tor mode uses `socks5h` remote DNS and relies
  on Tor's `ClientRejectInternalAddresses` (keep it enabled).
- **Manual raw headers:** Nonempty `raw_call` headers disable paid relays and
  cross-payer handoff, pin the initial payer, and restrict write retries to
  pre-dispatch TLS establishment failures. See
  [manual raw headers](agent-sources.md#manual-raw-headers).

## Provider protocol policy

- **Strict defaults:** Provider HTTPS (catalogs, help, APIs, and cross-origin
  operation URLs) requires HTTPS, HTTP/2-only ALPN, and TLS 1.3 minimum.
  `allow_http1` and `allow_tls12` opt in per provider or source without
  weakening certificate, hostname, redirect, or proxy rules. Generic dynamic
  imports and `raw_call` always use the strict defaults; curated dynamic
  imports preserve their bundled flags.
- **Private trust anchors:** `[network].trusted_ca_files` (or embedder
  `NetworkContext::with_trusted_roots`) extends both platform HTTP and gRPC
  web-PKI trust stores immutably at startup, including through Tor and
  `catalog warm --direct`. Missing or invalid PEM files fail startup without
  printing certificate contents.
- **Pool separation:** Pool keys and catalog cache keys include protocol flags,
  so `allow_http1` and `allow_tls12` connections are never reused by strict
  clients (`src/network_http_tests.rs`). `DEBUG` logs record negotiated
  `http_version` and stage without URLs or credentials.

## Serving startup SOCKS prerequisite

In Tor mode, serving startup runs one shared connection-only probe to
`socks_endpoint` (TCP or Unix socket; no credentials or SOCKS handshake) before
starting pending catalog fetches and startup wallet discovery:

- Retries every 5 seconds using `connect_timeout_seconds` per attempt.
- Logs `ERROR` immediately on failure (with endpoint and safe I/O kind) and
  repeats at most once every 60 seconds while waiting, then logs `INFO` once
  reachable (reporting TCP or socket reachability, not Tor bootstrap
  completion).
- Listener binding, fresh disk caches, CLI commands, and explicit financial
  commands do not wait on this probe. Stops on first reachability or shutdown.

## Integration supervisor boundary

The opt-in live integration runner keeps local MCP and authenticated Tor
control connections outside confined subprocesses via `network::local_control`
(loopback literals only, TCP-only; Unix SOCKS is covered by local fixtures).
`QualificationProbes` supplies positive and negative IPv4/IPv6 loopback egress
controls. External API traffic inside confined subprocesses still uses
`src/network.rs`. See
[owned-Tor qualification](../tests/live/INTEGRATION_REFERENCE.md#owned-tor-qualification).

## Verification

- **Local regression suite:**
  ```sh
  make test CARGO_ARGS='--offline --all-targets' TEST_ARGS='--test-threads=1'
  python3 vendor/verify.py
  python3 vendor/verify_pepper.py
  make check-compat
  ```
- **Regtest Ironwood lifecycle over fake SOCKS (Docker, no funds):**
  ```sh
  make test CARGO_ARGS='--offline --features zcash-regtest --lib treasury::regtest::tor_consensus_lifecycle' TEST_ARGS='--ignored --exact --nocapture'
  ```
- **Read-only live Tor smoke check (contacts `zec.rocks`, creates no wallet):**
  ```sh
  TOR_SMOKE_SOCKS=127.0.0.1:9150 make test CARGO_ARGS='--offline --test network live_tor_unfunded_smoke' TEST_ARGS='--ignored --exact --nocapture'
  ```
- **Controlled live-Tor observation:** See [tests/TOR.md](../tests/TOR.md) and
  [qualification status](testing.md#qualification-status). For OS-level egress
  enforcement beyond application tests, confine the executable with a firewall
  or container policy allowing only `socks_endpoint` and local MCP traffic while
  blocking IPv4/IPv6 direct egress and UDP DNS (keeping Tor outside that
  sandbox).

## Remaining timers and budgets

These timers and protocol windows are independent: transport progress renews
only read inactivity, never financial expiry or qualification authority. For
memory, cache, and queue bounds, see
[local resource limits](local-resource-limits.md).

| Timer or window | Default and range | Scope and behavior |
| --- | --- | --- |
| Connection establishment (`connect_timeout_seconds`) | 15s direct, 120s Tor (`1..=300s`) | Bounds TCP, Unix socket, SOCKS, and TLS setup. |
| HTTP and gRPC read inactivity (`read_timeout_seconds`) | 60s (`1..=86400s`) | Renews per response on nonempty DATA; no whole-download deadline. Header wait includes request upload (accepted limitation). |
| gRPC dispatch readiness | Matches `read_timeout_seconds` | Bounds waiting for channel and stream capacity before body dispatch (`grpc_dispatch_inactivity`); ends at dispatch. |
| Participating Tor HTTP retries (`tor_isolation_retries`) | `1 + 2` attempts (default 3) | Fresh Tor scopes on eligible statuses and errors; honors `Retry-After` on `429` and `503` with no local cap. Never applies to signed requests. |
| Startup SOCKS prerequisite | 5s retry, 60s `ERROR` reminder | Connection-only check while waiting for Tor; stops on first reachability or cancel. |
| MCP SSE keepalives | 2s | Keeps stateless HTTP client connections open; never renews remote timeouts or payment authority. |
| `rmcp` response teardown drain | 2s cancel and 5s close | Bounds response delivery only; accepted work drains in `Work` with no cutoff. |
| Pepper sync checkpoints and drain logs | 30s checkpoints and 10s drain `INFO` | Persistence and progress reporting only; never an abort deadline. |
| Sync mempool drain window | Bounded initial settle | Bounds initial mempool stream settling before shutting down that stream; not a sync deadline. |
| Treasury ZEC scan cadence | 60s after success; 90s, 180s, and 300s error backoff | Runs only when enabled funding, pending source or refund work, or active direct receive needs it; idle treasury does not poll. |
| Local SDK publication and reconstruction retry | 60s | Retries unacknowledged local publication or SDK reconstruction without triggering network scans. |
| Base wallet observation and retirement floor | 60s completion floor; backoff up to 300s | Per-wallet cooldown for startup, requested refreshes, initial credit, and candidate retirement proofs. |
| Managed reserve refill opportunities | Uniform 30-120s (mean 75s) per eligible group | Independent volatile timer for early rotation and vacancy fills; explicit `wallet bootstrap` allocates promptly. |
| Funding and direct-swap status polling (`funding.poll_seconds`) | 5s (`1..=60s`) with jitter; 60-300s error backoff | Governs successful recurring source and NEAR status polls and local scheduler wakes. |
| Swap delay health warning (`funding.swap_timeout_seconds`) | 1,800s from broadcast intent | Marks pool `funding_degraded` without abandoning work or authorizing replacement. |
| NEAR quote deadline (`funding.quote_deadline_seconds`) | 1,800s (managed and send); 7,200s (foreign receive) | Requested quote window; returned provider deadline governs preparation and first send. |
| Zcash consensus transaction expiry | 40 blocks pre-NU7, 120 blocks post-NU7 | Signed consensus height; inclusive at height N. |
| x402 authorization validity | `min(provider_timeout, 300s)` | Bounds newly signed EIP-3009 and Permit2 transfer authority; expiry never permits replay. |
| `wallet bootstrap` progress polling | 1s claim check and 60s progress `INFO` | Reports pending bootstrap jobs; no total-work deadline. |
| Local Tor control and qualification probes | 10s (Tor control) and 2s (qualification probes) | Diagnostic and qualification control-port checks only; `WARN` on expiry. |

### Source contract and upload limits

- **1Click quote and status contracts:** The
  [quote request contract](https://docs.near-intents.org/api-reference/oneclick/request-a-swap-quote)
  defines the requested `deadline` as the start of refund handling; the
  [Quote model](https://github.com/defuse-protocol/one-click-sdk-typescript/blob/main/src/models/Quote.ts)
  defines `quote.deadline` as address inactivity and `timeWhenInactive` as
  when the address becomes cold (slower processing). The
  [status contract](https://docs.near-intents.org/api-reference/oneclick/check-swap-execution-status)
  has no terminal `EXPIRED` status for unused `PENDING_DEPOSIT` quotes; local
  deadlines and signed expiry never fabricate a provider outcome. See the
  [deferred swap timing assessment](plans/deferred/near_swap_timing_tightening.md).
- **Published `EXACT_OUTPUT` excess handling:** Per
  [1Click swap types](https://docs.near-intents.org/integration/distribution-channels/1click-api/swap-types),
  excess deposits return to `refundTo` less `refundFee` in origin base units.
  Public docs do not specify the deployed `nep141:zec.omft.near` watcher's
  confirmation count, late-arrival rules, or ZEC refund minimum.
- **Reference implementations (not deployment guarantees):**
  - Near-One `bridge-sdk-rs` (`ed0bf23fbcb50b93481803fa6369b799b7c44c67`):
    `get_orchard_mode` uses shielded handling for Orchard-only UAs and defaults
    mixed UAs to transparent; `build_refund_chain_specific_data` builds
    shielded refunds following the active consensus branch.
  - Near-One `omni-bridge` (`9738d613...`) and `omni-bridge-services`
    (`00ccb5d1...`) are adjacent references, not attestations for
    `zec.omft.near`.
  - Zodl iOS (`bfce63e8...`, Swift SDK `1e632565...`) and Android
    (`2beecca8...`, Android SDK `c1378753...`) send shielded UAs with a 2-hour
    window.
  - Dry unfunded quotes check format acceptance only; live refund verification
    belongs to the
    [shielded-refund evidence recipe](../tests/live/INTEGRATION_REFERENCE.md#shielded-refund-evidence-and-qualification).
- **gRPC upload inactivity:** Because the tonic and h2 stack exposes no
  per-request upload-progress hook, `read_timeout_seconds` covers both upload
  and waiting for response headers. A timeout on source submission retains
  exact bytes and input ownership without authorizing replay.
