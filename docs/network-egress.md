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
callers and payment/funding state machines. Each HTTP cache slot holds an immutable
`HttpTransport` record pairing the pooled client with its canonical owner identity
and effective isolation scope, keyed by `SelectionPolicy` (`Legacy` versus
`Participating`), protocol compatibility, public-destination enforcement, owning
Tokio runtime, canonical identity, origin and read-inactivity timeout. Legacy and
participating lookups cannot consume or overwrite each other's cache entries.
Current production callers still acquire legacy deterministic transports; fresh
scope selection, conditional promotion and unsigned Tor circuit retries remain
pending activation as described in the
[Tor isolation retry plan](plans/tor_isolation_retries.md). Sync releases its
channels before tearing down its private runtime. Each cache retains at most 256
entries and evicts one least-recently-used entry when admitting another. Access
refreshes recency; interleaved wallet traversal does not flush frequently reused
provider clients. Eviction drops only cache ownership: acquired handles retain
connections and in-flight work without recreating the evicted cache entry. There
is no cache-maintenance timer or wallet-inventory limit.

Managed calls select an initial payer before provider I/O. Saved balances and
trusted price hints guide selection without proving affordability. Each attempt
obtains its own unsigned challenge and signs through the same wallet's transport
and cover owner. Original business arguments/body remain unchanged, including
supported POST requests. A recognized structured insufficient-funds response to a
managed signed request can traverse the captured eligible inventory once. Each
attempt retains its own authorization and possible charge. Other signed failures and
successful signed responses earn no repetition. Unsigned x402 POST follows the same
[accepted retry semantics](architecture.md#unsigned-x402-requests-and-post-retries)
as GET; HTTP method alone does not prohibit an authorized unsigned retry. See the
[best-effort wallet rules](wallet-rotation.md#best-effort-wallet-rules) and pinned public seller fixtures;
local fixture acceptance is not live-provider qualification.

Accepted calls retain their application owner through all attempts, signed
dispatch, response consumption and output conversion. A disconnected waiter cannot
orphan that work or free its financial liability. Ordinary advisory balance reads
run through the existing reconciliation owner, independently of foreground signing.
A failure during an active read schedules a follow-up; unrelated progress cannot
renew another response's inactivity timer or signed authority.

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
| `treasury/submission.rs` | Deposit/shielding broadcast and lookup, durable EVM recipient or owned transparent address scope |
| `treasury/expiry.rs`, `refunds.rs` | Shared scans use treasury identity; per-operation lookup/submission uses durable recipient/address scope |
| `wallet_cli.rs` | Same configured adapters as serving; local inspection stays offline |
| `examples/snapshot_spec.rs`, `quote_near.rs` | Same factory; both accept `--network-config` |

`PreparedTransaction` resolves its recipient from funding job/recovery records or
an immutable `operation_network` binding. Refund shielding records its job's recipient
before calculation. Automatic shielding without a job uses the encrypted original
preparation manifest: a jobless transparent input set, zero outgoing principal,
fee-only cost and no wall deadline derive a `zcash-transparent` isolation domain
from network, treasury UUID and canonical source address. It is stable across
restart and separate for each address. Existing EVM bindings retain precedence;
shared SDK scans still use the treasury identity. No new identity table or schema
conversion is needed. Missing/conflicting identities cannot reach submission or
lookup; invalid completed facts can still retain exact local bytes. Do not guess
identity from current pool roles.

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
budget alongside identity, transport and public-destination policy, preserving
the exact configured duration, including fractional milliseconds. Cover owners
use the same exact duration to keep distinct transport policies separate.

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
Quote and transaction expiry, confirmations, canonical coverage and
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

The serialized Store protects payer ownership and journal-before-disclosure.
Advisory Base reads and signed HTTP submission run outside that Store worker. Balance publication never
grants financial authority; slow seller responses do not serialize other calls.
The existing 128 foreground-owner permits are acquired before retained MCP tasks
are spawned and held through response-body conversion and result construction.
Nested managed/static/directory workflows share the actual task-local permit;
a retained lifetime token alone never bypasses admission. Disconnecting a waiter
does not release its permit. Excess work is rejected before provider I/O; after
owned work finishes the same capacity is available again. This bounds process
work and response memory, not spending or treasury capital.
Every signed authorization remains chargeable until its canonical outcome is known.
A separate reconciliation owner keeps bounded advisory wallet reads moving
independently; historical authorization reads are qualification-only. Cancellation
of a response waiter does not abandon its accepted pass.
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
| Treasury observation: 60-second minimum, errors back off to 300 seconds | Available treasury startup and explicit sync request one pass. Current enabled unfilled funding demand or validated pending source/shielding work owns recurring scans; idle treasury performs none. Shared scan timing is measured from attempt completion, so long later command work cannot postpone an already-due scan. Busy/locked read-only demand acquisition uses a 60-second local reevaluation wake while commands and notifications remain usable; unknown demand never authorizes a scan or resets observation timing. Scheduled source observers reuse healthy current-owner persisted scans without intervening tip probes; actual new-action validation still checks exact canonical height/hash and snapshot revision. Publication and SDK reconstruction have independent 60-second retries and never justify another scan alone. Owned shutdown retains identical completed-result publication retries without another calculation or submission. Scheduling/reuse never renews the original observation time or financial authority. Successful needed scans may start per-address owned-transparent shielding under current permission/fees. Removed age/lag gates remain removed; expiry/non-inclusion requires actual scanned coverage and signed protocol expiry remains authoritative. See [wallet observation policy](wallet-rotation.md#wallet-observation-policy). |
| Base resource ownership: 64 query owners, 32 active RPCs, 128 RPC callers | Bounds acquired query memory, active transports, and RPC work including callers awaiting transport capacity. Saturation rejects unsigned acquisition; no whole-work timeout or spending limit. Production owners acquire wallet and authorization pages of at most 64 rows each. Identical in-flight background balance reads share one retained query owner under these same bounds; completed results are not retained by that sharing layer and no local age policy is applied. |
| Managed payment owners: 128 | Bounds completed financial observations and private signing work retained while waiting for the group gate or signed dispatch. Saturation cannot sign or create a liability. |
| Advisory maintenance: four balance reads per group | One finite startup inventory pass, then explicitly requested refreshes only. Shared per-wallet acquisitions wait at least 60 seconds after completion, with independent error backoff up to 300 seconds; idle wallets are not swept. Zero success completes a request; failures and notifications arriving during reads retain pending work. Accepted scalar owners survive waiter cancellation. Indexed 64-row pages cover pending requests and the captured startup boundary. The five-second local scheduler/owner-restart cadence does not authorize Base reads during cooldown. Canonical pending authorization traversal remains qualification-only. See [wallet observation policy](wallet-rotation.md#wallet-observation-policy). |
| Managed payer selection | Indexed saved hints select among existing eligible identities, without RPC or historical repair. Insufficient-funds traversal captures the existing inventory and visits each identity once; it never allocates or waits for funding. |
| Runtime price hints: 24-hour receipt lifetime, 65,535 entries | Shares the API catalog disk-cache default lifetime. Bounded process memory for trusted operation/group/transport keys. Access and late validation never renew receipt age. Hints guide selection only; expiry or eviction triggers no HTTP, probe or authority renewal. See [local resource limits](local-resource-limits.md). |
| Quote deadline and Zcash transaction block expiry | External delivery/consensus constraints. Requested quote window defaults to two hours and is a preference; the returned deadline governs. The local 300-second margin and internal shielding wall expiry are removed. Consensus expiry remains inclusive at height N; zero has no expiry. The chain-specific 1Click arrival/confirmation/late-refund contract remains unresolved; broadcast acceptance is not settlement. |
| x402 authorization validity | Signed payment authority and canonical expiry recovery, independent of HTTP progress. |
| Payment challenge bodies | One collection under the provider's `max_response_bytes` (16 MiB default); direct low-level managed body-only challenges use the default. V1 body normalization adds no second cap or deadline. |
| Funding observation owner | Per store: four credit reads, four status reads, four prerequisite-complete unsigned quotes, one serialized source step and one pending quote prerequisite. Each of the four traversal lanes (credit, status, source and quote prerequisite) retains at most 64 acquired rows; independent cursors have no inventory-count cutoff. These bounds protect acquired work, not funding capital. |
| Funding scheduling metadata: 256 records per observation kind | At most 1,024 runtime records retain exact operation/kind due times and failure streaks. Active work and unelapsed backoff cannot be evicted; due inactive records may be forgotten. Reopen reconstructs missing work from committed facts without resetting transaction recovery attempts. |
| Explicit wallet bootstrap observation | One-second captured-claim polling and 60-second progress reporting; no total-work deadline. Ordinary serving has no bootstrap readiness poll or funded-payer startup gate. |
| Funding polling/backoff and swap delay threshold | One-second wake cadence and independent per-kind backoff up to 300 seconds. Recurring Base initial-credit reads begin only after durable source acceptance, with the first read when the recipient is due and a 60-second floor from the previous acquisition completion to the next read start. Preparation, startup, requested refresh and credit checks share the Store/wallet acquisition and its authoritative due time; credit scheduling adds no second backoff or jitter. Cached reuse neither republishes the scalar estimate nor counts a cached failure as a new recurring observation. Actual preparation requests one exact-destination discovery read after the SDK supplies a valid spendable proposal. SDK capital shortages or invalid proposals issue no destination read. Fresh allocations and unsigned quotes do not create Base polling. See [wallet observation policy](wallet-rotation.md#wallet-observation-policy). Recurring source and provider-status observations also have a 60-second floor after completion, preserving independent error backoff and jitter. Configured polling controls local inventory/owner wakeups, not permission for faster network observations; the health threshold reports delay without changing financial authority. Successful local quote→preparation→submission handoffs do not acquire a minute delay or clear earlier observation failures. These govern scheduling only; no whole-job timeout or replacement-send authority. A success in one observation kind cannot reset another kind's failure backoff or shorten the credit interval. A joined failed scheduler retries through its existing Store lease on the configured polling cadence; accepted source work retains that lease until it drains. Typed busy/locked inventory reads defer only their lane under the existing poll cadence, retaining its cursor and all active work; mutation/publication failures are not replayed by the inventory-read retry mechanism. Completed SDK results instead retain their existing owner, which retries identical local publication without recalculating or submitting. |
| Import success retention 300 seconds / failure backoff one second | Applies after completion only; active coalesced fetches cannot age out. |
| Dynamic import capacity and catalog bounds | Four concurrent imports process-wide, one per endpoint, 16 mutation jobs; saturation warns and rejects. Default endpoint dynamic-tool quota 512, exempt with `discover_on_demand`; no registration-count or per-source tool-count quota. [Agent sources](agent-sources.md#limits-and-network-policy) lists remaining document/registry bounds and WARN diagnostics. |
| Relay attempt backoff one second / three attempts per scoped target | Bounds actual proven pre-submission request failures. Read-only missing-payer waits consume no attempts. Possibly submitted failures stop relay spending instead of retrying. |
| Cache TTLs, warning suppression and database busy waits (2–5 seconds) | Storage/resource/diagnostic contention policy; cache persistence failure does not invalidate a complete response. |
| Optional cover episodes and three-second task cleanup | Bound optional unsigned work; cannot cut off the real paid call. |
| Local Tor control connection/probe limits (up to ten/two seconds) | Explicit diagnostic/qualification probes, not provider traffic. |
| Qualification deadlines, reservations and test-harness timeouts | Operator authority and bounded tests; normal network progress cannot extend them. |
| Sync mempool drain window | Returns control while scan workers remain, not a whole-sync deadline. |

Buffered response limits remain: default API 16 MiB, help 4 MiB and catalog 32 MiB,
plus explicit MCP, NEAR JSON, protobuf, image and cache limits. They bound memory
or stored data, not download duration. TLS/HTTP compatibility, authentication,
canonical proofs, amount caps and signed-work no-replay rules remain enforced.

### Source contract and upload limits

The 1Click [quote request contract](https://docs.near-intents.org/api-reference/oneclick/request-a-swap-quote)
and [returned Quote model](https://github.com/defuse-protocol/one-click-sdk-typescript/blob/main/src/models/Quote.ts)
distinguish the requested window from the returned deposit deadline. The SDK's
notification accelerates detection; notification or local broadcast acceptance
proves neither required confirmations nor timely settlement. No inspected public
contract establishes the `nep141:zec.omft.near` watcher's arrival event, compared
timestamp, confirmation policy or late-deposit/refund behavior.

Pinned investigation pointers remain Zodl iOS `6a7dc6b39cb8` (`Near1Click.swift` and
swap coordinator), Android `9f4e719e3eee` (`NearSwapDataSource.kt` and swap/status
use cases), Swift SDK `cf07eb228c1b`, Android SDK `ae538fc5b39f`, Near-One
`bridge-sdk-rs` `d1361ac1c68f`, `omni-bridge` `3f0349044938`,
`omni-bridge-services` `00ccb5d1dd89`, and
[btc-bridge deposit verification](https://github.com/Near-One/btc-bridge/blob/7740fae0ed8dc56571adfe2aaf36bd4edee0d441/contracts/satoshi-bridge/src/btc_light_client/deposit.rs).
Their UI heuristics and other bridge paths do not establish the deployed 1Click
Zcash route contract. No live spending or settlement qualification follows.

The gRPC response-header inactivity timer includes request upload because the
transport exposes no supported per-request upload-consumption signal. A progressing
upload may time out; sibling streams/keepalives cannot renew its timer. Uncertain
source submission retains exact bytes and selected inputs, without a paid retry.
