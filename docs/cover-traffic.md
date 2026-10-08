# Optional HTTP/2 cover traffic

Treazury can add bounded unsigned range reads and randomized request headers to
operator-selected API calls. Cover defaults on in Tor mode and off in direct mode;
it remains experimental.
It uses the same identity-bound network factory and reqwest pools as real calls.
Pooling permits multiplexing but does not guarantee a shared physical connection.
No production same-channel or resistance-to-traffic-analysis claim is made.

## Enable and inspect

`network.cover_traffic_enabled` overrides the mode-dependent default. Set it false
for a global opt-out, or true to enable cover in direct mode. In a provider TOML
file, set the top-level `cover_traffic_enabled = false` to disable both ranges and
padding for that provider. A deployment source can set the same field beside
`extends` to override an inherited provider policy. Provider true cannot override
global false. `config show` shows the effective network switch and authored
source overrides without opening secrets.

For authored static sources without a `[cover_traffic]` table, the default uses the
same-origin HTTPS catalog URL, with the same-origin `help_url` (often `llms.txt`)
as fallback. If only help qualifies as a candidate, it becomes the primary. Local
files, cross-origin documents, credentials and fragments are not candidates. When
neither URL is suitable, startup logs `cover_no_same_origin_resource`; set an
explicit profile or disable cover for that provider. No `/llms.txt` path is guessed.
Candidate selection does not prove range support: runtime qualification is still
mandatory. Inspection/listing does not send cover requests. Agent-added sources
remain outside automatic cover; remote document contents never authorize it.

The built-in range-only profile uses one stream, at most eight requests / 16 KiB
per episode, a ten-second deadline, a 1 KiB qualification and a 16 MiB resource
limit. Additional body targets follow bounded log-normal (median 8 KiB, sigma .65,
4–16 KiB bounds), ranges are uniform 1–4 KiB, start delay 0–100 ms, Weibull gaps
50–250 ms (scale 100, shape .8), and tail 1–3 seconds. Global limits also apply.
Header padding requires an explicit profile with a provider-compatible header.
Authored profiles replace the automatic profile; normal TOML composition applies.

Choose an explicit HTTPS static resource on the exact API origin. It must permit
unsigned, identity-encoded single-range reads with exact 206 responses. Do not use
paid endpoints, pricing probes or another origin. Header padding is independently
optional; configure a provider-permitted ignored extension field. The example
values below are engineering examples, not trace-fitted or privacy presets.

The network and provider portions normally belong in separate files. Inline
sources use `[sources.NAME.cover_traffic]` instead of `[cover_traffic]`.

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
# fallback_url = "https://api.example.com/llms.txt" # optional, same exact origin
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

`ranges_enabled` defaults true. Set it false for padding-only operation; at least
one of ranges or padding must be configured. All fields validate even when the
process switch is off. `volume_mode` accepts only `extra_body`: the sampled target
is additional cover-reader body bytes, including qualification. Real response
size does not drive this target. Header-value budgets are independent. PING,
target-total-volume and flow-control pacing settings are not accepted.

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

The implementation supports these five distributions for volume, range size, padding size,
start delay, request gap and tail. Each site samples independently; no recursive
mixture configuration or correlation model is included.

| Config name | Parameters beyond explicit min/max | Intended use | Sampling route |
| --- | --- | --- | --- |
| `uniform` | None | Baseline/control; bounded sizes or jitter | Integer uniform from `rand` |
| `exponential` | Positive `mean_ms` or `mean_bytes` | Memoryless timing baseline; skewed sizes if requested | Closed-form inverse CDF; `rand_distr::Exp` also exists |
| `weibull` | Positive `scale_ms` or `scale_bytes`, positive dimensionless `shape` | Flexible gap shape; shape 1 equals exponential, below 1 favors short gaps with a longer tail | Closed-form inverse CDF; `rand_distr::Weibull` also exists |
| `log_normal` | Positive `median_ms` or `median_bytes`, positive dimensionless log-space `sigma` | Skewed volume/header/range budgets and optional long-tailed delays | Closed-form Box–Muller normal, `exp(ln(median) + sigma*z)`, plus bounded rejection |
| `weighted_discrete` | `values_ms` or `values_bytes`, matching positive integer `weights` | Explicit multiple size peaks or a small empirical timing histogram | Integer cumulative weights; `rand::distr::weighted::WeightedIndex` is also available |

The [Rust distributions library](https://docs.rs/rand_distr/latest/rand_distr/)
provides Exp, Weibull and LogNormal; [WeightedIndex](https://docs.rs/rand/latest/rand/distr/weighted/struct.WeightedIndex.html)
provides categorical sampling. The implementation uses the already-resolved `rand` 0.9 dependency,
closed-form transforms and integer cumulative-weight sampling, avoiding an
additional statistics dependency. Box–Muller consumes two independent open-unit
uniform draws for each standard normal, without cached cross-owner samples. Constructor parameters are
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


Production sampling uses an OS-seeded RNG, with no persisted seed or cross-wallet
sample cache. Deterministic seeds belong only to fixtures.

## Identity, scheduling and payment safety

Owners are keyed by runtime, actual EVM/discovery identity, exact origin, transport
policy, effective timeout and public-destination policy. Conflicting settings for
one owner fail validation. Calls sharing an owner attach to one bounded episode;
additional calls do not extend its deadline or restart its volume budget. A
rotation creates a new identity/pool. Old in-flight calls retain their original
identity. Shared wallet bindings may share episodes across listeners.

Every real physical request takes priority: outstanding cover streams are canceled
and their local cleanup is awaited before dispatch. Cover stays paused through
challenge/sign/retry until final response headers, then may overlap body download
and the sampled successful-call tail. Thus cover does not run throughout the
entire paid handshake. A canceled qualification remains unavailable for that owner
for the process lifetime. This conservative rule prevents probe/retry storms.

Cover never signs, retries a signed request, obtains an allowance, promotes a
wallet or triggers funding. Static and managed calls retain ordinary financial
admission and receipt handling. Optional failures are advisory; a real API error
still follows the normal payment/output path. A rejected padded payment is never
replayed without padding. Shutdown cancels cover and drains accepted financial
work using the normal safety deadline.

Shared connections can still fail or experience resource contention. Optional
error handling does not guarantee real-call availability. A three-call Tor
experiment observed one API broken pipe; its cause remains unresolved. Cover can be disabled globally or per provider, and this failure is retained in
the qualification evidence.

## Qualification and refusal

The first range request is a budgeted qualification. Require HTTP/2, status 206,
exact Content-Range, a finite resource size, a matching declared length when
present, identity encoding and an exact bounded body. Subsequent reads preserve
resource size and any observed validator. Redirects, authentication/payment
challenges, ignored ranges (200), rate limits, compression, malformed ranges,
changed validators, overflows, early EOF and deadlines refuse the resource. An
optional `fallback_url` is tried at most once after completed initial qualification
failure, within the same episode/global budgets. Rate limits, HTTP/2 unavailability,
deadlines and cancellation do not trigger fallback. Success pins that resource for
the owner lifetime; failure of both candidates disables ranges. Failure after a
successful qualification does not switch resources. There is no full-download
fallback, periodic requalification, paid retry or cross-origin redirect. Logs and
scoped agent status retain the primary refusal and `cover_fallback_selected`.

Header values use random visible ASCII, protected-name/collision checks and a
separate conservative header-list budget. Sensitive/never-indexed HPACK fields
avoid dynamic table reuse; Huffman coding still affects encoded length. No header
contents are logged. Header rejection disables later padding independently of
ranges. When `allow_http1` is set, padding is conservatively skipped because the
transport cannot conditionally insert it after ALPN. HTTP/1 real calls still work;
HTTP/1 range responses are unavailable cover.

## Bounds and evidence

Global limits default to eight active episodes, six streams, 64 requests and 4 MiB
cover body per 60 seconds, 64 KiB header values per window, and 1024 retained owners.
Source concurrency is 1–3. Source request/body/header/deadline/resource bounds also
apply. Dispatched requests stay charged on failure; in-flight reservations do not
expire. Completed budget entries remain charged for a full window after completion,
a conservative rolling limit. Budget/history/owner exhaustion is an explicit
refusal, never permission to drop accounting or queue real calls indefinitely.

Byte counts describe application-reader bytes and uncompressed header values,
not encrypted wire traffic. Buffered TCP/TLS/Tor bytes can exceed the requested
range before cancellation. Observed overrun chunks remain accounted. Admission
adjustments, sampling exhaustion and failures emit reasons in stderr and scoped
agent evidence. Successful real results keep their content and append separate
cover advisories where needed.

`x402_treazury_cover_status()` is a local, no-argument tool exposed only for selected
static cover bindings when the effective process switch is on. It reports the listener's
selected sources, not wallet IDs or shared-owner totals. Each binding retains
32 events and an explicit eviction count. It performs no network I/O. Disabled
mode adds no tool, randomness, probes or background tasks.

Clean shutdown emits one stderr `TREAZURY_COVER_REPORT` JSON line with aggregate
operator counters: episodes, range requests/outcomes, observed cover body bytes,
qualified ranges, dispatched padding counts/value bytes and stream high-water mark.
Production connection affinity is explicitly unobserved. Incomplete shutdown or
truncated output cannot qualify a runner result; counters are not durable billing.

## Testing and follow-ups

See [the integration runner](../tests/live/INTEGRATION.md) for the unsigned fixture
and local distribution matrix. Local tests cover paid challenge/retry, managed
promotion, SOCKS separation, actual h2 streams, HPACK, refusal/faults and cleanup.
Real-Tor unsigned discovery qualification is distinct from funded EVM/rotation
qualification. More noise or variance alone does not demonstrate less inference.

See [qualification scope](testing.md#qualification-status) for the distinction
between unsigned Tor observations and combined paid qualification.
Live padding and funded rotations under cover remain unqualified.

See [deferred cover work](plans/deferred/http2_cover_traffic.md) for connection-owned
PING, total-volume targets, pacing and privacy measurements.
