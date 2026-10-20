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
workloads and no provider index. Captured Apollo and Straits test inputs have no
code consumer. Concordance source
objects and Locus free-gateway/discovery contracts now have focused fidelity
consumers. Their distinct production local catalogs remain necessary.

Use `scripts/zcash.sh test --test provider_catalogs --test config` from the repository
root. Tests use no Python, credentials or live vendors. Review input/schema/routing
changes together; never regenerate snapshots merely to obtain a passing result.
Preserve full inputs only where a concrete comparison needs the entire inventory.
The following capture notes describe existing inputs and current assertions;
selection/count claims are being normalized with their owning implementation.

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

The Exa request fixture was captured from `https://api.exa.ai/openapi.json` on
2026-10-03 with the Rust `snapshot_spec` utility. It retains all 64 operations so
the exact two-operation allowlist is tested against the real excluded inventory.
Only response documentation is removed. Exa's root `oneOf` for `ids` versus `urls`
is preserved; six Locus tools likewise retain their existing `anyOf` address
alternatives after flattening. Neither change adds or rewrites HTTP parameters.

The OneShot fixture was captured from `https://win.oneshotagent.com/openapi.json`
on 2026-10-03 using `snapshot_spec`. It retains all 147 operations (omitting
response documentation) to pin exclusions as well as synchronous search. The
selected request schema is unchanged; no OneShot-specific schema repair is needed.

The StableEnrich fixture was captured from `https://stableenrich.dev/openapi.json`
on 2026-10-03 using `snapshot_spec`. All 38 operations are retained to verify the
specific lifecycle exclusions, tag selection and request
routing. The additional help tool reads the vendor's unified `llms.txt` lazily.

The Agent402 fixture was captured from `https://agent402.tools/openapi.json` on
2026-10-03 using `snapshot_spec`. It retains all 607 operations and 24 tags while
omitting response documentation. Focused tests exercise supported JSON/base64 media, exact feedback parent exclusions
with public child preservation, query routing, nested gateway bodies and existing
image mappings. Full-inventory snapshots are not compatibility contracts.

The AgentUtility runtime catalog preserves all original request contracts, unioned
upstream/registry/cluster tags and alias/payment provenance. Tiny canonical/alias
snapshot tests verify the join independently of recommendations. Real provider
regressions exercise JSON modes, exact full-oneOf conversion losses, remote watch
ownership and existing satellite extraction. No full output/settings golden or
research tagging is required.

The RelayShield request fixture captures the vendor's public
`https://api.relayshield.net/openapi.json` request catalog, retaining all operations
and removing response documentation only. Its dedicated offline regression tests
the exact main allowlist, exclusions, prices, representative routing, and collision-free
composition with the authored phone companion. The companion's provenance and x402
v1 limitation are documented in `providers/CAVEATS.md`; neither fixture qualifies
funded settlement.

The IPIntel fixture captures `https://tools.ipintel.ai/openapi.json` using
`snapshot_spec`, removing response documentation only. Focused tests preserve DNS enum arrays and JSON routing, nullable satellite dates,
unrestricted validator JSON, the HTTP method enum and JWT interpretation. Shared
selection tests establish new-route visibility; no inventory count or all-prices
snapshot is required. No paid call was made.


The NewsGurus raw x402 catalog fixture preserves the vendor's 49 resources. The
locally authored `providers/newsgurus/openapi.json` projects its Bazaar path/query
schemas and required lists into ordinary GET parameters and publishes static
`x-payment-info` prices. A focused regression compares every parameter against
the raw fixture. The status contract is exposed with its original URL and price; real Bazaar path
templates remain intact rather than being replaced by concrete example tickers.

The Vaaya fixture preserves all 252 paths from the live vendor OpenAPI,
omitting response documentation via `snapshot_spec`. The companion discovery fixture
contains 249 distinct x402 actions. Focused tests check support boundaries,
restored public research and JSON contracts, body filters and regex escaping. These fixtures do not qualify
current availability, paid settlement or Tor access. See provider caveats for
schema/prose gaps and data limitations.

Tako search/answer/contents and RecallRadar search/get extend the Vaaya default
with synchronous research chains. The routing regression preserves RecallRadar's
open, property-free input schema while sending its documented `q` in the JSON
body, and verifies all five routes and structured estimates despite lower prices
in vendor prose. The company deployment selects these and focused company/public
record tools, inheriting its existing wallet and cap.

The NetIntel fixture is a faithful eight-path primary-schema subset for three
POST-preferred pairs (URL secrets and JSON arrays), compatible chat/embedding
aliases, and the canonical image mapping. It retains complete selected request
and response declarations and local dependencies; no live endpoint is called.
RootData's four captured Claw402 open object bodies verify prose-guided JSON
passthrough without invented required fields. HireScrape's policy test is
explicitly synthetic because no complete local vendor capture is available.

Concordance fidelity compares complete captured public GET objects after removing
only authored zero-price enrichment, and checks paid request schemas after local
reference expansion. Locus compares complete discovery objects the same way and
checks equality in both directions between all original free REST names and the
native gateway enum; retain those complete original contracts and local references,
not just handpicked names. Actual excluded job/result inputs establish lifecycle
boundaries. Runtime projections remain full production inputs, independently of
future fixture minimization.
