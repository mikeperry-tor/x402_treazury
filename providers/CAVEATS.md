# Provider behavior and caveats

Use these notes alongside [provider definitions](README.md). Prices and
contracts below reflect the committed configurations and captures; live x402
challenges always govern charges, and schema inspection or unsigned probes
never establish live settlement, current upstream uptime, or Tor delivery. See
[qualification status](../docs/testing.md#qualification-status).

## Compatibility and editorial selection

The [provider policy](README.md#provider-policy) separates compatibility
exclusions from editorial advice. Reusable definitions expose all supported
routes; recommendations require opt-in (`use_recommended_tools = true`). Path
`include` values can anchor generated names, so keep them with a `""` fallback
rather than hiding compatible routes.

## Cross-provider contracts

### Unsigned HTTP/2 cover resources

Cover requires a same-origin resource returning exact HTTP/2 `206` responses
for unsigned, identity-encoded single-range GETs (HTTP/2 `200` means ranges
were ignored). See [cover policy](../docs/cover-traffic.md).

Automatic cover tries same-origin `help_url` before the catalog URL. Working
primary resources verified for 1,024-byte `206` reads:

| Provider | Primary | Fallback |
| --- | --- | --- |
| PDL | `https://stablepeopledata.dev/` | None |
| StableEnrich | `https://stableenrich.dev/` | None |
| Otto (automatic) | `https://x402.ottoai.services/llms-full.txt` | Catalog (ignores ranges) |
| Vaaya (automatic) | `https://vaaya.ai/llms.txt` | Catalog (ignores ranges) |
| x402stock (automatic) | Help | Catalog (both honor ranges) |
| 2s | `https://2s.io/2sio-avatar.png` | None (help and catalog ignore ranges) |

Providers opting out of cover (`cover_traffic_enabled = false`) because their
checked static resources return `200`, redirect, are missing, are shorter than
1,024 bytes, or use HTTP/1.1: AgOSINT, AgentFund, AgentUtility, APEX Faucet,
Arkham, Atlas Google Patents, Banking Compliance, BotSmith, Claw402,
CompanyRecords, Curl, Duo Leads, Exa, ForgeMesh, Genuine Good, Global Legal
Entities, IPIntel, Kronos, Locus, LoneStarOracle, Maxwell, NetIntel,
Onchaincheck, RelayShield (and phone companion), SocialFetch, and Vextorium.
Deployments may re-enable cover by supplying a verified same-origin range
resource (for HTTP/2 providers).

### Same-path method contracts

Default `prefer_post = true` collapses interchangeable `GET` and `POST` twins
to `POST` before naming (use `prefer_post = false` to expose both methods):

- **GenuineGood (5 pairs) and Otto (3 pairs):** Canonical names are
  `genuinegood_search`, `genuinegood_detail`, `genuinegood_fit`,
  `genuinegood_brief`, `genuinegood_preflight`, `otto_llm_research`,
  `otto_tx_explainer`, and `otto_meta_intelligence`. `POST` carries JSON
  arrays cleanly, whereas `GET` documents comma-separated arrays unsupported by
  repeated-key query routing.
- **NetIntel (18 pairs):** Matching inputs, responses, and pricing metadata;
  `POST` keeps secrets out of URLs and arrays in JSON.

## Advertised tariff interpretation

- Reusable definitions prefer embedded `x-payment-info` (`source = "catalog"`),
  falling back to `unit` only when omitted by the operation. `usd_as_usdc = true`
  opts into 1:1 nominal USD-to-USDC ranking estimates while live challenges
  still govern payment.
- **Botsmith and Kronos:** Use `format = "x402"` for dollar-string `x-x402`
  prices (USDC on Base).
- **Exa and AGProxy:** Keep `pricing_key` display-only because published ranges
  depend on request options or traffic.
- **StableEnrich, GLIM, ShopSocial, CompanyRecords, and HealthTravel:** Parse
  nested USD decimal tariffs without confusing MPP or Stellar 7-decimal atomic
  units for 6-decimal Base USDC amounts.
- **Curl:** Its fixed per-call tariff prices the Curl relay call itself, not
  the relayed target.

## Reliability annotations

`reliability_tags` and `reliability_note` emit startup warnings without hiding
tools or enabling retries (see
[provider reliability observations](../docs/configuration.md#provider-reliability-observations)):

| Provider | Observed concern |
| --- | --- |
| OneShot | Repeated catalog HTTP `403` (help remained reachable). |

## Help and compatibility

Configured `help_url` documents load lazily on call and cache for the process.
Source and CLI tag filters retain help; positive listener tag filters drop
untagged help unless included by name. Media and async polling gaps are tracked
in [artifact storage](../docs/plans/artifact_storage.md) and
[polling tools](../docs/plans/deferred/polling_tool_support.md).

## Payment extension and offer boundaries

See [payment extensions and offer metadata](../docs/architecture.md#payment-extensions-and-offer-metadata)
for shared challenge admission rules. Ten raw unsigned challenges are pinned in
[provider_payment_challenges.json](../tests/fixtures/provider_payment_challenges.json).

- `payment-identifier`: optional request and payment deduplication (`required: false`);
  omitting it never authorizes replaying a signed request.
- `agentkit`: Exa's World-chain (`eip155:480`) proof and 100-use trial option;
  not required for Base USDC payment.
- `locus.freePreflight`: untrusted vendor hints; never auto-executed.
- `offer-receipt` and `otto-content-receipt`: seller-signed offer records (not
  verified buyer payment proofs).
- `sign-in-with-x` (SIWX): wallet-ownership authentication (e.g., Otto's
  one-hour read-only re-access); stripping it does not grant re-access or
  same-wallet job polling.

### Offer metadata is a separate compatibility boundary

| Provider | Accepted opaque fields | Managed result |
| --- | --- | --- |
| Agent402 | `accepts[].outputSchema` | Echoed unchanged on Base exact offers; `upto` and Permit2 remain unsupported. |
| Exa | `extra.breakdown`, `extra.totalUsd`, `extra.acceptId` | Echoed unchanged on Base EIP-3009 offers; `GatewayWalletBatched` unsupported. |
| Google Trends | `extra.merchant`, `extra.tier` | Echoed unchanged. |
| Locus | `GatewayWalletBatched` alternatives | Selects plain Base EIP-3009 USDC and strips top-level extensions. |
| Otto | Permit2 alternatives | Selects plain Base EIP-3009 USDC and strips top-level extensions. |

HALOWERK's `extra.asset = "USDC"`, `network_name`, `decimals`, and nested
`eip712Domain` are informational annotations; the effective signing domain and
token come from standard offer fields, while `extra.assetTransferMethod = "eip3009"`
remains subject to the shared EIP-3009 check.

## Pinned relay contracts

`agproxy-openapi.json`, `horizonpulse-openapi.json`, and `curl/openapi.json`
are unmodified vendor OpenAPI assets, and `readfetch-openapi.json` adds the
local `$ref` closure described under `readfetch.toml`. All four embed through
`src/providers/assets.rs` so relay bootstrap validates locally without remote
fetches.

To refresh pinned relay schemas:

```sh
make run CARGO_ARGS='--example provider_updates -- capture --provider agproxy --provider horizonpulse --provider readfetch'
make run CARGO_ARGS='--example provider_updates -- replay target/provider-updates/RUN'
```

Review the corresponding guides (`agproxy.shveik.dev/llms.txt`,
`horizonpulse.dev/llms.txt`, `readfetch.com/llms.txt`), verify local `$ref`
closure, run `relay_policy` and adapter tests, and follow the
[acceptance procedure](../docs/provider-updates.md#stage-and-accept-changes).
Note that AGProxy's lifecycle metadata sets no fixed advance-notice window for
API changes.

## Other provider contracts

| Definition(s) | Boundary or integration contract |
| --- | --- |
| AGProxy | Root `base_url` overrides `/v1`; both `/x402` and `/mpp` accept x402. |
| AgentFund, Brazilayer, Deepline, PDL | Path name anchors and request guidance; no extra compatibility exclusions. |
| Agent402 | Excludes wallet-owned memory, receipts, feedback submit, raw audio speech, and multipart transcription; keeps JSON and base64 media and PNG and JPEG modes. |
| AgentUtility | Local OpenAPI and registry tag and alias join; excludes persistent watch-page. |
| AgOSINT | Excludes `/mpp` rail; `/x402` anchors names; root `base_url` fixes `/v1`. |
| Arkham | Excludes three SIWX discovery routes. |
| BlockRun | Excludes async and hybrid media jobs, wallet lifecycles, `/api/v1/rpc`, and trade submission; keeps literal fixed-symbol paths. |
| botsmith, Google Trends | Structured price metadata is advisory. |
| Claw402 | Keeps open JSON passthrough for sparse RootData POST bodies. |
| Concordance | Local curated OpenAPI with inlined `$ref`s and authored prices; free `/catalog` completes series lookup. |
| Curl | Local schema; unsigned GET relay returning text only. |
| Datakoot, IPIntel | Open catalogs; interpret scores and hosted links carefully. |
| Exa | Exact `POST /search` and `POST /contents` payable island. |
| Glassnode | Hand-authored 4-route metadata-to-metric gateway; deeper `/bulk` subpaths unsupported. |
| HireScrape | Excludes SIWX skills, runs, snapshots, reports, and scraper requests. |
| Horizon Pulse | Default `POST` twins; excludes `POST /api/feedback` and `POST /api/vuln/lockfile`. |
| Kronos | `/api/v1` anchor with `""` fallback for public stats and methodology. |
| Locus | Local catalog with free `locus_list` and `locus_call` gateway; excludes participation writes and async job routes. |
| LoneStarOracle, x402stock | Full catalogs with optional compact recommendations. |
| NetIntel | 18 same-path `POST` twins; explicit PNG data-URI image mapping on `/ai-image/generate`. |
| News Gurus | Local Bazaar projection with declared path templates and `agent-status`. |
| OneShot | Exact island of `POST /v1/tools/search` and 11 public discovery GETs; excludes async EIP-712 and SIWX jobs. |
| Otto | Excludes `Execution` tag and `/full-auto` router. |
| RegimeShift | Relative spec server pinned to `https://regimeshift.xyz/api`; long-poll `/match` needs `--read-timeout-seconds 320`. |
| RelayShield | Excludes metered and account routes, TAP intent and settle, and file-scan polling; optional authored phone companion. |
| SocialFetch | Excludes account, monitor ownership, and webhooks; preserves public `GET /v1/monitors/sources`. |
| StableEnrich | Excludes Hunter verifier jobs, Cloudflare crawl jobs, and Aerial View video jobs. |
| Straits | Local authored union of 10 premium OpenAPI GETs and 38 documented free GETs. |
| Vaaya | Excludes MPP-only video, upload and async jobs, accounts, messaging, and payment forwarding. |
| x402-list | Exposes public reads, `/assess`, `/suggestions`, and `/submit`; excludes owner-token routes. |

## Reviewed but unavailable: Apollo

Apollo has no active provider definition because its request-ID retry and
recovery semantics remain unverified.

## Reviewed and rejected: x402node

`api.x402node.dev` has no enabled provider definition. Its Base USDC challenge
sets the EIP-712 domain name to `"USDC Coin"` instead of the contract's
`"USD Coin"`, causing EIP-3009 validation to fail with `payment rejected`, and
its pay-to wallet (`0x4466d4A84b7c49a6A094ec6eef4a0712D6dd125e`) shows no
recent incoming USDC transfers. Its working domains are also covered more
cheaply by APEX, 2s, Otto, and OpenWeb Ninja; revisit only if payment
settlement is fixed.

---

## Banking Compliance

`banking-compliance/openapi.json` is an authored HTTP projection combining
[root discovery](https://banking-compliance-production.up.railway.app/) (routes
and prices) with `POST /mcp` `tools/list` schemas (Draft-07 dialects preserved;
no inferred enums or bounds). Refresh MCP schemas via the
[manual public MCP procedure](../docs/provider-updates.md#manual-public-mcp-schema-evidence).

- `POST /api/campaign` ($5) is excluded because the free preview supplies no
  complete paid request schema.
- Keyword `APY` returns rules unfiltered but returns no matches with the
  advertised `Reg DD` filter. Checks can miss full APY wording requirements,
  and observed rule citations had section errors (e.g., citing `1030.8(a)` for
  `(b)`). Treat the 56 rules as advisory summaries alongside authoritative eCFR
  retrieval.
- Draft generation returns `needs_review` and warnings and can hallucinate a
  website or `"Member FDIC"` even with `fdic_insured=false`. It stays in the
  reusable provider with a warning and is omitted from `research_desk.toml`.

## Blockchain Money Map

Uses [vendor OpenAPI](https://api.blockchainmoneymap.com/openapi.json) and
[guide](https://api.blockchainmoneymap.com/llms.txt) (113 operations, no
same-path twins; `/api/x402`, `/api/devagentpal`, and `/api/agents` anchors
with `""` fallback).

- Base snapshots can return native balances and NFT and activity data with
  `Partial` or low confidence and token balances unavailable (empty holdings do
  not mean zero balance). Complements Arkham (which adds timestamps, fees,
  labels, and token transfers).
- The failure classifier classifies only supplied error codes without
  contacting targets or facilitators. The $5 deep report exceeds typical
  financial caps.
- Saved and deep reports persist at unlisted URLs accessible to anyone with the
  link. Hash notaries and ledgers return one-time lookup secrets that the
  caller must save; revocation propagation accepts separate authorization proof
  and may contact opted-in targets.

## socialfetch.toml | SocialFetch

- The live `https://www.socialfetch.dev/openapi.json` exceeds the default
  32 MiB `max_spec_bytes` and is rejected as incomplete (do not raise the
  global limit for this provider). Paid settlement can fail intermittently and
  page readers can return `503 temporarily_unavailable` without a receipt; both
  multi-listener examples use the research-desk social set instead.
- Excludes `GET /health`, account identity, monitor ownership, and webhooks
  while preserving public `GET /v1/monitors/sources`. Uses exact case-sensitive
  platform tags (`--tags Twitter,YouTube`); `include = ["/v1", ""]` strips
  `/v1` from names while keeping `/v2` prefixes (`base_url = "https://api.socialfetch.dev"`).
- `credit_pricing` converts `x-socialfetch-pricing` at a fixed display rate of
  `0.014` USDC per credit (preserving TikTok `transcript_target_not_video`
  1-credit `billableDomainErrors` and Instagram v2 `include` surcharges). Use a
  90-second read timeout for slow search or transcript calls.
- `tests/fixtures/socialfetch_openapi.json` is a scoped capture for regression
  tests and public demos; refresh via `snapshot_spec` to a temp path.

## deepline.toml | Deepline GTM (stable-deepline.dev)

POST contact-lookup tools from vendor OpenAPI (`additional_properties = false`).
`[pricing]` uses `usd_as_usdc = true` for fixed USD `x-payment-info` hints.
Waterfall endpoints are slow; run with `--read-timeout-seconds 120`.

## pdl.toml | People Data Labs (stablepeopledata.dev)

Person and company enrich (fixed USD with `usd_as_usdc = true`) and search
(dynamic size-scaled `min` and `max`), with `additional_properties = false`.
Overrides preserve the "free on no match" note for enrich and the "an empty
result set is free; recommended size 1-5" note for search in `description`,
keeping numeric tariffs in `pricing_description`.

## botsmith.toml | x402.botsmith.dev

GET trading-signal routes (X, Polymarket, perp funding and OI, stablecoin pegs)
using `format = "x402"` dollar strings ($0.015-$0.05 observed).

- X search can return a continuation cursor longer than the schema's declared
  input limit (do not truncate it or treat page 1 as complete).
- Post-authorization `404` responses can occur without explanatory detail.

## x402stock.toml | x402stock.xyz

Uses the `https://x402stock.xyz` Base USDC host (`agents.x402stock.xyz` is
MPP on Tempo). `/api/v1` anchors names with `""` fallback; `recommended_tools`
covers symbol discovery, company overview, FOMC calendar, and lazy `llms.txt`
help. Instructions preserve `{ticker, data, source, as_of}` conventions.

## google-trends.toml | Google Trends SEO keyword data (x402 Atlas)

Uses [vendor OpenAPI](https://google-trends.use.x402atlas.com/openapi.json)
($0.05 timelines, $0.03 regional and related via `x-payment-info`) plus lazy
help.

- `trend` and `trend_over_time` accept all windows (`trend_over_time` defaults
  to 12 months and reports granularity). When `data_source` indicates fallback,
  verify returned geography, window, and `notice` before making scoped claims.
- Related-query lists can include unrelated or spam terms even from primary
  sources.

## arkham.toml | Arkham Intel x402 rail (api.arkm.com/x402)

Paid POST blockchain-intelligence routes with `x-payment-info` (fixed per-call
hints via `usd_as_usdc = true`; credit weights explain the amount rather than
multiplying it) and lazy `llms.txt` help.

- Dynamic-priced `/swaps`, `/transfers`, and `/transfers/unenriched` publish
  wide ranges (e.g., `$0.40-$660`); pass a small `limit` to stay under caps.
- Excludes `/chains`, `/networks/status`, and `/arkm/circulating` (SIWX
  challenges).
- Behind Cloudflare (can 403 datacenter clients; if spec fetch fails with HTML
  403, snapshot from a browser to a local file).

## regimeshift.toml | RegimeShift (regimeshift.xyz)

Agent-SOFR, variance-aware max-LTV, and ETH and BTC volatility risk premium
($0.001/call via `x-payment-info`) plus free on-chain RFQ routes. Pins
`base_url = "https://regimeshift.xyz/api"` (spec server is relative) and marks
free endpoints explicitly. `/v1/intent/{id}/match` long-polls up to 300s; run
with `--read-timeout-seconds 320`. Excludes owner Safe and Zodiac provisioning.

## keenable.toml | Keenable (api.keenable.ai)

Exact x402 allowlist: `POST /v1/x402/search` and `GET /v1/x402/fetch` (omits
API-key, `/public`, auth, SELECT, GET search, and `session_id`).

- `fetch` returns Markdown (`live=false` indexed, `live=true` live JS render;
  Cloudflare walls return `403`; PDFs return plain text).
- Search snippets are long (set `snippet_max_length`); `prompt` returns only a
  model extraction. No catalog pricing extension (typical challenge $0.01).

## kronos.toml | Kronos Crypto Data (kronossignals.com)

Crypto market data under `/api/v1` (16 spot assets and HYPE derivatives-only,
BTC and ETH options, $0.05 ML forecasts, free `kronos_track_record`, $0.10
`briefing`) with `""` fallback exposing `/api/health`, `/api/stats`,
`/api/methodology`, and discovery. Uses `format = "x402"` dollar strings;
`kronos_track_record` keeps its free tariff in `pricing_description` and notes
that hit rate covers cache-source rows only and Brier score covers its audit
sample.

## concordance.toml | Concordance (concordancehq.duckdns.org)

Six paid POST aggregator tools ($0.01-$0.05) plus free `/`, `/catalog`,
`/healthz` (`include = ["/tools", ""]`) and lazy `/llms.txt` help. Uses local
authored `providers/concordance/openapi.json` (inlined `$ref`s and authored
`x-payment-info` prices absent from the vendor spec), pinned by
`tests/fixtures/concordance_openapi.json` and `tests/provider_catalogs.rs`.

## agentfund.toml | AgentFund US Economic, SEC & On-Chain Data (x402.agentfund.net)

Live vendor spec with inline POST bodies and `x-payment-info` ($0.001-$0.03)
covering US macro, SEC EDGAR, on-chain EVM reads, and JSON and tabular repair.
Condenses `x-guidance` into `instructions_text`.

- Pass `body = {}` when no filters are needed (omitting the body on retail
  sales can return HTTP `415` after a potentially chargeable authorization).
- Retail summary's advertised ex-autos label can return auto-inclusive headline
  numbers; verify against
  [FRED Census tables](https://fred.stlouisfed.org/release/tables?rid=9) and
  use 2s FRED series `RSFSXMV` for ex-autos retail sales.
- Portfolio calls can fail all four Base RPC reads (not a reliable balance
  source).

## otto.toml | Otto AI x402 swarm (x402.ottoai.services)

Live vendor spec with `x-payment-info`, default `POST` selection on three
twins, and lazy `llms-full.txt` help.

- Excludes `Execution` tag (`/swap`, `/bridge`, `/deposit`, `/withdraw`) and
  `/full-auto` spending router.
- `/video-gen` is dynamically priced `$0.46-$4.60` (exceeds default $1 cap at
  the high end).
- `stock_buy_tx` returns unsigned Base approval and swap transactions only
  (`x402_treazury` never executes them).
- Page extraction can return static text, incomplete JavaScript, or a bot
  challenge as a `200`; inspect content and note `publishedTime` is provider
  metadata.

## brazilayer.toml | Brazilayer registry & market data (api.brazilayer.com)

Brazilian CNPJ, PNCP tenders, EUDR, and macro and news feeds (Portuguese
response keys). Dereferences shared `{cnpj}` parameter `$ref`s, marks 16 free
routes explicitly, and restores truncated `integridade` and `nome` descriptions
via overrides (scoped fixture `tests/fixtures/brazilayer_openapi.json`).
Advertised prices come from external `/.well-known/x402` (`items[].accepts`),
matching only unique origin and template pairs (ambiguous records stay
unknown).

## brave-search.toml | Brave Search Proxy (search.agent.s.brave.app)

Fixed $0.005 (`5000` micro-USDC) x402 v2 Base USDC search proxy with pinned
`base_url` (spec omits `servers`). `llm_context` returns search-grounded
passages grouped by URL (not a live JS or PDF URL fetcher). Answers,
autosuggest, spellcheck, and summarizer are not payable routes.

## genuinegood.toml | Genuine Good Grants (genuinegood.online)

Five Grants.gov routes (`search`, `detail`, `fit`, `brief`, `preflight`) with
default `POST` selection (`GET` expects comma-separated arrays), `/v1/grants`
anchor with `""` fallback, `x-payment-info` ($0.05-$0.50; `preflight` $5.00
exceeds default $1 cap), closed schemas, and lazy `llms.txt` help. Excludes
`/v1/grants/pass` ($15 bearer-token pass unsupported here). Pinned against
`tests/fixtures/genuinegood_openapi.json`.

## locus.toml | Locus US property & public-records context (api.locus.report)

Uses authored `providers/locus/openapi.json` combining paid POST routes
($0.01-$2.49), a two-tool free gateway (`locus_list` over 137 free tools and
`locus_call` over 135 public reads, avoiding slug collisions with `/api/locus-*`
paid routes), public discovery manifests (`/.well-known/locus-tools.json` and
`/.well-known/locus-agent-endpoints.json`), and lazy help.

- Excludes free participation writes `locus_agent_feedback` and
  `locus_request_coverage` (unprotected mutations without idempotency keys) and
  pollable async jobs (`place-report-batch`, `record-batch`, `property-update`,
  `GET /api/property-update-jobs/{id}`).
- Omit optional `delivery` on `permit-closeout-check` (requires webhook and
  token polling). Satellite `includeImagery=true` stays JSON base64.
- `place-report` and `environmental-context` can take 60-90 seconds. Note
  vendor guidance prohibits insurance use while offering an insurance-nonrenewal
  dossier (keep unselected until clarified). Groundwater drawdown covers USGS
  monitoring wells, not parcel wells.
- Refresh from `https://api.locus.report/openapi.json`, recording SHA-256,
  preserving paid schemas (removing only root `$schema` and content-level
  request examples) and free gateway exclusions, rewriting `locus_call`'s
  full-object wrapper alternatives into a flat `{name, arguments}` body with
  the 135-entry enum, and refreshing `tests/fixtures/locus_openapi.json`.
  `X-Locus-Report-*` response headers are not surfaced over MCP.

## straits.toml | Straits.live Hormuz monitor (straits.live)

Authored union of 10 premium OpenAPI GETs and 38 free GETs documented at
[straits.live/api](https://straits.live/api) (`Free` and `Premium` tags; lazy
`llms.txt` help). With both tiers enabled, collision naming produces
`straits_vessels_get` (paid) and `straits_vessels_get_2` (free; supports
`flagged=true` and `region=hormuz|cape`, with Cape returning null).

- Excludes session creation, token-authenticated SSE streams, and webhook
  subscriptions.
- Retired Hormuz and Cape snapshot feeds are frozen as of 2026-08-28. Free JSON
  quota is 60/min and 1,000/day per IP; tick cache is 120s.
- Escalation index runs on a 10-minute cadence (literal feed name `index-5min`
  stays unchanged; older pages can be 5-minute; export rolls older rows into
  daily aggregates). `index` history pages up to 7 days via `before` and
  `nextBefore` (invalid cursors return uncharged `400`). Preserve cadence,
  `sourceKind` and `observedAt` staleness markers, and container vs. TEU units.

## lonestar.toml | LoneStarOracle catalog (lonestaroracle.xyz)

Unified [OpenAPI](https://lonestaroracle.xyz/openapi.json) opts into
`operation_url_extension = "x-direct-url"` because `/api/*` gateway paths
redirect `307` to per-service hosts. Original paths keep driving tool names,
filters, and mappings; nullable `anyOf` scalar query params work directly.
Optional cover is disabled (`cover_traffic_enabled = false`) because the
gateway negotiates HTTP/1.1 (`allow_http1 = true`) and operations span multiple
origins.

## glassnode/ | Glassnode x402 gateway (x402.glassnode.com)

Hand-authored `providers/glassnode/openapi.json` (vendor publishes no OpenAPI)
covering three $0.01 metadata discovery routes (`assets`, `metrics`, `metric`)
and one $0.05 templated metric route (`/v1/metrics/{category}/{metric}`), with
lazy help and a discover -> metadata -> fetch workflow in `instructions_text`.
Deeper subpaths such as `/bulk` are not addressable through the two-segment
template.

## exa.toml | Exa Search and Contents

Exact allowlist of `POST /search` and `POST /contents` (the only x402 routes in
Exa's [API spec](https://api.exa.ai/openapi.json) and
[x402 guide](https://exa.ai/docs/integrations/payments/x402/quickstart.md)) plus
lazy `exa_help`. No API key is needed; both operations are untagged.

- **Pricing:** `lite` and `instant` $0.004 (indexed, restricted inputs),
  `auto` and `fast` $0.007, `deep-lite` and `deep` $0.012, `deep-reasoning`
  $0.015; search summaries +$0.001 per result; Contents $0.001 per page per
  requested type (10-result x402 search max).
- **Schema notes:** Recursive `JsonValue` union normalizes cleanly for static
  loading (dynamic remote import still rejects whole-document cycles). Contents
  preserves root `oneOf` (`ids` vs. `urls`).
- **Caveats:** `contents.highlights.dynamic` and `verbosity` require
  `Exa-Beta: dynamic-highlights-2026-08-28`, which generated tools do not
  inject. Keep `stream` omitted or `false`. `maxAgeHours`: omitted = cache or
  fetch, `-1` = cache only, `0` = fresh; `snapshotAsOf` reads stored history.
  90-second read timeout.

## oneshot.toml | OneShot synchronous web search

Exact island of `POST /v1/tools/search` ($0.001 Base USDC via `PAYMENT-REQUIRED`
header; `usd_as_usdc = true`) and 11 public discovery GETs (`/health`, ACP
manifest, stats, status, soul reads), using `include = ["/v1/tools", ""]` and
lazy search-guide help.

- Excludes asynchronous HTTP `202` job routes requiring same-wallet `GET /v1/requests/{id}`
  polling and short-lived EIP-712 ownership proofs (tracked in
  [polling tools](../docs/plans/deferred/polling_tool_support.md)).
- Positive listener tag filters must include `Search` to keep search.

## stableenrich.toml | StableEnrich multi-provider data APIs

Synchronous JSON and image catalog across CompanyEnrich, Exa, Firecrawl,
FullEnrich, Google Maps and Solar, Minerva, PDL, Reddit, Serper, and Whitepages
(`/api` anchor with `""` fallback, 90s read timeout, exact vendor tag casing
such as `Fullenrich` and `Pdl`, lazy `llms.txt` help).

- **Excluded async workflows:** Hunter email-verifier POST and jobs (SIWX),
  Cloudflare crawl POST and jobs (JWT and SIWX), and Aerial View render-video
  and lookup-video (async paid polling). Note that the vendor spec lists `200`
  and `402` even on documented `202` routes.
- **Usage notes:** `x402_treazury` returns raw HTTP bodies without AgentCash's
  `{ success, data }` wrapper. Maps pagination repeats `textQuery` with
  `pageToken`; FullEnrich requires nonempty filters and `search_after`. Keep
  `stream` omitted or `false`. Solar and Serper asset URLs remain JSON
  metadata.

## stableninja.toml | StableNinja selected data APIs

Backup catalog ($0.01/call, `/api` naming anchor) covering marketplace, Yelp
(`page_size <= 50`, `num_pages <= 10`), Waze, and Google Play reads. Amazon
reviews expose cookie-less managed reads without durable access guarantees;
Google Play charts cover only the labeled head.

## agent402.toml | Agent402.Tools

Broad catalog with lowercase tags (`/api` and `/v1` prefixes stripped) and
advisory web-research recommendations. Narrow in deployments via source `tags`
(note `time` vs. `date-time` are distinct, and `catalog tags` includes excluded
routes).

### Exclusions and protocol boundaries

- Excludes wallet-keyed memory, account history and receipts, feedback
  submission (keeps public summary child), same-payer attestations, raw
  `/v1/audio/speech` (unsupported audio MIME), and `/v1/audio/transcriptions`
  and `/v1/pro/audio/transcriptions` (multipart upload misdeclared as JSON).
- Keeps `/api/onramp-link` (checkout link only), synchronous JSON and base64
  media, PDF, TTS, and video wrappers, and PNG, JPEG, and WebP screenshot, QR,
  crop, and image-generation routes (`n = 1`, explicit base64 and data-URI
  mappings).
- Some LLM and report tools exceed $1 or 90s; keep `stream` omitted or `false`
  and use small token budgets.
- `Idempotency-Key` is generated fresh per invocation (no durable replay across
  calls); ignore vendor prose claiming missing receipts prove no charge. Guide
  references `/api/find` and `/api/pricing` absent from the OpenAPI spec.

## agentutility/provider.toml | AgentUtility

Local `providers/agentutility/openapi.json` joins the API host's
[live spec](https://x402.agentutility.ai/openapi.json) (817 POST ops, 286
untagged) with [registry.json](https://agentutility.ai/registry.json) (282
aliases, 17 clusters, `x-agentutility-alias-of`) so cluster tags (`web-probe`,
`wordmint`, `edge-finance`) work without dropping untagged routes.

### Payment compatibility

Base USDC v2 exact challenges (`bazaar` and `builder-code` extensions stripped
by shared policy; Solana offers ignored; ignore stale `X-PAYMENT` guide prose).
Some routes exceed $1 or 90s; callers never supply upstream vendor keys.

### Output and workflow caveats

- Hosted `image_url`, `audio_url`, `video_url`, and `pdf_url` links stay JSON
  metadata. Four satellite routes (`satellite-address`, `satellite-bbox`,
  `satellite-tile`, `satellite-change`) extract base64 images via `content_type`.
- One-shot browser-session, IPFS-fetch, QR-code-generate, and six synchronous
  source audits (`oneOf` repo or supplied files) stay exposed. Excludes
  `watch-page` (365-day server-side state).
- `web-search` can fail with HTTP `424` (`upstream_status=429`) and patent
  keyword search can fail with USPTO `403`, while link extraction, static-page
  JSON, and archive lookup keep working (`research_desk.toml` excludes
  `web-search`). Preserve degraded markers on compose bundles.

### Refreshing the catalog

Download the public OpenAPI and registry JSON to temp files and run:

```sh
make run CARGO_ARGS='--offline --example snapshot_agentutility -- /tmp/agentutility-openapi.json /tmp/agentutility-registry.json providers/agentutility/openapi.json'
make test CARGO_ARGS='--offline --example snapshot_agentutility'
```

## BlockRun and Claw402

- **BlockRun:** Keeps `/api/v1` wire paths (guide often writes `/v1`), literal
  fixed-symbol paths (`BTC-USD`, `XAU-USD`, `EUR-USD`, `BTCUSDT`), synchronous
  speech and SFX hosted links, and stateless Messages and Responses (`stream`
  omitted or `false`, small `max_tokens`). Excludes hybrid and async music,
  image, and video jobs, phone and sandbox ownership, voice calls,
  `/api/v1/rpc`, and trade submission.
- **Claw402:** Keeps open JSON bodies for sparse RootData POST schemas: search
  (`query`, `precise_x_search`), project (`project_id` or `contract_address`,
  `include_team`, `include_investors`), org (`org_id`, `include_team`,
  `include_investments`), and funding (Unix-second `start_time` and `end_time`,
  `min_amount` and `max_amount`, `project_id`, `page`, `page_size`).

## openverbs.toml | OpenVerbs combined catalog

Combined [apex catalog](https://openverbs.com/openapi.json): 418 POST
operations at `/api/<verb>/v1/<operation>` across 93 verb families (`/api`
anchor with `""` fallback; omits per-service `/health`).

- Root `x-payment-info` lists MPP on Tempo (`network: "4217"`) while Base USDC
  sits under `protocols`, so `[pricing]` renders display tariffs without
  selection hints (live x402 challenges use the right Base `"USD Coin"`
  domain). Do not attach apex `/.well-known/x402` as `x402-resources` (its
  resource URLs use subdomain origins).
- OpenAPI tags mix 2,563 keyword values; filter by path verb segment rather
  than tags. No `llms.txt` exists (`404`).

## Curl HTTP Request

Pinned `curl/openapi.json` for `POST /curl` ($0.01 Base USDC). Returns target
status, headers, and UTF-8 text body only (binary API responses fail
terminally; bodyless `HEAD` works). Cannot forward conditional headers, and
follows redirects remotely (adapter checks requested, final, and every reported
hop against public-destination policy).

## relayshield.toml | RelayShield

Exposes pay-per-call routes plus keyless `POST /v1/email-check`,
`POST /v1/link-check`, and `POST /v1/wallet-risk` (`/v1/payg` anchor with `""`
fallback) and lazy `llms.txt` help.

- Excludes account and metered routes, file scanning (polling schema absent
  from OpenAPI), and TAP `POST /v1/tap/intent-bind` and
  `POST /v1/tap/settle-check` (API-key or bearer verifier entitlement).
- Bulk identity risk ($2.00) and campaign scanning ($5.50) exceed the $1 cap.
- `relayshield/phone-reputation.toml` is an authored companion for the phone
  endpoint omitted from OpenAPI (shares the `relayshield` prefix, omits help,
  and currently challenges with x402 v1).

## ipintel.toml | IPIntel.ai

Live [OpenAPI](https://tools.ipintel.ai/openapi.json) and lazy `llms.txt` help.

- Guide advertises `$0.001-$0.17` while current operation prices span
  `$0.001-$0.09` (satellite $0.09).
- Satellite returns hosted PNG links (`date = null` means latest; `fallback = true`
  may pick a nearby scene; keep Copernicus attribution).
- HTTP latency accepts `HEAD` or `GET` from a Cloudflare Worker edge (not ICMP
  ping). Domain-health can score `100` with DMARC `p=none`; email validation is
  probabilistic; JWT decode does not verify signatures.

## newsgurus/provider.toml | News Gurus Intel API

Local `newsgurus/openapi.json` projects `https://api.newsgurus.ai/x402/catalog`
(the vendor's `/openapi.json` hardcodes example IDs like `NVDA` and omits
fantasy and `/api/v1` paid routes), preserving declared GET path and query
parameters, `x-payment-info` ($0.002-$5; some fantasy calls exceed $1), and
lazy `llms.txt` help. Refresh from `tests/fixtures/newsgurus_x402_catalog.json`.
Derived trading intelligence (not a wire service); `primary_scoops` has no
ticker filter.

## vaaya.toml | Vaaya

Live `https://vaaya.ai/openapi.json` excluding MPP-only video, upload and async
jobs, persistent account, memory, and sandbox routes, messaging and purchases,
and the $5 `vaaya/fetch` payment forwarder.

- Firecrawl uses deprecated `/v1/scrape` (`proxy=stealth` can time out;
  `research_desk.toml` uses `stableenrich_firecrawl_scrape` instead). Paid
  receipts can report CDP `payment-method-required`.
- Many schemas are open (TheirStack jobs needs a date or company;
  technographics and Fundable need an identifier; CourtListener requires `q`
  even with `party_name`; RecallRadar search takes `q` in an open POST body).
- Description prose prices lag `x-payment-info` on Tako ($0.03, $0.03, and
  $0.05 vs. prose $0.012, $0.018, and $0.002) and RecallRadar ($0.12 and $0.03
  vs. $0.10 and $0.025). EDGAR autocomplete and CourtListener RECAP are
  incomplete indexes; preserve `ok`, `data`, and `charged_cents` envelopes.

## agosint.toml | AgOSINT

Excludes `/mpp`, keeps `/x402` anchor with `""` fallback for `/endpoints`, and
pins root `base_url` (fixing `/v1`). Company lookups require exactly one key
(`id` and `name` also require `country`), network uses `ip` or `asn`, and
sanctions uses `name` or `wallet`. Invalid inputs can still receive a paid 402
quote.

## datakoot.toml | Datakoot

Uses `https://x402.datakoot.com/openapi.json` (`/api` anchor with `""`
fallback) and apex help `https://datakoot.com/llms.txt`. Preserve alternate
CVE and dependency guidance, US-only location limits, and per-operation
estimates.

## hirescrape.toml | HireScrape

Excludes SIWX routes (`/api/skills/*`, `/api/snapshots*`, `/api/reports*`,
`/api/scraper-requests`), keeping `/api/tools` anchor with `""` fallback.

- Allowed scrapers can still hand off to unsupported SIWX snapshots above ~200
  records or on async modes (such as YouTube `videos_by_*`,
  `channels_by_keyword`, and `comments`; TikTok lacks a current sync and async
  table). No runtime prepayment guard blocks `async: true` challenges.
- Instagram, LinkedIn, YouTube, and TikTok can fail with upstream `401 Token
  expired`; Indeed jobs can return complete items with an `sd_sync_` ID. Trust
  schema mode enums over legacy guide examples (LinkedIn has `profile`,
  `company`, `company_posts`, `post`; YouTube has no transcript mode).

## netintel.toml | NetIntel

18 same-path `GET` and `POST` twins default to `POST` (keeping secrets out of
URLs and arrays in JSON); `/api/v1/chat/completions` and `/api/v1/embeddings`
aliases remain alongside `/v1`. `/ai-image/generate` has an explicit PNG
data-URI image mapping. Guide summary prices lag operation metadata.

## x402-list.toml | public directory

Exposes public reads, synchronous `/assess` (nonrefundable charge),
`/suggestions`, and `/submit` (required `body` `oneOf`, $1 nonrefundable review
queue). Excludes owner-token routes (`request-update`, `verify-ownership`,
`reissue-token`). Distinct from the embedded 3-read directory contract.

## agents-trust.toml | Agents Trust

Recommendations and endpoint evidence use the standard Base x402 flow.
Agents Trust's payment-attempt challenge extension is optional. Paid answers
can arrive before settlement. The response header carries an attempt ID for
the provider's free receipt lookup. The client has no recovery or
reconciliation action that uses this ID. The ID gives the agent no useful
next step for the returned answer, and a receipt status gives it no authority
to repeat the paid call. The client keeps the ID out of tool results. Receipt
lookup accepts an ID obtained separately. See the
[Agents Trust payment guide](https://agents-trust.com/guides/pay-a-402-from-an-agent/).

## agproxy.toml | AGProxy

Pinned `agproxy-openapi.json` with root `base_url` (fixing `/v1`), `/x402`
anchor with `""` fallback, and both `/x402/*` and `/mpp/*` aliases (both accept
x402).

- `/x402/fetch` supports `GET`, `HEAD`, `POST`, string bodies, custom headers,
  and country exits (returns a 1-element array with `status`, `final_url`,
  `headers`, `body` or `body_base64`, and `truncated`). Default response cap
  1 MB (up to 10 MB), outer JSON request cap 64 KiB (overrides 1 MiB body
  schema), 25s limit, up to 5 redirects (final URL only, no hop chain), no JS
  rendering.
- Quotes range $0.0156-$0.0372; target `403` and `404` responses are billed,
  and invalid inputs can return `402` with `X-Agdata-Invalid-Request: 1`.
  Vendor refund promises are not local spendability or replay authority.

## horizonpulse.toml | Horizon Pulse

Pinned `horizonpulse-openapi.json` (`/api` anchor with `""` fallback, 7 default
`POST` twins).

- **Excluded:** `POST /api/feedback` (unprotected persistent write) and
  `POST /api/vuln/lockfile` (missing `requestBody` and requires body-bound
  `lockfileBindingNonce` incompatible with random EIP-3009 nonces).
- **Pricing:** Omits `pricing_key` because `x-payment-info.amount` holds atomic
  units (`10000` for $0.01) while `priceUsd` and `x-premium-payment-info` are
  unconsumed by the generic adapter; `/.well-known/x402` is advisory only.
- **Relay (`/api/http`, $0.01):** Supports `GET`, `POST`, `HEAD`, `PUT`,
  `PATCH`, and `DELETE`, allowlisted headers, a 384 KB response cap and 64 KB
  body cap, 12s limit, and 3 redirects (final URL only). Target `4xx` and `5xx`
  responses are billed data; `/api/fetch` returns cleaned text, not raw
  responses.
- **Data notes:** SEC company facts return separate annual and 3-month flows
  (2s can return fiscal YTD; keep period dates). BLS jobs omits query param
  names (20-year max); `/api/domain/precheck` declares no params or body
  (arguments route as query parameters). Seller report retains checked URLs
  and addresses for 90 days.

## voipstore.toml | VoIPStore

Pins `base_url` from [guide](https://api.voipstore.xyz/llms.txt) (spec omits
`servers`) with `/agent/v1` anchor and `""` fallback.

- `POST /agent/v1/orders` takes `X-Quote-Id` and uses automatic
  `Idempotency-Key`. Excludes `/agent/v1/topup` to avoid persistent prepaid
  balances.
- Quotes last 5 minutes; catalog reads $0.002, status polls $0.001 (`received`
  with `sms_code` and `sms_text`, or `expired`).

## readfetch.toml | readFetch

Pinned `readfetch-openapi.json` with authored local `$ref` closure moving
identical `Auth`, `Cookie`, and `ScrapeOptions` definitions from per-operation
`$defs` bags into `components.schemas` (both scrape routes work; maintain as
`authored` on updates). Uses `/v1` anchor with `""` fallback for `/healthz`,
100s read timeout (vendor 95s ceiling), and per-tool `pricing_description`
overrides (spec omits `x-payment-info`; batch and crawl have 10-page minimums;
`/v1/probe` and crawl-status are free).

- **`/v1/fetch` relay:** Requests `binary: true`, returns exact bytes via
  `content_base64`, checks `final_url` (no hop chain), treats target `402` as
  terminal, enforces a 2,083-char URL cap, and caps target bodies at 10 MB or
  about 75% of the caller's outer relay limit for base64 expansion. `billed`
  tracks readFetch's charge; `provenance.extraction.truncated` marks
  truncation (falling back to the top-level final URL and untruncated state
  when optional `provenance` is absent). The adapter skips this relay before
  paying if the serialized JSON request envelope exceeds 1 MiB, if the
  caller's outer response budget leaves fewer than 1,024 bytes after 4,096
  bytes of JSON and header overhead, or if a required transport-managed header
  cannot be relayed (required business headers and explicit API-key headers
  are forwarded while OpenAPI `Authorization` declarations are ignored).
- Excludes `POST /v1/claims` and `POST /v1/feedback`.

## AgData, Degenscan, and Market Intelligence API

Scoped contract fixtures live in `tests/fixtures/catalogs/*-contracts.json`.

- **AgData:** Root `base_url` fixes `/v1`; both `/x402/*` and `/mpp/*` accept
  x402. Screenshots stay JSON arrays (`imageBase64` and `contentType`) to keep
  PDF support intact. Social reads cover Instagram post, TikTok video (dates
  are not latest-first), YouTube video, Facebook page, and LinkedIn company;
  comments and transcripts are separate.
- **Degenscan:** Six Carry reads accept keyless x402 ($0.03 observed on
  funding-matrix; tariffs in `pricing_description`). Public oracle submit and
  free status poll stay exposed (poll until done or failed). Excludes
  credential issuance, prepaid keys, and MCP transport.
- **Market Intelligence:** Excludes `GET /api/v1/decision` and
  `GET /api/v1/decision/{symbol}` (comma-separated `include` array requires
  `explode=false`, unsupported by repeated-key query routing), credit
  purchases, webhook subscriptions (including event catch-up), `POST` feedback,
  and MCP endpoints. Opportunity scans default to `POST`.

## CoinMarketCap

Authored `coinmarketcap/provider.toml` unions the
[keyless OpenAPI](https://coinmarketcap.com/api/documentation/openapi-keyless.json)
(prefixed `/public-api`), four paid `/x402/*` counterparts copied from
[standard OpenAPI](https://coinmarketcap.com/api/documentation/openapi.json)
(`quotes/latest`, `listings/latest`, `dex/search`, `dex/pairs/quotes/latest`;
$0.01 Base USDC), and the
[x402 guide](https://coinmarketcap.com/api/documentation/ai-agent-hub/x402.md).

- Documentation URLs return `403` without a `User-Agent`, so runtime `help_url`
  stays disabled and capture records unavailable evidence.
- DEX search uses `q`; pair quote uses `contract_address` with `network_id` or
  `network_slug` (guide's `pair_address` example is wrong). Comma-separated IDs
  and many numeric selectors are strings.
- When refreshing the authored union, strip API-key security only on the four
  copied `/x402/*` routes and preserve transitive `$ref` components and both
  public holder `POST` methods.

## Nansen

Uses [public schema](https://api.nansen.ai/openapi.json) and
[x402 guide](https://docs.nansen.ai/getting-started/agentic-payments/x402-payments.md).
Excludes API-key-only account, feedback, alerts, search helpers, trading and
perp execution, labels and DeFi-holdings, and SSE-only Agent fast and expert
routes.

- Keep `premium_labels` omitted or `false` on holder and leaderboard reads.
  Prefer `/api/v1` over deprecated `/api/v1beta1` aliases.
- `x-payment-info.price` uses raw dollar strings (e.g. `"$0.05"`). First-call
  promo requires `X-Payer-Address` (not injected by `x402_treazury`).

## AX1 Verdict

Authored projection of `https://console.ax1.vc/openapi.json` and
`https://console.ax1.vc/agents.md` (use `console.ax1.vc` directly; `www`
redirects). Selects the external request branch (`tokenAddress`, optional
`chain` and `requestKey`) from the upstream `anyOf`, excluding Privy Console
inputs and Console run reads while keeping public
`GET /api/q-verdict/operations/{operationId}` (verify branch meaning, not just
position, on updates).

- `POST` ($0.02 Base USDC, also covers Solana mints) can return `202` pending;
  save `operation_id` and poll the free `GET` receipt (never re-invoke the paid
  `POST` as recovery).

## StableTravel

Authored `stabletravel/provider.toml` repairs 33 placeholder paths in
`https://stabletravel.dev/openapi.json` (`id`, `dest_id`, `registration`,
`entity_type`) into required path parameters (recorded in asset provenance).

- Google Flights returns cash fares and booking links (not bookings;
  round-trip `type="1"` needs `return_date`, one-way `type="2"`; search returns
  `booking_token` while booking expects `departure_token`).
- Seats.aero returns cached mileage awards (`search` -> `trips` using
  availability ID; `routes` lists covered airport pairs). Maps stay opaque JSON.
- Flight-intent `POST` takes `ident` in both path and `body.ident` (the legacy
  `ident_body` override alias targets `body.ident`); no callback or alert
  lifecycle exists. 2s flight status and boards share FlightAware upstream.

## 2s (`2s.toml`)

Authored `2s-openapi.json` snapshots `https://2s.io/api/openapi` without
`responses` and `components.responses` (which contain external `$ref`s) and
repairs four `allOf` CSV query parameters (`dataTypes`, `symbol`, `ids`, `vs`)
into validated CSV strings (`/api` anchor, prefix `twos`). Import by bundled ID
or API base to get these repairs.

- **Excluded:** Wallet-owned storage, locks, queues, schedules,
  publish-subscribe, watchers, batch (can embed stateful primitives),
  `/api/countdown/gif`, and `GET /api/gov/osha-violations` (unresolved
  object-valued `initialPenalty` query filter).
- **Data caveats:** GitHub lookup can fail `401`; scholarly search can fail
  arXiv while returning PubMed; HICP `latest` can lag; security resolver
  returns issuer-level ISINs (including debt); 2s POI can `502` from Overpass.

## Strale (`strale.toml`)

Authored `strale-openapi.json` uses `/x402/v2` paths and repairs nine
complex-input operations (`api-health-check`, `api-mock-response`,
`fake-data-generate`, `html-to-pdf`, `http-to-curl`, `json-schema-validate`,
`llm-output-validate`, `risk-narrative-generate`, `tool-call-validate`) from
`GET` (which returns `405` after payment verification) to `POST` with full
`/x402/catalog` `input_schema` JSON bodies.

- **Excluded:** Account transaction listing and deletion, audit tokens,
  disputes, solutions, webhook secrets, and web3 assurance (keeps public
  transaction verification).
- **Offers and data:** V2 offers include legacy `maxAmountRequired` (echoed
  opaquely while signing `amount`). Polish lookup requires a 10-digit KRS
  number (company name fails `400`); Finnish PRH is a company registry (not
  patents); beneficial ownership is UK PSC only; MiCA registers can report
  unavailable inside HTTP `200` (no match is not compliance clearance).

## Maxwell International (`maxwell.toml`)

Remote OpenAPI (`GET` data and `POST` diagnostics). Excludes `/v1/domain/sale`
(manual domain transfer).

- Doctor test purchases charge each attempt and keep unused price differences
  (fee formula stays in `description`; omitted from `research_desk.toml`).
- Paid Polish-registry and court searches can return `402 payment_invalid` or
  `facilitatorStatus: 502` (`upstream_unreachable`); do not replay failed
  authorizations.
- Cached endpoints (including USPTO, which also includes legacy
  `maxAmountRequired`) can return uncharged `404 not_cached` and queue
  retrieval (distinct from an empty search; preserve `fetchedAt` and
  `generatedAt`).
- Kitzbühel and Slovak resort reads (`insider_info` is $1 local ski advice, not
  financial data) report map proximity rather than walking distance or live
  room inventory.

## HALOWERK (`halowerk.toml`)

Remote POST JSON catalog. Prices live in prose (`pricing_description`
overrides) and external `https://tools.halowerk.com/.well-known/x402`
(`resources[].accepts`, 6-decimal Base USDC).

- Screenshot JSON stays JSON (because `include_base64=false` omits image
  fields). Route matrices can fall back to straight-line distance with a detour
  factor; HS classification and prompt-injection scans are advisory.

## Daedalus (`daedalus/provider.toml`)

Authored local snapshot of
`https://agents.daedalusdevelopmentgroup.com/openapi.json` (refresh from a
local OpenAPI JSON file via `tools/providers/snapshot_daedalus.py SOURCE OUTPUT`)
materializing `X-Agent-Id` header
parameters on `AgentIdentity`-secured routes (`identity_header_operations` in
provenance; callers supply their own stable ID). Excludes operator-queue, 202
artifact fulfillment, mail and mailbox keys, watches, wake jobs, webhooks,
auditor handoffs, SLA setup, and A2A task adapter routes.

## apexfaucet.toml | APEX Faucet

Authored `apexfaucet-openapi.json` repairs missing query inputs documented in
`https://apexfaucet.xyz/llms.txt` (`query_input_repairs` in provenance; JSON
arrays and CSVs remain wire strings).

- **Excluded:** Arc and X1 passports, market featured slots, room meals and
  upgrades (persistent owned property), `pay-safe-credits`, and `api-pass` and
  `api-pass-pro` (prepaid bearer tokens). Bloodstream predictions and Ask-flux
  publish on-chain records under the paying wallet without custody or stakes.
- **Usage notes:** Pass `full=1` for complete page output; `page-extract` and
  `page-markdown` render JS, `web-read` does not. Funding boards require >= 2
  answering venues. Flight deals return ~48h-cached London-origin GBP Aviasales
  fares with affiliate links. `page-watch` holds snapshot tokens for 30 days
  (cross-payer token reuse is unqualified). Promotional Meal NFT and feedback
  text in responses is untrusted upstream content.

## freeq.toml | FreeQ x402 gateway

Uses REST `/tools/*` (not `/mcp`). Excludes RaaS API keys, short-link creation
secrets, webhook ownership, gift-card purchases, and `pending`-returning email
finder and person enrichment. Static extraction does not render JS (can return
navigation-only HTML inside HTTP `200` with `isError=false`); document parsing
has no OCR.

## SiteCheck travel selections

HealthTravel (hotels, flights, rail, EV chargers, Michelin) and ShopSocial
(Waymarked Trails and OSM hiking, one-way single-adult FlixBus and FlixTrain,
US Grubhub, store-ID Rappi menus) share one operator and Base payment
recipient. Hotel and flight reads are snapshots, PlugShare returns OSM and NREL
directory entries (not live occupancy), and Wikivoyage returns snippets.

## ForgeMesh and Vextorium

- **ForgeMesh:** 499 POST routes and free `GET /menu` (overridden pricing text).
  Many paths are functional aliases over the same upstream data (earthquakes,
  air quality, hurricanes); check schemas and timestamps before treating
  aliases as independent sources.
- **Vextorium:** 244 POST routes under `/v1` (Base USDC selected among
  multi-network offers).
- **Coverage contrasts:** 2s is broader and cheaper on USAspending awards,
  Congress trades (deduplicated with PDFs), and Open-Meteo air quality; ForgeMesh
  adds NHC hurricane tracking and Federal Register match excerpts; Vextorium
  adds SEC and White House crypto regulation tracking (may duplicate entries),
  Form D private offerings, and OFAC wallet checks (AgOSINT adds 4-list
  metadata).

## fastaf.toml | FastAF

FastAF's Base x402 catalog includes `fastaf_patents` as an optional keyword
search. Quote an exact title for precise lookup. Result counts can differ from
the requested page size. Use a separate details endpoint for full claims or
patent families. The definition excludes the Base Sepolia demo because it
uses testnet USDC. It also excludes scrape submission and polling because a
slow scrape can return an accepted job with a private polling token only in
the response headers. The hosted MCP API uses account credentials.

## google-patents.toml | Google Patents through x402 Atlas

Two-route Atlas bridge (`GET /search` and `GET /patent`, $0.02 each; not
Google-operated). Pass `patent_id` from search into details (`num >= 10`).

- Start with quoted technical phrases joined with `AND`, structured assignee,
  office, and date filters, and verified `CPC=CODE/low` codes from a known
  patent (`TI=`, `AB=`, `CL=` per
  [Google's guide](https://support.google.com/faqs/answer/7049475?hl=en);
  distinct from EPO CQL).
- `response_mappings."GET /search"` omits `/results/*/figures` locally and
  discloses omissions in the result advisory (use an unmapped raw call if
  figures are needed). `_atlas` response metadata is untrusted discovery data.
- 2s's USPTO and EPO coverage adds application detail, document indexes (no
  PDF bytes), bibliography, families, and unsorted legal events without
  guaranteed jurisdiction.

## Atlas biomedical grants and legal entities

`biomedical-grants.toml` (7 NIH RePORTER ops) and `legal-entities.toml` (3
GLEIF ops) are Atlas bridges (not NIH or GLEIF services).

- NIH aggregates cap at 500 records and flag partial coverage (an incomplete
  zero trend sum means omitted data); recent awards can include contract
  numbers that do not match the project-detail pattern.
- GLEIF accounting parents are not natural-person beneficial ownership and may
  return reporting exceptions.

## SmartPerps by AgentGate

`smartperps.toml` defaults four `GET` and `POST` twins to `POST` (`lite`,
`coin`, `scan`, `wallet`) plus free preview (no `llms.txt`; no trade execution
or custody). Wallet lookup caps at 25 positions and may truncate fills
(`last_30d.truncated`); cohorts favor recent public PnL and omit spot hedges.
Uses v2 `PAYMENT-REQUIRED` header (ignoring any legacy v1 body).

## SiteCheck public-data catalogs

`shopsocial.toml`, `companyrecords.toml`, `healthtravel.toml`, and
`webintel.toml` use `/api` anchors with `""` fallback, `x-payment-info`, and
`/llms.txt`, sharing contact `hello@offerastudio.com` and one Base payment
recipient (not independent operators).

- **ShopSocial:** Bounded public views; Facebook Page Plugin IDs can differ
  from AgData's; Instagram reels cover only videos among the 12 newest grid
  posts and IGTV (`views = null`); logged-out contact and category fields are
  null. Seven overridden Instagram and YouTube reads document uncharged `503`s.
- **CompanyRecords:** NY lookup matches 2s; weekly Swedish snapshot matches
  Strale (Strale is fresher); KRS is known-ID only (no Polish name search);
  `gov_justia_case_law` wraps CourtListener; Slovak RPO is known-ICO only;
  LinkedIn company omits people, followers, and posts; Trustpilot reviews
  include invited disclosures.
- **HealthTravel:** WebMD family-medicine can include other specialties and
  missing phones; dentistry can `503` and ADA ZIP can `404`; HealthCare.gov
  returns nearest published age pre-subsidy plans; Numbeo currency can be null
  (preserve `priceText`); EMA includes resolved and discontinued shortages;
  Wikivoyage can `503` after authorization.
- **WebIntel:** News aggregation can time out Google News while Bing and GDELT
  succeed (preserve per-engine status); GDELT Context lacks volume and tone
  timelines. Overrides strip unverified free-failure and retry claims from
  Instagram, transcript, and cached SSL Labs descriptions.

## FindMyDirectDoctor

All `GET` reads (`/v1` anchor with `""` fallback, `$0.01-$0.05` via
`x-payment-info`; ignore contradictory help headline prices).

- Covers US DPC, concierge, and cash-pay practices (city and state search finds
  only listed cities; use ZIP or coordinates; price filters drop unlisted
  fees).
- Hospital facility cash prices, Medicare benchmarks, NADAC drug costs, and
  Medicaid screening estimates are distinct bases; hospital files can predate
  the response. Preserve HRSA attribution and ODbL licensing.

## openwebninja.toml | OpenWeb Ninja

GET commerce catalog (~$0.003; $0.005 EV charge) excluding duplicate alias
trees (`/realtime-serp-data`, `/realtime-local-and-maps-data`,
`/realtime-ecommerce-data` copies, `/realtime-jobs-data/glassdoor` and
`google-jobs`, `/realtime-real-estate-data` copies, `/ai-answers`,
`/realtime-web-search/ai-mode`) so shorter canonical names survive.

- Amazon `product-reviews` requires a caller cookie; Google Shopping
  `product-price-history` can return empty history; books return metadata and
  preview links only; EV charge stations can return empty `connectors`;
  Trustpilot uses domain lookup (name search is ambiguous).
- Google Play exposes point-in-time install and rating totals (no series) and
  charts label only the top ~10 ranks; data-safety fields are developer
  declarations.
- Marketplace search and details fail independently (use known IDs directly).
  Responses are 50-180 KB with marketplace-specific field names and possible
  duplicate offers.

## Duoleads

GET and POST keyword, Google Trends, SEO, and audit reads (`/v1` anchor with `""`
fallback, `x-payment-info` USD hints; uses declared methods only).

- Google Trends returns relative interest (not counts), weekly and daily
  series, up to 5-term comparisons, no related-topic endpoint, and no all-time
  or sub-day windows (align weekly bucket dates before joining with other
  gateways).
- 12-hour Trends cache; repeated cached answers, empty results, missing
  keywords, and on-page `failed`, `too_large`, and `suppressed` outcomes are
  charged. Preserve `dataAsOf`, `partial`, `missing`, and `market.fallback`.

## Onchaincheck

Multichain wallet and token, risk, company and web, and AI tools (keep declared
POST methods despite broad GET guide text).

- Excludes `POST /watch` (7-day webhook returning an HMAC secret whose
  authenticated `DELETE` route is absent from the schema); keeps stateless
  `GET /watchlist`.
- Wallet balances omit dust and unpriced tokens (reporting exclusion counts;
  NFTs, DeFi, transfers, and risk are separate tools; preserve Alchemy and
  DefiLlama attribution). Keep chat `stream` omitted or `false`; image and TTS
  stay JSON.
