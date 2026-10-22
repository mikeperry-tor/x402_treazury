# Coinbase curated search with paid relay fallback

## Intent, scope and baseline

Integrate Coinbase-curated x402 resource search with `source_management = true`,
using the existing Tor isolation retry policy followed by an explicitly enabled
paid Curl fallback. Preserve bundled providers, x402 List, OpenAPI registration,
documented raw HTTP calls and optional usage-document inspection. Review the
whole discovery interface so agents can choose the shortest appropriate action.
The interface retains seven management tools; no bundle tools are added.
Implementation has not begun. The separately authorized live relay test below
establishes feasibility, not full financial or transport qualification.

Functional baseline: `a919de522b5bd5f373012d51f01bec2b0d5bf83e`. Read the parent
version of the retired `docs/plans/uniform_pagination_and_tool_inventory.md`,
especially P01A, alongside current consumers. The plan/index preparation is
`498f2570`; contract review and access evidence are `543a3c84` and its fixup
`1a7355d7`. Preserve unsquashed review history. The untracked
`discovery_confusion.md` supplies usability examples, not authoritative provider
counts, prices, reachability or instructions to make paid calls. Preserve all
unrelated tracked, untracked and ignored user files, including local deployments.

Currently the seven tools provide offline bundled metadata, x402 List browse/best,
optional service/base discovery, registration, endpoint-local search/call and raw
HTTP calls. `source_details` returns document locations/outcomes, not contents.
There is no Coinbase adapter or search relay integration. The existing automatic
Curl integration serves catalog/pricing acquisition and retains responses and
failures by URL; it cannot be used unchanged for fresh runtime searches.

Owning guides:

- [Agent sources](../agent-sources.md): tools, workflows, identities, registration,
  persistence and endpoint instruction placement.
- [Configuration](../configuration.md) and
  [catalog architecture](../architecture.md#catalog-configuration-and-execution):
  settings, selection, imports and execution.
- [Network egress](../network-egress.md),
  [local resource limits](../local-resource-limits.md),
  [best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules) and
  [wallet observation policy](../wallet-rotation.md#wallet-observation-policy):
  transport, retries, accepted work and financial ownership.
- [Provider policy](../../providers/README.md#provider-policy),
  [provider caveats](../../providers/CAVEATS.md),
  [testing](../testing.md#provider-regressions), [development](../development.md)
  and [complexity review discipline](../../tests/COMPLEXITY.md#review-discipline).

## External contracts and feasibility evidence

Use reviewed REST contracts through existing Rust/network infrastructure:

- [Bazaar extension](https://docs.x402.org/extensions/bazaar): discovery metadata,
  schemas and HTTP/MCP distinctions.
- [Coinbase buyer guide](https://docs.cdp.coinbase.com/x402/buyer/discover-services):
  public discovery without a CDP API key.
- [Resource search](https://docs.cdp.coinbase.com/api-reference/v2/rest-api/x402-facilitator/search-x402-resources):
  ranked search, `curatedOnly`, optional `bundleSlugs`, supported payment filters,
  upstream limit of 20, `partialResults`, and no search pagination.
- The local reviewed [Curl schema](../../providers/curl/openapi.json) and
  [relay contract](../configuration.md#paid-discovery-relay): paid POST `/curl`
  with a target URL/method, returning target status, headers, body and redirects.

Unauthenticated direct Coinbase search returned HTTP 200 with complete metadata
and `partialResults=true` for `query=weather`, `curatedOnly=true`,
`network=eip155:8453`, `scheme=exact`, `limit=1`. The same search through SOCKS5
remote DNS at `127.0.0.1:9150` returned Cloudflare HTTP 403 with
`cf-mitigated: challenge`. Keyless access from a Tor exit is not established.

One separately operator-authorized live toolbox call to paid Curl fetched that
search URL successfully: target HTTP 200 JSON, one resource with Bazaar metadata
and payment offers, `partialResults=true`, no redirects and no relay error. No
Coinbase credentials or headers were supplied. The operator identified
`comprehensive.toml` as live; it uses Tor at `127.0.0.1:9150`. The test reused the
running server without opening wallet state in another process. It establishes
search through the Tor-configured client to Curl's own target egress, without
packet-level route verification or an exposed settlement receipt/exact charge.
Sanitized evidence is under ignored `target/coinbase-contract-gate/`.

The selected scope accepts that route: our connection to Curl remains over Tor;
Curl sees search URLs and supplies target content/status/headers. Our TLS
connection authenticates Curl, and target-response integrity is trusted to Curl.
This is an explicit paid relay, not direct client egress or a Coinbase API key.
No additional live calls or credential acquisition are authorized by this plan.

### Deferred bundle access

[Bundle list](https://docs.cdp.coinbase.com/api-reference/v2/rest-api/x402-facilitator/list-x402-bundles)
and [bundle detail](https://docs.cdp.coinbase.com/api-reference/v2/rest-api/x402-facilitator/get-an-x402-bundle)
returned unauthenticated direct HTTP 401; Tor requests encountered the same
Cloudflare challenge. The docs show bearer headers without explaining this
exception to public discovery. No x402-paid bundle-access alternative was found
in the reviewed docs. Authentication intent and successful authenticated access
remain unverified; Curl bundle access was not tested.

Do not add bundle listing/detail tools, CDP keys/JWTs, workflow installation or
MCP execution in this implementation. Public/authenticated bundle access needs
separate contract review and authorization. Optional `bundle_slugs` remains only
a documented search filter for already-known slugs; it neither enumerates bundles
nor substitutes for complete ordered bundle details. Before D04 removes this
completed implementation plan, retain only this focused unfinished work in
`docs/plans/deferred/coinbase_bundles.md` and link it from the deferred-plan index;
do not preserve an archive of implemented search work.

## Behavior and preserved contracts

1. **Explicit directory choice.** Add `directory="coinbase"|"x402_list"` to
   `sources_search`; omission preserves x402 List. Local-only remains offline
   with existing pagination and no payer lookup. Reject any directory selector
   or remote-only fields with `local_only=true` before I/O. There is no search
   federation, automatic directory fallback, startup fetch or local index.
2. **Curated ranked search.** Coinbase accepts omitted `mode` or `best`;
   explicit browse is unsupported. Remove the global `mode.default=browse` from
   the emitted schema: omission still means browse for local/x402 List. Default
   Coinbase `curated_only` to true; explicit false broadens upstream results.
   Translate `q`, reviewed tags, known bundle slugs and advisory USD price
   filters once at dispatch. Retain actual upstream bounds and apply supported
   Base USDC/exact payment constraints server-side. Broad inventory and merchant
   browsing are outside scope. Price/curation/quality signals never grant payment
   authority or prove useful delivery.
3. **Complete faithful results.** Preserve entire upstream resource objects,
   including unknown/missing fields, exact URLs, protocol, declared methods,
   Bazaar input/output metadata, payment offers, quality/curation and `skillUrl`.
   Put local support annotations outside raw objects. Preserve same-URL method
   variants and unsupported protocols/payment metadata. Report `partialResults`
   without invented totals, pagination or absence claims. Do not turn examples
   or the Bazaar metadata-envelope schema into an invented argument schema.
4. **Shared Tor policy, then paid relay.** Select a participating transport for
   the endpoint source client's documentation identity and configured transport.
   Reuse `max_http_attempts`, `http_attempt`, eligible status/transport rules and
   `Retry-After`: one initial origin attempt plus at most two fresh isolation
   retries (three total), not two total and not an additional retry loop. Direct
   mode keeps its existing single-origin-attempt behavior. A complete valid
   search response may conditionally promote its origin scope; Curl results
   never promote it. Nonretryable origin failures proceed to configured fallback
   without gratuitous isolation retries, following existing catalog semantics.
5. **Explicit runtime relay opt-in.** Add `[discovery_relay].search = true`,
   default false, for Coinbase search fallback only. Existing `serve`, `warm`
   and authored `sources` selection retain their catalog/pricing meaning. Reuse
   the configured relay provider/bootstrap settings; do not synthesize a static
   source ID or require a relay-bound catalog on the listener. Validate and show
   this setting through existing configuration/inspection paths. Document in
   field help and inspection that `wallet`/`sources` govern static catalog/pricing
   work, whereas search uses the authenticated endpoint payer. Search enablement
   neither implies serve/warm nor prepares/funds wallets at startup. Existing
   deployments do not silently acquire new search-spending authority. Document
   the opt-in in maintained examples without editing operator-local files.
6. **Endpoint-owned uncached execution.** Search relay payment uses the
   authenticated listener's source-management client/wallet, including its normal
   per-payment authority; the static `discovery_relay.wallet` override does not
   select a search payer. Obtain the public origin identity without allocating
   or promoting wallets, waiting for funding or starting observations. After
   origin failure, an enabled fallback performs one retained paid Curl workflow
   with the exact target query. Reuse paid-client challenge/sign/journal handling
   and Curl envelope validation, not catalog URL caches, sticky per-target
   failures, shared `Relay::stopped` state, static-source wallet selection or
   `fetch_when_ready`/deferred readiness waits. Missing
   payment capacity fails this operation visibly. No new cache, queue, lock,
   limiter, counter or whole-work deadline is introduced. Preserve ordinary
   payment safeguards without importing the catalog relay's process-wide breaker;
   a failed runtime search does not disable future searches or static relay work.
7. **Visible relay and failure outcomes.** Return Coinbase directory provenance
   and distinguish origin versus Curl acquisition; disclose that fallback may
   cost and that repeating search can authorize another payment. Each invocation
   obtains a fresh search result; valid empty/partial results are successes and
   never trigger paid fallback. Validate relay target URL/method, target status,
   redirect/truncation/error indicators and complete search JSON with existing
   response bounds. Rejected inputs/local policy errors do not authorize relay
   spending. No Coinbase origin challenge is signed. Never automatically repeat
   an uncertain signed relay call or fall back to direct client egress. A failed
   search leaves other directories and published tools usable.
8. **Optional unsigned documents.** Add exclusive
   `source_details(documentation_url=...)`, alongside existing x402 List `slug`
   and exact API `base_url`. GET once using the source client's documentation
   identity/transport, `run_owned`, public-destination checks, `allowed_origins`
   and existing help bounds/inactivity semantics. Return complete accepted UTF-8
   usage text, content type and requested URL, or visible failure; reject
   HTML/non-text. Label URL provenance as caller-supplied, without claiming a
   verified Coinbase advertisement. No payment/Curl fallback, redirects, linked
   traversal or instruction execution. Metadata cannot extend allowed origins;
   separate skill hosts must independently satisfy operator policy.
9. **Existing shortest paths and ownership.** Known bundled IDs/spec URLs go
   straight to `source_add`. Details/help remain optional. Preserve exact bundled
   API-base matching and unknown-base location discovery without inferring bases
   from endpoint paths, hosts or spec `servers` claims. Adequate HTTP metadata
   permits exact documented `raw_call`; Bazaar schemas are not OpenAPI imports.
   Listener authentication selects the owner, never agent/session/name fields.
   Registration never funds wallets. All production HTTP constructors remain in
   `src/network.rs`; preserve Tor remote DNS, identity isolation, cancellation
   ownership, complete accepted output and no-automatic-replay semantics.

## Preimplementation contract and clarity gate

The original gate's bundle/public-Tor requirement is replaced by the selected
scope above. Independent review found the revised search/relay route feasible;
full exact-contract and emitted-interface review remain required before D01.
Reviewers are read-only and receive user intent, baseline/P01A, transcript,
external evidence, relevant consumers and validation evidence. Challenge necessity
of checks/state, source identity, paid fallback authority and simpler reuse.
Resolve material findings before implementing the affected decision.

Freeze this seven-tool sketch against the actual definitions a client receives:

| Tool (prefix `x402_treazury_`) | Inputs and outputs |
| --- | --- |
| `sources_search` | Flat local/directory/mode selectors with mutually exclusive branch fields and conditional defaults above. Legacy local/x402 List outputs unchanged. Coinbase returns complete upstream metadata/partial state, directory and acquisition provenance, with visible paid-fallback guidance. No local or remote pagination for Coinbase search. |
| `source_details` | Exactly one x402 List `slug`, API `base_url` or `documentation_url`; preserve existing outputs for legacy branches, return complete accepted text or explicit failure for documents. |
| `source_add` | Exactly one `provider_id` or OpenAPI `spec_url`, optional `name`; preserve import/ownership semantics, fix `next` for disabled/zero-visible-tool registrations. |
| `providers_list` | Optional `page`/`per_page`; unchanged complete offline metadata/pagination without payer lookup. |
| `raw_call` | Exact documented `method`/`url`, optional `query`/`body`; preserve omitted body versus JSON null, payment authority and no automatic replay. |
| `tools_search` | Optional endpoint-local `source_name`, `query`, `cursor`, `limit`; unchanged complete schemas, live BM25/enumeration and pagination. |
| `tool_call` | Exact endpoint-local `tool_id`/`arguments`; current binding resolution, no management-tool recursion. |

Before D01, freeze exact Coinbase field names/types/requiredness/bounds, outgoing
filters, complete result envelope and operation-scoped errors. Verify the new
relay opt-in, configuration validation and payer mapping through all consumers.
Review shared versus per-invocation relay state concretely: preserve accepted-work
and payment safeguards while excluding catalog caching, shared stopped state,
target failure retention, readiness queues and added serialization from live search.
Reuse coherent request/envelope code rather than copying financial workflows or
adding a discovery framework. Location probing's HEAD/GET and markdown classifier
must not accidentally define the explicit document-reader contract.

Walk through the scenario matrix using emitted definitions without initialization
instructions. Preserve direct registration, optional help, absent-base resolution,
unknown-price honesty, live availability rather than cached source summaries,
endpoint identities, complete outputs and tools/list continuation. Record concrete
ambiguities and dispositions under ignored `target/`; semantic assertions assist
but do not replace independent comprehension review.

## Ordered implementation commits

### D01 Curated resource search, shared retries and paid fallback

Land offline search contracts, functional dispatch, shared Tor retry behavior and
usable operator-opted-in Curl fallback together. Do not land a promised fallback
without its payer binding, response validation, retained ownership or recovery
behavior. Include tool/parameter/error guidance and permanent docs in this commit.

Consumers: `src/discovery/{directory,local,mod,tools}.rs`, the Coinbase adapter,
`src/discovery_relay.rs`, `src/network.rs`, `src/payment.rs`, `src/server.rs`,
configuration/deployment/inspection construction and catalog/pricing relay callers.
Inspect the full schema → identity → origin → relay → result chain, including
existing relay callers affected by shared code. Avoid unrelated payment refactors.

Acceptance evidence:

- Legacy x402 List/default/local behavior remains; local requests do no wallet or
  network work. Mixed/unsupported arguments reject before identity or I/O.
- Curated default, explicit broadening, known bundle filters, supported payment
  filters, advisory price filters and actual upstream bounds translate exactly.
- Exactly one selected directory workflow; initial origin plus at most two eligible
  Tor retries, or one origin request in direct mode. Honor valid Retry-After;
  no duplicate outer loop. Complete valid JSON governs conditional promotion.
- Valid empty/partial results never fall back. Disabled fallback and rejected
  arguments/origins produce no paid side effects. Enabled fallback carries exact
  query and endpoint payer identity. Public origin/identity acquisition performs
  no wallet allocation/promotion or observations; paid fallback adds no funding/
  readiness wait and retains ordinary paid-client journal/signing/state transitions.
  Search-only relay enablement works without serve/warm, static relay bindings or
  startup wallet preparation. Public origin attempts never check relay readiness.
- Curl gets one retained paid workflow; unknown signed failure grants no replay.
  Validate target, status, redirect/truncation/error and full JSON. Repeated search
  does not reuse stale catalog response/target failure entries. Unrelated requests
  make progress without newly introduced serialization; accepted work survives
  waiter cancellation and drain retains its financial owner.
- Preserve exact resource metadata, same-host/method variants, unknown fields,
  missing contracts and unsupported members. Complete outputs, honest provenance
  and no invented executable schemas/totals/pages.
- Permission and endpoint isolation, public destinations, remote DNS and no direct
  fallback hold. Existing catalog/pricing relay behavior and safety remain intact.

Update `docs/agent-sources.md` search/results/payment guidance,
`docs/configuration.md` relay setting/selection/inspection semantics, relevant
maintained examples and both network/resource inventories for changed ownership.
No new resource policy is needed merely to populate an inventory.

**G1 after D01 — Discovery transport and financial ownership.** Independently
review D01 and every fixup with complete local/x402 List/Coinbase/server/network/
payment/relay call chains and existing catalog/pricing consumers. Verify opt-in,
endpoint payer isolation, attempt counts, eligibility/Retry-After/promotion,
freshness, accepted cancellation ownership and no new paid replay or readiness
waits. Compare CogC/CycC against baseline, including unchanged high-risk consumers.
Review fixup causes and repeated assumptions for wider hazards, regression gaps
and responsibilities that can be removed. Resolve material findings before D02.

### D02 Optional unsigned usage documentation

Add `source_details(documentation_url=...)` with its complete unsigned reader,
identity, origin/help policy, retained ownership and visible outcomes. Keep
legacy slug/base semantics and no-Curl document behavior intact.

Consumers: `src/discovery/{mod,tools,source_details}.rs`, source-client identity,
network/output conversion, `src/server.rs` and registration/raw-call handoffs.

Acceptance evidence: exclusive selectors and source-management permission;
management wrappers cannot recurse through tool_call; on-demand alone still
exposes only tool search/call. Test text, rejected origins/separate skill hosts,
non-text/HTML, missing, redirect, 402 and oversized documents. Assert no payment,
relay, registration, wallet mutation or crawling. Reuse source-matched ownership
and slow-progress/inactivity tests where unchanged; unrelated progress cannot
renew this response. Known IDs/specs bypass details, sufficient HTTP metadata
permits exact raw calls, and help never grants execution authority.

Update `docs/agent-sources.md` workflows/document provenance/instructions and
affected configuration/network/resource guidance. Provider caveats need only
concrete compatibility findings, not a universal provider inventory.

### D03 Whole-interface ergonomics

Review all seven tools together: purpose, every parameter/branch/default, outputs,
errors, empty-result hints, paid-fallback disclosure, registration `next`,
initialization, endpoint instruction appends and operator inspection. Lead with
the shortest applicable action; keep branch detail beside selectors. Distinguish
bundled IDs, x402 List slugs, known bundle search filters, exact URL/method, API
base, spec/document URLs, registration name and callable tool ID. No bundle tools
or mandatory discovery pipeline are implied.

Fix the demonstrated `Manager::result` guidance defect: disabled or zero-visible-
tool registrations must not promise an executable next call. Add focused coverage
and appropriate exact-SHA/fixup review. Preserve complete outputs, pagination 25,
optional help and no cached source summaries. Update enduring workflows in
`docs/agent-sources.md`, relevant README examples and `docs/configuration.md`.

**G2 after D03 — Integration and clarity.** Independently review D01–D03 and all
prior fixups against baseline and inherited P01A fixes, with complete discovery →
origin/relay/document/import → publication/search → raw/bound call → payment
chains. Include all seven tools, visibility/permissions, cached/paginated
advertising, exact selectors, error recovery, permanent docs and G1 concerns.
Use emitted definitions and actual synthetic results for the scenario matrix.
Review complete CogC/CycC evidence, concrete simpler representations and fixup
patterns; decide whether wider audit, simplification or added regression coverage
is needed. Exact-commit approval does not replace this accumulated-work review.
Exit with no material findings before cleanup.

### D04 Canonical handoff and plan cleanup

Verify maintained guides contain all enduring rules, examples/counts/defaults
match the interface, and both resource inventories agree. Check AGENTS links to
source-management, network, testing and complexity guidance; repair missing links
without copying detailed contracts. Transfer only unfinished bundle-access work
to the focused deferred plan and its index link. Remove this completed search
plan and index entry; no completed-plan archive, transcript deletion or financial
evidence cleanup. Independently review the exact cleanup SHA and final combined
branch, including all earlier commits/fixups. Later material changes reopen the
affected gates and require appropriate validation before completion.

## Discovery scenario matrix

Evaluate shortest action, identifiers, provenance and incorrect inferences against
emitted definitions, synthetic results and error recovery, without relying on
initialization text.

| Scenario | Required clarity |
| --- | --- |
| Known bundled ID/spec URL | Register directly; details/help optional. |
| Local provider without configured base | Import resolves schema server, not spec origin. |
| Already callable API | Search schema then call on this MCP endpoint. |
| Unknown x402 List service | Its slug is x402 List-specific; optional details can locate help/specs. |
| Coinbase resource with sufficient metadata | Exact documented raw call without registration or repeat lookup. |
| Coinbase spec/skill link | Import published spec or optionally read usage help; no universal pipeline. |
| Missing method/schema or rejected help | Missing stays missing; do not guess paid arguments or declare unusability. |
| Known bundle slug | Optional search filter only; no bundle listing/order/completeness claim. |
| Tor origin challenge, relay enabled | Existing initial plus two retry policy, then paid Tor-to-Curl route with disclosed provenance/trust. |
| Relay disabled/unavailable or ambiguous paid failure | Visible scoped failure; no hidden spend, funding wait, automatic paid replay or direct egress. |
| Valid empty/partial search | No paid fallback or fabricated continuation/absence claim. |
| Repeated search | Fresh request; each fallback may charge, no process-lifetime catalog result cache. |
| Two listeners or concurrent searches | Listener identity owns payer; no global catalog queue or owner selection by names. |
| Same host/path with different methods | Preserve individual resource identity; no inferred API base or merge. |
| Ambiguous bundled matches | Explicit provider ID; no arbitrary companion. |
| Disabled/zero-visible-tool registration | Next guidance reflects availability and recovery options. |
| Unknown price/curation/quality | No free-call, cap, reachability or delivery guarantee. |
| Cached definitions after registration | No static source inventory; live search reflects visible tools. |
| More than 25 advertised definitions | Follow tools/list continuation; preserve invalidation. |
| Custom/suppressed initialization instructions | Tool requirements understandable; append semantics unchanged. |

## Validation and review ordering

- Complete the revised preimplementation exact-contract/clarity review before
  substantive code. Review the exact plan revision SHA independently as well.
- Every implementation/documentation/fixup/cleanup SHA receives an independent
  review by someone other than its author, with intent, scope, baseline, evidence
  and known concerns. Review surrounding consumers, not only changed lines.
  Fix findings with `git commit --fixup=<introducing-SHA>` and independently review
  each fixup; preserve unsquashed history through final integration review.
- G1/G2/cleanup examine all preceding plan commits and fixups, complete call chains,
  recurring causes, testing gaps and removable responsibilities. Material later
  changes reopen affected concerns; do not build on unresolved material findings.
- Use focused synthetic tests for dispatch, complete schemas/results, network
  policy, retry/relay sequencing, identities, permissions and forbidden financial
  side effects. Small external fixtures need a named parser/translation failure;
  no full inventories or golden descriptions. Confirm tests are wired and execute.
- Run focused discovery/server/relay/network tests through `scripts/zcash.sh test`.
  Exercise tools/list continuation and cached-wrapper search/call after registration,
  eager/on-demand modes and endpoint ownership. Keep validation sources/binaries
  frozen; requested feature configurations run sequentially.
- Choose permissions before tests: ordinary sandbox for socket-free targets,
  socket-capable execution up front for localhost fixtures under
  [test sandbox guidance](../testing.md#sandbox-permissions). Use temporary state
  and public unfunded deterministic keys. Prior operator authorization covered
  the completed relay experiment only; this plan authorizes no additional live
  wallet/paid tests, funded qualification or consensus/Docker execution.
- For substantive Rust changes and G1/G2 run
  `python3 scripts/complexity.py --base a919de522b5bd5f373012d51f01bec2b0d5bf83e`.
  Verify complete source-matched JSON/CSV, changed functions and unchanged high-risk
  chains, including shared relay/catalog/payment consumers. Compare moved functions
  explicitly; disposition warnings with concrete simplifications or justified
  ownership, not helper extraction just for scores. Reuse valid unchanged evidence.
- Run `cargo fmt --check` and focused wrapper Clippy; run
  `sh scripts/check_toolchain.sh` before plain Homebrew Cargo. Include applicable
  `tests/network_audit.rs` checks. Broaden/repeat only for new changes or unresolved
  concerns. Reports remain ignored under `target/`; offline fixtures do not prove
  live Tor access, settlement or comprehensive qualification.
