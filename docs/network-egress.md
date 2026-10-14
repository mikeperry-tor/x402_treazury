# Runtime egress inventory

See [Wallet selection and network isolation](architecture.md#wallet-selection-and-network-isolation)
for the selected pooling, cover-state and isolated Tor reuse tradeoffs.

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
tearing down its private runtime. Each cache retains at most 256 entries and evicts
one least-recently-used entry when admitting another. Access refreshes recency;
interleaved wallet traversal does not flush frequently reused provider clients.
Eviction drops only cache ownership: acquired handles retain connections and
in-flight work. Isolation, origin, runtime and transport keys remain unchanged;
there is no cache-maintenance timer or wallet-inventory limit.

Managed calls select the initial payer before provider I/O. A trusted price hint
can choose a cached sufficient wallet; it never supplies a payment challenge.
After a validated unpaid challenge, an insufficient payer may make one unsigned
handoff using the replacement wallet's own provider transport and cover owner.
The original business request is preserved for every supported method, including
POST. The replacement obtains its own challenge and remains fixed for the second
attempt. There is no handoff or replay after a signed submission. The
[admission contract](wallet-rotation.md#capacity-and-admission) and pinned local
middleware fixtures describe the supported boundary; this is not live-provider
qualification for arbitrary middleware.

Accepted calls retain their application owner through signed dispatch, response
consumption and output conversion. A disconnected waiter cannot orphan that work
or free its financial liability. Unsigned candidate acquisition has separate,
cancellable read ownership; completion of unrelated reads cannot renew a balance
observation or a signed authority deadline.

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

The reviewed reqwest patch starts the HTTP/1 and HTTP/2 header timer only after
that request acquires its connection, including DNS, SOCKS and TLS. No sibling
request or keepalive can start or renew it. Stream-capacity waiting and request
upload remain inside the post-connection header allowance (the same upload
limitation documented for gRPC below); there is no whole-response deadline.
Body inactivity renews only on nonempty DATA, and available buffered data is
consumed before checking elapsed time after consumer backpressure. Hyper's
separate stale-pool automatic retry is disabled too: an unsigned caller may use
its existing finite retry policy, but the transport never silently reconnects
and resends a signed request. Public destinations, remote DNS, TLS verification,
identity-bound pooling and ordinary error classification remain unchanged.


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
rechecks still determine whether evidence is usable. Base RPC response collection
has a fixed 16 MiB byte bound to protect memory before JSON decoding, including
chunked and lengthless bodies. Oversized responses produce a redacted typed failure
without retrying that endpoint. Configured fallback discards the partial view and
reacquires all evidence; it cannot reuse a balance from the rejected response.
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
The reviewed Hyper/h2 patches additionally observe pending-open cancellation and
discard unopened streams without transmitting HEADERS or RST_STREAM. This protects
HTTP payment headers as well as gRPC bodies when peer capacity returns after a
timeout or cancellation. Already transmitted requests retain unknown outcomes;
no cancellation or reset releases financial exposure or grants retry authority.
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
whole-tool deadlines remain independent. Keepalives start when the SSE response
opens; the pinned SDK's per-request protocol-negotiation path still waits for its
first protocol message before opening the stream. The installed oMLX initialize
path is covered by the slow-call interoperability experiment. The qualification
reader bounds the complete wire body and extracts the terminal JSON response for saved evidence.

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

Managed pool gates protect evidence publication, admission, signing and durable
journaling. Base RPC acquisition and signed HTTP submission run outside the gate.
Publication rechecks each wallet's captured evidence and shared endpoint generation;
a completed RPC alone cannot grant authority. A seller delay cannot serialize other
requests sharing a wallet.
The existing 128 foreground-owner permits are acquired before retained MCP tasks
are spawned and held through response-body conversion and result construction.
Nested managed/static/directory workflows share the actual task-local permit;
a retained lifetime token alone never bypasses admission. Disconnecting a waiter
does not release its permit. Excess work is rejected before provider I/O; after
owned work finishes the same capacity is available again. This bounds process
work and response memory, not spending or treasury capital.
Exposure stays reserved until confirmed chain reconciliation.
A separate reconciliation owner keeps bounded wallet and authorization reads moving
independently; cancellation of a response waiter does not abandon its accepted pass.
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

`source_url_info` makes unsigned HEAD requests and bounded GET validations for a
fixed, deduplicated set of specification/directory/LLM-documentation candidates.
At most 24 candidates and four concurrent requests limit speculative network traffic
and simultaneous document buffers. It uses the source-management wallet's existing
isolation identity, pinned for the batch, with public destination/origin checks.
Managed identity selection is read-only; no funding or allocation occurs. Only a
slug's directory lookup uses ordinary payment handling. Provider probes never sign,
never follow redirects and use existing spec/help byte and response-inactivity limits.


Dynamic imports and paid API calls, including source-management `raw_get` and
`raw_post`, use the same factory with a `public_only` pool
key. HTTPS, no credentials/fragments, public literal addresses and public hostnames
are required. In direct mode, a custom resolver rejects the entire result if any
answer is non-public, and returns only that validated set to the connector. Cached
connections already target validated addresses; each new DNS lookup repeats the check.
Trusted TOML sources retain their existing local-fixture/private-provider behavior.

Tor uses authenticated `socks5h` with the same discovery/payer identities and never
resolves the destination locally. Hostname answers hidden behind Tor rely on Tor's
`ClientRejectInternalAddresses` protection. Keep it enabled; a generic SOCKS server
may not provide it. Optional dynamic `allowed_origins` narrows public HTTPS access,
including direct raw calls. Raw calls use the resolved source-management payer and
existing response/read limits; redirects are not followed.
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
| Base block age and treasury observation age/lag | Treasury command sync can reuse only the same successfully scanned in-process wallet revision, still within its original observation age, after checking network and exact tip height/hash. Restart, mutation, missing hash, changed tip or expired evidence requires scanning. Reuse never refreshes timestamps; explicit sync and expiry recovery still scan. Evidence freshness for new authority; slow historical reconciliation and scans acquire new admission/catch-up evidence. Base balance acquisition runs once and retains canonical evidence; the serialized store rejects stale admission without restarting the sweep. Old observations are never relabeled. |
| Base resource ownership: 64 query owners, 32 active RPCs, 128 RPC callers | Bounds acquired query memory, active transports, and RPC work including callers awaiting transport capacity. Saturation rejects unsigned acquisition; no whole-work timeout or spending limit. Production owners acquire wallet and authorization pages of at most 64 rows each. Identical in-flight background balance reads share one retained query owner under these same bounds; completed results are not retained by that sharing layer and their freshness is never renewed. |
| Managed payment owners: 128 | Bounds completed financial observations and private signing work retained while waiting for the group gate or signed dispatch. Saturation cannot sign or create a liability. |
| Reconciliation: four balance reads and four history reads per group | Separate bounded rolling work permits balance refresh while historical nonce reads progress. Five-second cadence and per-authorization failure backoff up to 300 seconds control polling only. At most 256 retry records per group bound scheduling memory; saturation defers new observations until an inactive record is due, without evicting active work or unelapsed backoff; captured keyset passes have no inventory-count or elapsed-time cutoff. |
| Candidate acquisition: four reads, 64-row keyset pages | A captured inventory boundary prevents arrivals from extending a pass. Bound payer and ready reserves get priority while a traversal lane remains available. Sufficient cached evidence can finish before unrelated reads; group-scoped publication notifications wake waiting passes to recheck evidence and liabilities without new acquisition. Caller cancellation aborts only its unsigned acquisitions. |
| Runtime price hints: 300-second receipt lifetime, 512 entries | Bounded process memory for trusted operation/group/transport keys. Access and late validation never renew receipt age. Hints guide selection only; expiry or eviction triggers no HTTP, probe or authority renewal. |
| Quote deadline and Zcash transaction block expiry | External delivery/consensus constraints. Requested quote window defaults to two hours; the separate 300-second margin remains pending the provider contract. |
| Refund shielding local 24-hour deadline | Persisted authorization lifetime of that prepared operation, alongside consensus block expiry; never renewed during recovery. |
| x402 authorization validity | Signed payment authority and canonical expiry recovery, independent of HTTP progress. |
| Funding observation owner | Per store: four credit reads, four status reads, four prerequisite-complete unsigned quotes, one serialized source step and one pending quote prerequisite. Each of the four traversal lanes (credit, status, source and quote prerequisite) retains at most 64 acquired rows; independent cursors have no inventory-count cutoff. These bounds protect acquired work, not funding capital. |
| Funding scheduling metadata: 256 records per observation kind | At most 1,024 runtime records retain exact operation/kind due times and failure streaks. Active work and unelapsed backoff cannot be evicted; due inactive records may be forgotten. Reopen reconstructs missing work from committed facts without resetting transaction recovery attempts. |
| Funding polling/backoff and swap delay threshold | One-second wake cadence, independent per-kind backoff up to 300 seconds, and a 60-second polling floor after the health threshold. These govern scheduling only; no whole-job timeout or replacement-send authority. A success in one observation kind cannot reset another kind's failure backoff. |
| Import success retention 300 seconds / failure backoff one second | Applies after completion only; active coalesced fetches cannot age out. |
| Relay attempt backoff one second / three attempts per scoped target | Bounds proven unsigned failures. Possibly submitted failures stop relay spending instead of retrying. |
| Cache TTLs, warning suppression and database busy waits (2–5 seconds) | Storage/resource/diagnostic contention policy; cache persistence failure does not invalidate a complete response. |
| Optional cover episodes and three-second task cleanup | Bound optional unsigned work; cannot cut off the real paid call. |
| Local Tor control connection/probe limits (up to ten/two seconds) | Explicit diagnostic/qualification probes, not provider traffic. |
| Qualification deadlines, reservations and test-harness timeouts | Operator authority and bounded tests; normal network progress cannot extend them. |
| Treasury background sync cadence | Existing half-max-age cadence (1–60 seconds) is scheduled from the last successfully persisted scan completion, including command-triggered scans. Reuse, failed/cancelled scans and unrelated commands do not reset it. Long post-scan command work cannot postpone an already-due tick. This scheduling clock never renews the original sync observation or financial authority. |
| Sync mempool drain window | Returns control while scan workers remain, not a whole-sync deadline. |

Buffered response limits remain: default API 16 MiB, help 4 MiB and catalog 32 MiB,
plus explicit MCP, NEAR JSON, protobuf, image and cache limits. They bound memory
or stored data, not download duration. TLS/HTTP compatibility, authentication,
canonical proofs, amount caps and signed-work no-replay rules remain enforced.
