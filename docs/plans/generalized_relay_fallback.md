# Generalized unsigned-read fallback

## Intent, baseline and scope

Give supported public reads a consistent origin-retry/relay-fallback path. Extend
fallback to agent-added OpenAPI acquisition and refresh, provider help, explicit
usage documents and discovery probes, and unsigned public API/directory GET/HEAD
responses, plus otherwise-authorized unsigned x402 POST requests that a configured
relay can faithfully represent. Expose the Tor isolation retry count in `[network]`. Keep agent-facing
help focused on capabilities and inputs, without relay, retry or payer machinery.

Starting baseline: `a59770f107d8cb64f7df64b82d6554343b233ade` on
`coinbase_discovery`. Current fallback covers static catalog/pricing discovery and
Coinbase search; dynamic imports, help and other API reads do not share it. The
current Tor count is hardcoded to three total attempts in the common network
factory. Static relay state includes caching/readiness/breaker responsibilities;
Coinbase has a separate on-demand exchange. These are starting facts, not the
proposed architecture.

Provider additions in `962ffd50` subsequently supplied AGProxy and Horizon Pulse
definitions and reviewed their public request/response contracts. They did not
change the Curl-only runtime. The implementation baseline includes that commit;
retain the original network/payment baseline above for accumulated comparisons.
The method-list design below supersedes the earlier single-Curl assumption.

User review authorized implementation after the design and clarity reviews through
`23c7e450`. Implementation uses offline/local fixtures and public unfunded keys. It does not authorize live paid calls, accessing real wallet secrets,
stopping the live server, or modifying unrelated local files. Preserve `TODO`,
`discovery_confusion.md`, both `privacy.local*.toml` overrides and `run_servers.sh`.

Owning guides: [network policy](../network-egress.md),
[configuration](../configuration.md#paid-discovery-relay),
[agent sources](../agent-sources.md), [runtime architecture](../architecture.md),
[unsigned x402 semantics](../architecture.md#unsigned-x402-requests-and-post-retries),
[best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[wallet observation policy](../wallet-rotation.md#wallet-observation-policy),
[resource policy](../local-resource-limits.md), [testing](../testing.md),
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline) and
[development](../development.md). AGENTS.md already links these owning guides;
add a link only if an enduring contract gains a new owner.

## Preimplementation contract

### Configuration and reach

- Add `network.tor_isolation_retries = 2`: the number of fresh isolation attempts
  after the initial attempt. Zero means one origin attempt, not disabled fallback.
  Permit representable nonnegative values, rejecting arithmetic/representation
  overflow rather than imposing a new arbitrary cap. Effective inspection exposes
  the value. Direct mode and explicit direct warming retain one origin attempt.
- All participating Tor callers use this count: static catalogs/pricing, Coinbase,
  dynamic imports/refresh, help/documents/probes, unsigned API/directory calls and
  Base read-only RPC recovery, and eligible unsigned requests to a relay service
  itself. Disabling recursive fallback does not disable the relay leg's authorized
  Tor origin retries. Base retains its existing endpoint/method validation
  and separate direct behavior; it does not acquire a relay fallback.
- Use one `[discovery_relay].on_demand` switch, default true when the relay is
  configured, for all supported runtime reads. Replace `search` with a compatible
  input alias rather than adding overlapping flags. Both spellings together are
  a duplicate-field error. Inspection emits `on_demand`. Legacy `search=false`
  disables all on-demand fallback, including the newly supported reads; document
  this operator-facing expansion. `serve`/`warm` keep their static-acquisition
  meanings. No relay section means no paid relay. No per-tool fallback arguments.
- Preserve network-file whole-section replacement, omitted-relay preservation and
  declaration-relative provider paths. Standalone provider commands and wallet
  administration continue to accept the common network file without gaining
  deployment relay support or loading unused relay providers.

### Method-specific ordered relay lists

Selection is operator configuration, not a vendor-name rule. Use an ordered list
per original target method. The proposed Tor example places cheaper Curl before
AGProxy for GET and uses AGProxy for POST/HEAD; Horizon Pulse can be appended or
selected by the operator. Do not switch defaults in advance of D02 validation.

```toml
[discovery_relay]
serve = true
warm = true
on_demand = true

[discovery_relay.fallbacks]
GET = ["../../providers/curl/provider.toml", "../../providers/agproxy.toml"]
POST = ["../../providers/agproxy.toml"]
HEAD = ["../../providers/agproxy.toml"]
```

- Method keys are exactly `GET`, `HEAD` and `POST` in this scope. Reject unknown
  keys before I/O. An absent or empty list means origin-only for that method;
  there is no GET-to-HEAD/POST inheritance or implicit extra provider. Select using
  the original unsigned target request, never the relay service's outer POST.
- Each list contains provider TOML paths only. The provider definition declares
  its optional `relay_format` once (`curl`, `agproxy` or `horizonpulse`), resolved
  through normal provider configuration. No per-entry `adapter` field or repeated
  protocol label. These formats differ; do not infer them from a host/name/OpenAPI
  shape or autodetect an envelope after paying. Listed providers with missing or
  unsupported formats fail bootstrap before I/O; ordinary providers need no field
  unless used as relays. Keep format metadata out of agent tool descriptions and
  input schemas.
- Fallback paths resolve relative to their declaring deployment/network file;
  a provider's schema/help paths resolve relative to its own TOML. Whole-section
  override and offline inspection semantics remain unchanged. Config-only
  inspection reports lists without loading relay catalogs or requiring missing
  provider files to exist. Runtime local bootstrap validates selected formats,
  operations and filters before requesting payment.
- Legacy `[discovery_relay].provider` desugars to one Curl entry in the GET list,
  preserving current behavior for old Curl definitions without `relay_format`.
  Validate the existing Curl operation/filters and reject conflicting format
  metadata; implicit Curl applies only to this legacy syntax. Reject combining
  it with `fallbacks`, including an explicitly empty map. Canonical inspection exposes the resolved method lists.
  Shared `serve`, `warm`, `on_demand`/legacy `search`, `wallet` and `sources` retain
  their meanings; do not add a switch per provider/consumer. `wallet` and `sources`
  remain static discovery overrides, not runtime cross-owner selectors.
- Lists govern eligibility/order. Curl appears only in GET in the proposed
  example; there is no hardcoded Curl-is-GET-only selection branch. Its documented
  bodyless POST could be explicitly listed if faithfully representable. Every
  adapter must still reject/skip unsupported request fields before paid dispatch:
  configuration cannot make a missing header/body feature work. Do not silently
  discard required fields. Changing a header/parameter does not authorize
  forwarding target payment signatures or other credentials to a new party.
- Keep adapter translation small: the shared sequence owns attempts and financial
  lifecycle; an adapter encodes its actual request and decodes its envelope.
  AGProxy supports GET/HEAD/POST and a string body; Horizon Pulse supports headers
  and string/JSON bodies. Preserve exact target bytes/query/required headers.
  Their filtered header contracts must be established for each required header;
  lack of evidence must not be treated as support for payment/auth headers.
- Bootstrap all configured adapters from local schemas without fetching another
  remote schema/help/pricing document. D02 pins complete vendor OpenAPI documents
  for AGProxy/Horizon Pulse as runtime assets, adds `relay_format` to the three
  relay provider definitions, updates their TOMLs/manifest, and
  retains existing filters/validation. This is an offline-bootstrap consumer, not
  an inventory fixture. Record primary provenance/update instructions and verify
  references cannot escape to the network. Their reusable catalogs become pinned
  until deliberately updated; do not add duplicate miniature provider definitions.
- Reuse the single configured origin attempt sequence, then walk the selected list
  in authored order. Never restart origin retries between relay entries. Each
  authored position has at most one logical acquisition per caller request, with
  its authorized unsigned HTTP attempts inside the shared Tor count; no separate
  health scheduler, cost scoring, vendor ranking, deduplication gate or new ledger.
  An unrepresentable entry is skipped before I/O, not counted as a purchased call.
  Actual request bytes may prove an external limit incompatible; a caller's larger
  maximum accepted response size alone does not prove the actual response too large.
- Advance only on a proven pre-payment provider failure, or a complete, attributable
  target result for which the consumer matrix below still permits fallback. The
  matrix applies throughout the list, including existing broader static remote
  HTTP/parse fallback. Explicitly validated relay-declared delivery truncation
  can advance when another provider may deliver complete content. A target's own
  unsolicited 206/Content-Range or a complete-but-invalid document retains its
  consumer's terminal validation rule. A normal terminal result does not
  advance: in particular API/doc-probe 404/410, accepted empty results and target
  402 remain terminal under their contracts. A local policy/authority/bounds failure
  does not buy another entry. Distinguish a valid envelope's unusable target body
  from malformed/unattributable relay JSON.
- A list authorizes additional distinct paid acquisitions after definitive failed
  delivery; it does not establish the preceding relay was uncharged. Delivered
  blocking/error/truncated responses may already cost money. Preserve each actual
  authorization and possible liability under the existing owner and ordinary
  per-payment caps; do not introduce a local cumulative spending counter.
  Any unresolved paid relay outcome, including an invalid/unattributable envelope
  after possible payment, stops the sequence. A proven pre-payment provider failure
  may advance; do not mistake ambiguous paid failure for this case. A refund
  promise, timeout or vendor “not charged” prose is not proof granting another
  payment. Never replay a signed target request.
- Cache/provenance and existing static failure state include adapter/provider
  identity and existing payer/source/request scope. One entry's cached result or
  target failure must not masquerade as another entry's response or disable an
  independent provider. Preserve alias coalescing and accepted-work ownership
  across the complete list; on-demand calls gain no sticky cache or global breaker.
- Remove the static relay's separate hardcoded pre-payment network attempt counter
  and one-second retry backoff when replacing it with the common unsigned Tor
  sequence: one unsigned attempt in direct mode, or one plus
  `tor_isolation_retries` in Tor mode, using participating selection while retaining
  signed-request no-replay. Do not multiply network retries in caller/state/adapter
  layers. Existing
  static payer-readiness waiting and successful result sharing remain ownership
  responsibilities; readiness without I/O consumes no network attempt. Document
  the resulting direct-mode behavior instead of silently retaining an extra loop.

### Shared request path and response contracts

- One shared unsigned acquisition path owns origin attempt ordering,
  transient-failure classification, complete-body transport failure handling,
  `Retry-After` and one traversal of the method-specific fallback list. Migrate the existing static and
  Coinbase paths as well as new callers; do not leave vendor-specific retry loops
  or wrap an existing retry loop in another loop. Base and the existing x402 POST
  path share origin count/selection/retry-classification primitives without being
  forced into a GET response abstraction. Unsigned x402 POST and GET have the same
  origin retry eligibility; unifying API transport failure handling must apply to
  both. A configured POST entry may relay an otherwise-authorized unsigned x402
  POST when it faithfully preserves its body/required headers. Unsupported Curl
  payloads skip that entry before payment; they do not restrict POST retries or
  the other adapters. No signed replay is authorized.
- Retry the expanded eligible HTTP statuses/transient transport failures on fresh
  owner-bound scopes, with unchanged certificate/hostname/public-destination checks.
  Honor valid server delays, including the terminal delay before relay handoff.
  Complete valid origin responses finish immediately, including valid empty or
  partial directory results. Do not retry merely because a search found nothing.
  Local destination/authority checks and download-limit failures never trigger a
  paid fallback. Generic API output/schema conversion failures are terminal, not
  reasons to repeat an otherwise successful read through another relay.
- Caller validation remains responsible for accepted statuses, document parsing,
  cache/revalidation, output mapping and publication. Share transport mechanics;
  do not impose catalog JSON rules on text help, HEAD responses or generic APIs.
  Separate adapter envelope validation from those caller contracts. Preserve existing
  static catalog/pricing fallback on remote HTTP/parse failures, with the explicit
  local-policy/download-limit exclusions above. Static catalog loading currently
  offers a continuation for broadly caught errors: classify those failures during
  migration so a known local rejection cannot purchase another unusable response.
- An ordinary 404/410 from a documentation probe is a terminal negative result,
  not a reason to buy the same negative result. Blocking failures on these probes
  do use the shared attempts and fallback. Preserve their existing candidate set
  and concurrency; do not add a probe spending counter or silently exempt probes.
  Replace the probe-only HEAD-then-GET sequence with one GET acquisition and
  validate that complete document. This removes a redundant request and avoids
  separate paid HEAD/GET calls for one candidate. Update its diagnostic-only
  `head_status` consumers/tests rather than inventing a HEAD result from GET.
  The shared primitive still supports ordinary API/raw HEAD with empty-body and
  header/status semantics. Do not introduce a new cross-call cache.
- Enforce each caller's public URL/allowed-origin policy on the requested and
  reported relay destinations, retaining transport permissions, exact query
  encoding and body bounds. Origin requests retain no-follow behavior. All three
  services follow redirects remotely. Curl reports a chain; reject disallowed
  chains. AGProxy/Horizon Pulse report only a final URL: validate that destination
  and explicitly document trust in their remote redirect/public-destination
  enforcement, without claiming to establish intermediate hops. The operator
  explicitly accepted those services following redirects during implementation
  review. Configured relay use therefore permits their remote redirects even
  though ordinary origin HTTP remains no-follow; check the requested and final
  destinations and disclose this distinction in the owning operator guides.
  The operator also explicitly rejected excluding adapters solely because they
  cannot provide redirect-chain evidence. Apply the caller's URL/allowed-origin
  policy to the requested destination, every reported Curl hop and each reported
  final destination. AGProxy/Horizon intermediate destinations remain unknown;
  their configured use accepts that limited control and visibility, including
  when an origin allowlist is present. Document the remote redirect-policy and
  plaintext-integrity trust boundary without claiming every-hop enforcement or
  adding a new flag/evidence gate. Relay responses never promote an
  origin Tor scope or populate origin
  validators/caches as if obtained directly.
- Eligibility requires a public unsigned request faithfully representable by its
  configured adapter. Curl's envelope carries text; AGProxy/Horizon Pulse have
  binary/base64 representations. Decode only documented formats and validate
  explicit truncation/encoding before success. Preserve legitimate generic API
  range semantics; complete-document readers reject unsolicited 206/Content-Range
  responses on origin and relay paths. Unsupported bytes/headers are never silently
  rewritten to fit a cheaper entry.
- The complete-document portion incorporates the focused
  [unfinished audit](deferred/document_response_completeness.md) for help/import,
  static catalogs and relay decoding. Preserve complete schemas/help and visible
  failures; never cache or publish a partial document as complete.

### Explicit origin retry conditions and current gaps

An origin retry means another request to the original target on a fresh Tor
isolation scope. It is distinct from the ordered paid relay sequence, normal x402
challenge/signing, and structured-insufficient-funds wallet traversal. A failure
can qualify for one of these without qualifying for the others.

The proposed common origin retry requires all of the following:

1. A participating Tor operation has an unused configured isolation retry. Zero
   extra retries, direct mode and an explicit direct-warming override do not earn
   a fresh Tor attempt. Base's preexisting direct transport recovery stays separate.
2. The operation still has authority to make that unsigned attempt. No generic
   retry is granted after target signing, after the logical operation actually
   loses authority, on local policy denial, or by resetting the counter when a
   helper/caller resumes. A disconnected waiter does not cancel retained accepted
   work or remove its existing authority. Qualification retains
   its existing per-attempt authority checks.
3. The failed attempt has either an eligible HTTP status or an eligible transport
   failure, as defined below. The HTTP method alone does not disqualify an unsigned
   x402 POST. A successful HTTP response failing later API output conversion is
   not a transport failure.

| Condition | Fresh Tor origin retry? | Meaning and examples |
| --- | --- | --- |
| HTTP 400, 403, 408, 429, 500, 502, 503 or 504 | Yes, while retries remain | Proposed set adds 408/500/502/504 to the baseline 400/403/429/503. These can reflect transient request/gateway/origin failures, although another exit cannot guarantee recovery. A remote 400 retains existing eligibility without guessing from upstream prose. Honor valid `Retry-After` on 429/503 before the next attempt and eligible relay handoff. Do not introduce a delay cap or broaden delay parsing implicitly. |
| TLS handshake timeout, reset, EOF, or other errors accepted by `network::tls_failure` | Yes, while unsigned and retries remain; then eligible relay fallback | Includes connect-budget expiry during TLS and the existing nested `rustls::Error`/fixed handshake EOF cases. Keep certificate/hostname checks on every attempt; trying a new scope does not disable validation. |
| Other errors accepted by `network::eligible_retry_transport` | Yes in the shared transport path | Includes eligible connection establishment failures, remote name-resolution failures, connection resets, response-header failures, read-inactivity timeouts and incomplete-body I/O. Classification examines the actual error chain, not just an error label. See the current API gap below. |
| Other 5xx, for example 501, 505 or nonstandard 520 | No fresh origin retry | These remain outside the explicit origin set; proposed on-demand fallback may still use the relay list after one origin attempt. Do not silently treat every 5xx as a new-circuit trigger. |
| HTTP 401, 404, 405, 410 or 422 | No | Missing credentials, absent endpoint, unsupported method, gone resource or validation rejection do not earn a new circuit. Static discovery can still have its explicitly preserved broader relay behavior. |
| HTTP 402 | No generic status retry | Direct-origin 402 enters normal challenge handling when authorized; a pricing probe may inspect it. It is neither evidence of a Tor blockage nor automatic authority for another signed attempt. |
| Origin redirect response/error | No | Do not follow an origin redirect or purchase another request merely because it redirected. Preserve static catalog body parsing/legacy fallback where specifically documented. Configured relays may follow redirects remotely under the explicit trust contract above. |
| Local request construction, destination/allowlist, authority, or byte-limit rejection | No | Examples: malformed URL, forbidden private destination, missing payer authority or exceeded `max_spec_bytes`. These also cannot earn the new paid fallback. |
| Hyper parse/user/canceled errors rejected by the shared classifier | No | Examples: a malformed HTTP response rejected as a protocol parse error or an internally canceled request. Do not relabel a parser/caller error as network instability. |
| Transient SOCKS target connection/remote-resolution timeout, reset or temporary failure | Yes, while unsigned and retries remain; then eligible relay fallback | A different Tor circuit/exit can help. Use the connector error chain and actual failure stage; do not confuse target refusal with refusal by the local SOCKS listener. |
| Timeout connecting to the configured local SOCKS listener | Yes, within the same configured count | The timeout may be transient, even though fresh target isolation cannot repair the local socket. No extra proxy retry loop/backoff. A relay also uses that SOCKS listener: it cannot bypass the outage or use direct networking. Existing startup reachability waiting remains a separate prerequisite, not another per-request retry loop. |
| Proven connection refusal by the local SOCKS listener | No; no relay handoff for this failure | The configured proxy is unavailable. A fresh circuit or another relay cannot repair that dependency. Do not infer local refusal from an unlocated connection-refused label. |
| SOCKS authentication/credentials rejection, wrong proxy protocol or invalid local configuration | No; no relay handoff for this failure | Permanent setup errors are not exit failures. Preserve authentication and remote DNS requirements; do not downgrade or bypass them. |
| Complete invalid JSON/schema/text, unsolicited partial document, or API mapping failure | No | A fully delivered malformed OpenAPI document, an HTML page instead of accepted usage text, or a valid HTTP 200 that cannot be mapped to tool output. Static catalog/coinbase parse fallback remains a separately specified exception, not an origin retry. |
| Any generic failure after signing the target request | No | A timeout, TLS error, reset, 503 or unreadable body after signing may already have charged/executed. A connect/TLS/timeout label is not proof of nondispatch and creates no new signed pre-send replay exception. Neither fresh isolation nor another relay can be used to replay it. Preserve only the separately authorized structured-insufficient-funds traversal. |

Connection setup is explicitly covered by the shared policy: TCP/local SOCKS,
SOCKS negotiation and target connection, remote DNS, then TLS. A failure reliably
identified before HTTP dispatch does not carry target execution uncertainty, but
it still requires existing unsigned authority and a classified transient cause.
Do not require dispatch-stage instrumentation merely to retry an already-eligible
unsigned GET/x402 POST. Where the existing connector loses the cause/stage, inspect
its concrete error types and preserve cause information at the smallest shared
boundary needed to distinguish local setup from remote transient failure. A
wrapper phrase such as “failed to create underlying connection” is not itself a
permanent cause. Do not guess stages from arbitrary upstream prose, introduce a
second connection scheduler, or change timeout budgets. Unlocated refusal must
not be silently classified as proven local refusal; preserve other shared
transport eligibility unless a concrete exclusion is established. If preserving
connector causes requires a dependency patch, follow the existing vendor
provenance/update guidance; do not add a separate connector implementation.

Baseline implementation differences must be visible in implementation review:

- The current HTTP origin set is exactly 400/403/429/503. D01 deliberately expands
  it to 400/403/408/429/500/502/503/504 for every existing participating caller,
  including Base and unsigned API status handling. Remaining 5xx retain the
  separate fallback rule; the count and expanded classifications ship together.
- The current shared transport classifier rejects a broad SOCKS wrapper containing
  “failed to create underlying connection”, as well as authentication/protocol
  markers. D01 replaces that blanket connection rejection with the explicit
  transient/local-permanent distinctions above. Some timeouts already pass the
  classifier depending on their error chain; this is not a claim that all setup
  timeouts are currently excluded. Local permanent exclusions also prevent relay
  handoff, including the historically broader static fallback catch paths in D02.
- Static catalog/pricing, Coinbase and Base already use the shared status/transport
  classification in their participating loops. Caller response validation remains
  separate; Base also has its own method/result acceptance and direct recovery.
- Ordinary `PaidClient` unsigned GET **and POST** currently use the shared status
  set but retry transport failures only when `tls_failure` matches. Ordinary DNS/
  connection failures, resets and read-inactivity/incomplete-body errors that do
  not match TLS are therefore currently terminal there, even if the broader shared
  classifier would accept them. D04 unifies those unsigned transport cases for
  GET and POST together; do not describe this as merely moving unchanged code.
- Dynamic OpenAPI import, lazy provider help and source-details document/probe
  reads currently use single-request legacy acquisition, so even an otherwise
  eligible 403/503 or transport failure does not automatically receive the common
  isolation sequence. D03 supplies that missing path.
- A general timeout/reset label does not prove retriability: phase, authority and
  concrete error-chain exclusions still govern. The shared classifier accepts
  remaining transport errors after its explicit exclusions; avoid narrowing it
  accidentally with a newly invented list of only the examples above.

The maintained [unsigned x402 POST policy](../architecture.md#unsigned-x402-requests-and-post-retries)
explicitly assumes protected x402 execution is gated on payment acceptance. Under
that accepted model, unsigned POST is retryable on the same otherwise-authorized
basis as GET, without an extra provider idempotency proof or side-effect gate.
This is not a claim that arbitrary non-x402 HTTP POSTs are side-effect-free, or
that signed requests may be repeated. Keep this rule in the permanent guide and
its affected tests; Curl's inability to represent a particular body/header
combination must not restrict origin retries of unsigned x402 POSTs or other
configured adapters.

### Failure/response matrix

The same transport engine implements these caller contracts; the distinctions
are response semantics, not vendor-specific retry loops or configuration flags.
In Tor mode the proposed retryable statuses are
400/403/408/429/500/502/503/504 (baseline: 400/403/429/503); other remote
failures below may hand off directly to fallback without extra origin attempts.
For the new on-demand readers, eligible connection/TLS/read failures and HTTP
400/403/408/429 or 5xx can earn fallback. Local SOCKS refusal, authentication and
protocol/configuration failures cannot earn fallback; a local connect timeout can
use the common count, but any relay leg still depends on the same Tor proxy.
Keep the existing TLS eligibility while retaining certificate and hostname
validation on every attempt. Direct mode can
hand off after its one origin attempt. Locally rejected or unsupported requests
never earn fallback.

| Consumer | Successful/terminal result | Other response treatment |
| --- | --- | --- |
| Static catalogs | Complete accepted JSON/304 revalidation | Preserve remote HTTP/parse fallback, including existing non-transient HTTP errors; no local-policy/bounds fallback. |
| Static pricing | Usable origin 402 or 2xx (unknown price); required-argument skip remains terminal | Preserve fallback for unusable/missing **origin** pricing and remote errors. Any relayed 402 is terminal (usable estimate or unknown); a 2xx unknown price does not buy another probe. |
| Coinbase search | Complete accepted 200, including empty/partial-result metadata | Preserve existing remote status/JSON-parse fallback; bounds/policy failures terminal. |
| Dynamic OpenAPI, help, explicit docs and document probes | Complete accepted document; 401/404/410 and other non-blocking 4xx are terminal | Blocking statuses/transient failures above can fall back. Complete invalid JSON/schema/text, unsolicited partial documents and unsupported content are validation failures, not reasons to pay for another read. |
| Ordinary API/directory/raw GET/HEAD and otherwise-authorized unsigned x402 POST | Any accepted complete 2xx, including empty and legitimate range responses | Blocking statuses/transient failures above can fall back. Other 4xx/redirects are terminal under existing response policy; valid 200 with invalid directory JSON or output conversion failure is terminal, without fallback. Origin 402 enters normal payment handling; relayed 402 is handled below. |

The engine must distinguish transport/body-read failure from complete-but-invalid
content and local limits with typed outcomes, rather than guessing from upstream
error text. Directory validation does not become an excuse to replay successful
HTTP delivery. Test 404/410, every status in 400/403/408/429/500/502/503/504,
other 5xx, complete malformed 200, 204/206, redirects, headers/body failures and signed-phase suppression explicitly.

### Payments, ownership and agent interface

- Direct-origin 402 keeps normal challenge/sign/retry behavior. A relayed 402 does
  not enter that signing path. The new adapters' generic header/body support does
  not authorize forwarding a target signature or negotiating target payment through
  the relay; header fidelity and financial ownership have not been established.
  Surface the unpaid target failure; do not sign it, try the blocked origin again,
  or claim the target API was paid. This limitation does not change static pricing
  probes, which can read challenge metadata without signing it. A relayed pricing
  402 is always terminal: record a usable estimate when available, otherwise unknown
  pricing; do not advance to purchase another probe. An unusable origin pricing
  402 retains its existing fallback eligibility.
- Once any target request is signed, no generic status/TLS/body/output failure
  authorizes relay fallback or another paid target attempt. Preserve the existing
  structured-insufficient-funds traversal and original accepted request inputs.
  A relay request uses the same existing payment engine under the authenticated
  endpoint/source's bound authority, with ordinary caps and journal-before-
  disclosure. No new wallets, funding waits, monetary ledger or retry authority.
- Static tool/help readers use their existing source-bound client even without
  source management. CLI source refresh uses the saved registration owner and its
  configured existing payer if fallback is necessary; do not load every listener
  key, allocate wallets or start funding to service a document read. Missing
  payment authority leaves origin retrieval available and fallback unavailable.
  Offline configuration and catalog inspection gain no payment authority.
- Keep origin target-payment evidence distinct from relay payment evidence. Carry
  possible charge/accepted-work ownership through the existing financial owner,
  including output failure, cancellation and shutdown. Do not mislabel a relay fee
  as a target payment or hide possible charges when reporting a failed operation.
- Reuse existing accepted-call and source-mutation ownership. Retain paid fetching,
  decoding and help-cache/source-publication completion after a waiter disconnects;
  replacing a request inside `OnceCell::get_or_try_init` alone is insufficient.
  Concurrent compatible help/import requests must not duplicate accepted paid I/O.
  Source removal/revision changes still fence publication. No replay on restart.
- Share a stateless relay sequence with small adapter encoders/decoders between on-demand and static
  consumers. Keep static cache/readiness behavior with its existing owner; do not
  spread its run-wide breaker/attempt counters to ordinary calls. Explicitly disable
  fallback on the relay's own payment client to prevent recursive relay calls.
- Agent-facing descriptions, schemas and routine results describe the requested
  function, accepted inputs and relevant output. Remove obsolete “never pay”/“No
  charge” promises from help/documentation tools without replacing them with Curl,
  isolation, wallet or retry explanations. Keep existing general paid-call
  authority/uncertainty guidance where needed for correct agent action. Detailed
  mechanics belong in operator guides and sanitized stderr diagnostics. Preserve
  discovery choices, local BM25, endpoint-local IDs/scope, registration availability,
  full schemas and the earlier discovery-confusion fixes.

## Preimplementation and clarity gate

Independent design review has challenged the baseline and identified the material
boundaries above: relayed 402 must not reach direct signing, envelope decoding must
be separate from caller parsing, and accepted paid fetches must survive waiter
cancellation through publication. It also reviewed the generic `on_demand` alias,
negative-probe handling and common Tor count. These decisions are part of the
contract, not deferred implementation guesses.

Before D01, independently review this exact plan for clarity and design: identify
all caller integrations, operator defaults/aliases, origin/fallback triggers,
financial evidence, complete-document checks, unsupported requests and ownership
boundaries. Exit requires no unresolved material findings and a concrete lean
implementation approach. The user reviews this plan first. Reopen the affected
contract/gate when implementation evidence requires a material change.

Planning gate status: independent design and clarity reviews passed after resolving
the failure/response matrix and GET/HEAD scope. Retry conditions/unsigned POST
clarification also passed review. The method-specific ordered provider lists and
provider-owned format metadata passed renewed independent design and clarity
review. The expanded HTTP/setup-failure policy also passed renewed independent
design and clarity review, including the connector cause-retention approach.
User review authorized implementation beginning with D01. Material changes reopen
the affected concerns.

## Implementation status

D01 is implemented and validated; independent exact-SHA review approved
`8efe79e6` before D02 began. The configured count, expanded classifications,
zero-retry body handling and connector-cause preservation are in the owning guides.
Focused offline/local tests, formatting, all-target Clippy and exact-baseline
complexity analysis passed. The unsigned API transport classifier remains TLS-only
until the D04 integration, as specified above.

The operator accepted the relay redirect/visibility limitation during implementation;
renewed independent design review passed that explicit contract. It grants no target
signing, credential forwarding or uncertain-payment replay authority. D02 implements shared acquisition, ordered adapters and static/Coinbase consumers.
The affected library union passed 151 tests with one readiness-admission failure;
the correction then passed all 45 relay tests, including that unchanged regression.
Configuration/CLI validation, exact-SHA review and G1 remain pending. Method-list
examples do not establish live qualification.

## Ordered implementation commits

1. **D01 — Configurable shared Tor isolation count and transient classification.**
   Add the network setting, defaults/inspection and validation; replace the hardcoded count for every
   existing participating consumer together, including Base. Cover omitted/zero/
   positive values and overflow, whole network-file overrides, direct/pinned
   operations, zero-retry body handling and Base's existing endpoint-first behavior.
   Expand the common status set and classify connection-setup failures as specified
   above in this same commit; preserve other consumer response/financial contracts.
   Exercise TLS handshake timeout/reset/EOF, transient SOCKS target failures, local
   SOCKS connect timeout, proven local refusal, authentication/wrong-protocol errors
   and ambiguous wrapper errors using faithful connector chains and socket fixtures.
   Verify counts at 0/1/default, no multiplied attempts, unchanged verification and
   no direct escape. Preserve Base endpoint-first/direct behavior and the separate
   startup SOCKS prerequisite. Freeze expanded status/transport expectations in
   shared tests; D04 still supplies the broader API transport integration.
   Update `docs/configuration.md`, `docs/network-egress.md`, both timer/resource
   inventories and `examples/network/tor.toml` with the count semantics, expanded
   status set and connection-failure distinctions in their owning guides. No new
   total-work deadline, response timer, readiness gate or local cap.
2. **D02 — Shared unsigned acquisition and relay exchange.** Establish one origin
   retry/handoff path, method-specific ordered lists, and adapter envelopes with
   separate caller validation;
   migrate static catalog/pricing and Coinbase without parallel old loops. Separate
   stateless exchange from static scheduling/cache ownership and prevent recursion.
   Add adapter bootstrap assets/configuration, legacy-single-provider migration,
   method-list inspection/declaration-relative paths, provider-owned format
   metadata and the proposed example order. Keep format metadata out of agent help.
   Replace static relay's extra pre-payment network counter/backoff with the common
   retry policy in this same commit; preserve readiness/accepted-work ownership.
   Preserve current `search` configuration until D03 replaces it with its consumers.
   Cover complete progress versus stalls, retries/body failures/delays at counts
   0/1/default, valid empty results, cancellation, limits, trust/redirect handling,
   no relay scope promotion, cache/304 provenance, and static accepted-work drain.
   Test expanded statuses/setup failures through handoff: exhausted origin retries
   precede one list traversal; relay unsigned legs use the same classifier/count;
   local SOCKS refusal/auth/protocol errors do not purchase another provider, and
   local connect timeouts never bypass Tor. A paid relay timeout remains uncertain,
   even when labeled connect/TLS; no signed pre-send replay exception is introduced.
   Cover GET Curl-before-AG order, separate POST/HEAD lists, missing/empty lists,
   unknown keys/missing format/conflicting legacy metadata, no implicit vendor
   selection or repeated per-entry adapter fields, faithful body/header/byte encoding,
   offline bootstrap, explicit truncation, definitive paid-error advancement and
   ambiguous-payment stop. Test no repeated origin attempts or recursive fallback,
   and no cross-adapter cache/failure pollution.
   Correct static complete-document acceptance coherently with this migration.
   Update network/configuration/architecture/resource guides and affected tests.
3. **D03 — Generalize discovery and help reads.** Introduce `on_demand` with the
   legacy alias and integrate source add/refresh, provider help, explicit usage
   documents and discovery probes through the shared path, using existing bound
   authority and retained owners. Keep complete-document checks and publication/
   cache ownership with their consumers. Cover alias/default/duplicate-key behavior,
   scope/allowed-origin rejection before I/O, negative probes, partial/truncated
   documents, complete schemas/help, concurrent requests, cancelled waiters,
   source removal/revision fencing, and absence of new wallet/funding side effects.
   Remove obsolete free-fetch promises and internal mechanics from affected agent
   descriptions/result notes in the same commit. Update `docs/agent-sources.md`,
   operator configuration/network/architecture guides and affected examples.
4. **D04 — Ordinary unsigned API and directory reads.** Wire supported public GET
   and HEAD responses, including x402 List browse/best/details and raw GET/HEAD
   calls and supported unsigned x402 POSTs, through the same method lists while
   preserving normal origin 402 signing and existing unsigned POST authority.
   Apply the common unsigned transport retry classification to API GET and POST
   together; the existing TLS-only transport handling is an explicit
   baseline gap. Cover exact query/response/output mapping, supported text MIME/status,
   per-adapter unsupported body/headers/binary handling, relayed 402 without target signing,
   eligible and rejected transport errors for unsigned GET/POST, nonretryable
   statuses such as 401/404/405/410/422 and other-5xx fallback without fresh
   origin retries, all newly retryable statuses/setup failures, signed failures
   without fallback/replay, relay recursion exclusion, managed/static
   payer bindings, payment uncertainty and concurrent cancellation/drain. Verify
   preexisting typed insufficient-funds traversal and forbidden financial side
   effects. Do not duplicate a provider position's accepted call; additional
   configured entries require the sequence's definitive-outcome authority. Update
   network/payment architecture and agent/operator guides without adding transport
   prose or new parameters to agent-facing help.
5. **D05 — Final integration and documentation handoff.** Resolve accumulated
   findings with focused regressions; review complete caller/lifecycle chains and
   all earlier fixups. Verify README managed/Tor commands offline, defaults and
   aliases and method-list ordering in examples/inspection, concise agent-facing help and unchanged local
   search/discovery fixes. Consolidate lasting rules in owning guides, confirm
   AGENTS.md links, retire the incorporated document-completeness deferred plan
   only when its acceptance is actually complete, and remove this completed plan
   and its index entry. Do not create a completed-plan archive. Independently review
   the exact cleanup SHA and final combined branch after removal.

## Reviews, validation and exit criteria

Every implementation commit and fixup receives an independent read-only review of
its exact SHA and surrounding consumers. Supply intent, scope, this baseline,
commands/results and limitations. Address findings with
`git commit --fixup=<introducing-SHA>` and independently review each fixup. Preserve
unsquashed history through final integration. Do not build on unresolved material
findings. Coordinate file ownership and freeze sources during root-run validation;
reviewers do not run competing builds or edit sources by default.

- **G1, after D02 — Shared transport architecture.** Review D01–D02 and every fixup
  across configuration, HTTP selection, static downloads/pricing, Coinbase, Base,
  envelope validation, caches and paid ownership. Exit: exactly one origin attempt
  sequence plus one ordered method-list traversal, configured count everywhere,
  expanded HTTP/setup classification with concrete connector evidence, correct
  local-proxy exclusions and no signed pre-send exception,
  definitive-versus-uncertain fee handling, no lost validation/promotion/authority,
  no new unnecessary enforcement, and source-matched complexity evidence.
- **G2, after D04 — Runtime and payment architecture.** Review every prior commit
  and fixup across source management, help caches, directory/raw/API request
  execution, relay execution, signing, publication and shutdown. Assign independent
  financial/ownership and transport/consumer concerns. Exit: no relay-supplied
  challenge reaches signing, no signed failure triggers fallback, no accepted paid
  work is orphaned/duplicated, complete content/scope preserved, and agent help stays
  focused. Prior exact-SHA approval does not exempt unchanged adjacent paths.
- **G3, after D05 cleanup — Final integration.** Review the complete branch and
  cleanup SHA, all fixups, permanent documentation and actual validation artifacts.
  Exit: no material findings, every in-scope consumer uses the shared policy,
  appropriate tests pass, user files remain intact, and no unperformed live or
  financial qualification is claimed.

At every gate inspect fixup causes, repeated assumptions, high-risk unchanged
consumers and coverage gaps. Decide whether they require a wider audit, regression,
simplification or design change. Compare CogC/CycC with `a59770f1` using the pinned
analyzer and exact source hashes; explicitly compare relocated old/new functions,
full call chains and unchanged high-risk payment/mutation functions. Dispose of
warnings with concrete simplifications considered, rather than helper extraction
or “advisory” dismissal. Store generated reports under ignored `target/`.

Use the pinned `scripts/zcash.sh` wrapper. Choose focused library tests and the
`network`, `config`, `cli_contracts`, `startup` and socket-free `network_audit`
targets according to each commit's consumers; add meaningful shared synthetic
fixtures, not vendor inventory snapshots. Request socket-capable execution up
front where localhost fixtures require it. Assert actual executed test counts,
request counts/order and absence of forbidden signatures/funding/publication.
Retain positive controls. Run formatting, applicable Clippy and complexity checks;
perform a final combined application check appropriate to the accumulated payment/
network changes, with feature configurations sequential and sources frozen.
Offline fixtures establish mechanics only. Any live Tor or paid qualification
requires a separately agreed run and authority; do not use live wallet state as a
shortcut for this implementation.
