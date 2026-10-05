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

## MVP boundary

Deliver a range-and-header-padding, pooled/best-effort implementation first. Its purpose is to
establish bounded concurrent cover, correct isolation and unchanged payment/MCP
behavior before adding more transport controls. It does not claim a privacy
improvement or guaranteed same-TLS-channel cover.

The MVP includes:

- Explicit process and provider/source opt-in; disabled by default.
- Same-origin unsigned range qualification, cached for the owner lifetime.
- `extra_body` mode with bounded uniform, exponential, Weibull, log-normal and
  weighted-discrete distributions for byte sizes and timing.
- Independently optional, provider-approved request-padding headers, with separate
  outbound budgets and verified HPACK never-indexed handling.
- One to three rolling cover requests, ordinary response draining, and randomized
  request start/gap/tail timing. Timing controls schedule requests; they do not
  intentionally withhold HTTP/2 window credit or pause response bodies.
- Shared-owner episode accounting, hard episode/process limits, cancellation and
  payment/isolation regression tests.
- Explicit operator and scoped agent-visible degradation evidence, an unsigned
  experiment driver and real-Tor qualification without wallet spending.

`target_total_body`, correlated/trace-driven traffic models, connection-pinned
transport, randomized PINGs and flow-control pacing are follow-ups.
Reject their settings explicitly in the MVP, even in otherwise disabled config;
never accept an unsupported option and silently ignore it. Existing HTTP/1
compatibility remains available for real API calls, but cannot qualify HTTP/2 cover.

The full design below retains those follow-up requirements. Only the five MVP
milestones are prerequisites for a first usable implementation. Connection affinity
must be established before any later feature claims guaranteed same-channel cover.

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

The integration runner already supports registry-bound unsigned MCP execution,
owned Tor, macOS confinement and a planned help-cache outage. Reuse its ownership,
identity audit and evidence format; see [the runner guide](../../tests/live/INTEGRATION.md).
Managed/funded execution remains later work in [the integration plan](live_integration_testing.md).

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

Proposed MVP configuration, not yet accepted by the executable. These finite
values are provisional engineering defaults, not a measured privacy preset:

```toml
# Deployment network configuration
[network]
cover_traffic_enabled = true

[network.cover_limits]
max_active_episodes = 8
max_active_streams = 6
window_ms = 60000
max_requests_per_window = 64
max_cover_body_bytes_per_window = 4194304
max_padding_value_bytes_per_window = 65536

# Provider/source configuration (normally in a separate provider file)
[cover_traffic]
url = "https://api.example.com/openapi.json"
volume_mode = "extra_body"
concurrency = 2
max_requests_per_episode = 8
max_cover_body_bytes_per_episode = 65536
max_episode_ms = 10000
qualification_range_bytes = 1024
max_resource_bytes = 16777216

[cover_traffic.volume]
distribution = "log_normal"
median_bytes = 32768
sigma = 0.65
min_bytes = 16384
max_bytes = 65536

[cover_traffic.ranges]
distribution = "uniform"
min_bytes = 4096
max_bytes = 16384

[cover_traffic.start_delay]
distribution = "uniform"
min_ms = 0
max_ms = 100

[cover_traffic.request_gap]
distribution = "weibull"
scale_ms = 100.0
shape = 0.8
min_ms = 50
max_ms = 250

[cover_traffic.tail]
distribution = "uniform"
min_ms = 100
max_ms = 500

# Enable only after this provider explicitly permits this extension header.
[cover_traffic.padding]
header_name = "x-example-padding"
on_api_requests = true
on_cover_requests = false
max_value_bytes_per_request = 1024
max_value_bytes_per_episode = 8192
max_total_header_list_bytes = 8192

[cover_traffic.padding.size]
distribution = "log_normal"
median_bytes = 256
sigma = 0.7
min_bytes = 32
max_bytes = 1024
```

Use monotonic rolling-window accounting with atomic reservations across owners.
Count qualification in both request and byte budgets. A reserved request counts
once dispatched even if it fails; release only unused body allowance, not bytes
already received. In-flight byte reservations count against the global ceiling.
Keep the limiter's retained request history bounded by its configured admission
limit; expiration is not permission to forget still-outstanding reservations.
Refuse or skip optional cover when capacity is unavailable, without queuing real
calls or resetting the episode's deadline. Emit the specific limiting reason.

`concurrency` means simultaneous range streams, not total requests; initially allow
1..3. PING is connection-level and does not consume a request-stream slot. Expose
separate process-wide caps for active cover streams, active episodes, bytes and
requests per rolling time window. Padding has independent per-request, episode
and rolling-window byte limits in the MVP; add independent PING rate/count limits
when PING is implemented. Defaults must be finite;
review the provisional values above against fixture and opt-in live results.
Use a concrete bounded configuration in tests. Validate arithmetic, positive sizes,
finite floats, units, min/max ordering, impossible combinations and integer
conversion before any network or wallet access. `--show-config` reports effective
settings and inheritance; inspection never qualifies an endpoint or starts cover.

### Distribution choices and sampling contract

Network measurements do not support one universal packet-arrival or length law.
[Paxson and Floyd](https://web.stanford.edu/class/cs244/papers/paxson1995.pdf)
found exponential packet interarrivals missed burstiness in their measured traffic.
An [interarrival-model study](https://ir.canterbury.ac.nz/bitstreams/681998f5-30a2-46e7-a006-20ac74c3d203/download)
compares exponential, Weibull and log-normal candidates. These motivate flexible
experiments, not a claim that a particular distribution reproduces current HTTPS.

Length measurements also depend on layer: historical
[CAIDA packet traces](https://www.caida.org/catalog/papers/1998_inet98/Inet98.pdf)
show several size peaks related to protocol overhead and MTU, whereas a
[longitudinal study of aggregate traffic volumes](https://arxiv.org/abs/2007.10150)
finds log-normal fits. Aggregate volumes are not individual packet or API response
lengths. Use log-normal for skewed application-byte budgets and weighted discrete
choices for multiple size modes; do not copy historical packet-size frequencies
into a claimed modern HTTP/2/Tor profile.

Support these five distributions in the MVP for volume, range size, padding size,
start delay, request gap and tail. Each site samples independently; no recursive
mixture configuration or correlation model is included.

| Config name | Parameters beyond explicit min/max | Intended use | Sampling route |
| --- | --- | --- | --- |
| `uniform` | None | Baseline/control; bounded sizes or jitter | Integer uniform from `rand` |
| `exponential` | Positive `mean_ms` or `mean_bytes` | Memoryless timing baseline; skewed sizes if requested | Closed-form inverse CDF; `rand_distr::Exp` also exists |
| `weibull` | Positive `scale_ms` or `scale_bytes`, positive dimensionless `shape` | Flexible gap shape; shape 1 equals exponential, below 1 favors short gaps with a longer tail | Closed-form inverse CDF; `rand_distr::Weibull` also exists |
| `log_normal` | Positive `median_ms` or `median_bytes`, positive dimensionless log-space `sigma` | Skewed volume/header/range budgets and optional long-tailed delays | `rand_distr::LogNormal::new(ln(median), sigma)` plus bounded rejection |
| `weighted_discrete` | `values_ms` or `values_bytes`, matching positive integer `weights` | Explicit multiple size peaks or a small empirical timing histogram | `rand::distr::weighted::WeightedIndex` over integer weights |

The [Rust distributions library](https://docs.rs/rand_distr/latest/rand_distr/)
provides Exp, Weibull and LogNormal; [WeightedIndex](https://docs.rs/rand/latest/rand/distr/weighted/struct.WeightedIndex.html)
provides categorical sampling. Select compatible `rand`/`rand_distr` versions under
repository dependency policy and commit the lockfile during C1; do not add a full
statistics framework just to generate these samples. Constructor parameters are
those of the untruncated parent law: `mean` and `median` are not promises about the
bounded, rounded output. Proposed example settings are engineering examples, not
trace-fitted defaults or privacy claims.

Use explicit inclusive integer bounds L..H at every sampling site. Uniform samples
integers directly. For continuous laws, sample conditionally on [L, H+1), then
floor once to bytes or milliseconds; test this exact quantization contract. For
exponential/Weibull use conditional inverse-CDF sampling without a rejection loop:
for U uniform in (0,1), let a=(L/scale)^shape, b=((H+1)/scale)^shape and
`t = a - ln(1 - U * (1 - exp(-(b-a))))`; return `scale * t^(1/shape)`
before flooring. Exponential is shape=1, scale=mean. Use `expm1`/`ln_1p` and
validate finite, distinguishable transformed bounds; reject numerically unsupported
parameters rather than silently switching distributions or clipping.

For log-normal, accept only draws inside [L,H+1), with at most 128 attempts per
sample. Exhaustion disables the affected optional work with an explicit
`sampling_exhausted` reason; never substitute a clamped value or retry forever.
Narrow or extreme truncation can be inefficient: document this and test both normal
profiles and exhaustion. A reviewed inverse-normal implementation can replace this
later without changing the conditional distribution contract. Accepted samples
must not accumulate artificial endpoint mass from clamping unbounded draws.

For weighted discrete, permit 1..32 distinct values, all within declared bounds,
and checked positive integer weights/sum. No continuous rounding applies. Reject
oversized lists explicitly; do not truncate. Uniform equal bounds provide a fixed
value for deterministic profiles. Reject equal continuous bounds (use uniform),
unknown/inapplicable parameters, non-finite values, overflow and unsupported float
precision (H+1 must be exactly representable). Timing and header padding may be zero;
body budgets and range sizes must be positive. A zero padding draw omits the field.

Requested range size may be shortened by remaining resource/budget capacity, with
sampled versus dispatched bytes and an explicit limiting reason in evidence. This
is admission adjustment, not a new sample from the configured distribution. For
padding, skip the whole optional field when its sample does not fit; never clip it
silently or block the real call. Global/episode caps always take precedence.

Production uses an OS-seeded cryptographic RNG; injectable seeded RNG and virtual
time are test-only. No persisted RNG seed or identifier shared across wallets.
Independent draws cannot reproduce temporal correlation, self-similarity, TCP ACK
patterns or congestion feedback. HTTP/2/TLS/TCP/Tor determine packetization: these
knobs control application bytes and request scheduling, not actual packet lengths
or exact wire arrival times. Measure the resulting traces before making claims.


## Episode model and response-volume accounting

Schedule one episode per pool owner (per connection once an adapter exists), not one independent
cover loop per tool invocation. Key ownership by runtime, origin, EVM isolation
identity, transport/fetch policy and compatible cover configuration. Merge
concurrent calls sharing that owner into its active episode. Different listeners
sharing a wallet therefore share a budget. Never merge wallets or discovery.
Different configurations resolving to the same owner must fail validation with a
clear conflict, or require separate explicit wallet assignments; do not silently
multiply cover loops or choose one configuration by arrival order.

An episode starts at the first real API request, samples its body budget, start
delay and tail allowance once, and registers subsequent overlapping calls. Sample
range sizes and gaps as slots are replenished; sample padding for each physical
application request. Respect the configured concurrency bound. New calls do not
resample the episode target or reset its hard deadline. Once all real calls finish,
a bounded sampled tail may complete remaining cover. Results return to MCP as soon
as normal processing finishes; tail work is lifecycle-owned and may continue after
that return. Under sustained real traffic, reaching the episode cap disables cover
until the owner becomes idle; it does not immediately start unlimited new episodes.

The MVP implements `extra_body`; add `target_total_body` as an explicit follow-up:

- `extra_body`: sample a cover-body budget independent of real response lengths.
  This is the simpler baseline and must be measured, not assumed protective.
- `target_total_body`: sample target T independently at episode start. Maintain
  real received body bytes R, cover received bytes C and reserved in-flight cover
  bytes Q. Schedule at most `max(0, T - R - C - Q)` additional cover, subject to
  hard budgets. Account unsigned challenges, signed responses and overlapping
  real calls within the episode, including errors. Do not count request bytes or
  PING ACKs as downloaded body. Real traffic can exceed T; report `target_exceeded`
  without truncating or delaying it. In-flight traffic can overshoot too.

For `extra_body`, maintain received cover bytes C and reserved bytes Q and schedule
at most `max(0, sampled_budget - C - Q)` additional cover. Real body bytes do not
reduce this budget. Reserve stream/request/byte capacity before dispatch, and
retain reservations until a canceled stream has finished cleanup. Unknown-length
real responses must not prevent real results from being consumed or returned.

Use bounded chunks and the sampled gap to replenish 1..3 rolling range slots.
Do not eagerly download the entire sampled target before the real response arrives.
Sample the first delay, then reassess received/reserved bytes as each slot completes.
At real completion, use the tail only for the remaining cover allowance
(`extra_body`) or combined-volume deficit (`target_total_body`). If fewer
than the configured minimum range bytes remain, permit a final shorter range.
If bytes remain after the sampled tail or hard deadline, report `budget_unspent`
for extra-body mode or `target_unmet` for total-body mode, with the stopping reason.
Tail duration is a maximum opportunity, not an obligation to keep sending after
the budget is met. This algorithm is a best-effort volume target, not a guarantee
that combined totals follow the configured distribution; report achieved results.

The MVP must count and label received cover-body bytes exactly at its bounded
reader, and retain real-response size metrics only at their actually observed
layer. It does not require a transport rewrite solely to measure encoded real
bodies. Before implementing `target_total_body`, establish reliable incremental
real-body accounting, including the challenge and signed attempt.
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
In the MVP, failures and invalidation remain cached for the owner lifetime;
there is no automatic requalification, including after limiter recovery. Preserve
negative state when connection-pool entries are evicted, so eviction cannot cause
a probe loop. Bound this state explicitly and refuse new cover qualification with
a visible reason if its capacity is exhausted. A follow-up may add one bounded
requalification per eligible episode after an explicit cooldown.

Select offsets uniformly over valid starts for the sampled length. A short
resource may need repeated ranges, subject to the same total request cap. Never
fall back to full-document downloads or a different endpoint automatically.
Validate every response, not just the qualification response. Handle 200 (ignored
Range), 416, redirects, 401/403/402, 429, malformed ranges, unexpected compression,
truncation and oversized bodies explicitly. Stop reading/cancel the cover stream
at the bound; do not close the shared TLS connection deliberately. Mark capability
unavailable/degraded with a reason. The MVP issues no further cover requests to
an owner after such rejection. Any future retry policy must honor Retry-After;
if its delay exceeds supported scheduling bounds, leave cover disabled rather than
shortening the server-requested delay. Never sign a cover challenge or reuse payment middleware.

A server may ignore Range and bytes can arrive before cancellation. Keep HTTP/2
receive buffering bounded and qualify adverse fixtures; do not promise that the
requested range itself caps network traffic. Do not alter real-call flow control
just to improve cover performance without separate validation.

## MVP: outbound request padding headers

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
request size, using the selected bounded sampler above. Generate
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

Padding is independent of range qualification: a failed 206 probe disables ranges,
not explicitly authorized padding on later API requests. Padding rejection disables
padding, not otherwise qualified ranges. The process/source gates still apply to
both. Apply padding only on negotiated HTTP/2; HTTP/1 compatibility must report
padding unavailable too. If the current transport cannot enforce that per request,
skip padding on HTTP/1-allowed profiles with explicit evidence until it can.

Reserve value bytes atomically before constructing a header; retain the reservation
until the request is dispatched or abandoned. Dispatched bytes count against the
rolling budget even on failure. Count qualification padding if enabled, and both
unsigned challenge and signed retry separately. Do not charge ordinary request
bytes to the cover budget. Count the complete uncompressed HTTP/2 header list
(including per-field overhead) separately against its configured ceiling. When it
would exceed the limit, omit optional padding and report why; existing handling of
oversized real headers stays intact. Rate/window exhaustion must never block or
cancel a real payment request. Observe actual encoded sizes in fixtures; logical
header-value bytes are not a hard network-byte cap.

## Pooled MVP and follow-up connection-scoped transport/PING

First measure a range/header-padding prototype with reqwest, recording connection reuse in
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

## Deferred: response pacing with HTTP/2 flow control

Consider this only after the MVP is measured. The receiver can let a
stream consume its receive window, withhold WINDOW_UPDATE credit, then grant more
credit after a sampled interval. This can induce body-data bursts and pauses from
a compliant peer. It is a separate experiment from scheduling range requests.

Window exhaustion affects DATA frames, not headers, PING acknowledgments or other
control traffic. Bytes already allowed, sent or buffered can still arrive. A
response smaller than the initial window may finish without pausing. Exhausting
one stream's window does not make the TLS connection quiet while other streams
remain active; exhausting the connection-wide window stalls all response bodies.
Tor latency, TLS/TCP buffering and HTTP/2 implementation batching separate scheduled
credit release from the timing observed outside the connection.

Two possible follow-ups must remain distinct:

- **Cover-only pacing:** withhold credit only on cover streams while consuming real
  API bodies promptly. Preserve sufficient connection-wide credit and bound cover
  buffering so paused cover streams cannot starve real streams. This reshapes cover
  timing but cannot guarantee gaps in the combined connection traffic.
- **Combined-response shaping:** also pause real response bodies to induce gaps in
  aggregate DATA traffic. This changes the no-intentional-real-delay requirement.
  It needs a separate explicit operator mode and design decision, finite per-call
  and total added-delay limits, and payment/deadline/cancellation qualification.
  Existing provider deadlines and financial uncertainty rules still apply.

Reqwest's initial-window settings and delayed body polling do not provide a
reliable per-stream credit scheduler. Investigate the actual connection-owned
Hyper/h2 adapter before implementing this. The h2 receive-flow-control API exposes
`release_capacity`, but may batch updates; releasing received capacity also
replenishes stream and connection windows. Do not assume an exact frame-send time,
independent window control or exact byte boundary from this API alone. Keep the
connection driver running while delaying credit, so control frames and unrelated
streams continue progressing. Adaptive window behavior must be accounted for.

Test small and large bodies, already-buffered data, parallel real/cover streams,
connection-window starvation, cancel/reset, peer timeouts and credit batching.
Measure encrypted traffic outside the client along with real-call latency. Compare
against request scheduling alone at equal overhead budgets, and investigate whether
the induced rhythm becomes a recognizable fingerprint. No pacing feature or
configuration placeholder is part of the MVP.

## Resource, isolation and lifecycle invariants

Real calls take precedence over cover in local scheduling. HTTP/2 server stream
limits may be below our configured concurrency; skip or delay cover instead of
blocking payment admission. Cover cannot reserve USDC, create wallets, trigger
rotation/refills, hold wallet/store locks, or extend signed-payment drain deadlines.

For paid execution, bind each episode to the actual EVM address captured for that
attempt. Reviewed unsigned test cases instead retain their actual discovery owner;
never merge those owners with wallet traffic. On
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
actual real/cover body bytes, padding value bytes, range/PING counts, elapsed time, concurrency high-water
mark, and completion/degradation reason. Never log wallet addresses, SOCKS tokens,
full URLs, headers, contents or production RNG state. Shared aliases can be named
only in operator logs, respecting listener scope in any agent-facing information.

Every exhausted budget, disabled capability or incomplete target produces explicit
evidence. Rate-limit duplicate warnings by aggregating counts and emitting a final
summary, not by silently discarding them. Since tails can finish after the tool
result, add a local `treazury_cover_status` tool only to listeners with enabled,
selected static cover bindings. It never performs network requests or starts cover.
Enforce the same listener/source visibility on invocation as on listing. Return
bounded current/recent status with explicit retention/eviction counts. Do not
expose other listeners' aliases, calls, or shared-episode aggregate byte totals;
operator logs may retain the aggregate evidence. Define and pin its schema before
network integration, and reject tool-name collisions normally.

Known degradation at response time gets a short additional MCP text advisory,
separate from the unchanged vendor content. Cover failure must not change a
successful real call's `isError` status. Ordinary disabled mode adds neither a
status tool nor advisory. Do not rely solely on MCP `_meta`, which may not be
shown to the agent. Tail completion/degradation is available through scoped status;
there is no automatic status polling or unsolicited extra MCP request.
Status reports must not imply successful obfuscation merely because bytes were sent.
Do not alter vendor payloads, tool input schemas or execution permissions.

## MVP implementation order and acceptance

Each milestone is one separately tested commit. Implement these in order. M4 of
the integration harness is already qualified; reuse it for unsigned checks. No
funded live calls are needed to implement this MVP. Later funded qualification
still depends on the remaining integration-runner milestones.

1. **C1 — Configuration, budgets and pure episode scheduler.** Implement the two
   opt-in gates, composition/inspection, `extra_body`, all five samplers, rolling
   global body/header reservations and bounded owner/capability state. Test conditional
   quantiles, discrete frequencies, rounding, numerical extremes and sampler
   exhaustion using fixed seeds and deterministic tolerances. Specify the scoped
   status tool and advisory result shapes, with inventory/permission tests. Reject
   follow-up settings. Use virtual time and seeded test RNGs for exact boundaries,
   overflows, merging across listeners, owner conflicts, process cap contention,
   tails, sustained traffic, and cancellation. The scheduler dispatches only through
   a fake transport at this stage; disabled mode does no additional work.
2. **C2 — Unsigned range qualification and bounded reader.** Implement the ordinary
   network-factory client path, exact-origin validation, single-range requests,
   lazy qualification and lifetime positive/negative caching. Reserve qualification
   before I/O. Test 206/range/validator/length correctness and every adverse response
   listed above, including 402, ignored Range with a large body, compression and
   cancellation. Prove no credentials/payment middleware, redirect following,
   full-download fallback, requalification loop or disabled-mode probes. This
   remains independent of real payment dispatch until the next milestone.
3. **C3 — Header padding, payment-path integration and lifecycle.** Attach episodes to the actual
   attempt identity and existing pooled client; hook real-call start/completion
   without changing signatures, request bodies, receipts or retry rules. Connect
   bounded rolling range dispatch, validated optional padding and its independent
   reservations, tails, source-scoped status/advisories and
   application shutdown. Use local x402 fixtures for challenge/sign/retry, rejected
   settlement and ambiguous delivery, without real funds. Test protected header names, unchanged signatures/receipts,
   431/WAF rejection, padding-only operation after range refusal, and no automatic
   replay without padding. Assert limits across
   shared/different wallets and listeners, safe promotion attachment, no paid replay
   after cover errors, no lock held while doing cover I/O, and prompt real-result
   delivery even while cover tails or stalled cover streams remain active.
4. **C4 — HTTP/2/TLS-over-SOCKS and fault qualification.** Extend the existing
   transport fixtures to observe overlapping streams and physical connection IDs,
   correct SOCKS identities/remote DNS, and actual negotiated protocol. Decode HPACK
   to verify never-indexed padding, header-list accounting, Huffman size variation
   and independent random contents across requests. Exercise
   server stream limits, GOAWAY, stream reset, connection loss, slow/oversized cover,
   early EOF, provider refusal and shutdown. Include HTTP/1-compatible real calls
   with explicitly unavailable cover. Test connection-wide resource pressure and
   verify no cover-created reconnect/replay of a signed request. Measure connection
   reuse; production status must continue to say pooled/best-effort because fixture
   success is not a production affinity guarantee.
5. **C5 — Reproducible unsigned experiment and MVP documentation.** Extend the
   repository-owned runner with reviewed unsigned API cases that use the same
   provider-execution cover hooks; its existing help-only outage cases remain
   uncovered. Unsigned cases retain their actual discovery identity for both API
   and cover traffic; local fixtures qualify EVM identities separately. Do not
   present this as live EVM-identity or funded qualification. Compare no cover,
   ranges only, padding only and their combination across all five samplers,
   concurrency 1/2/3, fixed fixture
   response sizes and timings, repeated samples and test-only seeds. Run one bounded
   real-Tor qualification with an explicitly approved same-origin resource that
   actually passes range qualification. Unsupported providers are recorded as such,
   not counted as successful covered samples. Keep dedicated Tor state, and record
   all cover requests/bytes, padding value bytes, sampling/admission adjustments,
   failures, connection reuse and real-call latency. A public resource permitting
   ranges does not imply approval for arbitrary padding; enable live padding only
   for an explicitly configured compatible resource, otherwise record that live
   leg as unqualified and retain the local protocol evidence.
   Run default/no-Zcash regression tests, payment/MCP/isolation suites, Clippy,
   formatting and the constructor audit. Document operator setup, limits, scoped
   status, incompatible providers and observed evidence in maintained docs.

MVP acceptance requires all five milestones: disabled-mode equivalence, finite
resource use, unsigned cover only, correct owner isolation, unchanged paid-call
semantics in local fixtures, visible degraded outcomes, clean cleanup and an
unsigned real-Tor covered sample. If no approved endpoint supports the required
range behavior, report live qualification as incomplete rather than weakening the
endpoint contract. Do not call the MVP a same-channel or privacy-qualified release.
Actual paid receipt/debit and rotation qualification follows through the integration
plan, without enabling spending as a side effect of this work.

## Follow-up implementation order

1. **Target-total response volume.** Add encoded real-body accounting for challenges,
   retries and concurrent calls. Test overshoot, target-unmet outcomes and
   compressed/unknown-length/error bodies. Consider correlated or trace-driven
   models only with representative measurements and a separate bounded design.
2. **Connection-owned transport, then randomized PING.** Review the concrete adapter
   before replacement. Prove real/cover affinity and disabled-mode equivalence on
   the shared transport before adding bounded one-outstanding-PING scheduling,
   ACK timeout handling and liveness coordination.
3. **Flow-control pacing experiment.** Start with cover-only pacing and compare it
   with request scheduling. Combined-response shaping remains a separate opt-in
   design decision; do not introduce real-response delays under an existing policy.

Each follow-up must retain the MVP budget, identity and financial invariants and
add its own focused tests. Compare PINGs, volume targeting and combinations only
after those modes exist; they are not blockers for the MVP and must not be
advertised as implemented beforehand.

Measure encrypted byte counts in each direction, packet/TLS-record timing where
observable, body-layer counts, encoded header sizes and HPACK indexing, number of physical connections, peak memory,
real-call latency, request amplification, failures, and target attainment. Test
small/large/unknown-length/compressed/error responses and parallel mixed endpoints.
Use held-out repeated traces to assess response-size inference and endpoint
classification; increased variance alone is not evidence of reduced inference.
Retain failed/degraded samples. Separate observers of a provider connection from
observers of the whole client-to-Tor link.

Further real-Tor qualification uses explicit approved static resources and finite
budgets, preserves isolation and Tor state, and observes circuit-learning stability
as in `tests/STARTUP.md`. Never use routine startup pricing probes as cover. Live
x402 qualification later verifies receipts/debits and double-buffered rotation
under cover via [the integration plan](live_integration_testing.md).

## References

- [HTTP range semantics, RFC 9110 section 14](https://www.rfc-editor.org/rfc/rfc9110.html#section-14): partial responses and ignored/unsatisfiable ranges.
- [HTTP/2 flow control, RFC 9113 section 5.2](https://www.rfc-editor.org/rfc/rfc9113.html#section-5.2): separate stream/connection DATA windows and receiver credit.
- [h2 receive FlowControl](https://docs.rs/h2/latest/h2/struct.FlowControl.html): capacity release, connection/stream accounting and batched WINDOW_UPDATE emission.
- [HTTP/2 PING, RFC 9113 section 6.7](https://www.rfc-editor.org/rfc/rfc9113.html#section-6.7): connection-level fixed eight-octet payload and ACK.
- [HTTP/2 padding/privacy discussion](https://www.rfc-editor.org/rfc/rfc9113.html#section-10.7): limitations of padding and observable client behavior.
- [Reqwest 0.13.5 client controls](https://docs.rs/reqwest/0.13.5/reqwest/struct.ClientBuilder.html): fixed keepalive interval, not arbitrary randomized frame control.
- [h2 PingPong interface](https://docs.rs/h2/latest/h2/struct.PingPong.html): lower-level connection PING access requiring integration with the actual connection owner.

- [HPACK literal encoding and never-indexed fields](https://www.rfc-editor.org/rfc/rfc7541.html#section-6.2.3): avoiding dynamic indexing does not disable static Huffman encoding.
