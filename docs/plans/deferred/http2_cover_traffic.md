# Deferred HTTP/2 cover transport and shaping

The pooled MVP is documented in [cover traffic](../../cover-traffic.md). Retain its
identity, bounded-resource, unsigned-only, scoped-evidence and no-paid-replay
invariants in every follow-up. These items are not implemented settings.

## Sustained activity and episode renewal

Current cover scheduling groups calls with the same owner into one episode. The
owner includes the runtime, wallet identity, effective transport scope and origin;
it is not a process-wide episode. The episode's deadline starts with its first
call (ten seconds in the default profile). Later calls share its remaining volume
and request allowances without extending the deadline or replenishing those
allowances. A stopped episode remains attached while any real calls or cover
streams are active, so a later call can join an episode whose range scheduler has
already finished. A fresh episode requires a subsequent attachment after the old
episode is exhausted and its calls and stream cleanup have ended. See the
[current scheduling contract](../../cover-traffic.md#identity-scheduling-and-payment-safety)
and its implementation in [the owner registry](../../../src/cover/registry.rs),
[episode accounting](../../../src/cover/episode.rs) and
[range scheduler](../../../src/cover/runtime.rs).

For example, overlapping calls can keep the owner continuously active beyond the
first ten seconds. The original episode then expires, but new calls keep joining
it rather than obtaining another range-cover window. Exhausting its volume or
request allowance can stop range scheduling even sooner. Sustained activity can
therefore outlast the additional response traffic intended to obscure it. Separate
owners have separate episodes, so load elsewhere does not by itself cause this
failure. This describes missing range cover, not a claim that every form of header
padding stops at the same boundary.

Assess a supported way to renew bounded episodes during sustained activity rather
than require a fully idle owner. Compare renewal with a simpler scheduling change
before selecting an implementation. Do not merely extend a deadline indefinitely
or reset allowances on every attachment: that would change the operator's overhead
contract. Determine how old streams finish, how new calls select the current
episode, and how qualification knowledge and existing process budgets carry across
renewal without duplicate schedulers or cross-owner state. Renewal alone is not
sufficient if overlapping requests keep the scheduler paused as described below.

## Response-linked timing and qualification patterns

Every real physical request takes priority over range cover: outstanding cover
streams are canceled and their local cleanup is awaited before dispatch. Cover
then stays paused through the request/payment handshake until final response
headers. With overlapping requests, a pending request can keep the shared owner's
cover paused even after another request has received headers. This favors real
request progress, but associates cover suppression and resumption with real-call
events; it does not provide continuous cover through challenge/sign/retry.

The episode clock continues while cover is paused. If response headers arrive
after its deadline, that call gets no range-cover window from the episode. If
headers arrive just before expiry, little time remains for qualification, body
cover or a tail. The sampled successful-call tail is clamped to the original
episode deadline, rather than guaranteed to run for its sampled duration after
completion. When the last call drops without being marked complete, its tail ends
immediately. Failure and success can consequently produce different traffic endings.
These are range-scheduling behaviors; optional request-header padding does not
provide equivalent response timing or volume concealment.

The sampled `start_delay` is also measured from episode creation, not from response
headers or the first unpaused scheduling opportunity. It is enforced, but can
already have elapsed by the time headers arrive. Increasing it alone does not
decouple cover onset from response timing. Initial resource qualification has a
recognizable request shape: the default bounded range is `bytes=0-1023`. This is
a qualification read, not a full-resource download, and successfully cached
capability can be reused; it is not necessarily repeated for every call or episode.
Interrupted qualification and fresh transport scopes can require another probe.

Observers able to see encrypted traffic timing and volume may use cover gaps,
onsets and endings as clues to real-call boundaries, slow responses or failures.
The destination can additionally recognize the actual range requests. Neither
random sampling nor more bytes establishes that these clues are concealed; a
predictable qualification pattern can itself identify the cover mechanism.
Actual inference depends on the observer's position, workload, physical connection
reuse and Tor multiplexing. The current implementation is experimental and makes
no same-connection or traffic-analysis-resistance guarantee. Circuit separation
also does not conceal application identities disclosed to a destination.

Assess scheduling less tightly coupled to response events, including how renewal,
start-delay origin, successful and unsuccessful tails, and qualification reuse
interact. Compare supported alternatives at equal overhead before choosing changes;
do not assume moving the clock to response headers or adding rolling episodes
solves handshake visibility. Preserve real-request priority, accepted-work ownership,
identity isolation and no signed-request replay. Any intentional delay of real
traffic requires the separate combined-response shaping decision below. Do not
randomize qualification blindly: preserve range validation, resource permissions
and faithful capability detection.

### Scheduling assessment and acceptance

Use a controlled HTTP/2 fixture to exercise isolated calls and sustained overlapping
calls beyond multiple episode windows; deadline, volume and request exhaustion;
headers before, near and after expiry; overlapping pending headers; completed and
unsuccessful final calls; and waiter cancellation. Include fresh, cached,
interrupted and replaced-scope qualification, plus cover cleanup during renewal
and shutdown. Assert actual dispatches and retained ownership, with no duplicate
active scheduler, cross-owner state reuse or paid replay. Distinguish intended
operator-budget exhaustion from scheduling-induced absence of cover.

Compare uncovered traffic, current scheduling and selected alternatives using the
same real workload and equal finite overhead budgets. Measure achieved coverage
over time, gaps around real headers/completion/failure, qualification frequency,
physical connection reuse, encrypted wire patterns and real-call latency. Include
held-out call-boundary and outcome inference where making a privacy claim, and
report shortfalls and failed samples rather than infer protection from increased
variance. Direct fixtures establish scheduling correctness; actual Tor and
observer-specific privacy claims need their own measurements under the
qualification section below.

Exit this assessment with an evidence-backed scheduling decision and focused
implementation/regression boundaries, or explicit acceptance of the measured
limitations. Hand current behavior and supported claims back to the cover guide.
This track owns episode renewal and response-linked timing; the PING, volume and
flow-control tracks below do not substitute for this assessment. Complete the
independent design, exact-SHA/fixup and combined reviews required by the
[planning workflow](../../planning-workflow.md) before declaring selected work
complete. This plan grants no live-funded or real-Tor execution authority.

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
