# Provider contract fixtures

Provider-specific fixtures must reproduce a named quirk, failure, or authored
transformation; shared synthetic tests own generic parsing and selection. Never
freeze a full settings map or golden inventory solely because a provider was
added. All bundled TOML definitions are validated offline without Python,
credentials, or live vendor access:

```sh
make test CARGO_ARGS='--test provider_catalogs --test config'
make test CARGO_ARGS='--example provider_updates'
```

`provider_updates` tests synthetic capture, comparison, and candidate
mechanics (see [maintenance sweeps](../../../docs/provider-updates.md)); sweep
outputs stay ignored under `target/provider-updates/`.

## Retaining and refreshing fixtures

Retain fixtures by responsibility:

- **Request and routing quirks:** Exa's `ids` and `urls` alternatives, Locus
  address alternatives and gateway rewrite, IPIntel nullability and arbitrary
  JSON, and RecallRadar's open body and structured vs. prose price
  disagreement.
- **Mixed payment and lifecycle boundaries:** Exa, OneShot, StableEnrich,
  Agent402, BlockRun, RelayShield, and Vaaya.
- **Authored transformations:** NewsGurus Bazaar projection, AgentUtility
  registry tag and alias enrichment, SocialFetch version naming and credit
  metadata, Strale complex-input `POST` repairs (`strale_post_inputs.json`),
  and 2s CSV query parameter repairs (`twos_csv_parameters.json`).
- **Captured unsigned payment evidence:** AgentUtility challenges and
  `provider_payment_challenges.json`, consumed by signing tests.
- **Deployment routing witnesses:** `tests/fixtures/otto_routing_openapi.json`
  reproduces the removed `generate_meme` and `video_gen` selector failure in
  `research_desk.toml` (`tests/deployment_provider_updates.rs`).
- **Offline qualification request schemas:** Retain complete path objects and
  transitive local `$ref` closure so `tests/live/runner/presets.rs` validates
  reviewed requests against independently captured schemas (arguments never
  supply expected schemas).

Production local catalogs under `providers/` and the Kronos and RegimeShift
inputs in `tests/live/deployments/live-tor-providers.toml` are runtime inputs,
not prunable test fixtures. When refreshing a scoped fixture from the vendor's
OpenAPI URL, preserve complete path objects, descriptions, tags, payment
metadata, required lists, and naming-collision witnesses; never trim schema
properties or regenerate snapshots solely to pass a test. Label reduced
captures with `x-treazury-fixture-scope`.

## Reviewed request and method contracts

| Input | Retained contract and consumer |
| --- | --- |
| StableTravel paths | Original literal placeholder slots, request and response fields, and prices; verifies flight, airport-pair, award-trip, aircraft, and disruption routing repairs. |
| AX1 request branches | Original external and Console `anyOf` witness; verifies the runtime external projection and required token input. |
| CoinMarketCap projection | Original paid DEX query parameters and public holder `POST` body; verifies authored path and security projection. |
| Nansen keyless contracts | Request and auth excerpts for keyless analytics vs. API-key operations, feedback key constraints, and SSE agents (responses omitted). |
| BlockRun gateways | Request excerpts for synchronous Messages and Responses and the unrestricted RPC boundary (responses omitted). |
| AgData contracts | x402-capable aliases, root routing, screenshot and PDF JSON, and managed idempotency header. |
| Degenscan contracts | Keyless Carry price overrides, public forecast submit and poll, and credential and transport exclusions. |
| Market Intelligence contracts | `POST` opportunity constraints and prepaid, webhook, and feedback exclusions preserving public reads. |
| AgentFund | `/x402/treasury_yield_curve`: reviewed preset body schema. |
| Brazilayer | `/v1/cnpj/situacao/{cnpj}`: reviewed path argument and parameter and response reference closure. |
| Deepline | `/api/company/enrich`: reviewed enrichment request, nested schema, and guidance. |
| PDL | `/api/pdl/company/enrich`: reviewed company enrichment schema and field descriptions. |
| LoneStar | `/rates/rates`: reviewed rate request and original provider metadata. |
| GenuineGood | All five `GET` and `POST` grant pairs plus bearer-pass exclusion (`tests/catalog_selection.rs`). |
| Otto | Four `GET` and `POST` pairs, `/crypto-news`, `/swap`, and `/full-auto`: method serialization, preset, and execution exclusions. |
| NetIntel | Eight-path subset: URL-secret and array `POST` pairs, chat and embedding aliases, and PNG data-URI image mapping. |

## Compatibility witnesses

Scoped captures retaining complete path objects, top-level metadata, and
transitive local `$ref` closure:

| Input and original source | Regression responsibility |
| --- | --- |
| Exa (`https://api.exa.ai/openapi.json`) | Search and Contents alternatives, nested bodies, streaming guidance, and authenticated answer and batch witnesses. |
| OneShot (`https://win.oneshotagent.com/openapi.json`) | Synchronous search, all 11 public `GET` routes, and authenticated async research chain. |
| StableEnrich (`https://stableenrich.dev/openapi.json`) | Six lifecycle submission and result contracts, Maps query and nested JSON, streaming guidance, Solar image and link metadata, and Exa search. |
| Agent402 (`https://agent402.tools/openapi.json`) | PNG and JPEG guidance, JSON and base64 PDF, audio, video, skill, LinkedIn, and favicon positives, image mappings, public feedback child, onramp links, raw-speech, multipart, owned, and execution negatives, and DNS query. |
| BlockRun (`https://blockrun.ai/api/openapi`) | Fixed BTC, XAU, EUR, and BTCUSDT targets, hosted synchronous audio, phone lookup vs. ownership, hybrid and async media, sandbox and trade boundaries, and zero-token target. |
| Claw402 (`https://claw402.ai/openapi.json`) | Four open RootData bodies verifying prose-guided JSON passthrough; price-ranking query and tag selection. |
| IPIntel (`https://tools.ipintel.ai/openapi.json`) | DNS enum arrays, nullable satellite dates, arbitrary validator JSON, HTTP method enum, and unverified JWT interpretation. |
| RelayShield (`https://api.relayshield.net/openapi.json`) | Three keyless checks, scan immediate-result vs. missing poll contract, account and metered auth negatives, and selected security requests. |
| Vaaya (`https://vaaya.ai/openapi.json`) | Mixed CRW extraction and result, MPP, upload, owned, project, message, and forwarding negatives, Firecrawl JSON wrappers, hosted charts and images, open bodies, date and enum constraints, and Tako and RecallRadar chains with conflicting price prose. |
| VoIPStore (`https://api.voipstore.xyz/openapi.json`) | Quote-header purchase routing, managed idempotency, prepaid top-up exclusion, and catalog, quote, and status routing (health omitted). |

Additional notes:

- **AgentUtility:** Preserves original request contracts, unioned upstream,
  registry, and cluster tags, and alias and payment provenance; small snapshot
  tests verify the join independently of recommendations, and provider tests
  cover JSON modes, `oneOf` losses, watch-page exclusion, and satellite image
  extraction.
- **RelayShield phone companion:** Authored production definition with separate
  provenance and x402 v1 caveats in `providers/CAVEATS.md` (not pruned).
- **HireScrape:** Uses synthetic policy tests because no complete local vendor
  capture is committed.

## Authored projections and shared consumers

| Input | Consumers and fidelity contract |
| --- | --- |
| Straits history (`catalogs/straits-history.json`) | Three premium path objects verifying 7-day cursor pages, response contracts, older export resolution, and weekly risk-input lag. |
| SocialFetch | Twitter profile for public demos, live presets, and request validation; YouTube and versioned LinkedIn selection; Instagram hosted-media tariff and Web search and ask credit conversion; public monitor source vs. ownership, webhooks, and auth witnesses. |
| x402-list | Three directory read schemas compared against the embedded directory implementation; public score, assessment, suggestion, submit, and ownership witnesses; categories route for the help-outage example. |
| Concordance | Nine path objects: six paid requests compared against authored inlining, three public `GET` routes compared after removing authored free-price hints, and lazy `/llms.txt` help. |
| Locus | All 137 free REST objects verifying 135 gateway reads and 2 participation-write exclusions, both public manifests, 4 job and result objects, and paid reports compared against original schemas (minus `$schema` and media examples). |
| NewsGurus | All 49 raw Bazaar resources compared parameter-by-parameter against `providers/newsgurus/openapi.json`, preserving path templates, required lists, prices, prose, and status routing. |

Refresh these inputs alongside their consumers (`tests/socialfetch.rs`,
`tests/credit_pricing.rs`, `tests/public_demo_profiles.rs`,
`tests/provider_contract_repairs.rs`, `tests/deployment_provider_updates.rs`,
embedded-directory tests, and offline live-runner preset tests). Generic
`snapshot_spec` regressions verify response removal, Unicode, `$ref`
preservation, and deterministic bytes on synthetic inputs without a vendor
byte-stability golden.
