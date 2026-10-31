# Observed pricing in tool descriptions and results

## Intent and starting baseline

Make observed API pricing available to agents through concise tool descriptions
and a separate JSON text block in MCP call results. Review and clean up existing
provider pricing prose first, so new observations replace generated pricing and
do not accumulate alongside redundant credit conversions or uncertainty warnings.
Then simplify agent-facing API errors to their error kind, actionable request
correction where relevant, and retry properties before adding observed pricing.

This is an implementation plan. Its creation does not implement these behaviors
or authorize funded qualification. The starting revision is
`0ba0758395038ef8aa028ae703f93e97734754cf`. Preserve unrelated tracked, untracked
and ignored work, including the untracked `TODO` and `swap_logs.txt` present at
planning time. Reconfirm the baseline and intervening changes before execution.

Current behavior:

- `src/catalog.rs::operation_description` combines provider description/summary,
  structured spec pricing and optional credit conversion. It can preserve original
  credit-pricing prose alongside a generated USDC estimate. Explicit description
  overrides apply afterward. Description truncation is an existing separate setting.
- `src/catalog/credit_pricing.rs` converts structured credit tariffs; it does not
  generally parse prices embedded in arbitrary vendor prose.
- `src/rotation/manager.rs::pay_with_cover_observed` learns a selected offer after
  challenge validation and before financial admission. The process cache contains
  amounts, keyed by trusted source/revision, method, operation template, wallet group
  and transport scope. Receipt ordering, the existing 24-hour lifetime and existing
  capacity govern those hints; agent arguments do not partition them.
- Hints guide managed-wallet selection. They currently change neither descriptions
  nor call results. Static signing delegates selection to the SDK and does not
  populate this observation cache.
- MCP results contain typed content blocks, not necessarily a JSON provider body.
  `HttpOutput` becomes `ToolOutput`, and `ToolOutput::into_content` emits text,
  images and advisories. Text-only convenience methods also serve internal JSON
  consumers. The server additionally supplies `structuredContent` for selected
  management tools.
- Listing and search use captured listener catalog views. Search ranks canonical
  descriptions and uses generation-bound cursors. Current HTTP serving does not
  promise a tool-list-change notification channel; clients can retain descriptions.

## Owning contracts and preserved invariants

Read the linked contracts before changing their area:

- [Agent source documentation and endpoint instruction placement](../agent-sources.md#tool-documentation),
  [search semantics](../agent-sources.md#tool-search-matching-and-results), and
  [trusted invocation](../agent-sources.md#trusted-invocation-and-payment-scope).
- [Pricing metadata](../configuration.md#pricing-metadata-and-runtime-hints),
  [description and display limits](../configuration.md#download-and-display-limits),
  and [configuration](../configuration.md) for overrides and credit conversion.
- [MCP output boundaries](../architecture.md#mcp-and-output-boundaries),
  [catalog execution](../architecture.md#catalog-configuration-and-execution), and
  [selection/isolation](../architecture.md#wallet-selection-and-network-isolation).
- [Provider policy](../../providers/README.md#provider-policy),
  [provider caveats](../../providers/CAVEATS.md), and
  [provider regressions](../testing.md#provider-regressions).
- [Best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
  [wallet observation](../wallet-rotation.md#wallet-observation-policy), and
  [reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy).
- [Existing resource limits](../local-resource-limits.md),
  [timer inventory](../network-egress.md#remaining-timers-and-budgets),
  [development](../development.md), [validation](../testing.md#validation-order-and-continuation),
  [Make entrypoints](../../scripts/README.md), and
  [complexity discipline](../../tests/COMPLEXITY.md#review-discipline).
- [Independent workflows and plan requirements](../../AGENTS.md#independent-subagent-workflows).

Current challenge validation, signing authority, payment limits, journaling,
insufficient-funds traversal, accepted-work ownership and cancellation behavior
remain authoritative. Displaying an observation adds no replay, funding or signing
authority. Keep the existing cache scope, receipt ordering, TTL and capacity.
There are no new pricing probes, network constructors, timers, readiness gates,
persistent pricing records, allocation rules, dependencies or vendor patches.

## Provider review comes first

Review all shipped provider definitions before implementing either new display
surface. Inspect provider summaries, `instructions_text`, description overrides,
help descriptions and parameter descriptions, including authored local schema
prose where it contributes to the generated descriptions. Follow actual catalog
generation and override precedence rather than treating keyword matches as proof.

Remove repeated pricing uncertainty language: generic statements that displayed
prices are estimates, may vary, are not guaranteed, depend on the challenge or
cannot prove settlement. Remove repetitive instructions to distrust pricing or
avoid ordinary calls. Keep any necessary common interpretation once in shared
server guidance, using the shortest useful wording. Do not migrate boilerplate
into another tool field or reproduce it in every pricing JSON object.

The review must also identify authored price suffixes in overrides, credit text
already converted by catalog generation, and cases where a future observed line
would otherwise duplicate a price. Correct repository-authored strings at their
source where practical. Do not build a parser for all vendor pricing sentences.
An override's literal `Cost: ... [spec]` is authored text, not proof that catalog
generation owns that suffix. Remove such duplication only when the structured
fallback still preserves the useful price or the price is deliberately relocated
to its owning structured metadata; do not discard a provider's only tariff fact.

Known starting points, not an exhaustive edit list:

- Exa repeats `displayed prices are estimates` in tool descriptions. Remove that
  boilerplate while retaining concise facts about per-result summary charges and
  per-page content charges. Review adjacent request-size advice for unnecessary
  discouragement; express useful billing behavior directly.
- Datakoot repeats a guide/spec pricing discrepancy and generic estimate warning
  in source instructions. Move useful discrepancy rationale to maintainer caveats
  if still necessary, retaining task guidance such as US geographic coverage.
- SocialFetch source guidance describes configured credit conversion and the
  presence of base/surcharge text. Keep it accurate when observed prices replace
  generated pricing, and avoid repeating conversion prose per tool.
- FindMyDirectDoctor and WebIntel help descriptions are candidates for removing
  generic settlement/replay commentary. Review their concrete guide/schema
  differences separately from repeated pricing provisos.

Preserve concise facts that change request construction or actual billing, such
as charged errors, non-refundable operations, metering units, paid options or a
free quota becoming paid. Preserve unrelated API/data semantics. This pass is
about pricing clutter, not deleting coverage, async-completion, secret-recovery or
data-quality facts because they contain words like `not guaranteed`.

Runtime advisories about a particular accepted authorization, possible payment,
uncertain retry or actual output omission remain intact. They describe the
operation that occurred and are distinct from speculative provider boilerplate.
Preserving that evidence does not require exposing its full diagnostic prose:
the API-error presentation work below conveys the retry consequence once.

Keep a reviewed audit inventory under ignored `target/`, with inspected surfaces,
removed boilerplate, retained billing facts and any concrete unresolved cases.
Do not add inventory snapshots, blanket forbidden-word tests, remote catalog
fetches solely for this editorial pass, or a completed-audit archive. Place enduring
provider-specific explanations in `providers/CAVEATS.md` and general rules in the
owning documentation.

## Proposed description behavior

Use one self-contained line:

```text
Search company records. Observed cost: ~$0.005/call.
```

The observed line replaces the entire generated pricing section, including spec
credit text and the configured conversion. It does not append to that generated
blob. Each new rendering selects the current hint; no price history accumulates.
When the hint expires or is absent, use the existing complete fallback description.
In particular, the existing SocialFetch structured-credit fallback and conversion
arithmetic remain intact unless P1 identifies a specific editorial correction;
this plan does not introduce a second tariff-conversion or fallback-pricing redesign.
Do not infer `/page`, `/record` or another unit from an authored tariff: the
observed amount is for a request, so use `/call` and USDC dollar formatting.

### Retain generated boundaries at construction

Extend the existing catalog rendering path, rather than reparsing its final string.
Keep the canonical full fallback description and optional runtime-only information
identifying its pricing-free base and applicable description limit. Choose the
smallest representation that covers the real consumers; no pricing history or
parallel provenance framework is needed.

Capture the base before combining generated pricing. Handle the current exact
suffix-deduplication case: when authored text already ends in the identical suffix,
the retained base must not leave that same generated pricing behind. Preserve
unrecognized arbitrary vendor prose without heuristically deleting dollar amounts
or credit sentences.

Explicit description overrides invalidate the generated-base information. Render
their authored text followed by one observed line. P1 should remove unnecessary
repository-authored price duplication where known; externally supplied arbitrary
overrides are not parsed. Existing description limits and visible truncation remain
applicable to the final generated display, without new caps or silent truncation.
Review existing override precedence explicitly so a runtime overlay neither
silently resurrects replaced text nor changes unrelated override behavior.

`src/discovery/curated.rs::build` rebinds canonical tool names in descriptions.
Rebind both the full description and retained pricing-free base. Preserve this
information through ordinary clones and listener binding. Keep it out of stable
serialized formats: persisted registrations rebuild tools from saved source specs
through `import::saved_tools`. A deserialized tool without runtime rendering
information uses its full description as the base for one labeled observed line;
it must not guess which text was generated.

### Render from the trusted binding

Add a shared presentation method on or alongside `BoundTool`. Derive the lookup
key through the existing execution key path in `PaidClient::operation_key`, using
the same source/revision, method, normalized template, effective wallet group,
destination and transport policy. Tool names and agent arguments cannot select the
hint scope. Review direct-target operation URLs and dynamic source rebinding.

Use this method for `Server::definition` callers (`list_tools` and `get_tool`) and
tool-search item descriptions. Extend any other active definition/detail surface
that publishes the same callable description; inspect consumers rather than
assuming those three are exhaustive. Inspection without a live binding retains
authored/generated pricing and initiates no observation work.

Search continues ranking and paginating canonical snapshot descriptions. Apply
the live pricing overlay to selected result items afterward. Hint changes and
expiry do not reorder search, alter counts, mutate catalog generation or invalidate
cursors. Document that canonical text is indexed while the returned description
can contain a newer price. Stored catalog descriptions remain unchanged.

Fresh listings/searches see the current applicable hint. This work does not add
notifications, connected-client registries or guaranteed refresh of cached client
definitions. Review endpoint summaries and discovery keywords for accuracy; do
not add pricing boilerplate there as a substitute for concise tool descriptions.

## Proposed result content

For a single-text provider response, return:

```json
{
  "content": [
    {"type": "text", "text": "{\"temperature\":18}"},
    {
      "type": "text",
      "text": "{\"x402_pricing\":{\"amount\":\"0.005\",\"currency\":\"USDC\",\"basis\":\"observed_offer\"}}"
    }
  ]
}
```

The pricing block contains a complete JSON object. `amount` is an exact decimal
USDC string; `basis` identifies the observed offer without repeated warning text.
It is the most recent validated offer from that logical invocation, not a sum of
potentially chargeable attempts or a settlement receipt. Preserve that meaning in
the shared contract, not repeated per-call prose.

Append exactly one pricing text block after original provider text/images and
before existing advisories. Thus it is not promised to occupy literal index 1 for
image or multipart-content results. Keep the original first text, image metadata,
projection behavior and advisories intact. Do not inject fields into the provider
body, wrap its payload, concatenate the JSON objects, replace `structuredContent`,
or change existing `isError` semantics. A custom `_meta` mirror is unnecessary for
this initial implementation.

Multiple text blocks are valid MCP. Clients that concatenate all text and parse it
as one JSON document can still fail; two JSON blocks do not solve that client
assumption. Verify the wire contract and supported client paths, and document the
additive content-block change without claiming universal host compatibility.

### Capture the actual call observation

Define a small typed optional observation shared by the payment/output code. Use
existing integer amounts and one shared exact-decimal formatter for descriptions
and JSON. Do not format via `f64` or silently round small amounts.

At the managed `prepare_challenge` validation boundary, set a caller-owned optional
observation independently of `operation.zip(receipt)`. A missing cache key or
expired receipt must not suppress an otherwise validated per-call observation.
A local slot passed through `AdmissionObservation` is preferable to shared mutable
collectors, task-local registries or an observation ledger. Preserve the most recent
validated target offer across the existing wallet traversal without adding attempts.

Carry the observation through `execute_response_owned`, `response_output`,
`HttpOutput::render` and `ToolOutput`. Ensure image extraction and JSON projection
do not discard it. Only `ToolOutput::into_content` serializes the extra block.
Preserve `into_text` and body-only interfaces: internal help/directory consumers
must continue receiving the original parseable body rather than adjacent JSON
documents.

When a validated offer precedes admission, HTTP-body or output-conversion failure,
retain the observation in a small typed error context and extract it when building
the MCP error content. Preserve the original error category, qualification behavior,
possible-payment context and necessary internal diagnostics. Use P1E's concise
API-error presentation rather than displaying the typed context or diagnostic chain.
Review every early return after validation, not just the successful output path.
Malformed/unvalidated challenges never produce a price observation.

### Coverage and omission rules

- Cover managed target calls through direct tools and the discovery call wrapper.
  Managed manual raw calls return their own validated observation without creating an
  operation cache key or binding arbitrary destinations into catalog hints.
- Preserve current static-wallet signing and cache coverage. Its SDK-selected
  requirements are opaque at this boundary; do not duplicate SDK validation or
  add dependency patches merely to obtain display parity.
- Unsigned/free responses and local management operations without a validated
  target offer have no pricing block. Absence is not a synthesized zero price.
  Preserve existing zero-offer learning behavior.
- Help/cache-only results do not receive an old acquisition price as though it
  were newly observed. Relay acquisition fees do not become the target API price.
  Inspect nested `PaidClient` uses and retain that distinction across fallback.
- Aggregate directory search does not collapse multiple children into a scalar
  cost or silently attach the last child's price. Explicitly omit aggregate pricing
  for this change while preserving all existing child results and liabilities.

These omission rules constrain presentation only. They must not suppress API
work, change which wallets are tried, add probes or weaken financial evidence.

## Agent-facing API errors

API errors should tell the agent what kind of failure occurred and what it can
do next, retaining known HTTP status codes and identifying which service returned
them. The agent is not responsible for diagnosing transport, SDK, contract or
protocol implementation details. This is an API-error presentation change, not
a repository-wide logging cleanup or universal error-system redesign.

Replace the current model-facing dump of accumulated error contexts with a small
shared presentation path for API execution failures, used by direct calls and
the discovery call wrapper. Retain the typed error chain internally for existing
classification, payment evidence, qualification and diagnostics. Do not replace
internal errors with strings, or change all MCP errors to `error.to_string()`.
Inspect API failures returned through raw calls and affected nested wrappers too.
Apply the same presentation to API-failure text embedded within successful wrapper
or aggregate envelopes; preserve sibling results and existing envelope/error flags.

The presentation contract is:

- Lead with the established error kind: for example, payment rejected, invalid
  arguments, provider unavailable or response unavailable. Preserve useful
  parameter names and accepted-value corrections for errors the agent can fix.
  Use established local validation or recognized structured provider categories;
  arbitrary upstream bodies do not supply trusted request corrections.
- Include the provider API's actual HTTP status code and conventional status name
  when a response was received. Distinguish, for example, HTTP 402, 403, 404, 429
  and 503 instead of flattening them into a generic failure. Transport or body
  failures without an observed status must not acquire an invented HTTP code.
- Attribute each status to its source. A fetch provider's API HTTP response and
  a destination HTTP response reported inside its result are separate outcomes.
  A successful provider HTTP 200 containing a destination HTTP 404 is not a
  provider API HTTP 404. If both statuses are known, preserve both and label them
  `Provider API` and `Destination`; retain an established destination error kind
  such as access denied or not found when useful.
- Use actual outer HTTP response facts and existing documented response contracts
  or explicit mappings to identify nested destination statuses. Do not search
  arbitrary text for HTTP numbers, infer origin from a generic `status` field, or
  add a provider-specific parsing framework. Where destination attribution is
  not established, preserve the native provider result without inventing it.
  Preserve provider result bodies and existing envelope/error flags: an embedded
  destination failure is not automatically an MCP execution error.
- State one concise retry instruction when needed, derived from existing typed
  evidence and execution/replay rules. For potentially chargeable submitted work,
  `Do not retry automatically.` is sufficient; do not repeat settlement,
  authorization, execution and special-traversal explanations around it.
- Keep omitted-extension details, nested contract/SDK traces, internal fallback
  mechanics, empty object/null placeholders and repeated uncertainty disclaimers
  out of the agent-facing API error. Do not include diagnostic material merely
  because it was attached to an internal error context.
- Keep concrete provider error kinds distinct from local wallet or treasury
  conditions. A provider's insufficient-funds rejection does not establish a
  local balance diagnosis. Classify recognized structured evidence; do not parse
  arbitrary prose into new retry permission.
- Do not describe an error as retryable merely because it is transient or no
  signature was sent. Preserve the existing unsigned GET/POST and accepted-work
  replay contracts. Where safe retry is not established, do not invite replay.
- Preserve `isError`, existing ownership/recovery behavior and internal error
  identity. Unknown/unclassified failures still produce a concise meaningful
  failure kind and the applicable retry instruction, without leaking the whole
  chain as a fallback. Use existing typed categories and minimal local matching;
  do not create an error taxonomy framework or provider-specific string parsers.

### Concrete payment-error example

The motivating case is an API call marked as potentially chargeable, with a
provider API HTTP 402 response whose error text includes `insufficient_funds`.
The existing agent-facing message is:

```text
A payment may already have settled; an authorization may still charge or execute; do not automatically repeat this logical call: Managed payment omitted advertised x402 extensions; this does not establish the cause of failure. A submitted authorization may still charge. Only the built-in recognized insufficient-funds traversal may repeat the request: HTTP 402 Payment Required: "insufficient_funds: contract call failed: unable to call contract: execution reverted" {}
```

This combines the `PossiblePayment` context from `src/payment/submission.rs`,
`OMITTED_EXTENSIONS` from `src/rotation/manager.rs`, and the `HttpFailure` detail
built by `src/payment.rs::response_output`. `Server::call_tool_owned` currently
exposes their combined chain with `{error:#}`. These are presentation layers,
not evidence of multiple API executions.

The example establishes a provider payment rejection and the local possible-payment
marker. That marker is set before journal acceptance is acknowledged; it does not
alone prove authorization submission or disclosure. The example also does not
establish a shortage in the local wallet/treasury, settlement success, or exhaustion
of all eligible wallets. The extension-omission
observation does not identify the rejection's cause. Preserve the HTTP status and
retry property without asking the agent to diagnose those internals.

When the available structured evidence identifies insufficient funds, the intended
agent text is:

```text
Provider API HTTP 402 Payment Required: insufficient funds. Do not retry automatically.
```

If only HTTP 402 and possible-payment evidence are established, use
`Provider API HTTP 402 Payment Required: payment rejected. Do not retry automatically.`
Do not infer an insufficient-funds category by searching arbitrary error prose,
and do not describe every HTTP 402 as insufficient funds. Build synthetic fixtures
for both cases, retaining internal typed evidence while asserting concise external
output and unchanged dispatch/signature counts.

For a fetch whose provider request succeeded but whose documented result reports
a destination failure, the status distinction is:

```text
Provider API HTTP 200 OK; destination HTTP 404 Not Found.
```

The actual known failure and retry evidence determine the wording. Retry properties
apply to repeating the provider tool call; a destination HTTP 429 or 503 does not
by itself authorize another paid fetch. Preserve omitted-extension observations
and contract diagnostics internally where existing consumers need them. Their
retention does not require new logs, and credential-bearing upstream prose must
not be logged.

P2's pricing JSON remains a separate content block on these error results when
the invocation observed a validated offer. It must not reintroduce diagnostic
contexts or repeat the retry instruction in a second place.

## Ordered implementation commits

Use these stable identifiers when recording actual SHAs and review evidence.
Every commit includes its relevant tests and permanent documentation. Keep each
behavior and necessary replacement/recovery path together.

### P1 Provider pricing prose review and cleanup

Complete the provider review above before runtime implementation. Edit only
authored pricing clutter and associated stale guidance, covering all shipped
definitions and their actual generated surfaces. Preserve useful concise billing
facts, request contracts, provider selection and endpoint coverage. Review the
affected endpoint summaries/search guidance without inventing new capabilities.

Update the tool-documentation policy in `docs/agent-sources.md` and its provider
reference in `providers/README.md` to make the concise pricing rule durable. Put
any necessary individual provenance/discrepancy explanation in `providers/CAVEATS.md`.
Keep SocialFetch's interim wording true before and after P3.

Evidence: reviewed audit inventory and diff, offline bundled-definition validation,
affected provider/schema contract tests, and small regressions for named authored
transformations where useful. Avoid universal inventory snapshots and tests that
ban generic words. Record before/after examples relevant to P3 replacement.

**G1 Provider and design gate, after P1:** independently review the exact P1 SHA
and all fixups, surrounding generation/override consumers, the complete audit
coverage and retained billing facts. Resolve discovered override/credit/truncation
cases in the P2/P3 design before implementing affected paths. Exit when the added
pricing can be represented without new provider parsers or redundant disclaimers.

### P1E Concise API error kinds and retry properties

After P1/G1 and before P2, inspect the full API execution-to-MCP error path. Start
with `PossiblePayment`, `OMITTED_EXTENSIONS`, paid response-body failures, wallet
exhaustion, `HttpFailure`, `AdmissionError` and `Server::call_tool_owned`. Separate
internal error evidence from agent presentation as specified above, rather than
merely shortening every context string and continuing to dump the whole chain.
Carry an already observed outer HTTP status through body-read and output-render
failures with a small typed handoff; do not reconstruct it from diagnostic text.
Review the receipt/body projection in `src/payment/diagnostics/http.rs` and its
comments, which currently describe upstream prose as private tool-error content.
Align that contract with the new agent presentation without relocating raw upstream
prose into logs or extending its lifetime unnecessarily.

Implement and test the small shared API-error formatter and its direct/wrapper
handoff in one coherent commit. Preserve existing typed causes, diagnostics,
qualification categories, `isError` and retry enforcement. Avoid unrelated CLI or
operator-log wording changes. Update `docs/architecture.md` output/error boundaries
and `docs/agent-sources.md` with the agent-facing error-kind/retry contract; update
affected diagnostic documentation only to clarify the internal/presentation split.

Evidence: the concrete payment-error example in the API-error section becomes one
short payment-rejection message with its
HTTP 402 status; provider HTTP 403/404/429/503 remain distinguishable; successful
provider responses reporting destination failures retain both layers without
reclassifying the provider response or granting paid replay. Cover unavailable or
ambiguous destination attribution without guessing. Omitted
extensions, empty wrappers and contract traces are absent from agent content;
actionable argument corrections remain; signed and unsigned transport/body failures
produce the appropriate retry instruction; exhausted-wallet, relay and wrapper
paths retain classification and liabilities. Assert unchanged dispatch/signature
counts and no new retry rights. Use focused shared fixtures, not snapshots of all
provider error prose.

Independently review the exact P1E SHA and every fixup, including the full error
chain and consumers. Resolve material findings before P2 uses the new presentation
boundary. Include P1 and its fixups in this review and apply the complexity review
discipline to the new formatter and affected existing dispatch hotspots.

### P2 Per-call observed pricing in MCP content

Implement the typed observation, managed validation capture, output propagation,
typed error propagation and JSON content block together. Main owners are
`src/payment.rs`, `src/rotation/manager.rs`, `src/output.rs` and `src/server.rs`.
Inspect raw calls, media/projection, relay fallback, directory aggregation, help,
qualification and direct Rust interfaces as consumers. Do not silently widen the
existing cache's authority or scope to solve output propagation.
Use P1E's concise agent-facing error presentation; observation metadata must not
restore diagnostic-chain rendering on API failures.

Add focused validation/error, concurrency, wallet-handoff, output-format and
media tests with this commit. Update `docs/architecture.md` output boundaries and
`docs/configuration.md` with the exact block contract and omission rules.
Independently review P2 and each fixup before building P3 on its interfaces.

### P3 Replace generated pricing in live descriptions

Implement retained rendering information and the shared scoped presentation path.
Update catalog generation, overrides, curated rebinding, live tool definitions and
tool-search serialization together. Preserve fallback/expiry, limits, persisted
formats and canonical search semantics.

Add suffix/credit replacement, overlay, scope, reload, override, expiry and search
pagination tests with this commit. Update `docs/configuration.md` pricing/credit
guidance, `docs/agent-sources.md` discovery behavior and `docs/architecture.md`
catalog handoff. Update both cache inventories (`docs/local-resource-limits.md`
and `docs/network-egress.md`) where they currently describe hints as selection-only;
retain all existing bounds and lifetime semantics.

**G2 Combined architecture gate, after P3:** inspect P1, P1E, P2, P3 and every fixup
as one implementation. Trace validation through success/error/MCP output and
trusted bindings through cache lookup/description rendering/search pagination.
Inspect preserved financial ownership, cancellation, network isolation, override
handling, dynamic curation, reporting omissions and documentation together.
Review CogC/CycC against the starting baseline and local changes. Resolve all
material findings before broad runtime validation or building on affected choices.

### P4 Integration evidence and final provider presentation check

Complete cross-surface integration tests and resolve gaps exposed by G2. Verify
that a call reports its own offer while later descriptions show the newest
applicable cached hint, including concurrent reverse completion, source refresh
and expiry. Include direct and discovery-wrapper MCP paths, media and errors.

Recheck P1-cleaned provider descriptions through the implemented renderer. Confirm
observed lines do not resurrect removed prose, duplicate generated credit sections
or bypass authored overrides/limits. This verifies P1 rather than deferring the
provider audit until now. Update affected permanent examples/docs with the same
commit; add no unrelated production behavior to create an integration milestone.

**G3 Integration gate, after P4:** independently inspect the entire accumulated
branch, P1 audit dispositions, P1E error presentation, exact implementation/fixup SHAs, all prior gate
decisions and complete consumers. Examine recurring fixup causes and assumptions;
require wider targeted audit, simplification or regression coverage where a
pattern warrants it. Confirm validation and documentation handoff before cleanup.

### P5 Remove the completed plan after permanent handoff

Verify that current rules, examples, billing distinctions and support scope live
in the owning guides. Confirm `AGENTS.md` still links the correct provider,
tool-documentation, payment and output guidance; adjust links only where the
handoff changes their destinations. Update the documentation index and remove
this plan once all implementation and required reviews are complete. Retain only
concrete unfinished work in an appropriately scoped plan, without an implemented
plan archive. Preserve private financial and validation evidence under `target/`.

Independently review the exact cleanup SHA and final combined branch after removal.
Reopen any affected gate for material cleanup changes. A documentation-only cleanup
does not itself invalidate source-matched runtime evidence; verify links and final
scope explicitly.

## Focused acceptance coverage

Prefer shared synthetic fixtures and existing harnesses. Relevant existing owners
include `src/payment/hint_tests.rs`, `src/catalog_state/operation_url_tests.rs`,
catalog unit modules, discovery tests, and `tests/credit_pricing.rs`,
`tests/pricing.rs`, `tests/pricing_inspection.rs`, `tests/media.rs`,
`tests/provider_catalogs.rs` and `tests/provider_defaults.rs`. Confirm exact test
module wiring and executed counts rather than relying on filters that can run zero
tests. Do not freeze full provider inventories or rewrite unrelated fixtures.

| Concern | Required evidence |
| --- | --- |
| P1 editorial cleanup | Named boilerplate removed; useful charged-option/unit/error facts retained; TOML and authored schema contracts still valid. |
| Exact amount | Decimal strings cover sub-cent values, integer amounts and large supported values without floating-point loss; text and JSON agree. |
| Per-call ownership | Concurrent calls with different prices retain their own outputs even when receipts and completion order differ; no lookup of a newer shared hint at result time. |
| Validation and failure | Malformed challenges omit pricing; valid observed offers survive admission/body/render failures without new signing or loss of original error/possible-payment context. |
| Agent-facing API errors | Concise kind, observed HTTP status and applicable retry instruction; provider API versus reported destination status distinguished; no invented status/origin or changed success/error flags; useful request corrections retained; no diagnostic chain or duplicate warnings; direct, wrapper, raw, relay and known/unknown failures preserve typed evidence and replay rules. |
| Wallet traversal | Existing eligible-wallet attempts and private-header restrictions remain unchanged; latest validated offer is identified without presenting a total or permitting replay. |
| JSON and media | Original text is byte-for-byte preserved where the existing contract promises it; pricing is separately parseable; images, mappings, omissions and advisories survive. |
| Internal consumers | `into_text`, helper/directory parsing, `structuredContent` and qualification behavior retain their contracts. |
| Generated replacement | Full spec-plus-credit sections are replaced once; exact suffix deduplication and arbitrary provider prose are handled as designed; repeated renders do not accumulate. |
| Overrides and limits | Authored overrides retain their text; observed append respects applicable limits and visible truncation; base text is not accidentally resurrected. |
| Scope and lifecycle | Source/revision, method/template, wallet group, destination and transport isolation; direct-target URLs, removal/refresh, curation and reload. |
| Expiry and search | Existing receipt TTL/order/capacity unchanged; expiry restores fallback; updates leave canonical ranking, counts, generation and cursors unchanged. |
| Reporting boundaries | Static/unsigned/no-offer, cached help, relay-only fees and aggregate directory calls follow explicit omission rules without changing work or liabilities. |
| Forbidden side effects | Listing, lookup, rendering and search create no HTTP, signing, allocation, funding, retries, catalog writes or notification tasks. |

Use barriers and existing controllable clocks for deterministic races/expiry.
Cancellation/disconnection cases must show accepted work continues through its
existing owner and does not become a new paid request. Preserve existing slow
progress/stall tests and extend them only if an implementation change touches
those paths; this plan adds no failure timers.

## Independent review and complexity evidence

Reviewers receive user intent, baseline, commit scope, exact SHA, relevant consumers,
commands/results and the above invariants. Keep review/audit agents read-only;
delegate distinct concerns where useful and coordinate any edits with explicit
file ownership. Do not run competing builds or edit sources during validation.

The author does not review their own exact committed SHA as the independent
reviewer. Resolve findings with `git commit --fixup=<introducing-SHA>` and obtain
independent exact-SHA review for every fixup. Preserve unsquashed history through
the final integration review. Per-commit approval does not replace G1/G2/G3 or the
post-cleanup final review. If independent tooling becomes unavailable, disclose
the limitation and do not claim an independent review occurred.

Run the pinned complexity analyzer against the starting baseline, using
`COMPLEXITY_BASE_REF=0ba0758395038ef8aa028ae703f93e97734754cf` with Make checks.
Check resolved base, source hashes, complete analysis and exclusions before using
the reports. Compare inclusive and own CogC/CycC for changed functions and complete
call chains, especially `execute_response_owned`, managed payment admission,
`operation_description`, curated rebinding, output rendering, search serialization
and existing server dispatch hotspots. Inspect unchanged high-risk hotspots too.
Review local fixup comparisons when needed to distinguish local and branch effects.

Disposition every relevant warning explicitly. Prefer deleting duplicate formatting,
derived state or ownership choreography; do not add helpers merely to lower a score.
Passing tests and necessary behavior alone do not justify an unnecessarily complex
representation. Generated metric/review artifacts belong under ignored `target/`,
not in maintained documentation. Retain concise conclusions with review evidence.

## Validation ordering and completion

Use repository Make entrypoints with pinned Cargo/protoc. First coordinate editor
and other Cargo processes. Keep sources fixed while checks run, executable paths
fixed while tests use them, and feature configurations sequential. Socket-free
checks use the ordinary sandbox; select socket-capable execution up front for
localhost fixtures. Use deterministic unfunded keys and temporary state, never
real secrets, `.env` or live-funded qualification.

For each affected commit, run inexpensive contract tests before their broad suites.
After G2 and source stabilization, complete these stages in the documented order:

1. `make check-static COMPLEXITY_BASE_REF=0ba0758395038ef8aa028ae703f93e97734754cf`.
   Complete the independent complexity/design review before broad runtime suites.
2. `make check-clippy check-compat`. Inspect production compiler and all-target
   Clippy diagnostics as well as exit status, including emitted dependency warnings.
3. Run the focused tests selected from the acceptance matrix and actual consumers.
   Examples of entrypoint syntax are `make test CARGO_ARGS='--test credit_pricing'`
   and `make test CARGO_ARGS='--test media'`; select real new test names after they
   exist and verify nonzero execution. Do not narrow the fixed `check-*` stages.
4. Finish `make check-default check-proving check-lifecycle check-codec` sequentially.
   G3 requires all named stages accounted for, including any focused continuations.

P1's editorial/configuration checks can precede Rust implementation. Later source
changes invalidate relevant prior compiler, complexity and runtime evidence and
require revalidation. After a localized fix, rerun affected consumers and resume
unfinished stages; carry forward evidence only with a recorded scope justification.
Material later changes reopen the relevant architecture gate. No Docker consensus,
live settlement or real-Tor qualification is implied by these offline tests.

Record exact SHAs/source inputs, feature/profile selections, commands, diagnostic
dispositions, test counts and exits in ignored evidence. Before completion, verify
all provider audit findings, exact-SHA/fixup reviews, gate findings, test stages,
permanent documentation and post-cleanup review are resolved. Report precisely
which surfaces changed and the retained static/aggregate/client-refresh boundaries.
