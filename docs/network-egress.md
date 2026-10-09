# Runtime egress inventory

In direct mode, Base RPC providers contacted for verification can associate the
client's public IP address with queried wallet addresses, including when fallback
providers are used. Serving emits a warning about this exposure. In Tor mode,
providers see a Tor exit IP instead of the client's public IP, so this warning is
suppressed. RPC providers still receive the wallet addresses being queried.
`config show` lists the configured providers under `base_rpc_policy` in both modes.

`src/network.rs` owns the process's immutable transport policy, canonical isolation
identities, reqwest pools and tonic channels. Direct and Tor use the same factory,
callers and payment/funding state machines. Cache keys include identity, endpoint,
timeout settings and the owning Tokio runtime. Sync releases its channels before
tearing down its private runtime. Cache eviction drops ownership without interrupting
in-flight handles. Each cache retains at most 256 entries.

| Caller | Transport and identity |
| --- | --- |
| `main.rs`, `deployment.rs`, `catalog.rs` | Spec fetch, discovery origin |
| `pricing.rs` | Startup-only unsigned GET probe, discovery origin; successes/failures remain cached |
| `catalog_state.rs` | Lazy help fetch, help URL discovery origin |
| `discovery/import.rs` | Agent OpenAPI fetch, discovery origin, public-destination policy |
| `payment.rs` | Actual unsigned challenge and paid retry, selected EVM payer |
| `rotation/base.rs` | Address reads by EVM owner; chain/header reads by RPC discovery origin |
| `rotation/near.rs` | Metadata by origin; quote and status by immutable request recipient |
| `treasury/birthday.rs` | Mainnet tip and capability lookup, per-invocation bootstrap identity |
| `treasury/mod.rs`, `server.rs`, `send.rs` | Scan, witnesses, proposal and capability checks, treasury UUID |
| `treasury/submission.rs` | Deposit/shielding broadcast and transaction lookup, durable destination EVM identity |
| `treasury/expiry.rs`, `refunds.rs` | Shared scans use treasury identity; job-specific lookup/submission uses durable recipient |
| `wallet_cli.rs` | Same configured adapters as serving; local inspection stays offline |
| `examples/snapshot_spec.rs`, `quote_near.rs` | Same factory; both accept `--network-config` |

`PreparedTransaction` resolves its recipient from funding job/recovery records or
an immutable `operation_network` binding. Refund shielding records its job's recipient
before calculation. Missing/conflicting identities cannot reach submission or lookup.
Schema version 10 adds that binding table; it does not rewrite wallet keys or signed
bytes. Pending shielding operations created without a recoverable recipient binding
fail closed and require operator investigation; do not guess from current pool roles.

## Paid discovery relay

An optional deployment `[discovery_relay]` uses source-assigned wallets by default for
Curl HTTP Request fallback; an explicit wallet can override this. Shared sources
choose one assigned profile deterministically. Its paid connection follows the immutable deployment
network policy and resolved payer identity; it never falls back to direct egress. Curl
fetches the target using its own network and supplies the response envelope.
Origin TLS and response integrity are consequently trusted to the relay. Target failures
do not disable other targets. Relay-service failures or cancellation disable further
relay calls across all wallets for the run. See
[configuration and cache provenance](configuration.md#paid-discovery-relay).

## Explicit direct discovery warming

`catalog warm --config FILE --source ID --direct` is a separate, unsigned-only
process using the direct network factory. It does not construct a serving
`Deployment`, payer or treasury. It writes separately identified cache entries
bound to the deployment's original network/isolation policy. A later Tor process
may use those entries while fresh, with a direct-origin warning, but cannot
revalidate their validators over Tor or automatically refresh them directly.
Ordinary direct cache data remains separated from Tor. This operator-selected
exception exposes discovery interests to direct egress; it grants no payment or
funding authority. Qualification rejects warming and bypasses persisted entries.
See [cache warming](configuration.md#warming-a-selected-source-including-directly-for-a-tor-deployment).

## Timeout budgets and concurrency

Omitting timeout settings selects defaults; it does not disable timeouts. Provider
files inherited with `extends` can override the network read default, including
for discovery. The Curl relay uses its own provider settings. Relay request
failures report a fixed reason, available HTTP status, elapsed time, possible
submission state, and its configured read/byte limits without upstream prose.
An uncertain paid failure still disables the relay for the run.

All outbound HTTP uses `read_timeout_seconds`, defaulting to 60 seconds in both
direct and Tor mode. Each successful response read renews the inactivity budget;
there is no total HTTP response deadline. This covers catalogs, lazy help, pricing,
imports, ordinary paid API responses and Curl relay responses, including large
binary bodies. Waiting for response headers is also subject to this budget.
Byte limits remain enforced, and partial documents are never published.

A provider/source or source-management `read_timeout_seconds` explicitly overrides
the network default. Omission inherits it. HTTP pools key on the effective idle
budget alongside identity, transport and public-destination policy.

Connection establishment remains separate: `connect_timeout_seconds` defaults
to 15 seconds direct and 120 seconds Tor. Both settings are available in either
mode. Connection values must be 1..300 seconds, and network read values must be
1..86400 seconds; the read budget need not exceed the connection budget. Inspection
reports the effective network values in both modes.

For compatibility, network `request_timeout_seconds` is an alias for
`read_timeout_seconds`, with its literal value now specifying inactivity. Provider
`timeout`, legacy pricing `probe_timeout`, import `fetch_timeout_seconds` and CLI
`--timeout` are accepted as legacy spellings. New configurations use only
`read_timeout_seconds` / `--read-timeout-seconds`. When old provider settings contain
both `timeout` and `probe_timeout`, the general `timeout` wins. Authored Tor examples
now specify 60 seconds; operator-owned files are not rewritten automatically.

Base read RPCs, complete chain views and receipt verification use connection and
read-inactivity timeouts, with no separate total deadline in either network mode.
Progressing reads can finish; actual transport failures retain one unsigned retry
and configured whole-view fallback. Block age, confirmations and canonical block
rechecks still determine whether evidence is usable.
Managed payment admission has no overall elapsed-time deadline, including while
waiting for its pool gate. Legacy wallet `wait_seconds` is accepted but ignored
with a warning and omitted from effective configuration.

Treasury gRPC uses the network connection and read-inactivity settings in both
modes. The injected transport measures each response independently, below protobuf
decoding: nonempty HTTP/2 DATA frames renew only that response's timer. Silent
headers and stalled bodies fail with fixed `grpc_headers_inactivity` and
`grpc_body_inactivity` diagnostics. Other streams and keepalives cannot renew them.
Accepted limitation: the header timer starts when the request body is first
polled, so it includes request upload. The current transport has no per-request
HTTP/2 upload-consumption hook; a slow, progressing upload can therefore exhaust
`read_timeout_seconds` before response headers arrive. Upload progress does not
renew this timer. A timeout does not prove that a transaction was not submitted
and does not authorize replay. No additional library is vendored to change this.
Waiting for channel-buffer/HTTP/2 stream capacity is separately bounded by the
read-inactivity setting and reports `grpc_dispatch_inactivity`. Connector attempts
retain their full establishment allowance before that readiness window expires;
the readiness guard ends at dispatch.
Cancelled/timed-out queued calls also cancel their request bodies, so a pending
HTTP/2 stream cannot upload stale RPC bytes when capacity returns later.
Message-size limits remain in force. Legacy library `grpc-timeout` metadata is
removed at this transport boundary; application total-RPC wrappers and sync's
complete-message timers are removed. No large substitute duration is used.
Upstream constructors outside the injected application path keep their own policy.
Quote and transaction expiry, confirmations, financial evidence freshness and
qualification authority remain independent of network progress. Submission failure
still retains unknown outcome and exact saved bytes; no retry authority is added.

Sync launch waits for an explicit first-poll acknowledgement, retaining the task
handle across a cancelled waiter. Loader and scan-worker retirement drains useful
work with periodic stage/elapsed diagnostics instead of aborting after ten seconds
or reporting an aborted worker as successful. Explicit owner cancellation remains
separate; the mempool's bounded drain policy is unchanged.

Stateless MCP HTTP responses use SSE framing with keepalive comments every two
seconds while awaiting the tool result. These keep the client response connection
active, not the remote provider's progress timer or payment authority. Client
whole-tool deadlines remain independent. The qualification reader bounds the
complete wire body and extracts the terminal JSON response for saved evidence.

The pinned rmcp service has a response drain of two seconds after cancellation or
five seconds after transport closure. Its `timed out draining in-flight responses`
warning is emitted during teardown, not as a deadline on an active stateless HTTP
call. Accepted application work survives that drain, including directory calls.
`mcp_response_cancelled` reports cancellation while work is outstanding;
`mcp_work_finished_after_cancellation` reports its eventual completion without
claiming successful response delivery. These warnings do not establish why the
caller disconnected or authorize replay of uncertain paid work.

An x402 challenge and its single signed attempt are separate requests, each
subject to connection and read-inactivity limits. There is no whole-tool-call
timeout imposed by payment admission. Quote/authorization expiry and funded-test
authority remain independent of progress. Swap delay is a health warning; graceful
shutdown has no automatic cutoff. A second signal explicitly exits the executable
without joining blocking scan/proving workers; it does not release liabilities or
run wallet cleanup. Library serving returns a typed `ForcedShutdown` error for
its embedding process to handle. A signed-request timeout still leaves durable unresolved exposure.

Managed pool gates protect chain verification, admission, signing and durable
journaling. Release the gate before signed HTTP submission; a seller delay cannot
serialize other requests sharing a wallet. Exposure stays reserved until confirmed
chain reconciliation. A slow Base RPC can still delay admission to that pool;
waiters remain cancellable, and separate pools remain independent.
Help coalesces requests for the same document without blocking other resources.
Required catalogs must succeed; explicitly optional remote failures are reported
as unavailable during serving. The rolling catalog-load limit defaults to 2
(configurable 1..64). Concurrent catalogs retain discovery-origin
credentials; paths on the same origin share an identity, different origins do not.
Compatible aliases coalesce downloads within the load. See
[the deferred independent provider startup plan](plans/deferred/independent_provider_startup.md) for independent
loading and background pricing. Frozen catalogs and disabled probes avoid those
startup dependencies in the live qualification.

## Stable credential encoding

The token is lowercase hex SHA-256 of `x402_treazury-tor-isolation-v1` followed by a
zero byte, a four-byte big-endian namespace byte length, the namespace's UTF-8
bytes, and the identity encoding. Each identity field begins with its four-byte
big-endian byte length, including the kind field. Fields are:

- EVM: `evm`, `eip155:8453`, normalized 20-byte address.
- Treasury: `treasury`, persisted treasury ID string bytes.
- Discovery: `discovery`, canonical URL scheme, canonical URL host, two-byte
  big-endian effective port. Path, userinfo and query are excluded.
- Bootstrap: `bootstrap`, 16 random UUID bytes for that invocation.

This byte format is pinned by `tests/network.rs`. It contains no signing or
storage-encryption secret. Changing it requires a new versioned domain separator.

## Dependency boundaries

- The pinned x402 exact clients sign locally. The upto client is constructed with
  provider type `()`, whose optional nonce/allowance lookups return no result without
  opening connections. Do not attach an SDK-owned RPC provider.
- LightClient is created/restored offline. The application installs a `GrpcIndexer`
  built from its own tonic channel through the reviewed connector patch. Existing
  pepper-sync/indexer clones retain that channel. SDK online constructors, URI setters,
  migration transmission and optional nym/price workers are not used.
- Sapling parameters are downloaded/embedded at build time. Runtime proposal code
  reads embedded parameters; it does not fetch them. Cargo/build traffic is outside
  the runtime policy.
- reqwest uses `no_proxy`, explicit `socks5h` when enabled, no redirects, and no
  automatic retries. Both reqwest and tonic SOCKS paths use hyper-util's authenticated
  SOCKS5 connector with remote DNS. Authentication negotiation cannot downgrade to
  unauthenticated SOCKS. HTTPS/gRPC retain ordinary TLS verification and original SNI.

`tests/network_audit.rs` rejects direct network constructors outside the factory.
The only application-source exception is the disposable consensus test harness,
which administers its local Docker nodes independently of the wallet under test.
This text scan is a tripwire, not proof about future dependencies; review this
inventory whenever dependencies or network features change.

## Verification

`tests/network.rs` and `tests/support/socks.rs` exercise authenticated SOCKS,
remote hostnames, no-auth downgrade rejection, failure/timeout without direct
fallback, pooling/isolation, cache eviction, stable tokens and policy inspection.
`tests/managed.rs` runs real signing, Base reconciliation, rotation and uncertain
payment recovery in a subprocess with Tor policy and hostile ambient proxy settings.
`rotation::near::tests` verifies metadata/quote/status isolation and recovery from a
serialized quote through the proxy. `tests/treasury_sync.rs` exercises the operator sync command over the proxy, including
restart. Existing suites cover static replacement, quote validation, no paid replay,
financial accounting, refunds and expiry recovery.

Run the complete local regression suite:

```sh
scripts/zcash.sh test --offline --all-targets -- --test-threads=1
python3 vendor/verify.py
python3 vendor/verify_zingo.py
python3 scripts/check_compat.py
```

Run the Ironwood consensus lifecycle through the fake SOCKS proxy, alone in its
process because network policy is immutable (requires Docker, no real funds):

```sh
scripts/zcash.sh test --offline --features zcash-regtest --lib \
  treasury::regtest::tor_consensus_lifecycle -- --ignored --exact --nocapture
```

For controlled real-Tor circuit observation and per-process egress qualification,
use the opt-in runner in [tests/TOR.md](../tests/TOR.md). It uses an independently
installed Tor binary; this repository does not bundle Tor. The macOS qualification
passed with real circuit observations and positive/negative egress controls; see
[the evidence and limits](testing.md#qualification-status).
Local SOCKS tests and funded-swap tests remain separate evidence.

For a deliberately enabled, read-only real-Tor HTTPS/gRPC smoke check:

```sh
TOR_SMOKE_SOCKS=127.0.0.1:9150 scripts/zcash.sh test --offline \
  --test network live_tor_unfunded_smoke -- --ignored --exact --nocapture
```

This contacts zec.rocks, checks mainnet identity and Ironwood tree support, and
creates no wallet or transaction. A SOCKS fixture proves credentials supplied by
our clients; verification of actual circuit separation additionally requires a
controlled Tor daemon with `IsolateSOCKSAuth` and observation via its control port.
The application itself never uses a control port or sends NEWNYM.

For enforcement beyond these application tests, run the executable under an OS
firewall or container egress rule permitting only the configured SOCKS endpoint,
while allowing the MCP listener's local inbound/reply traffic. Put Tor outside that
restricted process/container boundary so its relay connections remain possible.
Rules must cover IPv4 and IPv6 and reject destination DNS/UDP egress. Do not enable
such rules globally on a user's machine as part of an application test.

## Agent-selected destinations

Dynamic imports and paid API calls use the same factory with a `public_only` pool
key. HTTPS, no credentials/fragments, public literal addresses and public hostnames
are required. In direct mode, a custom resolver rejects the entire result if any
answer is non-public, and returns only that validated set to the connector. Cached
connections already target validated addresses; each new DNS lookup repeats the check.
Trusted TOML sources retain their existing local-fixture/private-provider behavior.

Tor uses authenticated `socks5h` with the same discovery/payer identities and never
resolves the destination locally. Hostname answers hidden behind Tor rely on Tor's
`ClientRejectInternalAddresses` protection. Keep it enabled; a generic SOCKS server
may not provide it. Optional dynamic `allowed_origins` narrows public HTTPS access.
Tests exercise the production resolver wrapper with injected DNS answers, including
mixed/rebound rejection, plus actual direct-client rejection before connecting to
private targets. A subprocess executes the production import and guarded x402
client over HTTPS through fake SOCKS, checking exact discovery/payer identities.
Private unit-test contexts can trust a dedicated fixture CA; public URL checks,
certificate/hostname verification and proxy policy remain active. HTTP/gRPC TLS
failures, proxy faults, redirects and interrupted reads have local regression tests.
These fixtures do not establish real Tor circuit selection or OS trust-store
compatibility. No alternative production HTTP constructor exists.

## Provider protocol policy

Provider HTTPS uses HTTP/2-only ALPN/client mode and a TLS 1.3 minimum by default.
Provider/source `allow_http1` and `allow_tls12` independently permit older versions,
without weakening trust, redirect, retry or proxy policy. Catalog, pricing, help
and paid requests carry the same source policy. Agent imports and dynamic paid
bindings use strict defaults. Infrastructure adapters retain their existing
policy; explicit cleartext HTTP remains unchanged. See the
[configuration guide](configuration.md#provider-http-and-tls-policy).

Pool keys include protocol permissions as well as origin, isolation identity,
timeout, runtime and public-destination policy. Catalog/pricing caches also
separate differing protocol permissions. HTTP/2 streams within a connection share
one SOCKS identity; they cannot cross wallet/discovery boundaries. INFO response
logs record HTTP version and request stage, without URLs or credentials. TLS
versions are enforced through minimum-version settings, not inferred from HTTP.

`src/network_http_tests.rs` uses trusted fixture TLS through authenticated SOCKS.
It holds four streams open on one connection, verifies another wallet and discovery
use separate connections, and checks HTTP/1.1-only and TLS-1.2-only servers reject
strict clients even after a compatible client has established a reusable connection.
No production trust changes, real Tor, live provider calls or payments are involved.

## Integration supervisor boundary

The opt-in Rust integration runner keeps local MCP and authenticated Tor-control
connections outside the confined application. `network::local_control` accepts
only literal loopback addresses with a bounded connection deadline.
`QualificationProbes` and `qualification_probe` implement positive/negative
loopback TCP/UDP controls for IPv4 and IPv6; they are supervisor test facilities,
not provider-configurable transports. External catalogs/help/API requests still
use the production factory inside confined subprocesses. See
[the integration runner](../tests/live/INTEGRATION_REFERENCE.md#owned-tor-qualification)
for its installed-Tor ownership, private evidence and explicit outage scope.

## Remaining timers and budgets

These scopes are independent; progress renews only transport inactivity, never
financial or qualification authority.

| Timer or bound | Purpose and behavior |
| --- | --- |
| Connection establishment: 15 seconds direct, 120 Tor by default | Bounds connection/TLS/SOCKS establishment, configurable 1..300 seconds. |
| HTTP/gRPC read inactivity: 60 seconds by default | Each response renews on its own data; no whole-download/RPC deadline. gRPC header inactivity starts at request-body dispatch and can abort a progressing upload (accepted limitation); initial connections and reconnects retain the connector's establishment limit. |
| gRPC dispatch readiness: read-inactivity setting | Bounds waiting for channel-buffer/HTTP/2 stream capacity before body dispatch (`grpc_dispatch_inactivity`). A connector attempt reserves its full connection allowance plus one readiness window until the first dispatch proves establishment. Sibling responses/keepalives do not renew readiness. This guard ends at dispatch and never caps a progressing response. |
| Legacy indexer duration arguments | Required by the vendor API; injected transport strips total `grpc-timeout`. Unused vendor online/quick-send constructors retain upstream policy. |
| Startup/drain ten-second messages, sync 30-second checkpoints | Observability/persistence cadence, never an abort deadline. Sync launch uses an owned acknowledgement; scan retirement drains without a cutoff. |
| MCP SSE keepalive cadence: two seconds | Sends connection-liveness comments while awaiting the result; never renews remote-read inactivity, financial authority, or client whole-tool deadlines. |
| MCP two/five-second library response drains | Bound response delivery only; application-owned accepted calls continue until drain or explicit force. |
| Base block age and treasury observation age/lag | Evidence freshness for new authority; slow historical reconciliation and scans acquire new admission/catch-up evidence. Base balance acquisition runs once and retains canonical evidence; the serialized store rejects stale admission without restarting the sweep. Old observations are never relabeled. |
| Quote deadline and Zcash transaction block expiry | External delivery/consensus constraints. Requested quote window defaults to two hours; the separate 300-second margin remains pending the provider contract. |
| Refund shielding local 24-hour deadline | Persisted authorization lifetime of that prepared operation, alongside consensus block expiry; never renewed during recovery. |
| x402 authorization validity | Signed payment authority and canonical expiry recovery, independent of HTTP progress. |
| Funding polling/backoff and swap delay threshold | Scheduling/health only. Delayed swaps continue reconciliation with a 60-second polling floor; no replacement send. |
| Import success retention 300 seconds / failure backoff one second | Applies after completion only; active coalesced fetches cannot age out. |
| Relay attempt backoff one second / three attempts per scoped target | Bounds proven unsigned failures. Possibly submitted failures stop relay spending instead of retrying. |
| Cache TTLs, warning suppression and database busy waits (2–5 seconds) | Storage/resource/diagnostic contention policy; cache persistence failure does not invalidate a complete response. |
| Optional cover episodes and three-second task cleanup | Bound optional unsigned work; cannot cut off the real paid call. |
| Local Tor control connection/probe limits (up to ten/two seconds) | Explicit diagnostic/qualification probes, not provider traffic. |
| Qualification deadlines, reservations and test-harness timeouts | Operator authority and bounded tests; normal network progress cannot extend them. |
| Sync mempool drain window | Returns control while scan workers remain, not a whole-sync deadline. |

Buffered response limits remain: default API 16 MiB, help 4 MiB and catalog 32 MiB,
plus explicit MCP, NEAR JSON, protobuf, image and cache limits. They bound memory
or stored data, not download duration. TLS/HTTP compatibility, authentication,
canonical proofs, amount caps and signed-work no-replay rules remain enforced.
