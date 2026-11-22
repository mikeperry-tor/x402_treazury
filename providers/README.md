# Provider definitions

Reusable provider files in `providers/` describe public APIs independently of
wallet groups and listeners. Use the shared
[maintenance sweep](../docs/provider-updates.md) to capture and review upstream
changes, and record provider-specific compatibility rules and authored repair
rationale in [CAVEATS.md](CAVEATS.md). Never put provider-specific dispatch,
executable update recipes, or a transformation language into shared tooling or
maintenance metadata.

A deployment source can declare settings inline or extend a reusable file:

```toml
[sources.exa]
extends = "../../providers/exa.toml"

[servers.research]
listen = "127.0.0.1:8000"
bearer_token_env = "X402_TREAZURY_MCP_TOKEN"
wallet = "research"
sources = ["exa"]
```

This excerpt comes from the
[managed deployment](../examples/deployments/managed.toml), which adds wallet,
treasury, funding, and source-management settings.

## Inheritance and filtering

- **Single-level inheritance:** `extends` resolves relative to the deployment
  file; a local `spec` resolves relative to the file declaring it. Provider
  files cannot extend other files, and unknown keys fail parsing.
- **Full field replacement:** Local fields replace inherited values completely
  (arrays do not concatenate, `[]` clears, and local `overrides` or
  `response_mappings` replace the entire inherited table).
- **Distribution and filtering:** Deployments can use listener tag and tool
  filters or ordered [`send_tools` rules](../docs/configuration.md#source-tool-distribution)
  to route tool families across endpoints. Reusable providers and dynamic
  imports cannot declare `send_tools` or `wallet`.
- **Known incompatibilities:** Exclude known unsupported routes with
  `exclude_operations`, `exclude`, or `exclude_tags`, and document the concrete
  reason in [CAVEATS.md](CAVEATS.md). Unexpected omissions warn even when the
  remaining catalog is valid; never suppress warnings globally or mask
  compatibility omissions with tool-name filters.
- **Inspection:** Pass `--provider providers/pdl.toml` for standalone serving
  or inspection, or `--config FILE` for a deployment. Add `config show` to
  inspect resolved composition offline without fetching specs or reading keys.

## Provider catalog

| Provider | Definition | Catalog summary |
| --- | --- | --- |
| 2s | `2s.toml` | Authored request snapshot with CSV repairs; owned infrastructure, batch, GIF, and unresolved OSHA filtering excluded |
| AgData | `agdata.toml` | Web and social data; both x402-capable aliases; screenshot and PDF JSON |
| Agent402 | `agent402.toml` | Full supported catalog; advisory web-research recommendations and lowercase tags |
| AgentFund | `agentfund.toml` | Vendor OpenAPI |
| AgentUtility | `agentutility/provider.toml` | Full supported aliases and catalog; advisory research recommendations; enriched provenance |
| AgOSINT | `agosint.toml` | x402 registry gateway; root server URL repair |
| AGProxy | `agproxy.toml` | Pinned `agproxy-openapi.json`; residential fetch and proxy packs |
| APEX Faucet | `apexfaucet.toml` | Authored full catalog with query-input repairs; persistent identity and owned purchases excluded |
| Arkham | `arkham.toml` | Vendor OpenAPI |
| AX1 Verdict | `ax1/provider.toml` | External request projection; token Verdict submission and free public receipt |
| Banking Compliance | `banking-compliance/provider.toml` | Authored POST checks, rule lookup, and drafts; campaign omitted pending request contract |
| Biomedical Grants (x402 Atlas) | `biomedical-grants.toml` | Full NIH RePORTER search, project, publication, and funding-aggregate catalog |
| Blockchain Money Map | `blockchainmoneymap.toml` | Full vendor wallet, transaction, evidence-analysis, and agent-commerce catalog |
| BlockRun | `blockrun.toml` | Synchronous catalog with fixed-symbol reads; async jobs and ownership excluded |
| botsmith | `botsmith.toml` | Vendor OpenAPI |
| Brave Search Proxy | `brave-search.toml` | Vendor proxy catalog; fixed $0.005 searches; x402 v2 only |
| Brazilayer | `brazilayer.toml` | Vendor OpenAPI |
| Claw402 | `claw402.toml` | Full catalog, advisory market reads, sparse RootData JSON bodies |
| CoinMarketCap | `coinmarketcap/provider.toml` | Authored union of public keyless routes and four x402 counterparts |
| Concordance | `concordance/provider.toml` | Local `concordance/openapi.json` |
| Curl HTTP Request | `curl/provider.toml` | Local schema; `POST /curl` returns status, headers, and body text; advertised $0.01 |
| Daedalus | `daedalus/provider.toml` | Identity-header request repair; queued fulfillment and persistent ownership excluded |
| Datakoot | `datakoot.toml` | Public-data gateway; apex-domain help |
| Deepline | `deepline.toml` | Vendor OpenAPI |
| Degenscan Intel | `degenscan.toml` | Event intelligence, keyless Carry reads, and public forecast submit and poll |
| Duo Leads | `duoleads.toml` | Full synchronous keyword, Google Trends, SEO, and search-data catalog |
| Exa | `exa.toml` | Vendor OpenAPI; Search, Contents, and lazy help |
| FindMyDirectDoctor | `findmydirectdoctor.toml` | Full read-only doctor, clinic, hospital cash-price, and healthcare-access catalog |
| ForgeMesh Utility Grid | `forgemesh.toml` | Full fixed-price POST catalog; free route menu; duplicate aliases retained |
| FreeQ | `freeq.toml` | Remote REST catalog; pending completion, credential ownership, webhook lifecycles, and gift-card purchases excluded |
| Genuine Good Grants | `genuinegood.toml` | POST grant research; search and detail recommendations; bearer pass excluded |
| Glassnode | `glassnode/provider.toml` | Local `glassnode/openapi.json` |
| glim.sh | `glim.toml` | Vendor OpenAPI; X, Reddit, GitHub, Telegram, YouTube, Amazon, and web search gateway |
| Global Legal Entities (x402 Atlas) | `legal-entities.toml` | Full GLEIF search, registration, and accounting-parent catalog |
| Google Patents (x402 Atlas) | `google-patents.toml` | Full two-operation search and details catalog; claims and family data |
| Google Trends | `google-trends.toml` | Vendor OpenAPI |
| HALOWERK | `halowerk.toml` | Full JSON tool catalog; optional screenshot base64 retained as JSON |
| HireScrape | `hirescrape.toml` | Synchronous paid scraper namespace; SIWX lifecycle omitted |
| Horizon Pulse | `horizonpulse.toml` | Pinned `horizonpulse-openapi.json`; SEC, macro, and security reads |
| IPIntel.ai | `ipintel.toml` | Domain, DNS, IP, satellite, and developer tools; core lookup recommendations |
| Keenable | `keenable.toml` | Exact x402 search and fetch allowlist; Markdown fetch with indexed and live modes; SELECT, API-key, and keyless routes omitted |
| Kronos | `kronos.toml` | Market data plus public status and methodology |
| Locus | `locus/provider.toml` | Local `locus/openapi.json` |
| LoneStarOracle | `lonestar.toml` | Unified catalog with direct service URLs, including public previews and feeds |
| Market Intelligence API | `marketintelligence.toml` | Market research and analytics; POST opportunity scans; prepaid and webhook workflows excluded |
| Maxwell International | `maxwell.toml` | Public data and travel; manual domain transfer excluded |
| Medical Codes (x402 Toolkit) | `medical-codes.toml` | Full ICD-10, HCPCS, LOINC, RxNorm, and NPI-format reference catalog |
| medRxiv Preprint Details | `medrxiv-details.toml` | Date-window medRxiv preprint listings with cursor pagination |
| Nansen | `nansen.toml` | Keyless paid analytics; API-key account, trading, and SSE agents excluded |
| NetIntel | `netintel.toml` | Canonical POST and body contracts with image data-URI mapping |
| News Gurus | `newsgurus/provider.toml` | Vendor x402 catalog projection, including agent-status; lazy help |
| Onchaincheck | `onchaincheck.toml` | Public wallet, token, company, web, and AI catalog; secret-owned webhook subscription excluded |
| OneShot | `oneshot.toml` | Synchronous search and public discovery; authenticated jobs excluded |
| OpenVerbs | `openverbs.toml` | Combined apex catalog: 418 POST operations across 93 verb families; per-service `/health` omitted |
| OpenWeb Ninja | `openwebninja.toml` | Vendor OpenAPI; duplicate alias trees excluded for shorter names; marketplaces, reviews, Yelp, events, Google Play, jobs, and real estate |
| Otto | `otto.toml` | Vendor OpenAPI |
| People Data Labs | `pdl.toml` | Vendor OpenAPI |
| readFetch | `readfetch.toml` | Pinned `readfetch-openapi.json` with authored reference closure; raw HTTP relay, extraction, scrape, crawl, PDF, map, diff, screenshot, and batch |
| RegimeShift | `regimeshift.toml` | Vendor OpenAPI |
| RelayShield | `relayshield.toml` | Vendor OpenAPI; optional authored `relayshield/phone-reputation.toml` companion |
| RelayShield phone | `relayshield/phone-reputation.toml` | Authored vendor-omitted request contract |
| SiteCheck Company Records | `companyrecords.toml` | Full public company, government, B2B, and healthcare-directory catalog |
| SiteCheck Health & Travel | `healthtravel.toml` | Full public health, travel, transport, and vehicle catalog |
| SiteCheck Shop & Social | `shopsocial.toml` | Full public social and shopping catalog |
| SiteCheck Web Intel | `webintel.toml` | Full news, web, scholarly, and security catalog |
| SmartPerps (AgentGate) | `smartperps.toml` | Read-only Hyperliquid cohort analytics and public wallet data; POST twins |
| SocialFetch | `socialfetch.toml` | Vendor tagged OpenAPI |
| StableEnrich | `stableenrich.toml` | Synchronous JSON and image catalog; lifecycle paths excluded |
| StableNinja | `stableninja.toml` | Vendor OpenAPI backup; marketplace, Yelp, Waze, and Google Play reads with `/api` naming anchor |
| StableTravel | `stabletravel/provider.toml` | Full flight-data catalog with authored path-parameter repairs; cash fares, cached awards, and operational reads |
| Strale | `strale.toml` | Authored complex-input POST repairs; wallet-paid v2 capabilities and public discovery |
| Straits | `straits/provider.toml` | Authored free and premium JSON, CSV, RSS, and HTML contracts |
| Vaaya | `vaaya.toml` | Supported x402 JSON and image catalog; MPP and lifecycle paths excluded |
| Vextorium | `vextorium.toml` | Full POST catalog; `/v1` naming anchor |
| VoIPStore | `voipstore.toml` | Virtual-number catalog, quotes, purchases, and SMS status; prepaid top-ups excluded |
| x402-list | `x402-list.toml` | Public directory reads, assessments, suggestions, and listing submissions |
| x402stock | `x402stock.toml` | Full supported market and economic catalog; compact research recommendations |

Notes on special providers and pinned assets:

- **Pinned relay schemas:** Curl, AGProxy, Horizon Pulse, and readFetch pin
  their OpenAPI files locally so relay startup validates operations without
  remote fetches. See [pinned relay contracts](CAVEATS.md#pinned-relay-contracts)
  and [relay fallback](../docs/configuration.md#relay-fallback).
- **Authored local catalogs:** Locally authored catalogs (such as Locus,
  Straits, Concordance, Glassnode, and AgentUtility) are explicitly labeled and
  preserve documented request contracts, provenance, and `x-payment-info`
  hints.
- **x402-list:** Source management uses the embedded three-operation
  `x402-list/directory.openapi.json` contract automatically. Use
  `x402-list.toml` only when exposing the broader directory API (reads can
  become paid after a shared-IP free quota; owner-token workflows stay
  excluded).
- **Unavailable or rejected providers:** Apollo via Locus (uncertain
  request-ID retry and recovery; see
  [Apollo](CAVEATS.md#reviewed-but-unavailable-apollo)) and x402node (payment
  challenge failures; see
  [x402node](CAVEATS.md#reviewed-and-rejected-x402node)) are not shipped as
  enabled definitions.
- **Offline tests:** `tests/provider_catalogs.rs` and `tests/config.rs`
  validate all provider definitions offline without live vendor access or
  Python.

## Bundled provider metadata

`src/providers/assets.rs` embeds every shipped provider TOML and referenced
local OpenAPI asset directly into the executable, so local schemas work
independently of the current directory with no filesystem scan or writable
index.

- **IDs and descriptions:** Stable IDs match the TOML stem, a subdirectory
  `provider.toml` uses its directory name, and the RelayShield companion is
  `relayshield-phone-reputation`. Keep settings and the required concise root
  `description` in each TOML file rather than duplicating them in the manifest.
  Private local overrides are never bundled.
- **Relay format (`relay_format`):** Providers used in `[relay_fallback.relays]`
  declare `relay_format` (`"curl"`, `"agproxy"`, `"horizonpulse"`, or
  `"readfetch"`) once in their TOML; formats are never inferred from names or
  hosts. This metadata never enters MCP tool descriptions or schemas.
- **Dynamic discovery matching:**
  - `x402_treazury_providers_list_local` searches embedded metadata offline.
  - `x402_treazury_provider_details_remote` matches exact normalized HTTPS
    origin and `base_url` path to bundled candidates (definitions without an
    explicit `base_url` have no API-base lookup key).
  - `x402_treazury_providers_add_remote` reuses an existing generic
    registration for that exact canonical `spec_url` first; otherwise, a unique
    exact bundled `spec_url` (including query string) selects the bundled
    definition even when local management is off. Ambiguous matches return
    local choices when local management is enabled, or ask the operator to
    disambiguate.

## Provider policy

- **Expose supported functionality:** Restrict reusable catalogs only for
  concrete authentication, payment, request, output, lifecycle, or
  financial-authority incompatibilities (not for compactness, price, overlap,
  or editorial preference). Keep naming anchors (adding an empty `""` fallback when
  needed) and document exact operation allowlists when a mixed API has a
  specific payable island.
- **Recommendations and method twins:** Keep editorial subsets in
  `recommended_tools` (opt-in via `use_recommended_tools = true`; inspect with
  `catalog tools --all-tools`) rather than hiding supported routes. Same-path
  `GET` and `POST` twins default to `POST` before naming; inspect each method's
  request, response, and billing contract before opting out
  (`prefer_post = false`). See [Agent402 caveats](CAVEATS.md#agent402toml--agent402tools)
  for an example broad catalog with advisory recommendations.
- **Instructions and tool descriptions:** Attach `help_url` when a useful guide
  exists. Keep `instructions_text` brief and limited to cross-tool workflows or
  shared interpretation rules. Put per-tool requirements, sparse-schema notes,
  pagination quirks, and coverage limits in tool and parameter descriptions;
  omit payment and signing boilerplate, marketing, and maintainer provenance. Follow
  [tool documentation](../docs/agent-sources.md#tool-documentation).
- **Pricing separation:** Declare `[pricing]` (`source = "catalog"` preferred,
  or an explicit HTTPS URL) when the catalog uses a supported format, setting
  `unit = "call"` and `usd_as_usdc = true` only after checking the vendor's
  billing contract. In TOML description overrides, place current-call tariffs
  (including unconditional free claims) in `pricing_description` alongside a
  tariff-free `description` so observed offers replace only the price line.
  Leave dynamic, credit, and metered prices display-only. See
  [pricing metadata](../docs/configuration.md#pricing-metadata-and-runtime-hints).
- **Targeted regression fixtures:** Store provider-specific fixtures and tests
  only for a concrete quirk, bug, or authored transformation, using small
  faithful inputs and shared synthetic tests rather than universal settings
  snapshots. See [provider regressions](../docs/testing.md#provider-regressions)
  and [fixture maintenance](../tests/fixtures/catalogs/README.md).

## Response mappings

Providers and deployment sources can project successful JSON responses locally
by exact HTTP method and original OpenAPI path (independent of source alias or
prefix):

```toml
[response_mappings."GET /search"]
omit = ["/results/*/figures"]
```

- Paths use RFC 6901 escaping (`~0`, `~1`), with `*` traversing array elements
  only (never object keys). Only object fields can be removed, not array
  elements; unmatched paths are no-ops, and root paths or invalid escapes fail
  config load.
- Each path that removes fields appends an advisory with its removal count.
  Use `omit` for named verbose fields (never as an implicit size cap) while
  keeping IDs, pagination, dates, and completeness markers intact. Projection
  runs locally after the response arrives and does not affect `raw_call`. See
  [JSON response projection](../docs/configuration.md#json-response-projection).

### Inline image results

PNG, JPEG, and WebP HTTP bodies become MCP image content blocks automatically.
Unsupported binary MIME types, invalid UTF-8, invalid base64, and mismatched
file signatures return errors without retrying a paid call (there is no pixel
decoder or transcoder in `x402_treazury`).

Extract base64 or data-URI images from JSON responses with `response_mappings`:

```toml
[response_mappings."POST /images"]
images = [
  { pointer = "/data/0/b64_json", mime_pointer = "/data/0/media_type" },
]

[response_mappings."POST /crop"]
images = [{ pointer = "/dataUri", encoding = "data_uri" }]

[response_mappings."POST /generate"]
images = [{ pointer = "/image", mime_type = "image/png" }]

[image_limits]
max_image_bytes = 4194304
max_total_bytes = 8388608
max_images = 4
```

- **Defaults and limits:** The `[image_limits]` values above are the per-call
  decoded byte and count defaults, applied alongside `max_response_bytes`
  (16 MiB). `encoding` defaults to `"base64"`; `"data_uri"` requires `;base64`.
- **Pointers and MIME types:** Pointers are exact non-root RFC 6901 pointers
  (no wildcards) targeting required strings. Plain base64 requires exactly one
  of `mime_type` or `mime_pointer`; supported types are `image/png`,
  `image/jpeg`, and `image/webp`.
- **Execution order:** Image extraction runs before `omit` on the same parsed
  response; all configured extractions must succeed (no partial results). Each
  extracted field becomes a `{treazury_attachment, mime_type}` marker (1-based)
  in the JSON text followed by the MCP image blocks. Inline images are not
  persisted on disk; see the [artifact storage plan](../docs/plans/artifact_storage.md).
- **Inheritance and dynamic sources:** Mapping and limit tables replace
  inherited tables in full and validate at config load. Agent-added sources use
  the default image limits and cannot supply `response_mappings`.
