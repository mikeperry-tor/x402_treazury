# Improved pricing discovery

Status: proposed implementation; initial plan commit and coverage clarification
snapshot independently approved; exact coverage commit review pending. No implementation or live
payment qualification has occurred.

## Intent, baseline and scope

Provide explicit, reusable provider configuration for advertised pricing and use
usable advertised amounts as initial wallet-selection hints, before the first
unsigned API request obtains a payment challenge. This must work without a prior
API call, learned price, persisted price row or remote catalog cache entry,
including providers backed by local authored catalogs. Preserve the distinction
between an advertised estimate and an observed validated offer.

Starting branch: `main`. Starting baseline:
`4346421a1a4d0d53fc31e246ff6fde27f634a9f6`. Plan path:
`docs/plans/improved-pricing-discovery.md`. The pre-existing untracked `state`
entry is unrelated and must remain untouched. This plan authorizes no funded
probes, wallet-secret access, paid metadata acquisition, push or merge.

The research-desk audit found structured advertised pricing in live OpenAPI for
41 of 45 active sources: 19 also publish structured well-known prices, 22 provide
them only in OpenAPI, and Brazilayer/HALOWERK provide them only in well-known
metadata. Banking Compliance's checked live URLs returned 404; Concordance's live
schema lacked structured pricing. Both have authored local annotations. These
findings guide scope, not permanent availability guarantees. Generated evidence
is in ignored `target/pricing-audit/`; implementation must verify relevant source
contracts rather than depend on those artifacts being present in another checkout.

Current behavior:

- `Config.pricing_key` selects an operation member for description formatting;
  it does not seed wallet selection. Credit conversion and authored
  `pricing_description` are display-only behavior.
- `pricing::runtime::CatalogHints` owns observed prices tied to remote catalog
  cache generations/freshness. Local catalogs have no such observed-price owner.
- `BoundTool` passes an operation key into `PaidClient`; initial managed selection
  reads an observed hint or uses zero. After unsigned challenge validation,
  selection uses the actual offered amount and may hand off before signing.
- Saved balances and prices rank eligible wallets; they do not prove affordability
  or grant payment authority. An eligible fallback remains possible.

Owning guides, to read before affected implementation:

- [Configuration](../configuration.md), especially catalog caching, pricing
  descriptions and provider composition.
- [Catalog and execution architecture](../architecture.md#catalog-configuration-and-execution),
  [wallet selection](../architecture.md#wallet-selection-and-network-isolation)
  and [unsigned request semantics](../architecture.md#unsigned-x402-requests-and-post-retries).
- [Best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
  [wallet observation](../wallet-rotation.md#wallet-observation-policy) and
  [reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy).
- [Network egress](../network-egress.md), [resource limits](../local-resource-limits.md),
  [agent sources](../agent-sources.md), [provider policy](../../providers/README.md#provider-policy),
  [provider caveats](../../providers/CAVEATS.md) and [maintenance](../provider-updates.md).
- [Testing](../testing.md), [complexity review](../../tests/COMPLEXITY.md#review-discipline),
  [documentation policy](../README.md) and [planning workflow](../planning-workflow.md).

## Proposed contracts

### Configuration and normalized metadata

Add a typed `[pricing]` configuration describing a preferred source, a supported
format and an optional operation metadata key. Prefer the already-acquired
catalog; allow an explicit public credential-free metadata URL. Do not discover
well-known paths automatically, fetch both variants automatically, or introduce
priority arrays, provider-name dispatch or an extraction/transform language.

Illustrative syntax, to finalize at design gate D0:

```toml
[pricing]
source = "catalog"
format = "payment-info"
key = "x-payment-info"
```

```toml
[pricing]
source = "https://tools.halowerk.com/.well-known/x402"
format = "x402-resources"
```

Support existing `pricing_key` as compatible shorthand for catalog-based
payment-info parsing. Define deterministic inheritance, origin reporting and
conflict handling for legacy/new configuration; reject contradictory explicit
declarations rather than silently choosing one. Preserve unchanged legacy display
behavior. Keep format choices small and tied to demonstrated reusable layouts:
operation `x-payment-info`, operation `x-x402`, and the resource/payment-requirement
layouts needed by HALOWERK and Brazilayer. Use separate typed layouts if the last
two cannot be decoded faithfully as one. Add field pointers only if review finds
a concrete repeated need that these formats and the existing key cannot express.

Normalize advertised metadata once on the tool/catalog execution metadata and
derive descriptions and selection estimates from that interpretation. Preserve
fixed, dynamic/range, explicit free/zero and unknown distinctions, denomination,
unit, source provenance and applicable freshness. Retain useful display detail
when a tariff cannot yield a selection number. Authored description text and
credit conversion do not implicitly become monetary inputs.

Initially derive selection numbers only from supported, exact nonnegative
per-call USDC amounts, including documented USD-denominated USDC tariffs with
an established settlement contract. Decimal conversion uses exact arithmetic;
atomic amounts require the supported Base USDC asset/network and six-decimal
interpretation. Choose compatible payment alternatives, not their sum. Missing
currency, unsupported assets, ambiguous units, malformed/overprecise amounts,
dynamic minimums, credit rates and per-record/per-token tariffs must not silently
become fixed call charges. Preserve such tariffs for display where supported.
Request-dependent total calculation is outside the initial implementation.

### Initial provider adoption scope

The first rollout must cover the reusable provider files referenced by
[`examples/deployments/research_desk.toml`](../../examples/deployments/research_desk.toml),
not just demonstrate the parser with a few providers. At the starting baseline
there are 45 active source definitions and 45 distinct source `extends` files.
Also include the two additional relay-only definitions,
`providers/curl/provider.toml` and `providers/agproxy.toml`, for 47 distinct provider
files used by this deployment. Relay-only coverage means reviewing their own tool
pricing/configuration compatibility; it does not authorize paying for metadata
or enable hints on unrelated relayed operations. Commented-out SocialFetch is not
an active dependency. Resolve the actual source and relay references again during
P4 so changes to the deployment cannot silently escape coverage.

Apply supported pricing declarations in the reusable provider files, with existing
`pricing_key` retained where compatible shorthand suffices. Do not satisfy coverage
with research-desk-only overrides, runtime provider dispatch, or a blanket change
that merely declares every provider priced. Include local authored schemas as
well as remote catalogs, and preserve authored input/method repairs and tool
selection. Review other deployment consumers of every changed reusable definition
under the existing provider-maintenance policy.

For each in-scope provider file, P4 must record the chosen source/format/key,
currency/unit interpretation, and whether its relevant operations yield initial
selection hints. Where a file or operation remains display-only or unknown, record
the concrete reason and verify best-effort selection remains usable. A recognized
dynamic tariff is covered even when it cannot supply a fixed call estimate;
coverage does not mean inventing one. Review all configured operations and check
the research-desk-selected subset explicitly, rather than assuming a priced
operation elsewhere in the provider covers its deployed tools.

Required cases include ordinary `x-payment-info`, botsmith/Kronos `x-x402`,
HALOWERK/Brazilayer external metadata, authored local catalogs (including Banking
Compliance and Concordance), dynamic/metered tariffs and relay-provider definitions.
Treat the earlier audit's source choices as candidates to verify; external failure
or unsupported semantics must receive an explicit disposition, not disappear from
the denominator. Keep detailed generated per-provider evidence under ignored
`target/`; retain concise acceptance totals, exceptions and findings in this plan
and durable quirks in owning provider caveats. Do not add a universal fixture or
settings snapshot to prove inventory coverage.

### Initial selection, including cold caches

At the initial managed payer selection, before provider transport acquisition and
the first unsigned API request, choose the ranking amount in this order:

1. A currently valid observed challenge hint for the exact operation, if available.
2. A currently usable normalized advertised per-call estimate from its configured
   pricing source.
3. The existing unknown-price behavior (zero ranking amount).

Pass advertised metadata through the actual `BoundTool`/`PaidClient` call chain;
description rendering alone does not meet this requirement. Apply it to all
registered tool invocation paths, including dynamic sources and explicit operation
URLs. Manual raw calls have no registered operation price and retain their current
behavior. Static payer signing remains unchanged.

No observed-hint cache, persisted catalog/price row or previous unsigned request
is a prerequisite for step 2. Local authored catalogs supply advertised estimates
on their first build. Newly fetched remote catalogs/documents can supply estimates
from the acquired bytes even when HTTP caching is disabled or storage fails.
Local advertised metadata remains bound to its loaded configuration/catalog
generation; remote advertised metadata retains its own source freshness. Do not
create a remote cache slot merely to make a local estimate selectable.

This cannot supply a price before any pricing metadata exists: absent both loaded
catalog pricing and acquired external pricing, continue with step 3. Existing
catalog readiness semantics still govern whether a tool exists at all. An initial
invocation must not wait for external pricing acquisition, a balance RPC or
funding. An estimate arriving later may help subsequent invocations; it does not
restart an ongoing request or grant replay authority. Actual challenge selection,
validation and unsigned payer handoff remain authoritative after discovery.

Keep advertised fallback separate from observed `CatalogHints`; neither seeding
the learned-price map nor relabeling advertisements as observed is permitted.
Preserve existing receipt ordering, persisted observed-price restoration and
cache invalidation. An expired/invalidated observed hint can fall back to still
valid advertised data; refreshing advertisements cannot erase a valid observed
hint. Local catalogs' lack of learned-price persistence is an existing limitation,
not a prerequisite to expand in this work.

### External documents, mapping and lifecycle

Acquire explicitly configured metadata through `src/network.rs` using the source's
discovery identity, public-destination enforcement, compatibility settings and
Tor remote-DNS/no-direct-fallback contract. No payments or paid relay fallback;
a 402 is unavailable pricing evidence. Reuse appropriate existing HTTP acquisition
and cache machinery without introducing polling, whole-work deadlines, arbitrary
limits or a new persistence system. Apply existing document resource boundaries
with visible failure outcomes.

External enrichment is optional: source tools can publish and calls can proceed
while it is pending or unavailable. Scope failures to advertised hints and provide
safe diagnostics consistent with the owning logging severity policy. Tie work to
the existing source lifecycle; reload/removal/cancellation must not leave duplicate
unowned fetches or let an old source generation publish into its replacement.
Finalize the smallest existing lifecycle hook and shared enrichment representation
at D0; do not add a scheduler or readiness gate for prices.

Cache ownership must distinguish external URL/configuration/format and retain that
document's own acquisition/freshness facts. Reuse the configured catalog lifetime
policy where applicable, but never renew an external document by reading,
revalidating or replacing an unrelated OpenAPI document. Handle no-store and failed
cache persistence without making storage success necessary for fresh acquired
metadata. Discard external mappings on incompatible source/configuration changes.

Map external records against configured operation identities using method and
resolved original resource/template, including base paths and explicit operation
URLs. HALOWERK supplies explicit methods and resource URLs. Brazilayer omits methods
and uses example CNPJ URLs: accept a match only when the configured operation is
unique. Do not infer method from GET/POST preference, use arbitrary prefix matches,
or cross ambiguous templates/origins. Preserve authored method repairs; incompatible
or ambiguous records remain unknown with an actionable diagnostic. Synthetic
tests must cover the actual matching rule and its forbidden assignments.

### Preserved financial and execution invariants

Prices/balances remain advisory. An estimate above a cap does not remove otherwise
eligible wallets or become a new admission check; only the actual challenge and
current authority govern signing. No allocation, top-up, balance observation,
extra financial budget, replay or reserve consumption follows from a pricing read.
Keep journal-before-disclosure, accepted-work ownership, amount/cap enforcement,
wallet identity isolation and recognized signed-insufficient-funds traversal intact.
Unsigned discovery still obtains each payer's own challenge. Do not widen payment
rail support or financial authority to accommodate advertised metadata.

## Ordered commits and review gates

Stable identifiers remain in this plan through implementation and review.

| Step | Coherent change and consumers | Acceptance evidence and documentation |
| --- | --- | --- |
| P0 | Commit this active plan and resolve independent design findings. | Reviewed exact plan SHA; D0 dispositions and final configuration/lifecycle decisions recorded before implementation. |
| P1 | Typed pricing configuration, legacy compatibility and normalized catalog pricing. Consumers: config composition/origins, catalog conversion, local/remote tool metadata, descriptions and search output. | Focused parser/config/normalization tests; legacy display/credit/override behavior preserved. Update configuration, agent-source guidance and provider policy as applicable. |
| P2 | Carry advertised metadata into initial selection independently of observed cache ownership. Consumers: bound tools, registered/dynamic invocation paths, operation URLs, PaidClient and managed selection. | First-call cold-cache, local catalog, no-store/cache-write-failure, observed precedence/expiry, zero/unknown and advisory fallback tests. Assert selected identity before any unsigned request and forbidden RPC/funding/signing effects. Update architecture, wallet-selection and testing guides. Gate G1 covers combined P1–P2. |
| P3 | Optional explicit external metadata acquisition, freshness and unambiguous operation matching. Consumers: source build/publication/reload, shared network/cache lifecycle and tool execution metadata. | Localhost tests for delayed/failed/402 metadata with tools remaining usable; source replacement/removal, stale completion, no-store/expiry and method/template ambiguity. Assert no paid relay/payment, duplicate active acquisition or new readiness dependency. Update network/cache/resource inventories and source guidance where behavior changes. |
| P4 | Complete the initial provider adoption scope: all 45 active source provider files and two relay-only files currently referenced by research-desk. Prefer catalog, add well-known for Brazilayer/HALOWERK, retain compatible pricing_key shorthand and enable botsmith/Kronos x-x402 interpretation in reusable definitions. | Re-resolve all references and reconcile every provider file and deployed operation to a usable/display-only/unknown disposition. Validate composition and research-desk routing through production catalog paths, plus affected consumers in other deployments. Exercise initial hints for each supported layout and local/remote acquisition class with focused tests; small authored fixtures only for named quirks. Record coverage totals/exceptions, update caveats/guides. G2 requires complete accounted scope, not universal fixed-price availability, and covers all commits/fixups/call chains. |
| P5 | After G2 and applicable user review rounds, remove completed plan and repair its index link. | Enduring contracts in owning guides; exact cleanup SHA and final branch independently reviewed; unsquashed plan/review history preserved. |

D0 is a fresh independent design review of the exact plan snapshot and current
implementation. It must resolve configuration compatibility, normalization,
external acquisition/publication ownership and source freshness, external identity
matching and meaningful test choices. Challenge added responsibilities and verify
dependency/component contracts before inventing new machinery. G1 is the ownership
gate before layering external enrichment on selection. G2 reviews final combined
implementation, provider adoption, complete affected call chains and documentation,
including unchanged challenge admission and cache invalidation consumers.

Two concrete D0 decisions are required before implementation: define the usable
in-memory advertised lifetime when `catalog_cache_ttl_seconds = 0` or response
headers prohibit storage (existing cache metadata can be absent), and identify a
small source-generation handle for optional late enrichment across cloned tools.
Neither absent cache metadata nor missing disk rows may automatically reject an
otherwise fresh acquired estimate; neither may imply indefinite remote freshness.
Keep the handle separate from observed-price ownership and reuse existing source
lifecycle machinery rather than introduce a second publication framework.

## Validation and review discipline

Follow [validation order and continuation](../testing.md#validation-order-and-continuation)
and select checks by affected behavior. Complete applicable design and CogC/CycC
review and selected production compiler/all-target Clippy and compatibility stages
before broad runtime suites. Run affected cheap contract tests first; verify filters
execute the intended tests. Use Make's pinned entrypoints and coordinate builds
with other processes. Acquire socket-capable execution up front for socket fixtures.

Use focused deterministic tests rather than timing benchmarks or a validation
matrix. Additional cases include lower/higher observed prices, advertisements
above payment caps, low/unknown balances retaining eligible fallback, atomic versus
decimal alternatives, malformed fields and dynamic minimums, and concurrent calls
while external data is pending. Capture the initial selection in a socket-free
test or deterministic localhost fixture before the unsigned response is released.
Prove cancellation/reload cannot publish prices across source generations. Reuse
valid evidence after localized corrections and resume unfinished named stages.
No funded provider calls, Tor qualification or reproducibility/dependency audit is
needed solely for these changes.

Independently review every exact committed SHA and affected consumers, including
P0 and P5. Reviewers are read-only, not authors, and start fresh with self-contained
briefs containing intent, baseline, contracts, plan, exact scope, validation and
known limitations. Identify uncommitted snapshots by base and exact contents;
review the resulting committed SHA. Fix findings with `git commit --fixup` against
the introducing commit, independently review each fixup, and preserve unsquashed
history. At G1/G2 include all accumulated commits/fixups and complete call chains;
record initial assessment before consulting author rationale/prior findings.
Resolve material findings and compelling simplifications before gate exit.

For substantive Rust changes, retain ignored CogC/CycC reports against the starting
baseline and review hotspots for simpler ownership/decoding, not numeric thresholds.
Document source/build scope and limitations of all checks. Reopen affected gates
when later changes invalidate evidence. No implementation or validation claim may
be inferred from completing this planning task.

## Documentation handoff and evidence ledger

Permanent destinations: `docs/configuration.md` owns syntax, compatibility and
tariff meanings; `docs/architecture.md` and `docs/wallet-rotation.md` own initial
selection/observed precedence; `docs/agent-sources.md` owns dynamic source publication;
`docs/network-egress.md` and `docs/local-resource-limits.md` own acquisition/cache
lifecycle and inventories; provider README/caveats own supported formats and quirks;
`docs/testing.md` owns focused regression guidance where needed. Keep the existing
AGENTS.md change-area links to these guides accurate; add a link only if a new
owning section needs discoverability, not a duplicate policy in AGENTS.md.

Update permanent guides in their implementation commits. P5 removes this completed
plan only after final integration review and user review corrections, preserving
it in Git history. Report remaining work, missing coverage, working branch and exact
reviewed state at each handoff.

- Preliminary independent design assessment at starting baseline: supports typed
  source/format/key, normalized advertised tool metadata and separate observed
  cache. Findings incorporated above: ambiguous Brazilayer identity mapping,
  separate external freshness, local catalogs lacking observed cache ownership,
  dynamic/atomic semantics and nonblocking optional retrieval. This does not replace
  D0 review of this exact plan.
- P0 initial independent review: snapshot SHA-256
  `b38658da1ff544cf038db2a75a227cbb8c5eb07f52ba338821ae79e44a275999`
  against the starting baseline, plus the documentation index link. No material
  findings. Reviewer verified current pricing/selection/operation-URL call chains;
  disabled-cache in-memory lifetime and late-enrichment ownership were identified
  as explicit D0 implementation decisions above. Relative file links and
  `git diff --check` passed. Exact P0 commit
  `2d3cb49732e5cb680280b97b6fe0b8277efd5627` and its combined change against the
  starting baseline independently approved with no material findings. D0 must
  still approve resolved implementation decisions before substantive work.
- Coverage clarification requested after P0: make reusable research-desk provider
  files the mandatory initial adoption set, including the two relay-only files;
  strengthen P4/G2 acceptance without claiming every tariff is a fixed call price.
  Independent snapshot review against P0 approved with no material findings
  (SHA-256 `1fbc8037706fc3b6a43673e96c1c6b06f559838d43c557a4cd8ae5984050943f`).
  Reviewer independently verified the 45 + 2 file denominator, selected-operation
  coverage and relay-only distinction; relative links and diff checks passed.
  Exact committed-state review is pending.
- P1–P5, G1/G2: not started. No compiler/runtime validation claimed.
