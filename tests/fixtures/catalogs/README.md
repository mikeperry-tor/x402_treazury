# Provider contract fixtures

Provider-specific inputs must reproduce a named quirk, failure or authored
transformation; shared synthetic tests own generic parsing and selection. The
current blanket output/settings snapshots are scheduled for removal under the
[provider-curation plan](../../../docs/plans/provider_curation.md). Do not add a
new golden or freeze a full settings map merely because a provider was added.

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
and qualification before deletion. `cases.json` currently feeds the blanket
catalog test, the offline qualification preset test and the ignored startup
benchmark; migrate all three before removing it. Captured Apollo, Concordance,
Locus and Straits test inputs have no code consumer. Concordance, Locus and Straits have
distinct production local catalogs that remain necessary.

Use `scripts/zcash.sh test --test provider_catalogs --test config` from the repository
root. Tests use no Python, credentials or live vendors. Review input/schema/routing
changes together; never regenerate snapshots merely to obtain a passing result.
Preserve full inputs only where a concrete comparison needs the entire inventory.
The following capture notes describe existing inputs and current assertions;
selection/count claims are being normalized with their owning implementation.

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
32-operation allowlist, omitted async workflows, tag selection and request
routing. The additional help tool reads the vendor's unified `llms.txt` lazily.

The Agent402 fixture was captured from `https://agent402.tools/openapi.json` on
2026-10-03 using `snapshot_spec`. It retains all 607 operations and 24 tags while
omitting response documentation. The default golden contract contains 48 web
operations plus help. Targeted tests also exercise all 480 eligible operations,
data/crypto and LLM selections, known exclusions, prospective diagnostics, query
routing, nested gateway JSON and streaming guidance.

The AgentUtility catalog lives at `providers/agentutility/openapi.json`, captured on
2026-10-03 from the API-host OpenAPI plus documentation-host registry. The Rust
`snapshot_agentutility` utility keeps all 817 request contracts, enriches missing
tags, marks 282 aliases and adds a curated research tag. The provider allows 525
operations; its golden covers 15 research tools plus help. Tests also cover all
eligible tools, tag subsets, required-only schema alternatives and satellite image
metadata. A captured unpaid search challenge pins static signing compatibility and
managed omission of its bazaar/builder-code extensions; no live funds were used.

Seven additional lazy help URLs were verified in the provider audit; six affect
these golden catalogs (botsmith is covered by settings and the help-contract test).
Those six golden changes add only help tools, leaving all API contracts unchanged.
`provider_payment_challenges.json` captures ten unsigned live challenges from
2026-10-03 and 2026-10-04: Agent402 pins preservation of offer `outputSchema`;
Exa Search/Contents and all four Google Trends routes pin exact preservation of
reviewed informational fields through signing. Locus, Otto and SocialFetch also
pin signing with top-level extensions omitted. Seller
signatures and auth nonces are public fixture evidence, not buyer credentials;
no live settlement or authentication success is implied.

The RelayShield request fixture captures the vendor's public
`https://api.relayshield.net/openapi.json` request catalog, retaining all operations
and removing response documentation only. Its dedicated offline regression tests
the exact main allowlist, exclusions, prices, representative routing, and collision-free
composition with the authored phone companion. The companion's provenance and x402
v1 limitation are documented in `providers/CAVEATS.md`; neither fixture qualifies
funded settlement.

The IPIntel fixture captures `https://tools.ipintel.ai/openapi.json` using
`snapshot_spec`, removing response documentation only. Its 13 POST operations
plus lazy help have a full golden contract. Focused tests preserve all prices,
JSON body routing, DNS enums, nullable satellite dates and unrestricted validator
JSON values, and exclude prospective methods/routes. No paid call was made.


The NewsGurus raw x402 catalog fixture preserves the vendor's 49 resources. The
locally authored `providers/newsgurus/openapi.json` projects its Bazaar path/query
schemas and required lists into ordinary GET parameters and publishes static
`x-payment-info` prices. A focused regression compares every parameter against
the raw fixture. Its golden selects 48 task-facing reads plus help, omitting
agent-status diagnostics and retaining real path templates instead of the
vendor OpenAPI's concrete example ticker/token paths.

The Vaaya fixture preserves all 252 paths from the live vendor OpenAPI,
omitting response documentation via `snapshot_spec`. The companion discovery fixture
contains 249 distinct x402 actions; the default golden pins 46 synchronous
research calls plus help. Tests check discovery membership, x402 pricing,
exclusions, JSON body filters and regex escaping. These fixtures do not qualify
current availability, paid settlement or Tor access. See provider caveats for
schema/prose gaps and data limitations.

Tako search/answer/contents and RecallRadar search/get extend the Vaaya default
with synchronous research chains. The routing regression preserves RecallRadar's
open, property-free input schema while sending its documented `q` in the JSON
body, and verifies all five routes and structured estimates despite lower prices
in vendor prose. The company deployment selects these and focused company/public
record tools, inheriting its existing wallet and cap.
