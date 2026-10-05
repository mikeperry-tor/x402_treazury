# Deferred: polling tools and asynchronous provider jobs

Status: deferred design exploration. This document records the requirements exposed
by OneShot, StableEnrich, Locus and Exa and a possible shared approach. It does not authorize implementation,
expand any provider's enabled tools, or define a supported TOML configuration format.
Compare additional real providers before settling the abstraction.

## Problem and intended outcome

Treazury normally performs an HTTP request, handles an x402 challenge and paid
retry, and returns the response as the MCP tool result. Some APIs instead accept
paid work and return a job identifier before the useful result exists:

```text
submit → payment handshake → accepted job ID → poll status → retrieve result
```

Exposing only submission can charge the user for work that the agent cannot
subsequently retrieve. A polling integration must connect submission, authentication,
payment accounting, wallet identity, result retrieval, and recovery. HTTP 202 is
acceptance, not execution success or proof of payment settlement.

This does not inherently require an API key, a new x402 scheme, or a new MCP
transport. Ordinary MCP calls can return a job handle and later inspect it.
Submission may use existing x402 payment; subsequent requests may use an opaque
job token, an API key, a wallet address, or a provider-specific ownership signature.
These authentication mechanisms are separate from x402 payment signing.

The intended shared feature would let an operator enable reviewed asynchronous
workflows without implementing a separate job scheduler for each provider. It
must also work for otherwise synchronous APIs that optionally return jobs.

## Current implementation boundaries

Relevant existing code and fixtures:

- `src/catalog.rs`, `src/config.rs`: request schemas, routing, reusable provider
  settings and exact operation selection. Generic generation omits header inputs;
  it does not implement arbitrary vendor authentication from OpenAPI security prose.
- `src/catalog_state.rs`: a `BoundTool` captures the tool, source revision, base URL
  and `PaidClient` for an invocation. Retaining a cloned `PaidClient` alone is not
  a durable guarantee of the exact wallet used for a particular payment.
- `src/payment.rs`, `src/rotation/`: payer selection, managed admission, signer
  leases, settlement uncertainty and durable financial state. Static calls snapshot
  the payer; managed calls acquire a signer through admission. A job must capture
  the identity actually used, not merely the configured pool name.
- `src/network.rs`: shared HTTP/gRPC construction and identity-based connection
  pools/Tor isolation. Polling must use this layer in both direct and Tor modes.
- `src/server.rs`, `src/discovery/`: MCP execution, listener permissions, dynamic
  source revisions and immutable catalog snapshots.
- [OneShot definition](../../../providers/oneshot.toml),
  [provider caveats](../../../providers/CAVEATS.md), and
  [pinned request fixture](../../../tests/fixtures/oneshot_openapi.json).

The OneShot default currently contains only `oneshot_search` and `oneshot_help`.
Search returns results synchronously; help lazily retrieves its single API guide.
Keep this working subset until asynchronous operations have been qualified.

## OneShot evidence and specific requirements

The pinned OpenAPI fixture was fetched on 2026-10-03 and contains 147 operations.
Sources for revalidation:

- [Live OpenAPI](https://win.oneshotagent.com/openapi.json).
- [SDK overview and read authentication](https://docs.oneshotagent.com/sdk/overview.md).
- [Synchronous web search guide](https://docs.oneshotagent.com/api-reference/web-search.md).
- [Documentation index](https://docs.oneshotagent.com/llms.txt).

### Submission and results

Many research, enrichment, web-read, commerce, browser and build operations return
202 with `request_id`. Examples include `POST /v1/tools/web-read`,
`POST /v1/tools/research`, and `POST /v1/tools/enrich/email`. Polling uses
`GET /v1/requests/{id}`. The introductory spec prose also mentions a WebSocket
fallback at `/v1/requests/subscribe`; it is not needed for the proposed first version.

The fixture's `JobAcceptedResponse` requires `request_id` and may include `tool`,
`status`, `idempotency_key`, `receipt_id`, `settlement_status`, and a compute-specific
`goal_id`. Its status enum includes `queued`, `processing`, `accepted`, `pending`
and `active`. Do not reuse this enum blindly for poll responses.

`JobStatusResponse` requires `request_id`, `tool` and `status`. Its documented
statuses are `pending`, `processing`, `completed` and `failed`. It may contain
`result`, `error`, `error_category`, `error_code`, `error_status`, `receipt_id`,
`settlement_status`, and timestamps. Preserve structured results and diagnostic
fields. Keep job execution and settlement state independent; a completed job is
not proof that payment settled, and a failed job is not proof of a refund.

Compute has additional `/v1/compute/{goalId}` status, task, budget, funding and
control routes. It is a distinct lifecycle, not automatically covered by generic
`request_id` polling. Messaging, purchases and other external side effects also
need their own endpoint qualification rather than bulk enablement.

### Wallet authentication

The spec declares `X-Agent-ID` for `GET /v1/requests/{id}`. Other authenticated
routes and the SDK documentation describe `x-agent-proof` wallet-ownership proofs.
Do not assume every route requires the same proof, or infer that the polling
route's simpler security declaration proves a signature will never be required.
Revalidate requirements per operation against current documentation and behavior.

The documented proof is base64 JSON with `agent`, `scope`, `issuedAt`, `nonce`
and `signature`. Its EIP-712 domain is `{ name: "OneShot Agent Auth", version: "1" }`;
the type is `AgentReadAuth { agent: address, scope: string, issuedAt: uint256,
nonce: bytes32 }`. The signature must correspond to the `X-Agent-ID` wallet.
Proofs use single-use nonces and a five-minute freshness window. Common scopes
are `read` and `write`; selected durable submissions also document `submit`.
Verify exact encoding, timestamp units, scope and domain from the vendor SDK
before implementing signing. Generate fresh proofs for retries; never reuse a
consumed nonce or treat an x402 payment signature as an ownership proof.

OneShot also offers revocable `oneshot_…` bearer access tokens billed against
prepaid credits. They are an alternative credential/billing path, not a mandatory
API-key requirement for wallet-based integration. Seller `soul_…` credentials and
Stripe ACP bearer credentials are separate workflows and remain out of scope.

### Submission identity and recovery

Selected durable enrichment routes accept `Idempotency-Key`, `X-Agent-ID` and
`X-Agent-Proof`. For `enrich/email`, the fixture states that same-key/same-input
retries can return the original job and receipt when durable enrichment is enabled;
accepted work can complete and be billed after the client cancels. Credit admission
or replay may require a fresh proof with scope `submit`.

`GET /v1/submissions/recover` requires a fresh signed read proof and query fields
`endpoint` and `key`. The documented endpoint enum is only `enrich/profile`,
`enrich/email`, and `verify/email`; do not assume it covers other tools. Accepted
keys are retained for at least 24 hours, including during admission rollback.
Crucially, **404 is not proof that an in-flight submission cannot still be accepted**.
A recovery miss does not justify a new charged submission or a new idempotency key.

LinkedIn reply illustrates a different contract: `Idempotency-Key` is required,
accepted responses can be replayed, and outcomes may remain ambiguous while the
provider reconciles an upstream send for up to 24 hours. An accepted message cannot
be recalled, and ambiguous sends are not blindly resent or automatically refunded.
Do not generalize enrichment recovery rules to messaging or other side effects.

### Payment documentation discrepancy

The OpenAPI prose describes `Authorization` plus `X-Quote-ID`; the search guide
mentions `X-Payment-Proof`. The unsigned search checked during provider integration
instead returned standard `PAYMENT-REQUIRED` with x402 v2 `exact`, Base
`eip155:8453`, Base USDC, and 1000 atomic units ($0.001). Its JSON body was a
vendor-specific payment envelope; Treazury uses the standard header.

This established challenge compatibility for synchronous search only. It did not
qualify funded settlement, async submission, polling authentication or recovery.
Recheck each candidate async endpoint before deciding whether a payment adapter
is necessary. Do not implement legacy header translation based on prose alone.
OneShot's `x-payment-info` extension mostly describes authentication flags rather
than numeric prices; it cannot by itself define an async billing policy.

## StableEnrich evidence and contrasting requirements

The [StableEnrich request fixture](../../../tests/fixtures/stableenrich_openapi.json)
was captured on 2026-10-03 from its [OpenAPI](https://stableenrich.dev/openapi.json).
Its [llms.txt](https://stableenrich.dev/llms.txt) documents asynchronous behavior
that the spec's 200/402 response entries omit. The default
[provider](../../../providers/stableenrich.toml) excludes these workflows.
Only an unsigned synchronous Exa-search payment challenge was checked live;
no job, SIWX handshake, or funded request has been qualified.

- **Hunter email verification:** `POST /api/hunter/email-verifier` can complete
  immediately or return 202 with `jobId`, `status: pending`, `pollUrl` and
  `retryAfterSeconds` (example: 5). Do not repeat the paid POST to obtain the result.
  Poll `GET /api/hunter/email-verifier/jobs/{jobId}` for `completed` or `failed`;
  completed output is in `result`. Polls are free but require SIWX authentication
  from the wallet that paid. The spec omits the poll route's `jobId` path parameter;
  repair/validate that contract before enabling it. Success-response documentation
  alone cannot identify whether a provider has an async branch.
- **Cloudflare crawl:** `POST /api/cloudflare/crawl` is documented at $0.10 and
  returns 202 with a signed JWT `token`. Poll `GET /api/cloudflare/jobs?token=…`
  every 3–5 seconds; typical duration is 30–120 seconds. Possession of the token
  alone is insufficient: same-wallet SIWX is also required. Keep it encrypted and
  out of URL logs/errors. Responses carry records, counters and an optional cursor.
  The guide suggests completion when `finished + skipped >= total` or no pages
  remain queued, while individual records can be completed, errored, disallowed,
  skipped or cancelled. Revalidate aggregate counters, pagination and terminal
  failure semantics; “no queued records on this page” is not proof of job completion.
- **Google Aerial View:** paid render submission returns before rendering finishes;
  paid `lookup-video` reports processing or active state and video URLs. The guide
  lists $0.01 for each operation, so polling has a cumulative financial cost rather
  than being merely an authenticated read. Lookup also supports an address or a
  preexisting video ID. No same-wallet ownership requirement has been established
  for this pair; do not infer one from the other StableEnrich workflows.

SIWX is a distinct authentication contract from OneShot's named EIP-712 proof.
Determine the actual challenge, message, nonce, expiry, domain/origin binding and
supported wallet/network variants from current vendor/SDK evidence before adding
an adapter. Do not reuse OneShot signing rules or assume payment-enabled HTTP
clients automatically implement free authenticated polls. The help's AgentCash
`fetch_with_auth` recommendation describes a different client, not existing
Treazury functionality or a requirement to adopt AgentCash.

These examples support sharing job identity, scheduling and lifecycle management,
while showing why configuration cannot be only a status-field lookup. Candidate
extensions include mixed immediate/accepted responses, server-suggested delays,
opaque tokens combined with proof authentication, paginated results, progress-based
completion and explicitly paid polling. Prefer reviewed completion adapters or
small validated primitives over arbitrary vendor-provided expressions. Keep the
framework deferred until these contracts are revalidated and a first workflow is
selected; discovering a second provider does not automatically enable its tools.

## Locus: capability-token jobs and independently ready deliverables

The [live Locus OpenAPI](https://api.locus.report/openapi.json), reviewed 2026-10-03,
now documents `GET /api/property-update-jobs/{id}`. This route is absent from the
committed curated request catalog. Its presence resolves the earlier missing-route
question for **property-update only**, not for every batch workflow. Keep the three
paid submission routes excluded until their full lifecycles are supported.

- `POST /api/locus-property-update` documents 202 after settlement, with `jobId`,
  `statusUrl`, `expiresAt`, `pollAfterSeconds`, estimated duration, stage and
  `deliverables`. It may instead return a charge-free 200 thin-result diagnostic or
  409 clarification with `retryInput`. A proposed corrected input is a new operator/
  agent decision, not authorization to resubmit automatically. Provider settlement
  claims do not replace local financial reconciliation.
- Poll the fixed `/api/property-update-jobs/{id}` route with the opaque private
  token embedded in `statusUrl`, or its documented bearer-header alternative.
  The ID pattern is `update_` plus 32 hex characters. A 404 intentionally conflates
  unknown jobs with incorrect/missing tokens; it is not evidence of no accepted work.
  Polls are documented free. No same-wallet proof is established for this route;
  do not impose SIWX merely because another provider needs it. Still retain original
  network isolation and protect the capability in encrypted state/redacted logs.
- Status values are `payment_pending`, `queued`, `processing`, `ready`, `partial`
  and `failed`. Stages and per-deliverable state are separate. `flyerReady=true`
  exposes a PDF handoff while video may still render; video failure can coexist with
  useful report/PDF output. Define terminal/partial-success mappings explicitly;
  never wait for every artifact before exposing an already-ready authorized result.
- `flyerHandoff`/`flyerHandoffUrl` carry a ready proof-gated PDF URL and expiry. The
  emitted `share_` proof is a different capability from the private job token.
  Neither may be constructed by transforming the other. Passing the handoff to
  `locus-property-flyer` is a separate paid operation with separate admission.
- `POST /api/locus-place-report-batch` accepts 3–50 addresses, returns a job with
  counts and per-item statuses/report IDs, and documents `completed_with_errors`.
  Preflight can return a free diagnostic when no item is supported. Charge one
  reviewed batch amount, not an inferred payment for each item; preserve unsupported
  items and partial failures. Its returned `statusUrl` is not proof that it uses the
  property-update poll route or the same authentication/recovery contract.
- `POST /api/locus-record-batch` accepts 2–25 addresses and up to six free record
  lanes in a paid batch wrapper. A documented 200 can still contain `jobId`,
  `statusUrl` and queued work. Revalidate its poll endpoint, auth, completion, token
  expiry, webhook contract and recovery separately; none was exercised live.

This provider adds useful general requirements: acceptance-body discrimination even
on 200, token-only access, multi-item partial completion, staged artifact readiness,
capability separation, and TTL-aware retrieval. Connect ready-file capture to the
[artifact plan](../artifact_storage.md), without equating storage with job support.

## Exa: API-authenticated runs and batch files

The [Exa OpenAPI](https://api.exa.ai/openapi.json), reviewed 2026-10-03, describes
async workflows beyond the Search/Contents x402 subset:

- `POST /agent/runs` returns 200 with an immediate run object by default; SSE is an
  alternate `Accept` mode. `GET /agent/runs/{id}` reads `queued`, `running`,
  `completed`, `failed` or `cancelled`, output, stop reason and usage/cost fields.
- `POST /batches` returns a batch. `GET /batches/{id}` reports `in_progress`,
  `completed`, `cancelling`, `cancelled` or `expired` plus request counts and expiry;
  `/batches/{id}/cancel` is a separate operation. Cancellation is not proof that
  already dispatched requests were unbilled.
- Completed batches provide a short-lived `resultsUrl` for a JSONL file on object
  storage. The spec explicitly permits re-fetching the batch to mint a fresh URL
  after URL expiry. That is a status/result-link refresh, **not** replaying the paid
  submission. Keep job expiry distinct from URL expiry and artifact retention.
- These operations advertise API-key/bearer authentication and no `x-payment-info`
  offer equivalent to Search/Contents. Polling support would not make them usable
  through the present keyless provider automatically. A separately scoped credential
  integration and billing review would be needed; never forward that credential to
  the presigned result host.

Exa is comparison evidence for 200 acceptance, optional SSE, team ownership and
refreshable result URLs. It is not the recommended first x402 polling integration.

## Recurring reads, remote monitors and payment extensions

The catalog audit found several uses of “poll” that must not become job workflows:

- botsmith `/x/watchlist` and Kronos `/api/v1/market-pulse` return synchronous
  incremental feeds with cursors. Each invocation is a new read; there is no
  accepted paid job whose completion is being awaited.
- RegimeShift `/v1/intent/{intent_id}/match` is an existing long-poll read with a
  JSON default and optional SSE. Its financial-intent state and wait timeout are
  separate from job submission. Treazury does not implement the SSE branch.
- SocialFetch's excluded monitors/webhooks are persistent account-authenticated
  scheduled subscriptions with delivery/retry management, not one-shot jobs. Do not
  enable them with a generic completion poller. LinkedIn jobs are employment data;
  Hacker News polls are content, not async payment workflows.
- AgentUtility's reviewed spec/guide does not establish a caller-polled job contract
  for its hosted media generation outputs. Agent402 documents inline MP4 output.
  Do not infer client polling from slow calls, upstream model queues or “video.”
- [Straits](https://straits.live/openapi.json) sells a bounded SSE session at
  `POST /api/premium/stream/session`, returning a token and `streamUrl` for
  `/api/premium/stream`. Its `tick`/`change` events and `session_expired` end event
  describe a subscription window, not job completion. Its separate webhook POST
  funds 30 days or 100 deliveries and returns a per-subscription HMAC secret used
  for free cancellation. These excluded workflows need streaming/subscription
  ownership and lifecycle support; a generic job poller must not enable them.

Unsigned challenges add further evidence for authentication/idempotency boundaries:
Otto advertises `sign-in-with-x` and documents one-hour re-access on selected paid
reads; SocialFetch and Agent402 advertise optional `payment-identifier`; Exa advertises
an `agentkit` World-chain free-trial proof. These are not interchangeable with job
ownership. Managed Treazury currently strips top-level extensions; it neither signs
these proofs nor supplies payment identifiers. An async adapter needing them must
explicitly integrate reviewed authentication/idempotency behavior before activation.
Do not silently turn missing SIWX proof into another payment. See [payment extension boundaries](../../../providers/CAVEATS.md#payment-extension-and-offer-boundaries)
for the distinct reviewed offer metadata and unsupported authentication mechanisms.

## Candidate shared architecture

### Declarative workflow description

Keep operational differences in reviewed provider settings where possible:

| Concern | Candidate configuration |
| --- | --- |
| Submission | Exact method/path, permitted response codes, synchronous-result fallback |
| Acceptance | Job ID and optional receipt/token fields, expressed as bounded JSON pointers |
| Polling | Fixed endpoint template, method, escaped job-ID placement, response fields |
| Status | Explicit pending/success/failure mappings; reviewed aggregate-progress completion when necessary |
| Results | Inline result field or separately configured retrieval endpoint |
| Recovery | Documented idempotency/recovery support and retention guarantees |
| Authentication | Named reviewed adapter plus operation-specific policy |
| Payment | Separate submission, polling, result retrieval and cancellation policies |
| Limits | Poll interval/backoff, deadline, concurrency, response size and retention |

This is a design sketch, not proposed executable TOML. Avoid a generic scripting
language, arbitrary expressions, or OpenAPI-supplied signing/header templates.
OpenAPI can provide request schemas and hints; status conventions and security
requirements need explicit review. Do not infer a complete workflow from 202 alone.

An async response must match its declared acceptance shape. Missing job IDs,
unknown statuses and contradictory fields should yield explicit protocol errors
while preserving any known submission/payment evidence, not trigger resubmission.

### Job manager and invocation context

A shared manager could coordinate state transitions, polling, result caching,
backoff, deadlines and restart recovery. A job record would reference:

- An opaque local handle, provider job ID, workflow version and pinned source revision.
- The authenticated access scope and original source/wallet binding.
- The actual submitting wallet/key reference and network isolation identity.
- Idempotency key, request fingerprint, submission certainty and payment/receipt references.
- Provider execution status, polling schedule, last error and retained result metadata.

Persist enough context before submitting to recover an ambiguous outcome. Reuse
existing encrypted storage and blocking-worker conventions where appropriate;
do not hold a database transaction or a global registry lock across network I/O.
Private keys remain in their existing key-management boundary. Sensitive job tokens
and results require protected storage and redacted logs, not inclusion in handles.

Reference to a wallet does not mean holding a spend reservation or exclusive payer
lease for the lifetime of a long job. Determine how to retain read-signing capability
without blocking normal rotation or other payments. Wallet removal/key replacement
must not silently discard a live job's required authentication capability.

### Authentication adapters and network isolation

Use small, explicit adapters for supported mechanisms: anonymous/opaque-token jobs,
operator-managed credentials where supported, OneShot's wallet proofs, and
reviewed SIWX challenge-response flows. Keep
provider signing separate from payment authorization. Agents and imported specs
must not choose arbitrary EIP-712 domains, signers, proof scopes or identity headers.

Poll/recovery/result requests must retain the submitting identity through wallet
promotion, payer replacement and process restart. Use existing network constructors
and connection-pool isolation rather than a second HTTP stack. In Tor mode, keep
the job's wallet-associated traffic in that wallet's isolation context; never send
it through the general discovery client or the pool's new active wallet identity.
This maintains the existing isolation boundary, not a guarantee that the provider
cannot link a submission to its polls—which it inherently can.

Treat returned URLs and job IDs as untrusted data. Prefer fixed endpoint templates;
apply the existing destination/redirect policy to any explicitly supported result
URL. Escape IDs as path/query components and prevent token or proof forwarding to
an unauthorized origin. Reapply guarded egress restrictions on every attempt.

### MCP-facing behavior and access control

Initially prefer ordinary explicit status/result tools. Submission returns an
opaque local handle, acceptance status and a suggested next-poll delay. A later
call checks progress or retrieves a completed result. Short bounded internal
polling could be layered on later; no new MCP transport, notification channel,
or MCP task-protocol extension is required for the initial design.

Choose exact tool names and scope rules when implementing; do not enable management
tools globally merely because one listener has an async source. Current HTTP
permissions identify a listener through its bearer credential, not an individual
agent behind that credential. Do not promise per-agent ownership without adding a
real authenticated principal model. A handle is an identifier, not authorization.

Recheck access on each status/result/cancel call. Define the policy for source
updates, removal and permission revocation: a captured execution context is needed
for safe recovery, but must not preserve authorization after it has been revoked.
Keep financial reconciliation separate from permission to reveal results to a caller.

### Payment, retries, concurrency and shutdown

- Treat execution state, submission certainty and payment state separately. An HTTP
  error, local deadline or cancelled MCP call does not prove work was rejected.
- Default poll/status/result access to unpaid-only where appropriate. An unexpected
  402 must produce an explicit payment-required result, not an automatic charge.
  Support paid polling only with explicit operator policy, per-call admission and
  aggregate limits; retain the submitting identity without silently funding a new
  payment wallet merely to poll. Decide how an emptied/retired payer is handled.
- Generate stable logical submission keys before network I/O where supported.
  Never retry an ambiguous paid submission as a fresh job. Document vendor-specific
  safe replay guarantees; without them, surface uncertainty and retain recovery state.
- Coalesce concurrent polls for one job and bound per-provider and global polling
  concurrency. Back off with jitter and respect `Retry-After` within configured
  deadlines. Separate job polling from the startup pricing cache.
- Cancellation of waiting is distinct from cancellation of remote work. Expose
  remote cancellation only where supported, separately authorized and with explicit
  payment/refund semantics. Never report a refund solely because a job was cancelled.
- Shutdown should drain/persist accepted state and explain that it is doing so.
  Resume polling/reconciliation after restart instead of automatically resubmitting.
- Bound job counts, lifetimes, response bytes and stored results. Capacity, expiry,
  pruning and truncation must be visible in logs and agent-facing results/help.
  Reject new work before payment when capacity is unavailable; never silently evict
  an unresolved financial operation to meet a limit.

## Comparing future providers

Maintain a requirements matrix as additional providers are examined. Record observed
contracts and unresolved questions, not just whether an API advertises “async.”

| Provider/workflow | Acceptance and polling | Auth | Payment and recovery | Evidence boundary |
| --- | --- | --- | --- | --- |
| OneShot ordinary jobs | 202 `request_id`; `/v1/requests/{id}`; separate accepted/poll status enums | Agent ID; route-specific ownership-proof requirements need revalidation | Paid submission; some endpoint-specific durable keys/recovery | Pinned spec and SDK docs; only synchronous search challenge checked live |
| OneShot durable enrichment | Same job path; selected `/v1/submissions/recover` endpoint | Fresh read/submit proofs depending on operation | At least 24-hour key retention; 404 does not settle submission uncertainty | Documentation; no live job submitted |
| StableEnrich Hunter | Immediate result or 202 `jobId`/`pollUrl`; pending/completed/failed; `retryAfterSeconds` | Same-wallet SIWX | Paid submission; free polling; do not repeat POST | Spec + guide; poll path parameter missing; no live job/auth qualification |
| StableEnrich Cloudflare | 202 JWT `token`; query-token polls; records/counters/cursor | Token plus same-wallet SIWX | Paid submission, free polling; replay/recovery guarantees unverified | Guide; completion/pagination semantics need verification |
| StableEnrich Aerial View | Render then lookup by address/video ID; processing/active | Same-wallet requirement not established | Submission and each lookup paid | Guide; no live rendering or cumulative-cost qualification |
| Locus property-update | 202 job; token-protected status; independently ready report/PDF/video; partial state | Private token; separate share/PDF capability | Paid submit, documented free polls; expiry and 404 ambiguity; no replay contract established | Live spec documents poll route; no job submitted |
| Locus batch reports/records | 200/202 acceptance, counts/per-item results, returned status URL | Revalidate each batch poll route | Paid batch wrapper; unsupported items and partial failures | Spec only; property-update auth must not be assumed |
| Exa Agent runs | 200 run object or optional SSE; ID polling | API key/bearer, team ownership | Not established as x402-enabled | Comparison only; excluded from keyless provider |
| Exa batches | 200 batch; status/cancel; expiring JSONL URL | API key/bearer; presigned download capability | Refresh result link by reading batch, never resubmit to refresh | Spec only; needs credential and artifact integration |

For each new candidate, inspect job-ID stability, pagination/result size, expiry,
returned URLs, terminal states, timeout semantics, rate limits, authentication,
polling charges, cancellation, idempotency and recovery. Distinguish HTTP success
from job success and from settlement success. Record whether wallet-linked identity
is needed at all. Similar status fields alone do not establish compatible recovery
or authorization semantics.

The contrasting provider requirements may justify splitting a small generic status poller from
optional durable paid-job management. A synthetic provider is useful for tests,
but is not evidence that an abstraction fits the wider ecosystem.

## Deferred implementation order and validation

When resumed, work in focused commits and resolve the open decisions before
expanding provider defaults:

1. Revalidate the OneShot, StableEnrich and Locus contracts; use Exa as a
   contrasting credential-based case. Select a
   bounded read-oriented workflow, document payment/auth/recovery contracts, and
   capture reviewed fixtures. Do not begin with email, purchases or messaging.
2. Define workflow validation and the MCP handle/access contract. Add a synthetic
   provider with different status and ID fields and no OneShot-specific proof.
3. Implement durable job state and captured identity references, including ambiguous
   submission, capacity admission, rotation, key retention and crash recovery tests.
4. Implement the shared polling engine through the existing network layer. Test
   backoff, retry-after, parallel callers, deadlines and explicit result bounds.
5. Implement the reviewed OneShot authentication adapter and endpoint-specific
   recovery/idempotency logic. Test signatures against independent vendor-compatible
   vectors and a verifying local fixture, including nonce freshness and scope errors.
6. Integrate MCP submit/status/result tools and optional explicit cancellation with
   source/listener permissions. Test across listeners, wallets and dynamic source
   updates; retain synchronous tools unchanged.
7. Qualify the selected live workflow in a separately authorized bounded funded
   test before enabling it by default. Record submission, payment and result evidence
   separately, and expand the allowlist only for qualified operations.

Offline validation should cover accepted and synchronous responses; malformed IDs;
unknown statuses; failed/completed jobs with independent settlement state; lost
submission responses; safe replay versus unsafe retry; recovery 404 while a submit
is still pending; process crashes at durable boundaries; source removal/revocation;
parallel poll/result requests; wallet promotion during work; static key replacement;
proof expiration/replayed nonces; and unexpected paid polling without a signature.
Include 200 accepted jobs, token-protected 404 ambiguity, report-ready/video-failed
partial results, batch per-item failures, separate private/share capabilities, and
safe expired-result-URL refresh without repeating submission. Cursor-feed requests
and recurring monitor creation must not be mistaken for job status reads.

Network tests should assert correct SOCKS credentials and connection identity for
submission, status, recovery and results, including after rotation/restart and
across multiple MCP listeners. Test hostile result URLs/redirects, isolation of
unrelated jobs, and prevention of authentication forwarding. Exercise both direct
and Tor-configured stacks with local fixtures; real Tor qualification is separate.
Run the relevant suites in default-Zcash and no-Zcash builds, retain existing
payment/concurrency tests, and add region-coverage evidence for new state branches.

Open decisions include process-only versus durable jobs for nonfinancial providers,
permission-revocation recovery behavior, retained signer lifetime, paid-poll budget
semantics, MCP tool placement, data retention, and whether generic and financial
job management should be separate layers. Resolve these from provider evidence
rather than treating this sketch as a fixed API.


## Additional provider contracts: BlockRun and Apollo

The [provider caveats](../../../providers/CAVEATS.md) retains primary
sources and frozen request fixtures. These operations remain outside enabled
provider defaults; unsigned Tor quotes are not execution qualification.

- **BlockRun:** the unified guide describes image generation/image editing, video
  and voice-call jobs with free companion GET polling. The OpenAPI snapshot does
  not provide all companion GET request contracts. Establish actual completion,
  failure, TTL and ownership semantics before enabling submission. Do not infer
  authentication from a free poll price. Phone provisioning/renewal and sandbox
  lifecycle also require wallet ownership and cleanup design. Use the documented
  job identity rather than resubmitting a paid request when it times out.
- **Apollo via Locus:** ordinary POST may return 200 data with a durable request
  receipt/status URL, or 202 for an already-running request. Spec guidance names
  `x-locus-request-id`, but the live 402 used `locus-request-id` plus body requestId.
  Resolve this mismatch before implementing the single paid retry. Model request
  correlation, idempotency, 409 conflicts and terminal failure independently of
  job readiness. Review status URL origin/auth and retain the original body and
  identity; never silently pay again for recovery. This is not evidence that all
  Apollo calls are asynchronous.

These examples reinforce separate adapters for correlation/idempotency headers,
job ownership, completion state and artifact retrieval. Do not merge
these into an unvalidated universal POST-then-GET assumption.
