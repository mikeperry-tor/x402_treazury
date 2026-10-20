# Provider contract fixtures

Provider-specific inputs must reproduce a named quirk, failure or authored
transformation; shared synthetic tests own generic parsing and selection. Do not
add a golden or freeze a full settings map merely because a provider was added.
All bundled TOML definitions are validated offline; generated tool inventories
and settings maps are not regression fixtures.

Retain inputs by responsibility:

- Request/routing quirks: Exa's ids/urls alternatives, Locus address alternatives
  and gateway rewrite, IPIntel nullability/arbitrary JSON, RecallRadar's open
  body and structured/prose price disagreement.
- Mixed payment/lifecycle boundaries: Exa, OneShot, StableEnrich, Agent402,
  BlockRun, RelayShield and Vaaya, with meaningful positive/negative assertions.
- Authored transformations: NewsGurus Bazaar projection, AgentUtility registry
  tag/alias enrichment, SocialFetch version naming and credit metadata.
- Captured unsigned payment evidence: AgentUtility challenges and
  provider_payment_challenges.json, independently consumed by signing tests.
- Offline qualification request schemas: retained vendor inputs validate reviewed
  requests; they establish neither live compatibility nor spending authority.

Production local catalogs under `providers/` and the Kronos/RegimeShift inputs in
`examples/live-tor-providers.toml` are runtime inputs. Preserve them independently
of test pruning. Check consumers in discovery, media, payment, startup, examples
and qualification before deletion. The offline qualification preset uses an explicit
selected-source input map in `examples/live_integration/presets.rs` and resolved
local production catalogs. The ignored startup benchmark uses independent synthetic
workloads and no provider index. Source witnesses for authored transformations
have focused fidelity consumers; their distinct production local catalogs remain
necessary. Do not retain an unconsumed vendor capture as a completeness claim.

Use `scripts/zcash.sh test --test provider_catalogs --test config` from the repository
root. Tests use no Python, credentials or live vendors. Review input/schema/routing
changes together; never regenerate snapshots merely to obtain a passing result.
Preserve full inputs only where a concrete comparison needs the entire inventory.
The following notes describe scoped inputs, their concrete consumers and refresh
basis.

## Reviewed request and method contracts

These inputs are scoped captures, not complete current vendor inventories. The
retained path objects preserve original parameters, request schemas, constraints,
required lists, descriptions, tags, payment metadata and response declarations
where present, with their transitive local references. Top-level vendor metadata
is retained; `x-treazury-fixture-scope` labels the reduced capture separately.

| Input | Retained contract and consumer |
| --- | --- |
| AgentFund | `/x402/treasury_yield_curve`: independent reviewed preset body schema. |
| Brazilayer | `/v1/cnpj/situacao/{cnpj}`: reviewed path argument and original parameter/response reference closure. |
| Deepline | `/api/company/enrich`: reviewed enrichment request, full nested schema and guidance. |
| PDL | `/api/pdl/company/enrich`: reviewed company enrichment schema and full field descriptions. |
| LoneStar | `/rates/rates`: reviewed rate request; original provider metadata retained. |
| GenuineGood | All five real GET/POST grant pairs plus bearer-pass exclusion: actual method/body serialization and canonical naming in `tests/catalog_selection.rs`. This small capture remains complete because all six paths have consumers. |
| Otto | Four actual GET/POST pairs, `/crypto-news`, `/swap` and `/full-auto`: method/body serialization, reviewed preset and actual execution-authority exclusions. |
| NetIntel | Existing eight-path subset: URL-secret and array POST pairs, compatible aliases, canonical image mapping. No further reduction preserves these distinct consumers. |

The offline preset input map in `examples/live_integration/presets.rs` validates
reviewed requests against these independently captured schemas; arguments never
supply expected schemas. Refresh from the configured vendor OpenAPI URL, select
complete original path objects and their reference closure, and review request
and method/name consumers together. Preserve naming witnesses if removing a
sibling would rename a selected tool. Fixture size never justifies trimming
schema properties or vendor prose. None of these inputs grants paid execution
or establishes current upstream health.

## Compatibility witnesses

The following scoped vendor captures retain complete original path objects,
source metadata and transitive local reference closure. Their scope annotations
identify the exact pre-pruning capture basis. They reproduce concrete contracts,
not the current vendor's whole inventory:

| Input and original source | Regression responsibility |
| --- | --- |
| Exa — `https://api.exa.ai/openapi.json` | Search/Contents alternatives, nested bodies and streaming guidance; real authenticated answer/batch submission/result witnesses. |
| OneShot — `https://win.oneshotagent.com/openapi.json` | Synchronous search and all eleven reviewed public GETs; actual authenticated research submission/result chain. |
| StableEnrich — `https://stableenrich.dev/openapi.json` | Six actual lifecycle submission/result contracts; Maps query/nested JSON, streaming guidance, Solar image/link metadata and reviewed Exa search. |
| Agent402 — `https://agent402.tools/openapi.json` | PNG/JPEG guidance, all reviewed JSON/base64 PDF/audio/video/skill/LinkedIn/favicon positives, image mappings, public feedback child, planning/onramp links, exact raw-speech/multipart and owned/execution negatives, reviewed DNS query. |
| BlockRun — `https://blockrun.ai/api/openapi` | Fixed BTC/XAU/EUR/BTCUSDT targets, hosted synchronous audio, phone lookup versus ownership, real hybrid/async media, sandbox and trade authority boundaries, invalid zero-token target. |
| Claw402 — `https://claw402.ai/openapi.json` | Four sparse/open RootData bodies preserve prose-guided arguments; price-ranking query/tag selection. |
| IPIntel — `https://tools.ipintel.ai/openapi.json` | DNS enum arrays, nullable satellite dates, arbitrary validator JSON, HTTP method enum and unverified JWT interpretation. |
| RelayShield — `https://api.relayshield.net/openapi.json` | Three original keyless checks, scan immediate-result versus missing poll contract, account/metered auth negatives and the privacy view's exact selected security requests. |
| Vaaya — `https://vaaya.ai/openapi.json` | Real mixed CRW extraction/result, MPP/upload/owned/project/message/forwarding negatives; Firecrawl JSON wrappers, hosted chart/image positives, open bodies, date/enum constraints and complete Tako/RecallRadar research chains with conflicting price prose. |

Refresh from the original vendor document and select complete path objects and
reference closure. Keep required fields, descriptions and original operation
identities intact; no expected schema is generated from case arguments. Compare
retained generated method/path/name and complete tool contracts before and after
subsetting, preserving any collision witnesses. Tests deliberately include
synthetic future routes separately from captured vendor facts. JSON/base64 media
remain ordinary JSON text unless an existing explicit image mapping applies;
hosted URLs are not fetched automatically. No fixture establishes live availability,
settlement, current prices or Tor qualification.

The AgentUtility runtime catalog preserves all original request contracts, unioned
upstream/registry/cluster tags and alias/payment provenance. Tiny canonical/alias
snapshot tests verify the join independently of recommendations. Real provider
regressions exercise JSON modes, exact full-oneOf conversion losses, remote watch
ownership and existing satellite extraction. No full output/settings golden or
research tagging is required.

RelayShield's authored phone companion remains a production request definition,
with separate provenance and x402 v1 limitations in `providers/CAVEATS.md`.
Its schema is not reconstructed from the vendor capture and is not pruned.

The NewsGurus raw x402 catalog fixture preserves the vendor's 49 resources. The
locally authored `providers/newsgurus/openapi.json` projects its Bazaar path/query
schemas and required lists into ordinary GET parameters and publishes static
`x-payment-info` prices. A focused regression compares every parameter against
the raw fixture. The status contract is exposed with its original URL and price; real Bazaar path
templates remain intact rather than being replaced by concrete example tickers.

Vaaya's unconsumed whole-directory discovery capture is removed; no regression
claims complete action-directory equality. The OpenAPI witnesses independently
preserve wrapper, schema and workflow contracts. RecallRadar's property-free
request body stays open while its documented `q` is sent in JSON; Tako and
RecallRadar structured estimates remain distinct from lower description prices.
The privacy example's wallet and cap remain unchanged.

The NetIntel fixture is a faithful eight-path primary-schema subset for three
POST-preferred pairs (URL secrets and JSON arrays), compatible chat/embedding
aliases, and the canonical image mapping. It retains complete selected request
and response declarations and local dependencies; no live endpoint is called.
RootData's four captured Claw402 open object bodies verify prose-guided JSON
passthrough without invented required fields. HireScrape's policy test is
explicitly synthetic because no complete local vendor capture is available.

## Authored projections and shared consumers

| Input | Actual consumers and fidelity contract |
| --- | --- |
| SocialFetch | Twitter profile for both public demos, live presets and request validation; YouTube and versioned LinkedIn platform/name selection; captured Instagram hosted-media tariff and Web search/ask credit conversion; public monitor source versus real parent/instance/event ownership; webhook endpoint/redelivery and Auth witnesses. All complete original objects and references remain. The newer Instagram include-tariff rules also have independent synthetic regressions; this older capture does not invent newer routes. |
| x402-list | Three independent directory read schemas are compared to the embedded implementation; public score/assessment/suggestion and actual submit/ownership witnesses test reusable-provider boundaries. Categories remains for the tracked help-outage example's selected source. |
| Concordance | Nine complete original path objects: six paid requests compare against authored inlining, and three public GETs compare after removing only authored free-price hints. Native lazy help faithfully uses the exact `/llms.txt` URL without a duplicate source fixture operation. |
| Locus | All 114 complete free REST objects verify bidirectional gateway-name coverage, both complete public manifests verify free enrichment, four job/result objects retain real lifecycle evidence, and place-report compares its entire original request schema (minus only `$schema`) and body-required flag to the authored projection. Raw media examples remain intact in the source; the authored projection omits them. Runtime wrapped gateway inputs and paid flat inputs remain distinct. |
| NewsGurus | Keep all 49 raw resources: unique whole-projection fidelity compares real templates, schemas, required lists, prices, prose and response examples against the production projection. This is a justified full capture, not a provider-wide snapshot rule. |

Refresh these inputs together with their actual consumers: `tests/socialfetch.rs`,
`tests/credit_pricing.rs`, `tests/public_demo_profiles.rs`, the provider and
embedded-directory tests, and the offline live-runner preset tests. Both public
SocialFetch demos deliberately use the scoped capture and select only the real
Twitter profile contract; it is not a complete offline SocialFetch inventory.
The generic `snapshot_spec` synthetic regression retains response-only removal,
Unicode, reference preservation and deterministic byte coverage without a vendor
byte-stability golden. Unconsumed Apollo/Straits raw captures are not retained;
Straits' authored production catalog is preserved in full. All runtime projections,
Kronos/RegimeShift runtime inputs and payment evidence remain intact.
