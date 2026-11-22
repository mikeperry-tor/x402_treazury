# Optional HTTP/2 cover traffic

See
[wallet selection and network isolation](architecture.md#wallet-selection-and-network-isolation)
for the pooling, cover-state, and Tor isolation tradeoffs.

`x402_treazury` can add bounded unsigned HTTP/2 range reads and optional
randomized request-header padding to API calls. Cover defaults to **on** in Tor
mode for eligible configured sources and **off** in direct mode. This is a
research mechanism for traffic-shape experiments, **not a demonstrated defense
against traffic analysis**. Cover shares `src/network.rs`'s identity-bound
clients and connection pools, which permits HTTP/2 multiplexing without
guaranteeing a shared physical connection.

## Enable and inspect

`network.cover_traffic_enabled` overrides the mode-dependent default (`false`
for a global opt-out, or `true` to enable cover in direct mode). Providers can
set `cover_traffic_enabled = false` to opt out, and deployment sources can set
`cover_traffic_enabled` beside `extends` to override a provider's setting
(`true` still cannot override a global `false`). `config show` prints the
effective network switch and source overrides offline.

### Automatic resource selection

For static sources without an explicit `[cover_traffic]` table, the default
tries the same-origin HTTPS `help_url` (often `llms.txt`) first and falls back
to the same-origin catalog URL (or uses the catalog URL as primary when it is
the only match). Local files, cross-origin URLs, credentials, and fragments are
skipped, and no `/llms.txt` path is guessed.

- If neither URL qualifies, startup logs `cover_no_same_origin_resource`
  naming the API, catalog, and help origins and each rejection reason (without
  paths, queries, credentials, or fragments). Configure an explicit profile or
  disable cover for that provider.
- Agent-added sources do not receive automatic cover.
- If an [operation URL extension](configuration.md#operation-url-extensions)
  routes a call to a different origin than `base_url`, cover logs a warning and
  skips cover for that call (disable cover on multi-origin providers).

The built-in range-only profile uses 1 stream, at most 8 requests and 16 KiB
per episode, a 10-second deadline, a 1 KiB qualification probe, and a 16 MiB
resource cap. Additional body targets follow log-normal (median 8 KiB, sigma
0.65, bounds 4-16 KiB), ranges are uniform 1-4 KiB, start delay is uniform
0-100 ms, request gaps are Weibull 50-250 ms (scale 100, shape 0.8), and tail
is uniform 1-3 seconds. Header padding requires an explicit profile.

### Explicit profile configuration

Choose a static HTTPS resource on the exact API origin that supports unsigned,
identity-encoded single-range `206` reads (never a paid endpoint or pricing
probe). The network and provider blocks normally live in separate files (inline
sources use `[sources.NAME.cover_traffic]`):

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

`ranges_enabled` defaults to `true` (set `false` for padding-only; at least one
of ranges or padding must be configured). All fields validate even when cover
is globally off. `volume_mode` accepts only `"extra_body"` (additional cover
body bytes, including qualification, independent of response payload size and
header padding). PING, total-volume targets, and flow-control pacing are not
accepted.

### Distribution choices and sampling contract

Network measurements do not support one universal arrival or length law
([Paxson and Floyd](https://web.stanford.edu/class/cs244/papers/paxson1995.pdf),
[interarrival study](https://ir.canterbury.ac.nz/bitstreams/681998f5-30a2-46e7-a006-20ac74c3d203/download),
[CAIDA packet traces](https://www.caida.org/catalog/papers/1998_inet98/Inet98.pdf),
[aggregate volume study](https://arxiv.org/abs/2007.10150)). Five independent
distributions are supported for volume, range size, padding size, start delay,
request gap, and tail using the workspace's `rand` 0.9 dependency (no extra
statistics crate):

| Config name | Extra parameters | Intended use | Sampling method |
| --- | --- | --- | --- |
| `uniform` | None | Baseline, bounded sizes, or fixed values (`min == max`) | Integer uniform |
| `exponential` | Positive `mean_ms` or `mean_bytes` | Memoryless timing baseline | Closed-form inverse CDF |
| `weibull` | Positive `scale_ms` or `scale_bytes`, positive `shape` | Flexible gap shapes (`shape == 1` is exponential; `< 1` favors short gaps with a longer tail) | Closed-form inverse CDF |
| `log_normal` | Positive `median_ms` or `median_bytes`, positive `sigma` | Skewed byte budgets and long-tailed delays | Box-Muller normal plus bounded rejection (max 128 tries) |
| `weighted_discrete` | `values_ms` or `values_bytes` (1-32 distinct values), matching positive integer `weights` | Multiple explicit size peaks or empirical timing histograms | Integer cumulative weights |

Sampling rules:

- **Bounds and rounding:** Every site uses explicit inclusive integer bounds
  `L..=H` (continuous laws sample conditionally on `[L, H+1)` and floor once;
  equal continuous bounds are rejected in favor of `uniform`). Timing and
  padding may be zero (zero padding omits the header); body budgets and range
  sizes must be positive.
- **Exact transforms:** Exponential and Weibull compute `a = (L/scale)^shape`,
  `b = ((H+1)/scale)^shape`, `t = a - ln(1 - U * (1 - exp(-(b - a))))`, and
  `scale * t^(1/shape)` using `expm1` and `ln_1p`, rejecting numerically
  indistinguishable bounds rather than clipping. Log-normal rejects up to 128
  draws; exhaustion disables that optional work with `sampling_exhausted`
  rather than clamping endpoints.
- **Admission capping:** Remaining resource or budget capacity may shorten a
  requested range size (recording sampled vs. dispatched bytes and limiting
  reason); padding whose draw exceeds capacity is skipped whole rather than
  clipped. Production sampling uses an OS-seeded RNG with no persisted seed or
  cross-wallet sample cache.

## Identity, scheduling, and payment safety

Cover owners are keyed by Tokio runtime, canonical EVM or discovery identity,
effective Tor isolation scope (`Owner::from_transport`), exact origin,
transport policy, read-inactivity timeout, and public-destination policy.

- **Episode lifecycle:** Calls sharing an owner attach to one bounded episode
  without extending its deadline or resetting its volume budget. A new API call
  arriving before tail expiry cancels the tail; once expired, the episode
  cannot be revived. Different isolation scopes for the same wallet and origin
  keep separate owners and qualification state.
- **Tor scope retries:** When an unsigned API call retries on a fresh Tor scope
  after an eligible status (`400`, `403`, `408`, `429`, `500`, `502`, `503`,
  `504`), it releases the failed scope's cover lease and qualifies the new
  scope independently. Cover range success never promotes an API transport
  record.
- **API-call priority:** Before every physical API request, outstanding cover
  streams are canceled and cleaned up. Cover stays paused through challenge,
  signing, and retry until final response headers arrive, then may overlap
  body download and the post-response tail.
- **Payment isolation:** Cover never signs, retries signed calls, obtains
  allowances, promotes wallets, or triggers funding. Padded payment failures
  are never replayed without padding. An unsigned payer handoff ends the first
  wallet's cover call and binds the replacement wallet's owner. Shutdown stops
  cover tasks before draining accepted financial work.
- **Shared-connection caveat:** Shared connections can still experience
  contention or transport faults (a three-call Tor experiment observed one
  unresolved API broken pipe). Disable cover globally or per provider if a
  provider reacts poorly.

## Qualification and refusal

The first range request is a budgeted qualification starting at byte 0
(default `bytes=0-1023`, a bounded
[HTTP byte range](https://www.rfc-editor.org/rfc/rfc9110.html#section-14.1.2)).
Subsequent ranges pick random offsets within the observed resource length.

- **Acceptance criteria:** Requires HTTP/2, status `206`, exact `Content-Range`,
  finite size within `max_resource_bytes`, matching `Content-Length` if
  present, identity encoding, and exact body length. Subsequent reads require
  the same size and validator.
- **Refusal and fallback:** Status `200` (`cover_range_ignored`, body not
  downloaded), redirects, `401`, `402`, `403`, `429`, compression, malformed
  ranges, changed validators, and early EOF refuse the resource while the owner
  stays cached. `fallback_url` is tried at most once after initial
  qualification fails (not on rate limits, HTTP/2 unavailability, deadlines, or
  cancellation), logging `cover_fallback_selected`. Local episode deadlines and
  cancellation stop optional work and return an unfinished qualification to
  unknown rather than caching a refusal.
- **Header padding:** Uses random visible ASCII, protected-name checks, a
  separate header-list budget, and never-indexed HPACK fields (no header
  contents logged). Header rejection disables further padding independently of
  ranges. When `allow_http1` is set, padding is skipped because the transport
  cannot conditionally add it after ALPN.

### Cover diagnostics

Range and episode events record listener, source, ephemeral episode ID,
primary vs. fallback resource, requested offset and length, HTTP status and
version, observed bytes, stage, and elapsed time (never URLs, raw headers,
validators, padding contents, payment identities, or upstream prose).

| Event | Log level |
| --- | --- |
| Process-wide concurrency or budget refusal | `WARN` |
| Fresh resource refusal, fallback selection, or optional task or sampling failure | `INFO` |
| Completed episodes, `cover_budget_unspent`, cached refusals (`fresh_attempt=false`), local cancellation, and owner LRU eviction | `DEBUG` |

## Bounds and evidence

Global limits default to 8 active episodes, 6 active streams, 64 requests and
4 MiB of cover body per 60-second rolling window, 64 KiB of header padding
values per window, and 1,024 retained owners (source `concurrency` is `1..=3`).

- **Conservative accounting:** Dispatched requests stay charged on failure,
  in-flight reservations never expire, partially received bytes remain charged
  on cancellation, and completed entries stay charged for a full window.
  Exhausting a limit refuses optional cover without delaying API calls.
- **Owner reclamation:** At the 1,024-owner cap, insertion evicts the
  least-recently-used fully inactive owner (expired episode, zero active calls
  and streams, and all cleanup handles released). Eviction drops cached
  qualification state without resetting window budgets.
- **Shutdown report:** Clean shutdown emits one stderr `TREAZURY_COVER_REPORT`
  JSON line summarizing episodes, range outcomes, observed application-reader
  body bytes, qualified ranges, dispatched padding bytes, and peak active
  streams.

## Testing and follow-ups

Local tests cover paid challenge and retry flows, managed promotion, SOCKS
separation, HTTP/2 streams, HPACK, fault injection (using distinct payer
injected fault), preemption, refusal, and cleanup. Production cleanup has no
cutoff wait (the 3-second wait in `wait_experiment_idle` is test-only).

See [qualification status](testing.md#qualification-status) for current Tor
coverage (live padding and funded rotations under cover remain unqualified) and
[deferred cover work](plans/deferred/http2_cover_traffic.md) for future PING,
total-volume, and pacing experiments.
