# Generalized unsigned-read fallback

## Intent, baseline and scope

Give supported public reads a consistent origin-retry/Curl-fallback path. Extend
fallback to agent-added OpenAPI acquisition and refresh, provider help, explicit
usage documents and discovery probes, and unsigned public API/directory GET/HEAD
responses. Expose the Tor isolation retry count in `[network]`. Keep agent-facing
help focused on capabilities and inputs, without relay, retry or payer machinery.

Starting baseline: `a59770f107d8cb64f7df64b82d6554343b233ade` on
`coinbase_discovery`. Current fallback covers static catalog/pricing discovery and
Coinbase search; dynamic imports, help and other API reads do not share it. The
current Tor count is hardcoded to three total attempts in the common network
factory. Static relay state includes caching/readiness/breaker responsibilities;
Coinbase has a separate on-demand exchange. These are starting facts, not the
proposed architecture.

This turn prepares the plan for user review only. No implementation commit starts
before that review. Implementation uses offline/local fixtures and public unfunded
keys. It does not authorize live paid calls, accessing real wallet secrets,
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
  Base read-only RPC recovery. Base retains its existing endpoint/method validation
  and separate direct behavior; it does not acquire a Curl fallback.
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

### Shared request path and response contracts

- One shared unsigned GET/HEAD acquisition path owns origin attempt ordering,
  transient-failure classification, complete-body transport failure handling,
  `Retry-After` and at most one fallback handoff. Migrate the existing static and
  Coinbase paths as well as new callers; do not leave vendor-specific retry loops
  or wrap an existing retry loop in another loop. Base and the existing x402 POST
  path share count/selection primitives without being forced into a GET response
  abstraction. Preserve existing unsigned x402 POST retry authority; this work
  does not extend Curl to target POSTs or broaden POST replay behavior.
- Retry existing eligible blocking statuses/transient transport failures on fresh
  owner-bound scopes, with unchanged certificate/hostname/public-destination checks.
  Honor valid server delays, including the terminal delay before relay handoff.
  Complete valid origin responses finish immediately, including valid empty or
  partial directory results. Do not retry merely because a search found nothing.
  Local destination/authority checks and download-limit failures never trigger a
  paid fallback. Generic API output/schema conversion failures are terminal, not
  reasons to repeat an otherwise successful read through Curl.
- Caller validation remains responsible for accepted statuses, document parsing,
  cache/revalidation, output mapping and publication. Share transport mechanics;
  do not impose catalog JSON rules on text help, HEAD responses or generic APIs.
  Separate Curl envelope validation from those caller contracts. Preserve existing
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
- Enforce each caller's origin allowlist, public URL policy, transport permissions,
  exact query encoding, body bounds and redirect policy on both paths. Curl follows
  redirects remotely: reject disallowed reported redirect chains, preserve the
  existing explicit trust in Curl, and do not claim local checks prevent its remote
  fetch. Relay responses never promote an origin Tor scope or populate origin
  validators/caches as if obtained directly.
- Eligibility is limited by the bundled Curl request contract: a public target
  GET/HEAD faithfully representable by URL and method, without a body or required
  target headers. Do not silently discard headers or credentials to make a request
  eligible. The envelope carries text, not arbitrary binary bytes; reject unusable
  relayed binary/truncated content rather than return corrupted success. Preserve
  legitimate generic API range semantics; complete-document readers reject
  unsolicited 206/Content-Range responses on origin and relay paths.
- The complete-document portion incorporates the focused
  [unfinished audit](deferred/document_response_completeness.md) for help/import,
  static catalogs and relay decoding. Preserve complete schemas/help and visible
  failures; never cache or publish a partial document as complete.

### Failure/response matrix

The same transport engine implements these caller contracts; the distinctions
are response semantics, not vendor-specific retry loops or configuration flags.
In Tor mode the existing retryable statuses are 400/403/429/503; other remote
failures below may hand off directly to fallback without extra origin attempts.
For the new on-demand readers, eligible connection/TLS/read failures and HTTP
400/403/429 or 5xx can earn fallback. Keep the existing TLS eligibility while
retaining certificate and hostname validation on every attempt. Direct mode can
hand off after its one origin attempt. Locally rejected or unsupported requests
never earn fallback.

| Consumer | Successful/terminal result | Other response treatment |
| --- | --- | --- |
| Static catalogs | Complete accepted JSON/304 revalidation | Preserve remote HTTP/parse fallback, including existing non-transient HTTP errors; no local-policy/bounds fallback. |
| Static pricing | Usable 402 or 2xx (unknown price); required-argument skip remains terminal | Preserve existing unusable/missing pricing and remote-error fallback; a 2xx unknown price does not buy another probe. |
| Coinbase search | Complete accepted 200, including empty/partial-result metadata | Preserve existing remote status/JSON-parse fallback; bounds/policy failures terminal. |
| Dynamic OpenAPI, help, explicit docs and document probes | Complete accepted document; 401/404/410 and other non-blocking 4xx are terminal | Blocking statuses/transient failures above can fall back. Complete invalid JSON/schema/text, unsolicited partial documents and unsupported content are validation failures, not reasons to pay for another read. |
| Ordinary API/directory/raw GET or HEAD | Any accepted complete 2xx, including empty and legitimate range responses | Blocking statuses/transient failures above can fall back. Other 4xx/redirects are terminal under existing response policy; valid 200 with invalid directory JSON or output conversion failure is terminal, without fallback. Origin 402 enters normal payment handling; relayed 402 is handled below. |

The engine must distinguish transport/body-read failure from complete-but-invalid
content and local limits with typed outcomes, rather than guessing from upstream
error text. Directory validation does not become an excuse to replay successful
HTTP delivery. Test 404/410, 400/403/429/503, other 5xx, complete malformed 200,
204/206, redirects, headers/body failures and signed-phase suppression explicitly.

### Payments, ownership and agent interface

- Direct-origin 402 keeps normal challenge/sign/retry behavior. A relayed 402 does
  not enter that signing path: Curl cannot forward the target payment headers.
  Surface the unpaid target failure; do not sign it, try the blocked origin again,
  or claim the target API was paid. This limitation does not change static pricing
  probes, which can read challenge metadata without signing it.
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
- Share a stateless relay exchange/envelope decoder between on-demand and static
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
the failure/response matrix and GET/HEAD scope. User review remains pending; D01
has not started. Material changes reopen the affected concerns.

## Ordered implementation commits

1. **D01 — Configurable shared Tor isolation count.** Add the network setting,
   defaults/inspection and validation; replace the hardcoded count for every
   existing participating consumer together, including Base. Cover omitted/zero/
   positive values and overflow, whole network-file overrides, direct/pinned
   operations, zero-retry body handling and Base's existing endpoint-first behavior.
   Existing callers keep their functional retry/fallback behavior in this commit.
   Update `docs/configuration.md`, `docs/network-egress.md`, both timer/resource
   inventories and `examples/network/tor.toml` with the count semantics. No new
   total-work deadline, response timer, readiness gate or local cap.
2. **D02 — Shared unsigned acquisition and relay exchange.** Establish one origin
   retry/handoff path and a response envelope with separate caller validation;
   migrate static catalog/pricing and Coinbase without parallel old loops. Separate
   stateless exchange from static scheduling/cache ownership and prevent recursion.
   Preserve current `search` configuration until D03 replaces it with its consumers.
   Cover complete progress versus stalls, retries/body failures/delays at counts
   0/1/default, valid empty results, cancellation, limits, trust/redirect handling,
   no relay scope promotion, cache/304 provenance, and static accepted-work drain.
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
   calls, through
   the same fallback while preserving normal origin 402 signing and existing POST
   behavior. Cover exact query/response/output mapping, supported text MIME/status,
   unsupported body/headers/binary handling, relayed 402 without target signing,
   signed failures without fallback/replay, relay recursion exclusion, managed/static
   payer bindings, payment uncertainty and concurrent cancellation/drain. Verify
   preexisting typed insufficient-funds traversal and forbidden financial side
   effects. Do not buy another relay call for the same accepted attempt. Update
   network/payment architecture and agent/operator guides without adding transport
   prose or new parameters to agent-facing help.
5. **D05 — Final integration and documentation handoff.** Resolve accumulated
   findings with focused regressions; review complete caller/lifecycle chains and
   all earlier fixups. Verify README managed/Tor commands offline, defaults and
   aliases in examples/inspection, concise agent-facing help and unchanged local
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
  sequence, configured count everywhere, no lost validation/promotion/authority,
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
