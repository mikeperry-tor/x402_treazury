# Coinbase curated discovery and workflow bundles

## Intent and baseline

Integrate Coinbase-curated x402 resources and workflow bundles with
`source_management = true`, preserving the existing paths for bundled providers,
x402 List, OpenAPI registration and documented raw HTTP calls. Review the full
discovery API for clarity so agents can choose the shortest appropriate action.
This is a plan only; implementation and live financial qualification have not begun.

Starting baseline: `a919de522b5bd5f373012d51f01bec2b0d5bf83e`. That commit retired
`docs/plans/uniform_pagination_and_tool_inventory.md`; read its parent version,
especially P01A, alongside the current implementation. The untracked
`discovery_confusion.md` supplies agent-confusion examples and corrected claims;
preserve it without treating provider counts, prices, reachability or proposed
paid tests in that transcript as authoritative facts or execution instructions.
Preserve all unrelated tracked, untracked and ignored user files.

Currently seven management tools cover offline bundled metadata, x402 List
browse/best search, optional service/base discovery, OpenAPI registration,
endpoint-local tool search/call and raw HTTP calls. `source_details` accepts only
an x402 List slug or API base; its probes return document locations and outcomes,
not document contents. Remote discovery uses an existing endpoint wallet's
identity and transport. There is no Coinbase directory, bundle interface or
general advertised-document reader.

The plan and documentation-index entry are committed at `498f2570`; the
implementation baseline above remains unchanged.

Read and maintain these owning guides:

- [Agent sources](../agent-sources.md): tools, discovery workflows, identities,
  instruction placement, registration and persistence.
- [Configuration](../configuration.md) and [catalog architecture](../architecture.md#catalog-configuration-and-execution):
  selection, endpoint settings, schema import and execution.
- [Network egress](../network-egress.md), [local resource limits](../local-resource-limits.md),
  [best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules) and
  [wallet observation policy](../wallet-rotation.md#wallet-observation-policy).
- [Provider policy](../../providers/README.md#provider-policy),
  [provider caveats](../../providers/CAVEATS.md),
  [testing](../testing.md#provider-regressions), [development](../development.md)
  and [complexity review discipline](../../tests/COMPLEXITY.md#review-discipline).

## External contracts

Use reviewed REST contracts through existing Rust/network infrastructure:

- [Bazaar extension](https://docs.x402.org/extensions/bazaar): endpoint discovery
  metadata and HTTP/MCP resource distinctions.
- [Coinbase buyer guide](https://docs.cdp.coinbase.com/x402/buyer/discover-services):
  public discovery without a CDP API key.
- [Resource search](https://docs.cdp.coinbase.com/api-reference/v2/rest-api/x402-facilitator/search-x402-resources):
  relevance-ranked search, upstream `curatedOnly` and `bundleSlugs` filters,
  at most 20 results, `partialResults`, and no search pagination.
- [Bundle list](https://docs.cdp.coinbase.com/api-reference/v2/rest-api/x402-facilitator/list-x402-bundles)
  and [bundle detail](https://docs.cdp.coinbase.com/api-reference/v2/rest-api/x402-facilitator/get-an-x402-bundle):
  unpaginated bundle summaries and ordered resource metadata.

The REST examples include generic bearer headers despite the public-discovery
guide. Verify unauthenticated search/list/detail behavior before depending on it;
do not introduce credentials from generated examples. Resolve schema/example
contradictions against the authoritative contract, including bundle version
metadata. A development HTTP read can verify an interface, never paid delivery.
No probe may sign, pay, create an account or acquire credentials. If an operation
actually requires credentials, revise its design before implementation.

## Proposed behavior and preserved contracts

1. **Explicit directory choice.** Add `directory="coinbase"|"x402_list"` to
   `sources_search`. Omission preserves x402 List behavior. `local_only=true`
   remains offline with its current pagination and no payer lookup; reject a
   remote selector or remote-only fields combined with it before I/O. No search
   federation, silent fallback, startup fetch, crawling or local directory index.
2. **Coinbase curated search.** Use `q` with upstream `curatedOnly=true` by
   default, an explicit `curated_only=false` option, `bundle_slugs`, and reviewed
   upstream filters such as tags and maximum USD price. Translate names once at
   dispatch. Proposed mode semantics: Coinbase accepts omitted mode or `best`,
   both selecting ranked search; explicit `browse` is unsupported in this first
   increment. Existing x402 List browse/best behavior stays intact. Review this
   conditional default at the preimplementation clarity gate before freezing the
   schema. Broad Coinbase inventory/merchant browsing is outside the initial scope.
   Apply supported payment-network/asset/scheme search constraints server-side
   where the upstream contract supports them; price filters remain advisory.
3. **Faithful results.** Preserve directory provenance, exact resource URL,
   protocol type, declared method, request/output metadata, advertised payment
   offers, curation/quality signals and `skillUrl` when present. Preserve unknown
   or missing fields faithfully. Search returns complete metadata for its selected
   resources, including Bazaar schemas, so usable requests do not require another
   catalog fetch. Report upstream partial results without invented totals,
   continuation pages or claims that empty results prove absence. Do not silently
   turn HTTP examples or a schema for the Bazaar metadata envelope into a request
   argument schema. Missing contracts remain visibly missing.
4. **Two bundle tools.** Add `x402_treazury_bundles_list` and
   `x402_treazury_bundle_details(bundle_slug=...)`, identifying Coinbase as their
   directory. List returns summaries; detail preserves member order and complete
   resource metadata using the same representation as search. Bundles recommend
   workflow components; they do not install sources, execute steps, require every
   member or grant payment authority. Preserve unsupported members with an explicit
   support indication rather than silently dropping them or changing bundle order.
5. **Optional document inspection.** Extend `source_details` with an exclusive
   `documentation_url` input for explicitly reading an advertised concise usage
   guide such as `skillUrl`. Retain the existing slug and base branches, and label
   `slug` explicitly as an x402 List service identifier. Return complete accepted
   text with its URL/provenance; apply existing help acquisition bounds and visible
   failures. This branch is an unsigned read: no payment, redirect following,
   automatic linked-document traversal or execution of document instructions.
   Reuse the endpoint source client's documentation identity/transport and public
   destination/origin checks; cross-origin skill hosts cannot authorize themselves
   through catalog metadata. Explicit document input still respects operator
   `allowed_origins`. Do not use raw paid calls to fetch help or large specs.
6. **No invented resource lookup.** Coinbase search has a substring URL filter,
   not a documented exact-resource detail operation. Search and bundle detail
   therefore supply resource schemas directly. Document inspection needs no
   registration, server-side selection token, result cache or repeat catalog
   search. Do not derive an API base by stripping an endpoint path, merge different
   methods/resources sharing a URL or host, or convert a resource URL to a spec URL.
7. **Existing execution paths.** Known bundled provider IDs or published OpenAPI
   URLs go directly to `source_add`. Optional `source_details(base_url=...)`
   retains exact bundled-base matching and unknown-base document discovery. Never
   infer a bundled match from the resource's host or spec document's `servers`
   claim. Documented HTTP endpoints can use `raw_call` with their exact method and
   arguments. Bazaar JSON Schema is not an OpenAPI import. MCP resources are
   discovery metadata only until a separately designed MCP transport exists.
8. **Preserve authority and progress.** Listener authentication selects the owner;
   registration, names, filtering, persistence and payment limits stay operator
   controlled. All production HTTP constructors remain in `src/network.rs`.
   Preserve identity-bound direct/Tor transport, remote DNS, no direct fallback,
   cancellation ownership and existing read-inactivity/download behavior. Public
   discovery/document reads must not wait for funding, allocate/promote wallets or
   start observations. Actual API calls retain normal challenge/sign/journal/retry
   and no-automatic-replay semantics. A failing directory affects its own request.
9. **No new resource policy.** Reuse existing response/help bounds and transport
   semantics. Preserve actual upstream argument constraints; do not introduce
   additional local caps, queues, time budgets, caches, spending counters, provider
   price enrichment or pre-import tool counts. Update both resource inventories
   if implementation changes their descriptions or ownership, without inventing
   policy solely to populate an inventory.

## Preimplementation design and clarity gate

### Access gate: unresolved

Unsigned contract checks found an external blocker before D01. Direct search at
`https://api.cdp.coinbase.com/platform/v2/x402/discovery/search` returned HTTP 200
with resource metadata and `partialResults=true` for `query=weather`,
`curatedOnly=true`, `network=eip155:8453`, `scheme=exact`, `limit=1`.
Direct bundle list and `bundles/defi-trading` returned HTTP 401 with
`Unauthorized`, including a repeat with an explicit JSON Accept header.
The public-discovery guide therefore does not establish working unauthenticated
bundle access on these routes.

The user additionally requires keyless discovery over Tor and authorized testing
through `127.0.0.1:9150`. All three routes returned HTTP 403 HTML with
`cf-mitigated: challenge` through SOCKS5 with remote DNS. These are upstream
access failures, not evidence that request parsing or Tor transport succeeded at
the API layer. No paid calls, credentials, redirects or direct fallback were used.
Minimal diagnostic evidence is under ignored `target/coinbase-contract-gate/`.

After explicit operator authorization, one live toolbox `raw_call` to the paid
Curl relay fetched the same Coinbase search URL successfully: HTTP 200 JSON,
one complete resource with Bazaar metadata/payment offers, `partialResults=true`,
and no redirects or relay error. No Coinbase API key or headers were supplied.
The operator identified `comprehensive.toml` as live; its network policy uses Tor
at `127.0.0.1:9150`. This establishes search through the live Tor-configured
client to Curl's own target egress, not direct Tor-exit access to Coinbase or
independent packet-level route verification. The test reused the running server;
no second process opened wallet state. Exact settlement/charge was not exposed by
the MCP result. Sanitized evidence is `target/coinbase-contract-gate/relay-search.json`.
Bundle access through Curl remains untested. Integrating an operator-opted-in
paid search relay requires a design revision; the existing automatic relay
integration covers catalog/pricing discovery, not this search tool.

Independent read-only review agrees that this gate cannot pass unchanged.
Resolve public bundle access and the Tor requirement, or explicitly revise the
scope and acceptance criteria before D01. A pending scope question offers public
search/document inspection with honest Tor failures and deferred bundle tools,
or retaining the original scope until upstream access is resolved. Neither
alternative establishes working Tor discovery. Preserve this unfinished plan
and its index entry while these requirements remain unresolved; do not retire it
as completed work. Do not infer permission to acquire credentials or substitute a
relay, direct fallback or challenge workaround. The subsequent one-call relay
test was separately authorized; it does not by itself authorize automatic paid
fallback for future discovery calls.

### Reviewed input/output sketch

The independent clarity review identified the following implementation contract;
external access remains unresolved as above. Existing outputs remain unchanged
except for the demonstrated registration-guidance defect described below.

| Tool (all names prefixed `x402_treazury_`) | Inputs and result contract |
| --- | --- |
| `sources_search` | Flat `local_only`, `directory`, optional `mode`, and branch-specific fields. No global `mode` default: omission means browse for local/x402 List and ranked search for Coinbase. Local accepts only `q`, `page`, `per_page`, optional browse mode and its true selector; any explicit directory or remote field rejects before payer lookup/I/O. Omitted directory preserves x402 List with its existing browse/best arguments and outputs. Coinbase accepts omitted mode or best, rejects browse/pagination/x402-only fields, defaults curated filtering on, and translates only reviewed upstream filters. Return explicit directory provenance, complete upstream metadata and upstream partial state without invented totals or continuation. |
| `source_details` | Exactly one x402 List `slug`, exact API `base_url`, or explicit `documentation_url`. Preserve legacy listing/matching/discovery outputs. Document reading uses one unsigned GET, the source client's documentation identity/transport, accepted-work ownership, origin policy and help bounds. Return requested URL, caller-supplied URL provenance, content type and complete accepted UTF-8 text or a visible failure. Reject HTML/non-text; never treat a caller-supplied URL as verified Coinbase advertisement. |
| `source_add` | Exactly one known `provider_id` or OpenAPI `spec_url`, optional `name`; existing import, endpoint ownership and registration outputs. Tailor `next` to disabled/zero-visible-tool registrations instead of promising an executable call. |
| `providers_list` | Optional `page`/`per_page`; existing complete offline metadata and pagination without payer lookup. |
| `raw_call` | Exact documented `method`/`url`, optional `query`/`body`; preserve body omission versus explicit null, ordinary payment authority and no automatic replay. Sufficient resource metadata can support a direct documented call. |
| `tools_search` | Optional endpoint-local `source_name`, `query`, `cursor`, `limit`; existing live complete schemas, BM25/enumeration and pagination. |
| `tool_call` | Exact endpoint-local `tool_id` and `arguments`; current binding resolution, with no management-tool recursion. |
| `bundles_list` | Empty object; explicit Coinbase provenance and unpaginated upstream summaries. Subject to unresolved public-access gate. |
| `bundle_details` | Exact `bundle_slug`; explicit Coinbase provenance, ordered members and the same complete resource representation as search. Subject to unresolved public-access gate. |

Keep each complete upstream resource object intact, with any local support
annotation outside that object. Preserve unknown fields, missing contracts,
same-URL method variants and all payment offers. Do not convert the Bazaar
envelope schema or examples into invented request schemas. Existing location
probing's HEAD/GET sequence and markdown classifier are unsuitable as the
explicit document-reader contract. Catalog metadata cannot extend
`allowed_origins`; a separate skill-host origin must satisfy the existing policy.
Do not add provenance tokens, result caches or a same-origin requirement.

The seven baseline-tool walkthroughs retain direct known-ID/spec registration,
optional details/help, schema-server resolution for absent configured bases,
unknown-price honesty, endpoint-local identities, live search rather than cached
source summaries, complete outputs and tools/list continuation. Full synthetic
result and emitted-definition walkthroughs remain D03/G2 work, not accomplished
by this input/output sketch.

Before D01, an independent read-only reviewer challenges the proposed schema,
metadata representation, documentation branch and full user workflows. Give the
reviewer the user intent, baseline, retired P01A, transcript, current consumers and
external contract evidence. Resolve material findings before implementation;
update this plan if exact selectors, mode semantics or boundaries change.

Freeze a concise input/output sketch for every tool, including unchanged tools,
and walk through the scenarios below using the definitions a client actually
sees. Evaluate whether branch-specific defaults and identifiers are clear without
server initialization instructions. Prefer deletion of redundant text and state
over adding more wrappers or a discovery framework. The current plan has already
received independent design and ergonomics analysis; this gate verifies the exact
implementation contract and any intervening changes.

## Ordered implementation commits

### D01 Coinbase curated resource search

Add reviewed offline request/response contracts and functional dispatch together.
Keep old directory and local behavior explicit; share only representations and
transport behavior with real consumers. Implement Coinbase search with complete
resource metadata and correct mixed-argument validation. Include useful descriptions,
parameter help, result/error guidance and maintained documentation in this commit.

Relevant consumers: `src/discovery/{directory,local,mod,tools}.rs`, directory
request binding and `PaidClient` source identity, `src/server.rs` dispatch and
definition construction, deployment/inspection consumers and network audits.
Inspect the full chain from schema validation through response conversion.

Acceptance evidence:

- Omitted selectors retain x402 List browse/best behavior and ordinary payment
  handling; local enumeration/search remains offline without wallet work.
- Coinbase defaults to upstream curated filtering; explicit broadening, bundle
  filters, supported payment filters and advisory price filters translate exactly.
  Unsupported or mixed fields reject before I/O. Preserve upstream bounds.
- One selected directory request per invocation; no automatic catalog fallback,
  fetch on startup, provider probe, registration, wallet allocation or funding.
- Full exact URL/method/schema/provenance survives conversion, including different
  resources on one host, missing metadata, partial/empty results and unsupported
  protocol/payment metadata. No invented executable schema or pagination.
- Authenticated listeners use their own configured payer/isolation identity;
  direct/Tor/public-destination behavior and response failures stay scoped.

Update `docs/agent-sources.md` search/selection/results and
`docs/configuration.md` where advertised settings or instruction guidance changes.
Update network/resource inventories if their descriptions change.

**G1 after D01 — Discovery dispatch and authority review.** Independently review
D01 and all fixups alongside complete old/new directory, local, server dispatch,
network, response and payment consumers. Verify unauthenticated contract evidence,
absence of funding readiness gates and new replay authority, source identity,
partial-result honesty and the clarity of conditional search defaults. Compare
CogC/CycC with the agreed baseline, including unchanged high-risk consumers.
Analyze fixup causes for wider hazards, missing regression coverage and removable
responsibilities. Resolve material findings before D02.

### D02 Workflow bundles and optional usage documentation

Add bundle listing/detail using D01's resource representation. Add the unsigned
`source_details(documentation_url=...)` branch with the existing document identity,
help acquisition policy and operator-origin restrictions; preserve legacy slug/base
semantics and accepted work ownership. Implement each capability with its necessary
handoff and recovery behavior in this commit, rather than landing an unusable
reader or installing sources automatically.

Relevant consumers: `src/discovery/{mod,tools,source_details}.rs`, the D01 adapter,
`src/server.rs`, source-client/documentation identity and output conversion, and
registration/raw-call guidance. Inspect existing unsigned probe helpers before
sharing them; generic spec acquisition need not have the intended document identity.

Acceptance evidence:

- Bundle summaries have no fake pagination; detail preserves metadata and member
  order. Empty/unknown bundles and unsupported members have honest outcomes.
- All new tools require source-management permission; on-demand mode alone still
  grants only tool search/call. Management tools cannot recurse through tool_call.
- Advertised usage documents are fetched only on explicit inspection, unsigned
  and without link crawling, redirects, payment, registration or wallet mutation.
  Validate destination/origin policy including separate skill-host origins and
  direct/Tor identity reuse. Missing/rejected/oversized/non-text documents produce
  visible, operation-scoped outcomes without claiming usable request contracts.
- Accepted document work retains existing cancellation ownership; slow progress
  versus true inactivity uses existing response behavior, with unrelated progress
  unable to renew that response. Reuse valid source-matched tests where unchanged.
- Known IDs/specs bypass details; sufficiently documented resource metadata goes
  directly to raw_call; a skill document or bundle never grants execution authority.
  Preserve exact method/query/body semantics, especially same-URL method variants.

Update `docs/agent-sources.md` tool/workflow/instruction sections and affected
configuration/network/resource guidance in this commit. Update provider caveats
only for concrete compatibility behavior, not a universal provider inventory.

### D03 Full discovery ergonomics and clarity

Review and improve all seven baseline tools plus both bundle tools together.
Include top-level descriptions, every parameter and schema branch/default,
outputs, errors, empty-result hints, registration `next`, initialization guidance,
endpoint instruction appends and operator inspection. Functional commits already
supply correct guidance; this commit resolves whole-surface inconsistencies and
unnecessary repetition exposed by the independent walkthrough.

Lead each description with its purpose and shortest applicable action. Put
branch-specific detail with its selector/parameter. Distinguish bundled provider
definitions from Coinbase-curated resources, and keep these identities distinct:
`provider_id`, x402 List service `slug`, `bundle_slug`, exact resource URL/method,
API base, spec URL, document URL, registration `source_name` and callable `tool_id`.
Do not append another directory's full prose to an already dense overview.

Review `Manager::result` guidance for disabled registrations and registrations
with zero visible tools: successful registration must not imply an executable
next call. Scope any resulting behavior fix to demonstrated result-guidance defects,
with regression coverage and its own exact-commit/fixup review as applicable.

The independent clarity reviewer uses the scenario matrix below against emitted
definitions and actual synthetic results, not source strings alone. Record review
evidence under ignored `target/`, naming concrete ambiguities and their disposition.
Automated semantic assertions support the review; they do not prove comprehension.

Update enduring workflows in `docs/agent-sources.md`, relevant README examples,
and `docs/configuration.md` instruction placement guidance. Preserve the absence
of cached source-state summaries, complete outputs, pagination defaults and
optional provider help established by the retired plan.

**G2 after D03 — Full integration and ergonomics review.** Independently review
D01–D03 and every prior fixup against the baseline, with complete discovery →
document/import → catalog publication/search → raw/bound call → payment chains.
Include G1 concerns, all nine tools, visibility/permission combinations,
cached/paginated definitions, exact selectors, error recovery and permanent docs.
Review relevant inherited branch fixes from retired P01A so new wording does not
undo them. Revisit every fixup cause and repeated assumption; decide whether it
requires a wider audit, regression test, simplification or design change. Compare
complete CogC/CycC evidence and evaluate concrete simpler representations for
hotspots. Exact-commit approvals do not replace this combined review. Exit with
no material findings before cleanup.

### D04 Canonical handoff and plan removal

Verify maintained guides contain all enduring rules, examples match the final
interface, wrapper counts/defaults are accurate, and both resource inventories
remain consistent. Verify `AGENTS.md` links to the owning source-management,
network, testing and complexity guidance; repair links when needed without copying
detailed contracts into AGENTS. Remove this completed plan and its index entry.
Do not archive it, remove the untracked transcript or alter financial evidence.

Independently review the exact cleanup SHA and final combined branch, including
all earlier commits and fixups. Material later changes reopen affected gate
concerns and receive appropriate revalidation before completion.

## Discovery scenario matrix

For each scenario, review the shortest correct action, required identifiers,
provenance and common incorrect inference against descriptions, schema, outputs
and error recovery. Use small synthetic inputs and actual emitted tool definitions.

| Scenario | Required clarity |
| --- | --- |
| Known bundled ID or known OpenAPI URL | Register directly; details is optional. |
| Local provider with absent configured base | Import resolves schema server; never infer it from spec origin. |
| Already available API tool | Search when schema is needed, then call on the same MCP endpoint. |
| Unknown x402 List service | Its service slug belongs to x402 List; optional details can locate specs/help. |
| Coinbase resource with sufficient request metadata | Exact documented raw call is possible without registration or another lookup. |
| Coinbase resource with OpenAPI or skill link | Import the published spec server-side or optionally read usage help; no universal pipeline. |
| Missing method/schema, no spec, rejected help | Missing information stays missing; do not guess paid arguments or claim unusability. |
| Bundle and its selected members | Recommendations and member order do not authorize execution or install tools. |
| Same host with different paths or methods | Preserve individual URL/method identity; do not merge into a service or infer a base. |
| Ambiguous bundled matches | Select an explicit provider ID; no arbitrary companion selection. |
| Disabled registration or zero visible tools | Next guidance reflects actual availability and recovery options. |
| Empty/partial remote search | Directory matching/completeness differs from local BM25; no fabricated continuation. |
| Unknown price, curated label or quality signal | No free-call, payment-cap, reachability or paid-delivery guarantee. |
| Cached definitions across registration | No static source inventory; live search reflects current visible tools. |
| Identical wrappers on two MCP endpoints | Client prefix selects the listener; names/registrations cannot select owners. |
| More than 25 advertised definitions | Follow tools/list continuation to find wrappers; preserve cursor invalidation. |
| Custom endpoint instructions or suppressed initialization text | Essential tool guidance remains understandable and append semantics unchanged. |

## Validation and independent review ordering

- Plan preparation receives independent read-only design and ergonomics review.
  Before substantive implementation, complete the exact-contract gate above.
  The coordinating agent integrates findings and owns completion.
- Every implementation, documentation, fixup and cleanup commit receives an
  independent review of its exact committed SHA and relevant surrounding consumers.
  The reviewer must not be the author. Give reviewers intent, scope, baseline,
  validation evidence and known concerns; ask them to question unnecessary checks
  and verify claims. Keep reviewers read-only and avoid competing builds/edits.
- Address findings with `git commit --fixup=<introducing-SHA>` and independently
  review every fixup. Preserve unsquashed history through final integration review.
  Do not build on unresolved material findings. Reopen affected gates after material
  changes; per-commit approval never exempts code from accumulated-work review.
- Use focused shared synthetic tests for selectors, schema/result fidelity,
  identities, permissions and forbidden side effects. Add small faithful external
  contract fixtures only for a named parser/translation failure or authored
  transformation. No full provider inventories or golden description snapshots.
  Assert schema branches/defaults and semantic guidance without treating keyword
  presence as the whole ergonomics review.
- Run relevant discovery unit and server/integration tests through
  `scripts/zcash.sh test`, confirming changed tests are wired and actually execute.
  Select filters/targets after inspecting module wiring; a zero-test success is not
  evidence. Exercise complete tools/list pagination and cached-wrapper search/call
  after registration, including eager/on-demand and endpoint ownership cases.
- Choose permissions before tests: ordinary sandbox for socket-free targets;
  socket-capable execution up front for localhost fixtures under
  [test sandbox guidance](../testing.md#sandbox-permissions). Use temporary state
  and public unfunded deterministic keys. No live wallet secrets, paid provider
  calls, funded qualification or consensus/Docker work is authorized by this plan.
- For substantive Rust changes and G1/G2 run
  `python3 scripts/complexity.py --base a919de522b5bd5f373012d51f01bec2b0d5bf83e`.
  Verify complete source-matched analysis, then inspect full JSON/CSV for changed
  functions and unchanged high-risk call chains. Compare renamed/moved functions
  explicitly. Disposition new/worsening warnings and concrete simplifications;
  advisory scores and passing tests do not justify current organization. Do not
  extract helpers merely to lower a score. Reuse unchanged valid artifacts.
- Run `cargo fmt --check` and focused Clippy through `scripts/zcash.sh`; run
  `sh scripts/check_toolchain.sh` before plain Homebrew Cargo commands. Scale the
  final checks to actual affected consumers, including `tests/network_audit.rs`
  where applicable. Do not repeat a full suite after localized fixes without new
  concerns. Keep sources/executables frozen during validation and requested
  feature configurations sequential. Store reports under ignored `target/`.
- Before retiring the plan, verify canonical documentation and AGENTS links, review
  the cleanup SHA, then review the final combined branch. Report concrete evidence,
  findings and limits; offline fixtures never establish live settlement or Tor.
