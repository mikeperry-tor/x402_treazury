# Tor isolation retries for discovery, APIs and Base RPC

## Intent, baseline and status

Proposed runtime work; this plan does not implement retries or authorize funded
qualification. Starting code baseline: `970f6bde1334eb42a540288ea1432381e1b9ae64`.
Accompanying documentation clarifies the already accepted unsigned x402 POST
semantics; it must not describe the proposed runtime behavior as delivered.

Try up to two additional Tor isolation groups after selected unsigned failures.
Deliver catalog/pricing, API payment/cover and Base RPC integration in separate
phases, sharing the HTTP transport-selection machinery.
Reduce avoidable paid discovery relay use and recover API calls affected by an
unusable exit, without replaying signed work or changing payer ownership. Retain
a successful replacement in the existing provider transport-cache entry; after
eviction or restart create a fresh isolation rather than reconstructing a failed
primary. Other provider origins on the same wallet retain their own transports.
Zcash/gRPC is a [separate post-merge extension](deferred/zcash_isolation_retries.md),
not a dependency of this batch or its cleanup.
Different circuits can still select the same exit IP; this is best-effort circuit
separation, not guaranteed exit diversity or a cure for all Tor blocking.

Owning guides: [network policy](../network-egress.md),
[resource limits](../local-resource-limits.md),
[selection/isolation](../architecture.md#wallet-selection-and-network-isolation),
[unsigned GET/POST semantics](../architecture.md#unsigned-x402-requests-and-post-retries),
[wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[wallet observation](../wallet-rotation.md#wallet-observation-policy),
[configuration](../configuration.md#paid-discovery-relay),
[cover policy](../cover-traffic.md), [source management](../agent-sources.md),
[development](../development.md), [testing](../testing.md),
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline) and
[Tor evidence](../../tests/TOR.md). Read affected guides before implementation.

Current behavior:

- `network.rs` derives stable SOCKS credentials and pooled clients from canonical
  owner identities. There is no request-scoped circuit retry discriminator.
- `catalog.rs::prepare_json_discovery` and `pricing.rs::probe_request` select the
  configured paid relay after one failed origin attempt. Fresh caches can avoid
  origin I/O. Pricing discovery is one-shot/coalesced, not a recurring probe.
- `payment.rs` obtains an unsigned response through the selected payer's client,
  then uses that same client for its signed request and associated cover owner.
  Generic unsigned status failures do not currently rotate circuits. Recognized
  structured signed insufficient funds permits the existing eligible-wallet
  traversal; generic signed errors do not.
- Base RPC stores HTTP clients for chain/header and wallet reads. Its read-only
  transport retries certain failures once on the same client; configured endpoint
  fallback is a separate outer loop. It has no paid Curl relay fallback.
- Zcash uses a separate runtime/identity/endpoint-keyed gRPC cache, with active SDK
  scan and transaction-submission ownership. This batch leaves it unchanged.

## Accepted behavior and boundaries

Acquire the currently selected transport from the existing client cache, then try
at most two fresh attempt scopes sequentially after eligible unsigned failures.
Keep the retry sequence local to its existing operation owner; no persistent
retry ledger, separate winner registry, global circuit epoch, control port,
NEWNYM, new readiness gate or general workflow framework. Two additional attempts
bound the newly introduced speculative request amplification, not financial
capital or the duration of progressing work. Do not add a whole-work deadline,
shorten existing timeouts or tune unrelated resource caps.

### Selected transports, pooling and lifetime

Keep the canonical cache key: owner identity, provider origin, runtime, protocol
permissions, public-destination policy and timeout. For participating Tor HTTP
paths, its value owns a coherent immutable transport record containing the client
and its effective isolation scope. Return that record together, rather than a
bare client plus separately reconstructed identity. The canonical owner still
identifies the payer/discovery principal; it is not replaced by the fresh scope.

On every participating cache miss, including first acquisition after restart,
create a fresh owner-bound scope. Retry candidates also use unique fresh scopes.
This deliberately replaces deterministic primary SOCKS credentials for these
participating paths: retain existing credential/token domains and canonical owner
formats, but do not require their former primary token values. Direct mode and
nonparticipating HTTP/gRPC constructors keep their existing behavior. Ensure
participating and nonparticipating lookups cannot accidentally consume or replace
each other's cache records; represent the transport-selection policy explicitly
where the same otherwise canonical key can be used by both. Such separation
must not broaden which callers can retry.

Each candidate client has its own reqwest pool and scope-specific SOCKS credentials.
Changing only auth metadata or cloning the previous client is insufficient: an
existing HTTP/2 connection performs no new SOCKS handshake. Keep private retry
candidates outside the canonical selected-entry slot until eligible success; do
not add a second cache indexed by every historical attempt.

On eligible fresh-candidate success, conditionally replace the canonical entry
with that candidate's transport record. Replace only if the entry still holds
the exact record acquired when this sequence began; immutable record identity
(e.g. Arc identity) is sufficient, with no counters. Never insert on promotion
when the slot is absent. Eviction/reinsertion, another successful promotion or a
late completion from an old selected transport must not restore a stale choice.
A candidate losing this replacement race still finishes its successful operation
through its pinned transport; it incurs no extra retry, failure or payer
acquisition. Failed candidates and paid relay successes never change the origin's
selected transport.

Discovery promotion requires a complete valid catalog or a successfully
interpreted pricing result, including existing successful no-challenge outcomes.
API promotion occurs on a valid accepted unsigned 402 challenge, or after a
successful unsigned response completes normal consumption/output validation.
A 402 proves unsigned connectivity, not payment acceptance or future paid success;
normal payment checks still apply. Malformed challenges, rejected/partial output,
cover qualification alone and signed outcomes cannot trigger promotion. Promotion
authorizes no extra request, signature, cache refresh or financial replay.
For Base, promote only after the complete RPC result passes its existing
method-specific validation, not merely HTTP 200 or a valid JSON-RPC envelope;
transport promotion does not certify a whole financial chain view.

Subsequent logical operations reacquire the canonical selected record rather than
reusing a caller-held obsolete client. Inspect pre-acquired catalog/pricing clients
and deferred loaders explicitly. Already accepted/in-flight operations retain their
exact record; a valid challenge's signed dispatch and response handling stay pinned
even if another operation promotes or evicts the entry meanwhile. Dropping the
failed response releases that response/stream; do not force-close the old client
or connection, flush the provider's pools, or disrupt sibling HTTP/2 streams.

Selection is scoped by the full cache key, not wallet-wide. Other origins using
the same wallet do not migrate. Source aliases sharing an owner, origin and all
other key settings intentionally share selection; different transport/public/
timeout policies retain separate selections. Discovery-origin and payer identities
remain separate even for one provider; catalog success does not migrate API traffic.
Optional cover attached to a participating API follows its effective scope without
turning cover fetches into independent retry/promotion consumers.

Retain the existing LRU capacity and eviction rules: replacement updates one slot,
not a growing history. The winner is retained only while its entry is cached.
After eviction, a new acquisition uses a fresh scope, not the forgotten winner or
the original deterministic scope. Restart behaves the same; selection is not
persisted. Fresh scope does not promise a different Tor exit IP. Old acquired
records can outlive eviction solely for their existing operation/cover ownership;
they must not repopulate or bypass selection for a new logical operation.

All constructors stay in `network.rs`, retaining remote DNS, no direct fallback,
public-destination rules, runtime/origin/protocol/timeout separation and existing
cache retention. Do not change persistent state formats, environment contracts
or wallet roles.

### Request and financial boundaries

The accepted x402 model gates protected execution on payment acceptance. Unsigned
POST and GET are equally eligible; preserve exact business arguments and body
bytes and add no POST-specific idempotency proof or business-side-effect gate.
This x402 assumption is not authority for arbitrary infrastructure POST replay.
Base retry authority instead comes from its existing read-only JSON-RPC allowlist;
Base calls never sign an x402 payment or invoke the paid Curl relay.

| Outcome | Planned handling |
| --- | --- |
| Unsigned HTTP 400, 403, 429 or 503 | Eligible for the finite isolated retry sequence in discovery, API and Base; no upstream prose classifier. A repeated 400 may simply be a bad request, and account-wide throttling may survive rotation. |
| Valid unsigned 402 | Stop circuit retries; validate the challenge normally. For API calls, retain its exact transport through signing and signed response consumption. |
| Other completed HTTP statuses | No new circuit retry, including success, redirects, 401, 404 and other 5xx. Existing discovery relay and Base endpoint-failover eligibility remain separate. |
| Discovery/Base connection/reset/read-inactivity failure | Eligible for discovery and supported read-only Base operations, using their typed transport evidence; never interrupt a response still making progress. Do not repeat a proven local SOCKS-listener outage expecting exit rotation to repair it. |
| API transport failure without an eligible completed status | Outside the initial API phase; preserve current error behavior rather than expanding uncertainty handling. |
| Policy, TLS trust, size, parse or challenge-validation failure | No new circuit retry. Preserve existing rejection and discovery relay handling; never weaken validation to recover. |
| Signed status or transport/output failure | No new generic retry, fresh challenge or signature. Preserve journaled liabilities and existing structured insufficient-funds traversal. |

For eligible 429/503 responses, honor valid `Retry-After` delay-seconds or HTTP-date
before the next network attempt, including relay fallback to that same target.
Missing, malformed or past values add no delay. Use no arbitrary maximum delay or
whole-work deadline. Scheduling must remain asynchronous and independent of other
operations; qualification authority is rechecked after any wait. Discard unused
error bodies without waiting for them. Never log credentials, private request
arguments or credential-bearing upstream prose. Report stage, ordinal, fixed
reason and available status through the existing safe diagnostic conventions.

Fresh valid caches still win without retries. Retries must not refresh failed
cache entries, persist challenges as reusable authority, change provenance or
turn one-shot pricing into background work. Waiter disconnect does not cancel an
already accepted payment owner. Explicit force/owner termination cannot launch
later dispatches or orphan disclosed financial work. Do not extend a signed
expiry, response inactivity budget or qualification allowance through retries.

## Ordered commit boundaries and acceptance

Use the seven ordered boundaries below. Each is independently buildable and
reviewable, with its own focused behavioral evidence and permanent documentation.
T1/T2 prepare the shared mechanism; T3 activates discovery. T4 adapts payment/cover
ownership; T5 activates API retries. T6 integrates Base RPC; T7 closes the
documentation handoff. Zcash is deferred until this batch is merged. Preparation
commits must not partially enable a retry path or change ordinary traffic counts.
No commits or runtime implementation are requested merely by writing this plan.

Resolve material independent design findings before implementing the affected
boundary. Keep the identifiers stable during execution. If a prerequisite cannot
compile and pass required lints without unused scaffolding, revisit its concrete
representation in independent design review before proceeding; do not add temporary
lint allowances, exports, flags or abstractions merely to preserve an artificial
split. Legacy acquisition should use the shared representation/helpers naturally.
Each commit owns its necessary replacement/recovery path and tests, rather than
leaving correctness repairs for the next boundary. For each implementation SHA,
record the exact focused test commands and nonzero executed case counts, applicable
build/lint results, formatting result, changed-guide checks and independent review
findings. A gate may reuse unchanged source-matched evidence; it must not substitute
a checklist or passing zero-test filter for behavioral evidence.

### T1 — Coherent transport handles and explicit selection policy

Refactor the HTTP cache value in `network.rs` into an immutable transport record
carrying client, canonical ownership and effective scope together. Preserve the
existing canonical key dimensions, LRU behavior and public constructor contracts
where needed by unaffected consumers. Make the distinction between legacy and
participating selection explicit so neither can populate or consume the other's
records accidentally. Do not select fresh scopes, promote records or add retries
to ordinary callers in this commit. Legacy HTTP credentials, connection reuse and
gRPC behavior remain unchanged.

Inspect factory wrappers, pre-acquired startup/catalog/pricing clients and payment
transport selection to establish where coherent handles will be consumed later.
Do not force unrelated RPC, treasury, help/import or gRPC callers onto new behavior.

Acceptance evidence:

- Focused network retention, HTTP/2 and boundary fixtures demonstrate repeated
  acquisition shares the intended record/pool, while owner, origin, runtime,
  protocol, public-destination and timeout differences remain separated.
- Pin legacy SOCKS credential values and request/connection counts for unaffected
  Tor callers; direct mode, redirects, TLS and public-address validation retain
  their existing results. No additional requests or signing occurs.
- Acquire a handle, evict its cache entry, and finish its existing work through
  the retained client without canceling sibling streams or recreating the entry.
- Validate the representation against existing `network::retention_tests`,
  `network::http_tests`, network boundary tests and `tests/network_audit.rs` as
  affected. Record the actual executed cases; no speculative new retry tests yet.
- Update the network guide with the delivered record/policy representation and
  unchanged current behavior. Keep activation explicitly pending T3/T5/T6.

Exit: focused checks, formatting, relevant lint/build checks and independent
exact-SHA review pass. Compare substantive refactor complexity with the original
baseline; resolve findings before T2 builds on the transport representation.

### T2 — Fresh scopes, conditional replacement and cache lifetime

Implement the participating transport acquisition and replacement mechanism using
T1's record, without enrolling ordinary discovery/API/Base callers yet. Participating
misses create fresh owner-bound scopes; retries can acquire private candidate
records with separate clients/pools. A successful consumer can conditionally
replace its canonical slot using the exact originally acquired record identity.
A missing, evicted/reinserted or already-replaced entry cannot be overwritten by
that stale sequence. A lost promotion race is advisory, never a request failure
or permission for more I/O. Do not add a winner registry, history cache or counters.

Use shared construction/acquisition helpers; keep eligibility, success validation
and retry scheduling with the consumers introduced in T3/T5/T6. Nonparticipating
production callers continue their existing credential and request behavior.

Acceptance evidence:

- Exercise the actual factory with fake SOCKS/TLS: selected acquisition and two
  private candidates have distinct auth and physical pools; candidate creation
  does not replace the selected entry. After promotion the next acquisition
  returns the exact winning record. Cloning a client/changing metadata cannot
  satisfy the connection-separation assertion.
- Deterministically interleave two recoveries, a late original success, eviction,
  reinsertion and a lost promotion race. Check exact-record replacement, no stale
  overwrite/resurrection, no extra dispatch and continued candidate completion.
- Force normal LRU eviction and recreate the context for restart. New participating
  acquisition uses a fresh scope rather than the forgotten winner or deterministic
  original. Verify unchanged capacity and no accumulated candidate/history entries.
- Other origins on the same wallet, other wallets, discovery versus payer owners,
  and differing policy/runtime/timeout keys remain independent. Equivalent aliases
  share selection. Mixed legacy/participating lookups cannot cross-consume records.
- Keep an original HTTP/2 sibling stream open while a candidate opens a separate
  SOCKS connection, completes and replaces the slot. Original progress continues;
  later acquisitions use the winner and acquired old handles remain usable.
- Cover these behaviors in owning network factory/retention/HTTP/2 fixtures;
  rerun affected T1 compatibility cases to prove ordinary callers remain legacy.
- Update network and resource guidance to describe the delivered primitive,
  cache-lifetime retention and fresh-on-miss behavior, explicitly distinguishing
  availability of the mechanism from consumer activation. No new resource cap.

Exit: independently review the exact SHA and substantive complexity changes;
resolve transport/promotion ownership and race findings before enrolling T3.

### T3 — Catalog/pricing retry and promotion integration

Enroll `catalog.rs` and `pricing.rs` in participating Tor acquisition. Reacquire
the selected record for each new logical operation; inspect pre-acquired clients
and `deployment/startup.rs` deferred loaders so stale client clones cannot bypass
selection. Accepted operations keep their pinned record. Integrate at most two
additional attempts for the specified unsigned statuses/transport failures, with
existing typed evidence for proven local SOCKS outage and no proxy probes or
health registry. Keep cache validation and pricing interpretation with their
existing owners; factor only genuinely shared transport/eligibility work.

Promote only complete valid catalogs or successful pricing interpretations.
A valid price challenge is discovery success, not authority to pay the target;
2xx no-challenge pricing remains unknown and successful. Malformed pricing headers
follow existing relay behavior without cycling. Honor Retry-After before another
origin attempt or the existing relay continuation. On exhaustion/ineligible failure,
preserve relay eligibility, readiness, payment ownership and breaker behavior.
An origin recovery cannot contact the relay or promote a relay-supplied result.

Inspect serving startup/publication, standalone loading, strict inspection, catalog
warming (including explicit direct warming), coalescing, disk provenance and
source-assigned relay payers. Direct mode, local files and qualification rules
remain unchanged. In this phase, lazy help/import, `source_url_info`, RPC, treasury,
cover fetches and internal relay-service requests gain no retries. Add no relay fallback paths.

Acceptance evidence:

- Catalog/pricing fixtures exercise each eligible status and transport outcome,
  success at every attempt position, exhaustion and ineligible errors. Assert
  exact origin/relay counts and reuse of the promoted scope by subsequent work.
- Validate promotion timing for complete parse/price success, valid 402 and 2xx
  no-challenge pricing. Malformed, oversized, interrupted or failed candidates
  and relay results cannot promote. Retain existing size/error visibility.
- Fresh disk caches avoid I/O; validators and origin/relay/direct-warm provenance
  stay correct. Coalesced operations have one finite sequence. Relay continuation
  wait/resume never repeats completed origin work or duplicates paid dispatch.
- Retry-After tests cover delay-seconds, HTTP-date, malformed/past/missing values
  and final relay handoff. Use controlled fixture time/events, not slow sleeps.
- Exercise slow progress versus true inactivity, unrelated concurrent progress,
  waiter disconnect, owner termination and startup while discovery is degraded.
  Listeners and usable catalogs stay available; no signature/funding is needed
  merely to try another origin circuit.
- Integrate factory race/HTTP/2 evidence through real discovery consumers,
  including deferred/pre-acquired clients, cache eviction and policy separation.
  Use owning relay/pricing unit fixtures, `tests/pricing.rs`, `tests/startup.rs`
  and relevant deployment warm/deferred tests; choose actual cases for the change.
- Update configuration, network, both resource inventories and testing guidance
  with delivered discovery activation, added latency, retained selection lifetime,
  unchanged relay financial policy and no guaranteed exit-IP diversity.

### Gate G1 — Shared transport and discovery, after T3

Cumulatively review T1–T3 and every fixup with unchanged relevant consumers, not
just the discovery diff. Trace startup/warm/inspection → catalog/pricing caches
and coalescing → selected transport/retry/promotion → relay continuation/payment.
Review excluded HTTP/gRPC callers for accidental enrollment. Exit requires:

- Evidence for all T1–T3 acceptance cases tied to exact reviewed SHAs, including
  real fixture HTTP/2/SOCKS separation, sibling progress, stale-record races,
  eviction/restart freshness and discovery success/promotion boundaries.
- No hidden winner/history registry, deterministic rollback, policy mixing,
  orphaned work, duplicate I/O, changed relay spending authority or startup gate.
- Original-baseline CogC/CycC comparison and explicit dispositions for new/worsened
  warnings and unchanged high-risk factory/discovery/startup hotspots. Evaluate
  simpler ownership/control flow, not helper extraction solely to lower scores.
- Root-cause review of all prior fixups and repeated assumptions; add regression
  coverage, simplify or expand the audit when patterns warrant it. Material
  findings are resolved and independently re-reviewed before exit.
- Permanent guides accurately distinguish delivered discovery from pending API
  behavior; focused test counts/results and remaining evidence limits are clear.

T4 must not build on unresolved material transport or discovery findings.

### T4 — API and cover transport ownership without retry activation

Adapt `PaidClient::execution_transport` and its downstream consumers to carry the
coherent transport record through unsigned dispatch, admission, journal/signing,
signed dispatch, response consumption and output conversion. Keep API acquisition
legacy and extra unsigned attempts disabled in this commit; T5 switches enrollment
and status handling together. Preserve static/managed payer selection, pre-signing
handoff, existing structured-insufficiency traversal and qualification accounting.

Add effective scope to `cover::registry::Owner` separately from canonical payer
identity. Thread matching records through engine/session/episode ownership and
cleanup, including legacy-compatible scopes. Different scopes cannot reuse
incompatible qualification or retained clients. Optional cover failures must not
become payment readiness gates. Keep accepted owner lifetime and no-replay
boundaries intact; do not change financial state formats or add ownership tables.

Inspect generated MCP calls, direct client callers, directory calls, raw GET/POST,
unsigned-only qualification and internal paid relay calls. They must retain their
existing request/signature counts. Cover attached to a participating transport
can follow that handle without independent retry/promotion authority.

Acceptance evidence:

- Real static/managed `PaidClient` GET/POST fixtures retain exact business bytes,
  payer, legacy scope and the same transport from challenge to signed dispatch.
  Generic unsigned 400/403/429/503 still dispatch once with no new enrollment.
- Exercise different effective scopes in cover fixtures to prove incompatible
  sessions/qualification cannot merge. Retained old sessions finish after record
  replacement/eviction; new acquisition can attach only to its matching scope.
- Exercise cover unavailable/draining/qualification failure and cleanup alongside
  active calls: ordinary payment remains available and unrelated progress cannot
  renew another response or signing authority.
- Preserve cancellation/restart financial ownership, journal-before-disclosure,
  insufficient-funds traversal and pre-signing payer handoff. Check forbidden new
  funding/allocation/reconciliation waits and duplicate signatures remain absent.
- Run affected payment ownership, handoff, best-effort and cover runtime/protocol
  fixtures plus relevant `tests/payments.rs`/`tests/managed.rs` cases. Internal
  relay tests retain their breaker, attempt and financial accounting behavior.
- Update architecture, cover and network guidance for delivered coherent-handle
  ownership and explicit pending API enrollment. No claim that T5 retries exist.

Exit: independent exact-SHA and substantive complexity review of the full
payment/cover handle chain passes before enabling T5. Material ownership findings
must be fixed and independently reviewed, not deferred to the cumulative gate.

### T5 — Unsigned API status retries and retained promotion

Enroll ordinary x402 API calls in participating Tor acquisition and implement the
400/403/429/503 unsigned retry sequence using T2's mechanism and T4's pinned
handles. This covers generated MCP tools, static/managed payers, directory calls
and source-management `raw_get`/`raw_post`. Internal paid discovery relay calls
remain explicitly excluded, preserving their own unsigned recovery/attempt
accounting. Unsigned-only qualification retains challenge refusal and current
authority checks, including after Retry-After waits.

Pin the payer within each unsigned sequence. Changing circuits cannot switch
wallets, allocate/fund, wait for balance reconciliation or reacquire application
ownership. Promote a valid accepted unsigned 402's record before normal payment
handling; retain that exact record through signed dispatch and response handling,
even if another caller promotes/evicts meanwhile. A complete successful unsigned
output can also promote; malformed 402, failed output, cover-only success and
signed outcomes cannot. A lost promotion race does not fail/repeat the call.

Preserve payer handoff: a newly selected payer obtains its own challenge. Each
separately authorized next-wallet stage after structured signed insufficiency may
use its own finite unsigned circuit sequence; generic signed responses cannot
start one. Every earlier authorization remains potentially chargeable. Drop
failed unsigned responses and release their cover leases without closing old
connections or forcing provider-wide cover resets. Account for additional cover
requests rather than claiming API attempt counts bound total traffic.

Acceptance evidence:

- Real PaidClient status matrices cover each eligible unsigned status before 402
  and free success, success at each ordinal, exhaustion, ineligible statuses,
  transport failures outside scope and unchanged direct mode. Verify exact POST
  body/query/method and pinned owner; GET and POST use the same eligibility.
- After valid 402 or completed unsigned success, subsequent logical calls use
  the winner. Same-wallet other-origin calls retain their record/cover; discovery
  success cannot migrate API traffic. No promotion follows malformed/partial or
  rejected output, paid results or relay results. Cache-only hits send no probe.
- Interleave valid 402 with replacement/eviction before signing: signature and
  response collection use its exact original record and matching cover. Repeat
  factory race/eviction/restart tests through actual API consumers and sessions,
  not solely helper tests. Retain old active HTTP/2 streams during promotion.
- Eligible unsigned failures produce no signature, journal liability, allocation
  or funding. Signed versions of all four statuses, uncertain submission,
  stalled/interrupted paid bodies and output failure produce no generic replay
  or new authorization. Accepted work survives waiter disconnect and restart
  retains original liabilities without replaying the logical call.
- Combine structured insufficiency, payer handoff and circuit retry in one flow;
  assert exact request/signature counts, captured eligible-wallet traversal,
  retained charges, independent reconciliation and qualification authority.
- Test Retry-After and unrelated concurrent progress through API ownership;
  owner termination cannot dispatch a later attempt. Cover failure/retirement
  stays optional, old sessions drain, new scopes requalify as needed, and internal
  relay behavior/financial breaker remains unchanged.
- Exercise generated MCP and raw/directory entry points with owning unit and
  relevant payment/managed/discovery integration fixtures. Record actual cases,
  nonzero counts, exact side-effect assertions and source-matched results.
- Update architecture, network, cover, source-management, wallet and testing
  guidance for delivered API retries and financial boundaries. Update both
  resource inventories for added attempts/scheduling; preserve unsigned POST
  semantics and existing paid polling/submission restrictions.

### Gate G2 — Combined transport, discovery and payments, after T5

Cumulatively review T1–T5 and all fixups, including G1 concerns and unchanged
consumers. Trace generated MCP/raw/directory → accepted owner → selection →
unsigned retries/cover → admission/journal/signing → signed response/output, plus
discovery → relay and all excluded infrastructure paths. Exit requires:

- Exact-SHA evidence for T4/T5 and source-matched earlier evidence or justified
  targeted revalidation after later changes. Reopen G1 concerns touched by shared
  transport/cover edits; per-commit approval never exempts them from this gate.
- No generic signed replay, lost liability, owner bypass, duplicate active I/O,
  synchronous funding gate or scope mismatch. Validate stage-based authority for
  both GET/POST and every recognized-insufficiency/payer-handoff transition.
- Winner reuse, stale-promotion rejection, independent provider scopes, coherent
  challenge/signing/cover handles, old stream/session survival and fresh-on-miss
  behavior proven through complete application paths with finite cache retention.
- Original-baseline complexity review of the combined implementation and full
  call chains, including unchanged high-risk payment/cover hotspots. Explicitly
  evaluate simpler representations before retaining complex coordination.
- All T1–T5 fixup causes, repeated assumptions and coverage gaps reviewed together;
  resolve material findings with regression coverage and independent review.
  Widen the audit or simplify the design when recurring patterns justify it.
- Maintained guidance matches delivered behavior; no claim of live circuit,
  settlement or exit-diversity qualification follows from local fixtures.

T6 must not build on unresolved material G2 findings. No cleanup or completion
declaration while material findings remain open.

### T6 — Base RPC isolation recovery within each configured provider

Enroll `src/rotation/base.rs` and `base/transport.rs` in the same selected-record,
fresh-on-miss, candidate and conditional-replacement machinery used by discovery
and API calls. Do not create a blockchain-specific HTTP factory or winner map.
Replace retained raw clients with coherent acquisition at the existing logical
read boundary; later reads must see a promoted record, while active requests keep
theirs. Preserve wallet identity for address reads and discovery-origin identity
for chain/header reads. There is no x402 challenge, payment or paid relay fallback.

Keep the existing read-only allowlist: `eth_chainId`, `eth_getBlockByNumber`,
`eth_getTransactionReceipt` and `eth_call` with their existing constrained uses.
Preserve exact endpoint URL, method, parameters, block hashes/tags and canonical
flags through a retry. No transaction broadcast, wallet allocation, funding or
additional maintenance demand is authorized by connectivity recovery.

**Ordering and retry ownership:** for each logical read on the currently selected
endpoint, use its selected transport, then at most two fresh scopes for eligible
400/403/429/503 or typed transport failures. Only after that finite sequence fails
may the existing outer provider fallback advance to the next configured endpoint,
where the same policy applies. Do not sweep all endpoints before rotating, restart
an entire multi-RPC view on every circuit change, or add a new outer retry round.
A full chain view contains multiple logical reads; three transports per read is
not a promise of only three total requests per provider view.

For participating Tor reads, replace the existing two-attempt same-client retry
and its 200 ms backoff with this one sequence; do not multiply them. Honor valid
Retry-After on eligible responses before retrying the same target, with no new
cooldown, health state, readiness gate or whole-work deadline. Exhaustion adds no
wait before an independent configured endpoint. Direct mode keeps its existing
same-client retry behavior. Existing endpoint-failover eligibility remains owned
by each caller: ordinary `current_balance` and strict view/receipt verification
currently differ. In particular, adding circuit eligibility for HTTP 400 must
not silently broaden strict endpoint-failover classification. If that caller does
not authorize endpoint fallback for the final error, return it after exhaustion.

TLS trust, policy, size, invalid envelopes/results, wrong-chain responses and
canonical/evidence conflicts gain no new circuit retries. Existing authorized
fallback to another configured endpoint still applies. Validate method-specific
results before promotion (Base chain ID, block/quantity/contract/receipt structure
as applicable). A valid zero balance or null/unconfirmed receipt remains an ordinary
result/pending observation, not a failure or reason to rotate. Do not add a complete
financial proof/readiness gate solely to promote a successful transport.

When changing provider endpoints, preserve complete fresh chain-view acquisition,
EIP-1898/pinned reads, canonical rechecks and endpoint provenance. Do not mix evidence
from different providers or confuse transport scope with durable financial
`endpoint_generation`. Provider ordering/evidence uses the full configured URL;
transport selection retains the shared owner/origin/policy key, so endpoints on
the same origin with different paths can intentionally share a selected transport
while retaining distinct evidence identities. Never log credential-bearing URLs.

Retain existing query/RPC permits, shared/coalesced scalar ownership, cancellation
semantics and observation cadence across each finite sequence. New scopes do not
create new maintenance owners, per-scope balance caches, faster polling, freshness
or liability authority. Success replaces only the relevant canonical selected
record; unrelated wallet/provider scopes remain independent. Losing a promotion
race cannot repeat or fail a successful RPC read.

Acceptance evidence:

- Real BaseRpc fixtures exercise each allowlisted read and eligible status/typed
  transport failure through fake SOCKS. Verify selected → fresh → fresh ordering
  within endpoint A before endpoint B; test success at each position, exhaustion,
  final return and next-provider recovery with exact request/connection counts.
  Prove the former two-attempt loop is not nested around the new sequence.
- Test caller-specific endpoint-failover decisions after exhaustion, including
  HTTP 400 in scalar versus strict view paths, and existing statuses/RPC error
  codes that may fail over without circuit retry. Direct behavior stays unchanged;
  invalid/write methods fail before I/O and no generic JSON-RPC replay is added.
- Assert exact repeated payloads and successful-result validation before promotion.
  HTTP 200 with wrong chain, malformed result/envelope, invalid receipt, TLS or
  oversized response cannot promote or earn circuit repetition. Valid zero/null/
  unconfirmed results neither allocate nor trigger another read automatically.
- A later same-key read uses the winning record. Verify separate wallet/discovery
  identities, other origins and policies, same-origin URL aliases with distinct
  evidence provenance, stale-promotion races and fresh-on-eviction/restart behavior.
  Retained raw clients must not bypass the selected record.
- Fail midway through a strict view and recover on another provider: require a
  complete new view, original pinned-hash/canonical validation and no cross-provider
  evidence combination. Transport promotion never releases payment liabilities,
  changes financial endpoint generations or treats unknown settlement as unused.
- Exercise shared concurrent balance reads, a disconnected waiter and retained
  owner, canceled/terminated operations, permit ownership and unrelated progress.
  Slow response data remains valid; true inactivity can recover without new whole-
  work timers or doubled resource acquisition. Retry-After affects only the target.
- Keep an HTTP/2 sibling stream active during recovery/replacement; verify separate
  SOCKS auth/connection, uninterrupted old work and subsequent winner reuse. Assert
  no Curl relay, payment signing, funding/allocation or extra observation demand.
- Extend `src/rotation/base/transport/tests.rs`, owning Base view/evidence/receipt
  and shared-read fixtures, plus affected network/managed/treasury-consumer tests.
  Verify nonzero executed cases and actual negative-side-effect assertions. Reuse
  unchanged discovery/API evidence only where hashes and dependencies still match.
- Verify qualification consumers use private, session-bound observations of actual
  fresh scope credentials and canonical owner/origin association, rather than
  predicting random scopes from deterministic owners. Preserve independently
  derived permissions, actual-credential circuit separation, strict unknown-scope
  refusal, archived interval re-audits and standalone receipt observation. No normal
  serving history or new authority; no live qualification claim from fixtures.
- Update network (caller inventory and Base retry behavior), configuration Base
  policy, architecture, wallet observation/reconciliation, both resource inventories
  and testing guidance in this commit. Document endpoint-first ordering, exact
  attempt scope, retained selection lifetime, unchanged endpoint diversity and
  absence of relay fallback. Keep Zcash explicitly unimplemented.

### Gate G3 — Complete HTTP batch including Base, after T6

Cumulatively review T1–T6 and every fixup, revisiting G1/G2 concerns alongside
unchanged consumers. Trace Base maintenance/qualification/treasury consumers →
retained query/coalescing owner → provider fallback → logical RPC → selected
transport/retry/promotion → method validation/evidence publication. Also retrace
catalog/pricing → relay and API → cover/journal/signing after shared changes.
Exit requires:

- Exact-SHA T6 evidence and source-matched prior validation, or targeted reruns for
  affected shared changes. Verify endpoint-first counts, no stacked retry layers,
  caller-specific failover decisions, no paid relay and unchanged direct behavior.
- No mixed-provider evidence, wrong-chain acceptance, durable endpoint-generation
  changes, liability release, owner bypass, polling acceleration or new readiness
  gate. Promotion success is validated connectivity, not financial proof.
- Shared record/cache rules remain unified across discovery, API and Base with no
  new history map, credential leak, client-clone bypass or cross-owner migration.
  Confirm race/eviction and HTTP/2 sibling progress through real Base call chains.
- Review original-baseline CogC/CycC for combined HTTP/financial call chains,
  including unchanged high-risk Base transport/view/receipt/shared-owner functions.
  Evaluate simpler state/control flow and explicitly disposition relevant warnings.
- Examine all prior fixup root causes, recurring assumptions and coverage gaps;
  resolve findings with regression tests, simplification or a wider audit as needed.
  Independently review fixes and reopen affected prior gates before exit.
- Owning guides accurately describe all three delivered integrations. Zcash remains
  deferred, existing gRPC/submission behavior unchanged, and no local fixture result
  is represented as live circuit or settlement qualification.

No cleanup or merge readiness while material G3 findings remain open.

### T7 — Permanent documentation handoff and plan removal

Verify delivered rules already live in their owning guides from T1–T6; this is
verification and cleanup, not deferred documentation implementation. Ensure
AGENTS.md links to unsigned semantics, network policy and complexity review remain
accurate. Confirm selected-entry replacement, full-key/provider separation,
fresh-on-miss lifetime, pinned financial/cover ownership and retry exclusions
are explicit in maintained guidance and the corresponding testing destinations.

Update existing GET/POST and deferred polling plans only where they contradict
delivered behavior; preserve paid submission/polling no-replay rules. Remove this
plan only after discovery, API and Base phases and G1/G2/G3 acceptance are complete,
fixing incoming links in the same commit. If concrete work remains, retain a focused unfinished
plan instead. Preserve the independent unfinished [Zcash extension](deferred/zcash_isolation_retries.md)
and its post-merge dependency when removing this plan. It must not link back to a
deleted completed plan; hand off prerequisite references to merged code and the
maintained guides. Zcash work does not block this batch's completion. Do not retain
a completed archive or erase financial evidence.

Acceptance evidence: check maintained rules against actual constructors/callers,
validate changed local Markdown paths/anchors and search for dangling plan links,
stale commit/gate references or proposed-versus-delivered contradictions.
Verify no runtime/source changes are hidden in cleanup and previous validation
artifacts match unchanged sources. Documentation-only cleanup needs no redundant
Rust rebuild; any runtime change reopens affected checks and reviews. Independently
review the exact cleanup SHA after committing, including any cleanup fixup.

### Gate G4 — Final integration, after T7

Review the final combined branch and exact cleanup SHA independently, including
all T1–T7 commits/fixups and relevant unchanged call chains. This gate remains
necessary after the plan file is removed; its requirements must remain available
to the reviewer through the branch/review description. Exit requires:

- G1/G2/G3 findings resolved, exact fixups reviewed, and later changes assessed for
  reopening earlier financial, concurrency, network, complexity or documentation
  concerns. Review recurring fixup causes across the entire branch again.
- Permanent documentation/AGENTS links and tests fully hand off enduring rules;
  no dangling plan links, completed-plan archive or unsubstantiated qualification.
- Exact final SHA identified, source-matched validation and complexity evidence
  available under ignored/private storage, and remaining limitations stated.
- No unfinished required implementation/validation. Material findings receive
  introducing-commit fixups, independent review and affected gate revalidation
  before completion; cleanup never grants replay or erases financial evidence.

## Review and validation ordering

Before substantive implementation, give an independent read-only design reviewer
the user intent, accepted POST semantics, baseline, this plan and preserved
invariants. Ask them to challenge unnecessary gates, state and enforcement, not
merely approve the proposed arrangement. Parallelize distinct review concerns
where useful; coordinate edits explicitly and freeze sources/executables during
validation to avoid competing builds.

Independently review every exact committed SHA, including T7; reviewers must not
be authors. Address findings with `git commit --fixup=<introducing-SHA>`, review
each exact fixup independently and preserve unsquashed history through final
integration review. At each gate inspect all prior fixups in surrounding call
chains: identify root causes, repeated assumptions, hazards and coverage gaps;
add regression coverage and decide whether wider audit, simplification or design
change is needed. Prior per-commit approval is not gate approval.

Capture an agreed original-baseline complexity report with the pinned analyzer
before substantive Rust edits. For every substantive Rust refactor (including
T1/T2/T4/T6) and at G1/G2/G3, compare changed functions and full call chains with:

```sh
python3 scripts/complexity.py --base 970f6bde1334eb42a540288ea1432381e1b9ae64
```

At G4 reuse verified artifacts only where source hashes still match; otherwise
revalidate affected changes. Use local fixup comparisons where needed. Explicitly
disposition relevant CogC/CycC warnings and unchanged high-risk hotspots after
evaluating concrete simplifications; helper extraction or passing tests alone is not justification.
Retain reports under ignored `target/`, per the complexity guide.

Run focused default-Zcash tests through `scripts/zcash.sh`, check that each named
financial test is wired and executes nonzero cases, and run formatting plus
required checks appropriate to changed modules. Do not run no-default-feature
configurations, live funded calls or Docker consensus tests without separate
explicit authorization. Use existing network/relay/payment/startup fixtures and
record exact commands/results for reviewers. No broad rerun after a localized
fix unless changes or unresolved concerns justify it. Fake SOCKS evidence proves
credentials and routing policy, not actual Tor circuit/exit selection; any claim
about real circuits needs separately enabled controlled Tor observation.
