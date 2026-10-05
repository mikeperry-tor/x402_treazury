# Deferred HTTP/2 cover transport and shaping

The pooled MVP is documented in [cover traffic](../../cover-traffic.md). Retain its
identity, bounded-resource, unsigned-only, scoped-evidence and no-paid-replay
invariants in every follow-up. These items are not implemented settings.

## Pooled transport failure investigation

The [qualification scope](../../testing.md#qualification-status)
observed one API broken pipe in a three-call Tor run after cover deadlines. The
cause is unresolved. Before expanding deployment, compare covered/uncovered runs
at equal real workload using a controlled HTTP/2 server: idle connection closure,
GOAWAY, RST_STREAM, tail cancellation and subsequent real calls. Record physical
connection reuse and close reasons, with bounded repeated direct/Tor cases.
Distinguish provider/Tor failures from cancellation-related pool behavior. Preserve
failed samples and never add automatic signed-request replay to mask errors.

## Connection-owned transport and PING

Introduce a network-factory-owned Hyper/h2 adapter only if measurements justify it.
It must carry both the real request and cover on the same negotiated connection,
retain SOCKS/TLS/public-destination checks, and expose connection identity without
credentials. Sending PING on an unrelated helper connection is not useful.
Reqwest periodic keepalive is not a randomized PING scheduler. PING's standard
fixed eight-byte payload adds timing/control noise, not significant response volume.
Bound count/rate/outstanding acknowledgments and lifetime; merge owners safely.

Qualify stream limits, GOAWAY, reconnects, reset, cancellation, TLS policy, HPACK,
isolation and managed payment admission before moving production transport.
Retain real-request priority and never reconstruct/replay a signed payment when
cover or a connection fails. Report actual affinity and downgrade reasons.

## Target total body volume

A separate `target_total_body` mode could sample desired real-plus-cover body bytes
and issue only the remaining deficit. Unknown/compressed/streaming and error bodies
need explicit counting rules. In-flight cover and real data can overshoot a target;
report sampled targets, achieved totals, layer, cancellation and budget shortfall.
Targets below the real response size cannot conceal its size. Do not silently
reinterpret `extra_body`, which currently means independent additional volume.

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


## Qualification

Compare request scheduling, padding, PING and pacing at equal finite overhead
budgets using fixed fixture sizes/timings, all supported distributions, concurrency
1/2/3, repeated independent runs and an uncovered baseline. Measure both application
and encrypted wire bytes/timing, physical connections, peak memory, latency,
request amplification, failures and target attainment. Use held-out traffic for
response-size inference/endpoint-classification evaluation. Increased variance
alone is not privacy evidence; independently sampled timings do not reproduce
self-similar traffic. Distinguish provider-link from client-to-Tor-link observers.

Use dedicated persistent Tor state and inspect circuit-learning stability for
performance work. Live padding requires an explicitly compatible resource;
range support alone does not establish permission for arbitrary headers. Keep
failed/degraded samples. Later funded coverage uses the
[deferred live acceptance plan](live_integration_acceptance.md) to check receipt/debit,
refill concurrency and rotation under cover, after an uncovered baseline.

## Primary references

- [RFC 9113 flow control](https://www.rfc-editor.org/rfc/rfc9113.html#section-5.2)
- [RFC 9113 PING](https://www.rfc-editor.org/rfc/rfc9113.html#section-6.7)
- [h2 FlowControl](https://docs.rs/h2/latest/h2/struct.FlowControl.html)
- [h2 PingPong](https://docs.rs/h2/latest/h2/struct.PingPong.html)
- [HPACK never-indexed encoding](https://www.rfc-editor.org/rfc/rfc7541.html#section-6.2.3)
