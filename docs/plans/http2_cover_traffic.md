# Optional HTTP/2 cover traffic

## Purpose and limits

Add operator-controlled, bounded cover traffic concurrent with provider API calls.
Use unsigned byte-range GETs of a provider's static public document to vary inbound
volume, optional randomized padding headers for outbound requests, and an
independently randomized HTTP/2 PING schedule. Keep
ordinary tools, payment authorization, wallet rotation and Tor isolation intact.
This is experimental traffic-analysis resistance, not a privacy guarantee.

The intended observer is outside the provider TLS connection, including an exit
or passive observer of that connection. The provider or terminating CDN can read
requests and distinguish cover streams. Other observers see different aggregation
and timing (including Tor cells); a local-body measurement is not their wire view.
Cover cannot remove bytes already sent, conceal arbitrary large responses within a
small budget, or hide call activity if it exists only during calls. Repeated
observations can average away simple independent noise. A distinctive cover policy
may itself fingerprint this client. Qualification must test those limitations.

Prioritize response-volume variation. Parameterized arrival and tail timing are
secondary; precise bandwidth shaping and continuous idle cover are outside the
first prototype. Never slow delivery of real results intentionally to hit a cover
target. Scheduler delays and backpressure do not guarantee remote sending times.

## Existing implementation and integration points

- `src/network.rs` owns all outbound constructors, authenticated remote-DNS SOCKS,
  TLS verification, identity-bound pools and `HttpPolicy`. Provider HTTPS defaults
  to HTTP/2 and TLS 1.3, with independent compatibility flags.
- `src/payment.rs` selects an identity before each unsigned challenge and retains
  it for the associated signed retry. Managed pre-signing promotion can restart
  this process with a different identity. Cover must follow these actual identities,
  not a mutable wallet-profile name.
- `src/catalog.rs`, `config.rs` and `deployment.rs` handle provider composition and
  resolved source bindings. Provider settings can be overridden by a source;
  nested tables currently replace in full. Preserve that rule.
- `src/limits.rs` and payment response reading enforce real-response bounds.
  Introduce incremental cover accounting without changing application payloads,
  binary/image handling, receipts or spend reservations.
- `src/network_http_tests.rs` proves concurrent streams can share one TLS-over-SOCKS
  connection and different wallets/discovery use different connections. Sharing a
  reqwest client is not a contractual guarantee that particular requests use one
  physical connection. Connection loss, pool eviction and server limits matter.

No cover runs for catalog startup, pricing, help, treasury/NEAR, Base RPC or
lightwalletd in this prototype. No cover runs when disabled: no capability probes,
extra connections, timers, RNG draws or worker tasks.

## Configuration and ownership

Add a process-level operator switch `network.cover_traffic_enabled = false` and an
optional provider/source `cover_traffic` table. Both must authorize activation.
Standalone uses its existing network-config mechanism for the process switch.
Agent-added sources cannot supply cover settings or select arbitrary cover URLs;
leave dynamic sources uncovered initially. Remote OpenAPI fields never authorize
traffic. Static source aliases may inherit the provider table normally.

Example of proposed provider settings, not validated production defaults:

```toml
[cover_traffic]
url = "https://api.example.com/openapi.json"
volume_mode = "target_total_body"
concurrency = 2
max_requests_per_episode = 8
max_cover_body_bytes_per_episode = 262144
max_episode_ms = 15000

[cover_traffic.volume]
distribution = "uniform"
min_bytes = 32768
max_bytes = 262144

[cover_traffic.ranges]
min_bytes = 4096
max_bytes = 65536

[cover_traffic.start_delay]
distribution = "uniform"
min_ms = 0
max_ms = 100

[cover_traffic.request_gap]
distribution = "uniform"
min_ms = 50
max_ms = 500

[cover_traffic.tail]
distribution = "uniform"
min_ms = 250
max_ms = 1500

[cover_traffic.request_padding]
enabled = false
header_name = "x-cover-padding"
max_header_value_bytes = 4096

[cover_traffic.request_padding.size]
distribution = "uniform"
min_bytes = 128
max_bytes = 2048

[cover_traffic.ping]
enabled = false
max_per_episode = 20
ack_timeout_ms = 5000

[cover_traffic.ping.interval]
distribution = "uniform"
min_ms = 250
max_ms = 1000
```

`concurrency` means simultaneous range streams, not total requests; initially allow
1..3. PING is connection-level and does not consume a request-stream slot. Expose
separate process-wide caps for active cover streams, active episodes, bytes and
requests per rolling time window, padding-header bytes, plus PING rate/count. Defaults must be finite;
choose conservative production values only after fixture and opt-in live trials.
Use a concrete bounded configuration in tests. Validate arithmetic, positive sizes,
finite floats, units, min/max ordering, impossible combinations and integer
conversion before any network or wallet access. `--show-config` reports effective
settings and inheritance; inspection never qualifies an endpoint or starts cover.

Start with uniform and truncated log-normal distributions for byte budgets, and
uniform and truncated exponential distributions for intervals. Define log-normal
parameters as median bytes and dimensionless log-space sigma; exponential as mean
milliseconds, always with explicit finite lower/upper bounds. Require positive
sigma/mean. Sample from the conditional distribution inside the bounds, not by
clamping unbounded draws (which creates spikes at boundaries). Use inverse-CDF or
a documented bounded sampler; an exhausted sampler reports an error rather than
spinning forever. Round once to integer units with tested boundary semantics.
Production uses an OS-seeded cryptographic RNG; injectable seeded RNG and virtual
time are test-only. No persisted RNG seed or identifier shared across wallets.

## Episode model and response-volume accounting

Schedule one episode per active connection/pool identity, not one independent
cover loop per tool invocation. Key ownership by runtime, origin, EVM isolation
identity, transport/fetch policy and compatible cover configuration. Merge
concurrent calls sharing that owner into its active episode. Different listeners
sharing a wallet therefore share a budget. Never merge wallets or discovery.
Different configurations resolving to the same owner must fail validation with a
clear conflict, or require separate explicit wallet assignments; do not silently
multiply cover loops or choose one configuration by arrival order.

An episode starts at the first real API request, samples its budget and timing decisions once, with the configured
concurrency bound, and registers subsequent overlapping calls. New calls
do not resample a target or reset the hard deadline. Once all real calls finish,
a bounded sampled tail may complete remaining cover. Results return to MCP as soon
as normal processing finishes; tail work is lifecycle-owned and may continue after
that return. Under sustained real traffic, reaching the episode cap disables cover
until the owner becomes idle; it does not immediately start unlimited new episodes.

Provide two explicit modes:

- `extra_body`: sample a cover-body budget independent of real response lengths.
  This is the simpler baseline and must be measured, not assumed protective.
- `target_total_body`: sample target T independently at episode start. Maintain
  real received body bytes R, cover received bytes C and reserved in-flight cover
  bytes Q. Schedule at most `max(0, T - R - C - Q)` additional cover, subject to
  hard budgets. Account unsigned challenges, signed responses and overlapping
  real calls within the episode, including errors. Do not count request bytes or
  PING ACKs as downloaded body. Real traffic can exceed T; report `target_exceeded`
  without truncating or delaying it. In-flight traffic can overshoot too.

Use bounded chunks and the sampled gap to replenish 1..3 rolling range slots.
Do not eagerly download the entire sampled target before the real response arrives.
Sample the first delay, then reassess received/reserved bytes as each slot completes.
At real completion, top up only the remaining deficit during the tail. If fewer
than the configured minimum range bytes remain, permit a final shorter range.
If bytes remain after the sampled tail or hard deadline, report `target_unmet`.
Tail duration is a maximum opportunity, not an obligation to keep sending after
the budget is met. This algorithm is a best-effort volume target, not a guarantee
that combined totals follow the configured distribution; report achieved results.

Count raw encoded body bytes where possible before application decoding, and
label all metrics with the layer measured. Cover requires identity encoding.
Real compressed bodies retain existing decoding behavior; instrument the transport
boundary if necessary to avoid mislabeling decoded bytes as wire bytes. HTTP/2
headers, framing, TLS records, TCP retransmissions and Tor overhead are separate.
Flow-control windows and already-buffered data mean cancellation cannot enforce
an exact network-byte ceiling. Enforce application byte limits, bound reservations,
report overrun observations, and measure actual encrypted traffic separately.

## Range endpoint qualification and request rules

Require an explicitly configured, unsigned, side-effect-free, same-origin HTTPS
resource. `openapi.json` is a candidate, not an automatically selected destination.
A spec on another hostname or a redirected CDN URL cannot share the API's TLS
connection and is ineligible. Do not add cache-busting queries, credentials,
payment headers, cookies or arbitrary headers. The explicitly validated padding
header below is the sole optional exception. Use only ordinary GET with a single
`Range: bytes=start-end` and `Accept-Encoding: identity`. No multi-range requests.

Lazily qualify on the first eligible API episode, under its existing wallet
identity, using a small bounded range request. Count qualification against that
episode and global budgets. Do not probe from discovery identity and then present
that as same-connection cover. Cache qualification per owner, with no polling.
A cold first call can be uncovered; expose that honestly. Qualification verifies
206, valid matching Content-Range, bounded known total representation length,
identity encoding and an exact-length body; Content-Length alone is insufficient.
Accept-Ranges alone is not proof. Retain a validator when present; if length or
validator changes, stop and invalidate qualification rather than probing in a loop.
A future episode may perform one bounded requalification after an explicit cooldown.

Select offsets uniformly over valid starts for the sampled length. A short
resource may need repeated ranges, subject to the same total request cap. Never
fall back to full-document downloads or a different endpoint automatically.
Validate every response, not just the qualification response. Handle 200 (ignored
Range), 416, redirects, 401/403/402, 429, malformed ranges, unexpected compression,
truncation and oversized bodies explicitly. Stop reading/cancel the cover stream
at the bound; do not close the shared TLS connection deliberately. Mark capability
unavailable/degraded with a reason and obey Retry-After where applicable, bounded
by a configured cooldown. Never sign a cover challenge or reuse payment middleware.

A server may ignore Range and bytes can arrive before cancellation. Keep HTTP/2
receive buffering bounded and qualify adverse fixtures; do not promise that the
requested range itself caps network traffic. Do not alter real-call flow control
just to improve cover performance without separate validation.

## Outbound request padding headers

Optionally add a provider-approved extension header whose contents are irrelevant
to the API. Servers are not required to ignore arbitrary headers: a vendor, CDN or
WAF may reject them, enforce smaller header limits, or incorporate them into
application behavior. Do not auto-enable padding because one ordinary request
succeeds. Provider configuration explicitly authorizes the header name, maximum
size and affected requests. Start with one field rather than many randomized field
names, which increases header count and fingerprinting. The example name is a
placeholder, not a recommended universal identifier.

Apply this to real API requests and optionally cover GETs, with separate settings
for each; never add it to treasury, RPC, catalog, pricing or help requests. Protect
all existing routing, authentication, framing and payment fields. Reject standard
or reserved names, pseudo-headers, duplicate names, CR/LF, invalid values, and any
attempt to replace Host/authority, Content-Length, Transfer-Encoding, Authorization,
Cookie, Range, content negotiation or x402 headers. Only a validated extension
name is permitted. The padding contains no wallet identifier, request arguments,
secrets or persistent marker.

Initially sample the additional header-value byte count, independently of actual
request size, using the same bounded uniform/truncated-log-normal sampler. Generate
fresh cryptographically random ASCII contents of that length; keep name/value and
total header-list limits separate from HTTP/2 maximum frame size. Respect the
server's advertised header-list limit when available through the transport adapter;
otherwise use a conservative configured budget. Report skipped/reduced padding and
cap exhaustion explicitly. Extra bytes cannot shrink an already large request.

HTTP/2 HPACK changes the encoded size. Mark the value sensitive/never-indexed,
verify this on the wire in fixtures, and avoid assuming repetitive zero/space padding retains its literal byte size. Never-indexed does not disable Huffman encoding; random ASCII
still has encoding overhead/variation. Measure encoded HEADERS/CONTINUATION and TLS
bytes before offering a target-total-request-size mode. Do not claim that a sampled
2 KiB value creates exactly 2 KiB of encrypted cover. The peer can still read it;
this does not protect against the provider. Never log padding contents.

Attach headers only in transport request construction, without changing JSON bodies,
URLs, payment requirements, signature-covered data or authorization handling. Sample
once per physical application request (unsigned challenge versus its signed retry
are distinct requests); retries prohibited by existing financial rules remain
prohibited. Pin already constructed signed-request padding along with its request.
A 431, rejected header or connection failure must not cause a signed request to be
resent without padding. Disable padding for later independent calls, expose the
reason, and preserve the original financial outcome. Qualification must include
successful challenge/sign/retry fixtures with header padding and unchanged receipts.

## Same-connection transport and PING

First measure a range-only prototype with reqwest, recording connection reuse in
controlled fixtures. Label it pooled/best-effort until physical connection identity
is observable in production. Same SOCKS credentials alone prove neither same TLS
connection nor same Tor stream. Distinct TLS connections can be separated by the
intended observer and do not satisfy the strong same-channel objective.

Before claiming same-channel cover or implementing randomized PINGs, add a
connection-scoped transport handle behind the existing network factory. Investigate
an instrumented Hyper/h2 connector or a narrowly maintained dependency extension.
The handle must expose connection identity, negotiated protocol, lifetime,
connection-scoped PING scheduling and a way to bind cover to the real connection.
Avoid a separate cover-only TLS client. Keep signing and request/response semantics
in the existing payment path. If reqwest cannot expose the needed controls, write
and review the concrete adapter design before broad transport replacement; do not
silently approximate randomized PING with fixed keepalive intervals.

The adapter is shared by enabled and disabled operation and must preserve runtime
ownership, TLS/SOCKS validation, public-destination restrictions, all pool keys,
redirect refusal, timeouts and disabled automatic retries. Existing HTTP/1.1
compatibility remains usable for real calls; report cover unavailable when the
actual connection is not HTTP/2. No paid replay after GOAWAY, resets or connection
loss. Cover ceases on a lost connection; only a subsequent ordinary real request
may establish another covered connection. It must not reconnect a signed request.

PING is optional timing noise, not response-volume padding. Use standard PING/ACK,
not invented payload sizes or raw-byte injection into a TLS socket. Keep at most
one cover PING outstanding per connection. Sample the next interval after ACK;
record the difference between scheduled and actual sends. Disable cover PINGs on
ACK timeout without deliberately terminating real streams. Rate-limit and count
PINGs independently of range downloads, coalescing with any necessary transport
liveness mechanism. Servers may enforce stricter PING policies or close a connection;
report that outcome and never retry a paid request because cover caused a failure.

## Resource, isolation and lifecycle invariants

Real calls take precedence over cover in local scheduling. HTTP/2 server stream
limits may be below our configured concurrency; skip or delay cover instead of
blocking payment admission. Cover cannot reserve USDC, create wallets, trigger
rotation/refills, hold wallet/store locks, or extend signed-payment drain deadlines.

Bind each episode to the actual EVM address captured for that attempt. On
pre-signing promotion, stop scheduling old-identity cover and attach the new
attempt to its own episode. Existing old calls retain their old connection. A
bounded tail belongs only to that old identity; never move it onto the new wallet.
An ephemeral wallet reused across providers still has separate origin connections.
Do not keep old connections alive beyond the bounded episode for cover purposes.

Track all tasks in application lifecycle ownership, with cancellation and joins.
Client/MCP cancellation, shutdown, provider failures, stream resets and exhausted
budgets stop new cover work; signed real calls keep existing safety/drain semantics.
Cancelling a cover stream must not cancel other streams. Cover failures degrade
cover rather than failing a successful API result. Shutdown logs distinguish
abandoning optional cover from deliberately draining financial work.

## Visibility

Emit sanitized start/end summaries with configured source ID, mode, sampled target,
actual real/cover body bytes, range/PING counts, elapsed time, concurrency high-water
mark, and completion/degradation reason. Never log wallet addresses, SOCKS tokens,
full URLs, headers, contents or production RNG state. Shared aliases can be named
only in operator logs, respecting listener scope in any agent-facing information.

Every exhausted budget, disabled capability or incomplete target produces explicit
evidence. Rate-limit duplicate warnings by aggregating counts and emitting a final
summary, not by silently discarding them. Since tails can finish after the tool
result, expose additive bounded transport-status metadata on later tool responses
or a scoped status tool; choose one consistent with existing MCP result schemas
in milestone 1. Known degradation at response time should be visible immediately.
Status reports must not imply successful obfuscation merely because bytes were sent.
Do not alter vendor payloads, tool input schemas or execution permissions.

## Implementation milestones and tests

Each milestone is a separately tested commit. No paid live tests are necessary
until the existing automated live-integration harness is ready.

1. **Configuration and pure scheduler.** Implement strict composition, offline
   inspection, global gates, owner-conflict validation, distribution sampling,
   incremental accounting and agent-visible status design. Use seeded tests and
   virtual time for exact bounds, invalid/non-finite values, overflow, target
   below real volume, reservations, concurrency merging and sustained traffic.
2. **Unsigned range qualification.** Implement same-origin validation and a bounded
   reader, with fixtures for all status/header/length/compression failures above.
   Assert zero payment headers/signatures, no redirects, no cache-busting, bounded
   requests, capability caching/cooldown and no disabled-mode traffic.
3. **Request padding and range-only episodes.** Add validated extension headers,
   independent size distributions, never-indexed handling and byte-budget accounting.
   Test protected/invalid header names, header-count/list limits, HPACK behavior,
   431/WAF-style rejection, no secret/identifier leakage and no signed replay.
   Integrate real response accounting, pooled transport,
   concurrent calls across listeners, identity promotion, independent source
   policies and lifecycle cancellation. Prove real results return before tails;
   global/per-owner budgets hold; failed cover never retries paid calls. Use
   real TLS-over-SOCKS fixtures to measure same-connection behavior and account
   for actual HTTP/2 stream limits. Do not claim exact affinity yet.
4. **Connection-scoped transport and randomized PING.** Review the adapter design,
   then prove connection affinity, protocol fallback behavior, bounded scheduling,
   one outstanding PING, ACK timeouts, GOAWAY/reset handling and unchanged signing.
   Assert disabled cover uses the same transport code with zero extra traffic.
   Reject experimental PING configuration explicitly until this capability exists.
5. **Qualification and operator documentation.** Run default Zcash regressions,
   payment/MCP/isolation suites, Clippy, formatting and constructor audit. Add an
   opt-in unsigned experiment driver with fixed fixture response sizes/timings,
   repeated samples and reproducible test-only seeds. Compare no cover, ranges,
   PINGs, padding headers, and combinations at concurrency 1/2/3 and equal overhead
   budgets. Assess upload and download inference separately. Document known
   limitations and migrate implemented behavior to maintained docs.

Measure encrypted byte counts in each direction, packet/TLS-record timing where
observable, body-layer counts, encoded header sizes and HPACK indexing, number of physical connections, peak memory,
real-call latency, request amplification, failures, and target attainment. Test
small/large/unknown-length/compressed/error responses and parallel mixed endpoints.
Use held-out repeated traces to assess response-size inference and endpoint
classification; increased variance alone is not evidence of reduced inference.
Retain failed/degraded samples. Separate observers of a provider connection from
observers of the whole client-to-Tor link.

Optional real-Tor qualification uses explicit approved static resources and finite
budgets, preserves isolation and Tor state, and observes circuit-learning stability
as in `tests/STARTUP.md`. Never use routine startup pricing probes as cover. Live
x402 qualification later verifies receipts/debits and double-buffered rotation
under cover via [the integration plan](live_integration_testing.md).

## References

- [HTTP range semantics, RFC 9110 section 14](https://www.rfc-editor.org/rfc/rfc9110.html#section-14): partial responses and ignored/unsatisfiable ranges.
- [HTTP/2 PING, RFC 9113 section 6.7](https://www.rfc-editor.org/rfc/rfc9113.html#section-6.7): connection-level fixed eight-octet payload and ACK.
- [HTTP/2 padding/privacy discussion](https://www.rfc-editor.org/rfc/rfc9113.html#section-10.7): limitations of padding and observable client behavior.
- [Reqwest 0.13.5 client controls](https://docs.rs/reqwest/0.13.5/reqwest/struct.ClientBuilder.html): fixed keepalive interval, not arbitrary randomized frame control.
- [h2 PingPong interface](https://docs.rs/h2/latest/h2/struct.PingPong.html): lower-level connection PING access requiring integration with the actual connection owner.

- [HPACK literal encoding and never-indexed fields](https://www.rfc-editor.org/rfc/rfc7541.html#section-6.2.3): avoiding dynamic indexing does not disable static Huffman encoding.
