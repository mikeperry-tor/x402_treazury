# Provider behavior and caveats

Use these notes alongside [provider definitions](README.md). Prices and vendor
behavior describe the committed configuration and fixtures; live challenges remain
authoritative. Captured request contracts and unsigned challenges establish local regression
behavior, not current upstream availability or funded qualification. Public primary
schema/guide URLs are recorded in each definition and the notes below. Where fresh
documentation cannot be reached, retain the capture basis and explicit uncertainty.

## Compatibility and editorial selection

The [provider policy](README.md#provider-policy) governs additions. Existing compact
allowlists and tag selections are being normalized; the table distinguishes actual
support boundaries from editorial defaults without claiming that pending selection
features already run. Path `include` values also anchor names and must not be
removed blindly. Price, a diagnostic label or a hosted asset link alone is not a
compatibility blocker.

| Definition(s) | Boundary or integration contract |
| --- | --- |
| AgentFund, Brazilayer, Deepline, PDL | Gateway/path name anchors and request guidance; no identified editorial operation restriction. |
| botsmith, Google Trends | Ordinary catalog; probes or structured price/offer metadata have separate contracts. |
| AgOSINT | `/mpp` is the unsupported payment rail; `/x402` anchors names while public discovery remains visible. Root base URL repairs upstream `/v1`. |
| Arkham | Three discovery routes demand SIWX; ordinary paid routes remain usable. |
| Datakoot, IPIntel | Supported catalogs are open to new routes; request guidance and output interpretation remain provider-specific. |
| GenuineGood | Bearer-token pass purchase unusable here; ordinary grant reads include expensive calls subject to payment caps. |
| Kronos | Path name anchor; disabled probes reflect observed discovery slowness, not a tool gate. |
| LoneStarOracle, x402stock | Public previews, feeds and overlapping data remain available; compact research advice requires explicit opt-in. |
| TeleSINT | `/watch` owns persistent wallet state and remains excluded. Public status and new stateless operations remain visible. |
| Agent402 | Preserve wallet-owned and actual execution boundaries. JSON/base64 media, PNG/JPEG modes, planning and human onramp links are usable; raw speech and multipart transcription are unsupported. Compact tags/inventory are editorial. |
| BlockRun | Restrict async jobs, hybrid/async result retrieval, wallet-owned lifecycle and fund submission. Valid fixed-symbol paths are usable literally; zero placeholders retain an unverified intended-target limitation. |
| Claw402 | Market-read subset is editorial. Sparse RootData schemas allow JSON body passthrough; see contracts below. Image result envelope remains unverified. |
| Exa | Exact Search/Contents payable island remains justified in the surrounding authenticated API. |
| OneShot | Retain synchronous search and eleven verified public discovery GETs; authenticated jobs/account workflows remain unsupported. |
| HireScrape | Exclude specific SIWX skills/runs, snapshots, reports and scraper-request workflows. `/api/tools` is a naming anchor, not a blanket outside-path incompatibility. Advertised public marketplace schemas are absent; do not invent routes. |
| NetIntel | Eighteen reviewed same-path pairs have matching structural inputs, results and payment metadata; POST avoids URL secrets/array serialization. Full paths retain compatible gateway aliases; no method opt-out is indicated. |
| Otto | Execution tag and full-auto router exceed ordinary read authority; paying-wallet state and hosted links retain their documented meaning. |
| RegimeShift | Unsigned intent JSON is distinct from signing/transferring collateral. Diagnostics outside name anchor are editorial; owner Safe/Zodiac provisioning is excluded. |
| RelayShield | Exclude metered/account workflows, preserving payg and three keyless public checks outside that namespace. File-result schema is absent from OpenAPI although root discovery advertises it; URL scan retains immediate heuristics. |
| SocialFetch | Exclude account, monitor ownership and webhook lifecycle. Public `/v1/monitors/sources`, diagnostics and ordinary ask are compatible; exact parent-operation exclusions preserve the public child. |
| StableEnrich | Crawl/job, Hunter verifier lifecycle and aerial-video rendering/result families are unsupported; synchronous/image contracts remain. |
| Vaaya | Exclude specific MPP-only, upload/async, persistent account, message/purchase and payment-forwarding contracts. Broad research subset is editorial; complete result chains matter. |
| x402-list | Broader public GET catalog is compatible; ownership/reissue and submit conversion loss have explicit boundaries; assessment and suggestions remain available. Embedded directory is a separate intentional three-read consumer contract. |
| AgentUtility | Preserve enrichment/alias provenance. One-shot browser, IPFS and QR return ordinary JSON, including encoded media. Persistent watch ownership and six full oneOf conversion losses require restrictions; optional formats do not justify whole-family suppression. |
| Concordance | Preserve authored price/ref enrichment; free catalog discovery completes series lookup and was omitted only for compactness. |
| Curl | Authored request/response relay contract; single-operation list is redundant. |
| Glassnode | Authored metadata-to-metric workflow; deeper bulk paths cannot be addressed by the current gateway template. |
| Locus | Authored list/call gateway covers all 114 free REST duals; two public unified discovery manifests are separate compatible contracts. Batch/update job lifecycle remains unsupported. |
| News Gurus | Bazaar projection preserves real templates; agent-status omission is editorial. |
| Straits | Authored free/premium contracts, IMO templates and CSV/JSON outputs; preserve naming anchors. |
| RelayShield phone companion | Authored vendor-omitted endpoint, with separate payment/version and unknown-result limitations. |

### Same-path method contracts

GenuineGood's five grant pairs and Otto's generate-meme, llm-research,
tx-explainer and meta-intelligence pairs have equivalent captured inputs,
requireds, result envelopes and advertised payment metadata. POST carries JSON;
GET array forms document comma separation that generic repeated-key query routing
does not supply. These pairs support default POST selection before naming.
Captured metadata does not prove equal actual charges. No reviewed pair requires
a preference opt-out. Other captured twins belong to excluded state/account
families. NetIntel's complete retrieved primary schema has eighteen same-path
GET/POST pairs with matching structural inputs/requireds, response declarations,
security and payment metadata. Secret-bearing GET descriptions explicitly favor
POST; JSON bodies also avoid query-array serialization uncertainty. No opt-out
is indicated, and equal advertised metadata does not establish equal charges.

The canonical names are `genuinegood_search`, `genuinegood_detail`,
`genuinegood_fit`, `genuinegood_brief`, `genuinegood_preflight`,
`otto_generate_meme`, `otto_llm_research`, `otto_tx_explainer` and
`otto_meta_intelligence`. Both old method suffixes are replaced in privacy selections, the live-Tor example,
reviewed qualification preset and independent request fixture. Operators must update
exact selectors; set `prefer_post = false` in a provider/source to inspect both methods.
Do not create aliases or alter explicit raw HTTP methods.


## Reliability annotations

The provider definitions contain `reliability_tags` and `reliability_note`
observations. [Warning semantics and tag meanings](../docs/configuration.md#provider-reliability-observations)
are separate from OpenAPI tool filtering.

| Provider | Observed concern |
| --- | --- |
| AgentFund | Repeated upstream timeout in the treasury yield-curve response. |
| AgentUtility | Repeated upstream rate limit in web search. |
| Arkham | One paid response-body loss; subsequent responses completed. |
| OneShot | Repeated catalog HTTP 403; help remained reachable. |
| Kronos | Slow unsigned pricing discovery; separate intermittent catalog/help connection timeouts. |
| RegimeShift | Intermittent catalog/help timeouts and partial upstream data. |
| Concordance | One query/result-metadata relevance mismatch. |

These are stage-specific compatibility concerns, not current health guarantees or
evidence that Tor caused a failure. Startup warnings describe the concern without
changing selection or permitting retries. See [qualification limits](../docs/testing.md#qualification-status).

## Help and compatibility

Lazy help is configured for 21 providers. New guides cover botsmith (free JSON
catalog), Brazilayer, Kronos, Otto (full manifest), RegimeShift (gateway quickstart),
SocialFetch and x402-list. AgentFund, PDL and Deepline have no verified standalone
text guide at their obvious URLs; retain their existing embedded/authored guidance.
All help remains lazy and process-cached. Positive listener tag filters remove
untagged help; explicitly include help in tool-name selections where desired.

Media and async constraints are described below and in the artifact/polling plans. Top-level extension stripping is
not universal managed compatibility. Agent402's `outputSchema` is accepted as
opaque object/boolean metadata and preserved without evaluation or reference
fetching. Exa Search/Contents pricing and offer labels, and all four Google
Trends routes' merchant/tier labels, are accepted with type checks and preserved
in the outgoing offer. Atomic amounts and signing mechanisms remain strictly
validated. Local compatibility fixtures do not prove live settlement.

## socialfetch.toml — SocialFetch

Uses the vendor's platform tags; all platforms are included by default.
Account identity, monitor ownership and webhook lifecycle are excluded. Exact
GET/POST parent exclusions preserve the public stateless
`GET /v1/monitors/sources` child. Diagnostics and ordinary ask remain available.
`--tags Twitter,YouTube` selects a subset. `LinkedIn` spans v1 and v2; add
`--include /v1` when only v1 is wanted. Tags are exact and case-sensitive.

`include = ["/v1", ""]` strips `/v1` from tool names while the empty root fallback
retains other API versions: `socialfetch_twitter_profiles_handle` and
`socialfetch_v2_linkedin_*`. Original request paths remain intact; keep the base
URL at `https://api.socialfetch.dev`.

Credit prose comes from `x-socialfetch-credits-pricing`. The provider's
`credit_pricing` setting converts structured `x-socialfetch-pricing` metadata
into per-tool USDC estimates, including metered costs and maximums, while retaining
that prose. This includes Instagram v2 `include` surcharges and the recognized
`always` / `fixed_price_only` exact-payment hints; those hints do not select payment
behavior. The configured 0.014 USDC/credit is a historical estimate, not a verified
current x402 tariff; the payment challenge remains authoritative. Instruction
overrides and platform filters retain estimates. Pricing probes are disabled;
no endpoint sweep is needed. Fetching the live spec still requires one startup
request. Use a 90-second timeout for slow search/transcript calls.

`tests/fixtures/socialfetch_openapi.json` pins the 2026-10-02 source: 259
operations. It retains tags, descriptions,
request schemas and vendor extensions, omitting response documentation only.
Regenerate it with the Rust development utility (replace the URL with a local
spec path for offline operation):

```sh
cargo run --locked --manifest-path Cargo.toml --example snapshot_spec -- \
  https://www.socialfetch.dev/openapi.json tests/fixtures/socialfetch_openapi.json
```

## deepline.toml — Deepline GTM (stable-deepline.dev)

GTM contact lookup: 11 POST tools mirroring the vendor's OpenAPI spec —
prices via `x-payment-info`, requireds, the ads `platform`/`media_type`
enums, waterfall-provider detail in summaries. Tool schemas are closed via
`additional_properties = false`. Waterfall endpoints are slow — run with
`--timeout 120`.

```bash
x402_treazury serve --provider providers/deepline.toml --timeout 120
```

## pdl.toml — People Data Labs (stablepeopledata.dev)

Person/company enrich + search (4 POST tools). The spec carries prices
(dynamic `min`/`max` for the size-scaled search endpoints) and full param
docs; `overrides` restores the two semantic notes the spec lacks — "free on
no match" on the enrich tools and "free when the result set is empty /
recommended size 1-5" on the search tools — plus the curated `min_likelihood`
and `size` hints. Tool schemas are closed via `additional_properties = false`.

## botsmith.toml — x402.botsmith.dev

Market-signal API for trading agents (X/Twitter intel, Polymarket, perp
funding/OI, stablecoin pegs). All routes are flat-price GETs ($0.015/call at
the time of writing), so no `pricing_key` is configured: the server probes
each route once at boot with an unpaid GET (expect `402` + challenge, $0
spent) and renders the live price into every tool description, cached for the
process lifetime.

## x402stock.toml — x402stock.xyz

Market, economic, SEC and congressional-trade data use the direct
`https://x402stock.xyz` x402/USDC-on-Base host; the alternate
`agents.x402stock.xyz` server is an MPP/Tempo pay host. `/api/v1` shortens
existing names, with an empty fallback for other paths. Public macro, filings,
perps, snapshots, market-status and holiday routes are available rather than
hidden for overlap or compactness. Feedback is not excluded merely because it
is a write; no wallet-owned or unsupported-auth contract is established for it.

`recommended_tools` suggests symbol discovery, company overview/disclosures and
the FOMC calendar, plus help. This advice does not limit the default catalog or
raise payment caps. Prices come from vendor metadata or eligible unsigned probes;
restored routes without embedded pricing can now participate in that existing
probe policy. Vendor `llms.txt` help remains lazy. Source instructions preserve
path placeholders and the `{ticker, data, source, as_of}` result convention.
Only a selected calendar request schema is retained for offline qualification;
the current complete spec/guide could not be retrieved during this audit. Neither
that schema nor the configured URL establishes current upstream availability.

## google-trends.toml — Google Trends SEO keyword data (x402 Atlas)

Four GET endpoints on [google-trends.use.x402atlas.com](https://google-trends.use.x402atlas.com):
interest over time ($0.05/call, worldwide), interest by region, related
queries and related topics ($0.03/call each, per-country). Prices surface
from the `x-payment-info` extension, so boots make zero probe requests.
Guidance: long vendor descriptions describe each route's limits (empty
result sets, established search terms) — never re-enable trimming for this
spec; `help_url` serves the vendor `llms.txt` via `google_trends_help()`.

## arkham.toml — Arkham Intel x402 rail (api.arkm.com/x402)

Blockchain-intelligence API (address/entity intelligence, balances, flows,
HyperCore): 89 paid POST endpoints exposed (parameters go in the JSON body).
Every paid operation carries an `x-payment-info` block, which `pricing_key`
surfaces into tool descriptions — prices mirror the classic credit weights at
**$0.20 per credit**, so no probing happens (these are POST routes anyway,
and the probe is GET-only by design). Most operation descriptions already
state the price in prose (`… $0.20 per call.`), so the rendered extension
line is dropped there and the price appears exactly once; the three
dynamic-priced routes keep both texts because their prose lacks the max.
Guidance: `help_url` points at the vendor `llms.txt` (rate limits, error
handling, per-row pricing) served as `arkham_help()`; `instructions_text`
summarizes body-params + credit pricing + rate limits.

- Three **dynamic-priced** endpoints (`/swaps`, `/transfers`,
  `/transfers/unenriched`) publish min/max bounds in `x-payment-info`
  (e.g. `$0.40–$660`); the default `X402_MAX_PRICE_USD=1.00` cap rejects
  the expensive end pre-sign. Pass a small `limit`.
- Three free discovery endpoints (`/chains`, `/networks/status`,
  `/arkm/circulating`) answer `402` with a Sign-In-With-X wallet-signature
  challenge instead of a payable x402 quote — the provider configuration **excludes**
  them, since this server cannot answer that challenge.
- The API host sits behind Cloudflare and 403s some non-browser clients
  (notably curl from datacenter IPs); the server's spec fetch has worked
  reliably, but if a boot fails with an HTML/403 spec error, snapshot the
  catalog from a browser and point `spec` at the local file.

## regimeshift.toml — RegimeShift (regimeshift.xyz)

Agent-SOFR (decentralized USD short-rate benchmark), variance-aware max-LTV
and ETH/BTC volatility risk premium at **$0.001/call** each (`x-payment-info`
in the spec), plus a **free** on-chain RFQ clearinghouse: lend/borrow intents,
open intents, matches with EIP-712 signed quotes, active/liquidatable loans,
loan registry. The spec's `servers` block is relative, so the conf pins
`base_url` to `https://regimeshift.xyz/api`; free endpoints carry overrides
documenting that they cost nothing (the server would otherwise render its
default "paid per call" line). The `/v1/intent/{id}/match` tool long-polls up
to 300s — run with `--timeout 320`.

```bash
x402_treazury serve --provider providers/regimeshift.toml --timeout 320
```

## kronos.toml — Kronos Crypto Data (kronossignals.com)

Crypto market data for trading agents: 36 GET tools under `/api/v1` — per-asset
signals/price/ohlc/volatility/liquidation maps (`{asset}` paths, 16 spot
assets + HYPE derivatives-only), market-wide overview/scan/funding extremes,
BTC/ETH options (GEX, IV surface, implied probabilities), ML forecasts
($0.05) with a **free** `kronos_track_record` accuracy audit, macro context,
and the $0.10 all-in-one `briefing`. The `/api/v1` anchor keeps existing tool names short. Prices
live in vendor prose (the spec's `x-x402` extension has the wrong shape for
`pricing_key`). Startup probes are disabled (`probe_pricing = false`) because
pricing discovery was repeatedly slow; a deployment source may explicitly
re-enable them. Vendor prose and the free `kronos_track_record` override remain.
Actual paid calls still use the live challenge and enforce spend caps.

The `/api/v1` anchor preserves existing names; an empty fallback exposes the
public `/api/health`, `/api/stats`, `/api/methodology` and discovery manifest.
These are not authentication boundaries. The saved full input is also an
explicit runtime source in the live-Tor example, so it is retained independently
of regression pruning. A cached primary OpenAPI document supported this public-route distinction;
it is not a current health check.

## concordance.toml — Concordance (concordancehq.duckdns.org)

Cross-source research aggregator: 18 refreshed-on-schedule free and keyed
sources (SEC EDGAR, FRED, World Bank, UN Comtrade, GDELT news, CourtListener,
govinfo, arXiv/CORE, Hacker News, Stack Exchange, Wikipedia, weather,
DefiLlama, finnhub/twelvedata/polygon quotes) surfaced as six paid POST tools
($0.01–$0.05/call, USDC on Base or Solana). The vendor publishes both an
OpenAPI spec and an `llms.txt`; the spec's request bodies are FastAPI `$ref`s
(these deref fine since the flatteners learned local refs) but the spec
carries **no pricing at all** and POSTs are never probed — so the conf points
at the curated `providers/concordance/openapi.json`, with references inlined
and prices embedded from vendor documentation and challenges. The original vendor
spec is retained in `tests/fixtures/concordance_openapi.json`; the curated tool
contract is pinned by `tests/provider_catalogs.rs`. Boot
makes zero probe requests (all paid routes are POST, and the probe is
GET-only by design). The authored projection preserves the complete captured public GET objects for
`/`, `/catalog` and `/healthz`, adding explicitly authored zero-price hints.
`include = ["/tools", ""]` preserves paid names and adds `concordance_root`,
`concordance_catalog` and `concordance_healthz` without changing wire paths.
Catalog discovery completes the source/series lookup workflow. The existing
native `concordance_help` fetches the exact `/llms.txt` document, faithfully
representing that fourth public resource without a duplicate HTTP tool.
Captured source comparisons preserve the six paid request contracts and local
reference closure. These are offline schema facts, not current availability or
funded completion claims.

## agentfund.toml — AgentFund US Economic, SEC & On-Chain Data (x402.agentfund.net)

The cleanest spec in this directory: 21 POST tools under `/x402` — US macro
(Treasury curve with computed 2s10s/3m10y spreads, BLS CPI, jobs, PCE, GDP,
retail sales, housing starts, EIA energy, release calendar), SEC EDGAR
(insider Form 4, XBRL financials, 13F holdings, filings feed, full-text
search), on-chain EVM reads (token balances, portfolios, cross-chain
balances, Chainlink oracle prices, gas), plus deterministic
`structured_json_repair` and `tabular_to_json` for messy LLM/tool output.
Every request body is inline (no `$ref`), the spec carries an absolute
`servers` URL, and every op publishes a correct-shape `x-payment-info` price
($0.001–$0.03/call, USDC on Base; failed upstream calls are never charged) —
so the conf points at the **live vendor spec** and needs no base_url pin, no
local catalog, no overrides, and makes zero probe requests. No vendor `llms.txt`;
the spec's top-level `x-guidance` is condensed into `instructions_text`
instead. (The vendor also serves these tools natively over MCP at `/mcp`;
this conf exists so the wallet/spend-cap story stays in this server.)

## otto.toml — Otto AI x402 swarm (x402.ottoai.services)

The default collapses four interchangeable same-path GET/POST pairs to POST before
naming. Otto is an
autonomous agent swarm selling market/token intelligence, DeFi & markets
data, web/domain intelligence, real-world data (FX, weather), AI creative
tools (image/video gen, research, tx explainer), portfolio reads, a
meta-intelligence router ($0.001) and a feedback/refund route (needs the
original payment tx hash; vendor-capped at $0.01). Every op carries a
correct-shape `x-payment-info` (fixed and dynamic min/max entries), so
prices render from the **live vendor spec** with zero probes and no pricing
overrides; where the vendor prose already states its own price the rendered
line dedupes away (31 tools at time of writing) — every tool still shows a
price exactly once. Absolute `servers` URL, inline POST bodies. Lazy help reads
`https://x402.ottoai.services/llms-full.txt`, the consolidated guide rather than its
link index. It includes excluded execution routes and SIWX re-access that Treazury
does not implement; the help description makes those boundaries explicit. Caveats:

- **Fund-moving routes are excluded on purpose**: the conf drops the whole
  `Execution` OpenAPI tag (`exclude_tags`) — `/swap`, `/bridge`,
  `/deposit`, `/withdraw` — plus the `/full-auto` spending router by path.
  Either would let an agent move or commit the server wallet's funds;
  re-add them only deliberately. Category subsets can be mounted per server
  with `--tags` (eight tags, `catalog tags` inventories them).
- `/video-gen` is dynamically priced **$0.46–$4.60/call**; the default
  `X402_MAX_PRICE_USD=1.00` cap rejects the expensive end pre-sign.
- 68 of 88 services also quote Solana; the live 402 `accepts[]` is
  authoritative and Base-USDC exists on every route. Permit2/`upto` accepts
  need the one-time USDC allowance documented in the repo README.

## brazilayer.toml — Brazilayer registry & market data (api.brazilayer.com)

Official Brazilian registry and market feeds: Receita Federal CNPJ registry
(registration, partners, sanctions/integrity screening, Central Bank and CVM
checks), PNCP public tenders, EUDR deforestation profiles, and
commodity/macro/news datasets — **53 GET tools**; response keys use Portuguese
legal terms (`razao_social`, `situacao_cadastral`, …). Every paid route states
its price in prose ($0.001–$0.10); the ten routes sharing one
`components/parameters` `$ref` for `{cnpj}` get that path argument through the
catalog builder's op-level `$ref` parameter deref. The sixteen free routes
(`/v1/health`, CNPJ/CPF validators, twelve `*_amostra` dataset samples) carry
free-semantics overrides; `overrides` also restores the price the vendor's
long integridade description drops and completes two `nome` param descriptions
the vendor spec truncates with malformed keys. Pinned against
`tests/fixtures/brazilayer_openapi.json`.

## genuinegood.toml — Genuine Good Grants (genuinegood.online)

Independent paid US federal grant discovery over Grants.gov public records:
five data routes — search, detail, fit, brief, preflight — each shipping
GET+POST twins that share a path. Default POST selection exposes five API tools
plus `genuinegood_help()` without redundant method suffixes. Every op publishes a correct-shape
`x-payment-info` ($0.05 / $0.08 / $0.20 / $0.50, preflight $5.00 per call,
USDC on Base), so prices render from the **live vendor spec** and boots make
zero probe requests; the vendor prose states no dollar amounts, so the
rendered price line survives the dedupe and each tool shows its price exactly
once. The `/v1/grants` naming anchor preserves existing names, with a root fallback
for other supported paths. Search/detail/help recommendations are advisory;
fit, brief and preflight remain available by default. POST bodies are inline with real constraints and the schemas are
closed via `additional_properties = false`. Caveats:

- `/v1/grants/pass` ($15 non-renewing 30-day pass) is **excluded on
  purpose**: the pass buys bearer-token (`Authorization` header) access,
  which this server cannot attach to vendor calls (headers are not
  agent-settable; x402 replaces auth), so activating it would be $15 for a
  token no tool can use.
- preflight ($5.00) exceeds the default `X402_MAX_PRICE_USD=1.00` spend cap
  and is rejected pre-sign at the default — raise the cap to use it.
- The vendor guidance prefers the POST forms; the GET twins accept the same
  input as query parameters, but the server sends array query values as
  repeated keys while the vendor documents comma-separated values — treat
  the POST twins as canonical.
- No llms.txt overflow to worry about: `help_url` serves the vendor's compact
  `llms.txt` (per-route prices, payment contract, free preflight sample) as
  `genuinegood_help()`, and `instructions_text` condenses the spec's
  top-level `x-guidance` — noting that the 402/sign/retry dance the vendor
  describes for raw HTTP clients is automatic here, plus the research-aid
  disclaimer. Pinned against `tests/fixtures/genuinegood_openapi.json`.

## locus.toml — Locus US property & public-records context (api.locus.report)

The curated `providers/locus/openapi.json` exposes **108 paid POST tools**, a
two-tool free gateway, two public discovery manifests and a help tool. Prices are
embedded from the spec's `x-payment-info.price` blocks ($0.01–$2.49 per call),
so boots make zero probe requests (`probe_pricing` is false), plus a **two-tool
free gateway** mirroring the vendor's own remote-MCP shape: `locus_list`
(catalog of the 114 free tools with `compact`/`category` query knobs) and
`locus_call` (wrapped `{name, arguments}` body; the `name` enum pins the 114
valid tools, and the spec's oneOf-of-114-wrappers body — which the flattener
cannot expose — is rewritten into that flat form). Design notes:

- The spec's `/api/locus_*` underscore routes are free REST duals of the same
  tools, but their slugs collide with the paid `/api/locus-*` hyphen twins
  under path-derived naming, so the free tier rides the gateway instead. All 114
  captured free REST routes occur in the gateway enum: no functionality is lost
  by this representation. The public `/.well-known/locus-tools.json` unified
  free/paid schema directory and `/.well-known/locus-agent-endpoints.json` bundle
  directory are distinct contracts preserved as complete captured objects with
  authored free-price hints. The empty naming fallback preserves their full names;
  `/api` and `/tools` anchors retain existing names. Bidirectional source/gateway
  name equality verifies coverage without discarding actual original schemas.
- The vendor quotes four payment rails; the live 402 puts plain EIP-3009
  USDC-on-Base first in `accepts[]`, which is what our EVM wallet pays (the
  SDK filters the Solana/Tempo/Circle-Gateway entries out).
- Pollable-job routes are excluded on purpose (`place-report-batch` $0.25,
  `record-batch` $0.05, `property-update`): they return `statusUrl`/
  `webhookUrl` jobs. The live spec now documents token-protected
  `GET /api/property-update-jobs/{id}`, but the curated catalog omits it and
  Treazury has no durable job/token lifecycle. Batch polling contracts need separate
  review. See the deferred polling plan. Response headers (X-Locus-Report-*) are
  not surfaced over MCP.
- Satellite `includeImagery=true` returns optional base64 fields without a configured
  image mapping. Use the metadata default until optional extraction is implemented;
  flyer/image URLs remain JSON metadata, with no automatic capture.
- Slow: place-report can take tens of seconds, environmental-context 60–90s.

```bash
x402_treazury serve --provider providers/locus/provider.toml --timeout 120
```

## straits.toml — Straits.live Hormuz monitor (straits.live)

The authored catalog exposes free and premium resources tagged `Free` / `Premium`;
listener tag selection can mount either tier. Original premium GET request objects
retain their digest provenance. Additional free contracts come from the vendor's
[human API documentation](https://straits.live/api), rather than the premium-only
OpenAPI document. Known free additions carry explicit zero-price hints; no endpoint
response or current service health was verified.

Free resources include the canonical `/status` bundle, AIS vessel counts, public
CSV downloads, RSS feeds, the HTML data catalogue and embed widget. CSV columns
and undocumented JSON response fields are not inferred. HTML is ordinary text,
without browser execution. `/report` renders the same daily-brief prose already
represented by `/api/v1/briefs`. The retired Hormuz and Cape snapshot feeds remain
available as historical data frozen on 2026-08-28; their retirement markers must
not be interpreted as live observations. Respect published per-format quotas and
upstream timestamps/staleness indicators.

With both tiers enabled, ordinary collision naming produces `straits_vessels_get`
for the paid manifest and `straits_vessels_get_2` for free counts. A single-tier
catalog names its selected operation `straits_vessels`. The free count request
supports optional `flagged=true` and `region=hormuz|cape`; Cape returns null counts
because it is not measured. Other existing names and requests remain unchanged.

Session creation and the token-authenticated premium SSE stream remain outside
this catalog, as do subscription-secret webhook lifecycle operations requiring a
callback receiver. Their ownership/lifecycle contracts are distinct from ordinary
GET resources. Configured lazy help fetches the vendor's `llms.txt`; positive tier
listener tag filters omit untagged help; source/CLI tag selection retains configured
help. The existing client-side challenge-description
normalization remains necessary for premium routes with long descriptions.

## lonestar.toml — LoneStarOracle catalog (lonestaroracle.xyz)

The unified [OpenAPI](https://lonestaroracle.xyz/openapi.json) uses the
`https://lonestaroracle.xyz/api` gateway. Original service paths stay on the wire;
`x-direct-url` is informational. The captured gateway metadata classifies prices
and free hints, while live challenges remain authoritative. Authored overrides
and local-reference request conversion remain unchanged.

Public demos, previews, feed pages, refinery data, lease-auction/portfolio reads
and audit-target listings are included. Their former omission was editorial;
no subscription-management or wallet-owned contract appears in those captured
requests. Recommendations suggest macro/rates, news and token research plus help,
without hiding other supported tools. The guide remains lazy. Current public
schema/help retrieval was unavailable during this audit; saved request contracts
and historical unsigned observations do not qualify present service behavior.

## glassnode/ — Glassnode x402 gateway (x402.glassnode.com)

Glassnode publishes no OpenAPI spec; this pair works around it:

- `providers/glassnode/openapi.json` — a small **hand-authored OpenAPI catalog** of the four
  gateway routes (the gateway mirrors `api.glassnode.com` paths one-to-one):
  one templated metrics tool (`/v1/metrics/{category}/{metric}`, **$0.05**)
  plus the metadata discovery trio (assets / metrics / metric, **$0.01**
  each). Prices are embedded as `x-payment-info` blocks, so boots make zero probe
  requests.
- `providers/glassnode/provider.toml` — the configuration: pins `base_url` + `prefix`, carries the
  discover→metadata→fetch workflow in `instructions_text`, and serves the
  vendor's x402 doc page via `glassnode_help()` (`help_url`).

The agent flow is: list metric paths → fetch one metric's allowed
parameters/interval values → pull data. Metric paths are two-segment
(category/metric); deeper sub-path variants (e.g. `/bulk`) are not addressable
through the templated tool.

## exa.toml — Exa Search and Contents

The [vendor spec](https://api.exa.ai/openapi.json) contains 64 operations in the
2026-10-03 fixture. Only `POST /search` and `POST /contents` support x402 according
to Exa's [unified x402 guide](https://exa.ai/docs/integrations/payments/x402/quickstart.md).
An exact method/path allowlist exposes `exa_search`, `exa_contents` and lazy
`exa_help`. Account/team inspection, monitors, recurring jobs, batches, Agent,
Websets, webhooks/events and deprecated `findSimilar` are omitted. New methods
or child routes cannot expand this catalog implicitly. The selected operations
are untagged, so a positive listener tag filter would hide them (and help).

No API key is needed: Treazury handles Base USDC payment. The spec's global
API-key security declarations also describe the wider account-based API; they
are not a reason to add an API key to these x402 requests. Exa's help page has
generic API-key/SDK instructions above the x402 guide; provider guidance explains
that Treazury users can skip those. The help guide covers both operations without
loading the full multi-product documentation corpus into context.

Vendor `x-payment-info` supplies dynamic price ranges, but these are not total
request caps. Requested search type, result/page count and content options affect
the actual price, calculated upfront. Authored descriptions make the live
challenge authoritative and recommend small batches. Pricing probes are disabled:
both paid operations are POSTs and pricing is request-dependent. One unsigned
request per operation returned HTTP 402 with x402 v2 exact Base USDC options;
the sampled requests quoted 7000 and 1000 atomic units respectively. These were
unpaid discovery checks, not funded settlement qualification or universal prices.
Other networks and gateway variants in the challenges are not enabled by this
provider; the existing payment compatibility policy still selects the supported
Base USDC option.

The vendor's OpenAPI 3.1 nullable unions, nested content options and local schema
references work with the generic converter. Contents uses a root presence-only
`oneOf` requiring exactly one of `ids` or `urls`. The converter now preserves
presence-only `oneOf`/`anyOf`/`allOf` alternatives and renames body keys in them when
query/body arguments collide. This is agent-facing schema information, not a new
runtime JSON-Schema validator; other arbitrary root body predicates are not
claimed to be supported. The vendor still validates requests. Routing tests pin
nested JSON unchanged and the client-visible required alternatives.

Search supports SSE when `stream=true` is combined with structured output.
Instructions and the parameter description tell agents to omit it or use false;
this provider does not impose or silently rewrite request values. Treazury does
not relay a streaming Exa response as incremental MCP output. The 90-second
request timeout allows slower search/crawling; an operator may override it.
All selected request fields and descriptions remain available, including vendor
deprecation guidance, without a description-length cap.

## oneshot.toml — OneShot synchronous web search

The [vendor OpenAPI spec](https://win.oneshotagent.com/openapi.json) supports
synchronous `POST /v1/tools/search` plus eleven verified public discovery GETs:
`/health`, `/.well-known/acp/manifest.json`, `/v1/stats`, `/v1/status`, `/v1/soul`,
`/v1/soul/{wallet}` and `/v1/soul/{browse,search,service-types,stats,templates}`.
These explicitly declare no authentication or payment. An exact support island
preserves these reads while excluding authenticated or asynchronous marketplace
operations. `include = ["/v1/tools", ""]` shortens the existing search name and
retains full path segments for discovery names; HTTP paths stay unchanged.
Search and help are advisory recommendations, not default catalog restrictions.

This is a deliberately partial integration. Most research, enrichment, web-read,
commerce, browser and build operations return HTTP 202 with a job ID. Retrieving
results uses `GET /v1/requests/{id}` with wallet identity; other reads/mutations
also require short-lived EIP-712 ownership proofs. Some submissions require a
stable idempotency key or a submission proof. The generic paid transport does
not implement these OneShot-specific lifecycle/authentication requirements.
Exposing the submit tools alone could charge for work whose result the caller
cannot retrieve, so they are excluded rather than presented as usable defaults.

Adding those tools requires capturing the submitting payer for later polling,
including across managed-wallet rotation, implementing the documented proof and
idempotency requirements, and handling cancellation, job completion, and recovery
without accidentally resubmitting paid work. Agent-controlled identity headers or
polling with whichever wallet is currently active would be incorrect. This needs
a separate integration change rather than a larger TOML allowlist.

The OpenAPI security prose describes `Authorization` plus `X-Quote-ID`, and the
[search guide](https://docs.oneshotagent.com/api-reference/web-search.md) describes
`X-Payment-Proof`. An unsigned live search on 2026-10-03 instead returned a standard
base64 `PAYMENT-REQUIRED` header with x402 v2 `exact`, network `eip155:8453`, Base
USDC, and `amount = "1000"` ($0.001). The JSON body is a different, vendor-specific
payment request envelope; Treazury uses the standard header. No legacy header
translation is added. This verifies challenge compatibility, not funded settlement;
no wallet credentials or payment were sent.

`x-payment-info` here contains authentication flags, not a price object. The
provider therefore uses an authored $0.001 description from the vendor operation
text, with the live challenge and configured payment cap authoritative. Pricing
probes are disabled; the selected paid endpoint is POST. The schema's query-length
and result-count bounds and original JSON field names are preserved without repairs.

The lazy help tool fetches the single search API guide, not the full service index.
Provider/tool guidance explains that its legacy payment-header instructions are
not setup steps for Treazury. The existing process cache and explicit document
limit reporting apply. Positive listener tag filters must include `Search` to
retain search; help has no operation tags and is excluded by positive tag filters.

The deferred [polling-tool plan](../docs/plans/deferred/polling_tool_support.md)
records the OneShot contracts, possible shared job-management design and questions
to compare against future providers before implementation.

## stableenrich.toml — StableEnrich multi-provider data APIs

The [vendor spec](https://stableenrich.dev/openapi.json) exposes synchronous
JSON/image operations across CompanyEnrich, Exa, Firecrawl, FullEnrich, Google
Maps/Solar, Minerva, PDL, Reddit, Serper and Whitepages. Specific lifecycle paths
below are excluded; new compatible routes remain available. `/api` is a naming
anchor with an empty root fallback, preserving existing names and HTTP paths.
The compact research recommendations are advisory.

Use exact upstream tags to reduce context further, for example source `tags =
["Exa", "Firecrawl"]` or `["Google Maps"]`. Tags use the vendor's casing, including
`Fullenrich`, `Companyenrich` and `Pdl`. Source tag selection retains the generated
help tool; a listener's positive tag filter excludes untagged help as usual.

The [unified llms.txt](https://stableenrich.dev/llms.txt) provides useful examples,
pricing, identity-matching cautions and pagination guidance. It also describes
AgentCash's tool names and client-specific `{ success, data }` response wrapper.
Treazury does not use those tools or add that wrapper. Provider guidance tells
agents to interpret actual HTTP response bodies and not assume another `.data`
layer (individual upstream bodies may legitimately contain a `data` field).
Help is fetched only on call and cached for the process lifetime, with configured
size limits reported explicitly. It includes workflows outside the selected catalog;
the instructions/help description name these exclusions rather than modifying or
silently cutting down the vendor document.

Excluded operations:

| Workflow | Reason |
| --- | --- |
| Hunter email-verifier POST and jobs GET | May return a job even when most calls complete immediately; free polls require same-wallet SIWX authentication |
| Cloudflare crawl POST and jobs GET | Returns a signed JWT token; free polling also requires same-wallet SIWX |
| Aerial View render-video and lookup-video | Asynchronous render/paid-poll pair; excluded together pending lifecycle and cumulative-cost support |

The spec lists 200/402 even for operations whose guide documents 202. Hunter's
`jobs/{jobId}` route also omits its path-parameter declaration. These defects affect
excluded routes, so no schema repair or custom transport was added. A 200 response
entry alone must not be treated as evidence that an operation is synchronous.
Solar rgb-image is enabled: its operation summary describes JSON with an image URL,
while PNG/JPEG binary responses are also supported by the typed image transport.
JSON metadata passes through unchanged; returned asset URLs are not automatically
fetched. Solar data-layers returns metadata/signed GeoTIFF URLs, and Serper image
search returns JSON search results. Neither triggers a second asset request.

Embedded `x-payment-info.price` values supply tool pricing; startup probes are
disabled. Some no-match enrichment/search results are documented as unbilled,
and real challenges remain authoritative. FullEnrich requires actual filters;
use `search_after` rather than relying on page/offset alone and verify returned
company identity before further enrichment. Retain these vendor caveats and
schemas rather than inventing stricter filters from divergent prose. Search and
answer streaming parameters are documented as false/omitted; values are not
silently rewritten. A 90-second timeout accommodates slower synchronous requests.

One unsigned `/api/exa/search` request returned standard x402 v2 exact offers,
including Base USDC for 10000 atomic units ($0.01). No wallet or payment was used;
this verifies the sampled challenge, not funded settlement or every upstream.
The [deferred polling plan](../docs/plans/deferred/polling_tool_support.md) records
the new SIWX, JWT-token, mixed synchronous/asynchronous and paid-poll patterns.

## agent402.toml — Agent402.Tools

The [vendor OpenAPI](https://agent402.tools/openapi.json) provides a broad
catalog with lowercase tags. The full supported catalog is available by default;
a compact web-research recommendation list is advisory. Source tags or listener
selection can narrow context deliberately without changing compatibility.

Other vendor groups include `network`, `encoding`, `identifiers`, `conversion`,
`text`, `time`, `date-time`, `validation`, `math`, `research`, `wallet`, `payments`,
`chain`, `ai`, `api`, and `x402`. Tags such as `data` are broad: they include some
local utilities and OCR as well as live public data. `time` and `date-time` are
distinct. Wallet-owned memory is excluded; stateless agent planning, skill packs and
workflow prompts remain available. `catalog tags` reports the unfiltered vendor catalog, so it
also lists tags on excluded operations. Selecting only excluded operations fails
with no matching operations.

```sh
target/debug/x402_treazury catalog tags --provider providers/agent402.toml
target/debug/x402_treazury catalog tools --provider providers/agent402.toml --tags network
target/debug/x402_treazury catalog tools --provider providers/agent402.toml --tags data,crypto
target/debug/x402_treazury catalog tools --provider providers/agent402.toml --tags llm
```

In a deployment, select tags on the source to narrow the full catalog;
a listener can only narrow the source inventory. For example:

```toml
[sources.chain_data]
extends = "../providers/agent402.toml"
tags = ["crypto"]
```

Then use listener `include_tools = ["agent402_sol_*", "agent402_help"]` to retain
only Solana tools and help. Source tag selection retains generated help; positive
listener tag filters remove untagged help. Both `/api` and `/v1` prefixes are
stripped from generated names while original HTTP paths remain intact, so
`agent402_search` calls `/api/search` and `agent402_chat_completions` calls
`/v1/chat/completions`. Subsets continue sharing the configured wallet according
to the existing deployment binding rules.

### Exclusions and protocol boundaries

- Wallet-keyed memory and account history/receipts are excluded. Feedback submission
  is excluded by exact method/path, preserving the public summary child. Their
  remote identity follows the paying address; rotation changes that identity and
  can strand state. A future deliberate stable-wallet integration needs its own
  contract rather than silently treating these as ordinary rotating-wallet tools.
- Same-payer attestation remains a wallet-identity boundary. Delegated execution
  needs its reviewed authority/result contract; deterministic action planning,
  workflow prompts and fixed synchronous JSON skill packs are compatible.
  `/api/onramp-link` returns a human checkout link and quote; it does not fund
  a wallet or authorize a transfer. It must not be suppressed as a funding action;
  actual faucet/transfer workflows such as `/api/testnet-fund` are distinct.
- Screenshots, QR, image crop and six image-generation endpoints are enabled.
  PNG/JPEG/WebP bodies become MCP image blocks. Explicit mappings extract
  `/image`, `/data/0/b64_json` or `/dataUri` from reviewed JSON envelopes while
  preserving metadata with attachment markers. Image generation's `n` is fixed
  to one in the vendor contract. No media URLs are automatically fetched.
- The primary OpenAPI documents PDF manipulation, audio conversion/normalization,
  the three `/api/tts*` routes and synchronous `/v1/videos/generations` as JSON
  containing base64 strings. LinkedIn articles and media/PDF skill packs also
  return JSON envelopes. These remain ordinary text/metadata without automatic
  decoding or asset retrieval; optional image mappings are separate.
- Resize/convert/thumbnail support PNG/JPEG, with PNG default where documented.
  Select supported formats; optional BMP output does not block the whole route.
  Favicon lookup returns JSON metadata and a data URI, including non-image or
  missing-result cases, and needs no mandatory mapping. Raw
  `/v1/audio/speech` returns unsupported audio MIME types. Conversely,
  `/v1/audio/transcriptions` and `/v1/pro/audio/transcriptions` require multipart
  file parts despite their misleading JSON request declarations; generic JSON
  routing cannot implement that upload contract. Optional report/video 202
  responses require `Prefer: respond-async`, which this integration does not set.
- Gateways/reports remain opt-in through tags. Some tools exceed the normal $1
  payment cap or take longer than the provider's 90-second timeout; choosing a
  tag does not raise either limit. Use small output-token budgets. Streaming is
  documented as false/omitted and exposed stream fields receive explicit guidance;
  the transport does not implement incremental SSE output or silently rewrite
  arguments. This is not a full OpenAI/Anthropic/Gemini SDK replacement.
- No PoW solver, MPP/card/credits flow, hosted-MCP payment extension, or new chain
  support is introduced. Treazury uses its existing Base USDC x402 policy. Managed
  wallets remain exact EIP-3009 only; static `upto` support still has the documented
  approval requirements. Metered exact quotes and actual-usage `upto` ceilings are
  not interchangeable billing promises.

The spec has embedded `x-payment-info.price`, including request-dependent prices;
startup probes are disabled to avoid a large pricing sweep. The descriptions and
live challenges remain the source of each call's payment terms. A single unsigned
`GET /api/search` returned x402 v2 Base USDC `exact` and `upto` offers for 10000
atomic units ($0.01). This checks a representative challenge, not funded execution
or the behavior of every catalog endpoint.

The vendor describes idempotent replay using the same key, payment credential and
body. Although the spec includes optional `Idempotency-Key` headers, the generic
catalog does not expose arbitrary headers or provide durable vendor replay keys.
Do not promise that a new tool invocation is a replay. The guide also asserts
that a missing payment receipt proves no charge; this is not a safe conclusion
when a response is lost or a call times out. Existing uncertainty accounting
remains authoritative and automatic resubmission is not added.

The [llms.txt](https://agent402.tools/llms.txt) is useful but large, with payment
setup, tool lists, vendor guarantees and excluded workflows. It is fetched only
when help is called and cached for the process. The help description explains its
broader scope; no default description cap or silent document trimming is added.
Preserve partial/truncated flags in vendor results such as bounded synchronous
site crawls. The guide references `/api/find` and `/api/pricing`, which are absent
from this OpenAPI snapshot; no fictitious discovery tools are generated for them.
The selected schemas required no converter patch. No polling lifecycle is inferred
merely because a call takes time; the documented bounded site crawl returns its
possibly partial JSON result directly.

## agentutility/provider.toml — AgentUtility

The documentation host's `/openapi.json` returns 404; the live spec is at
[the API host](https://x402.agentutility.ai/openapi.json). The 2026-10-03 capture
has 817 POST operations, all with embedded `x-payment-info.price`. The separate
[registry](https://agentutility.ai/registry.json) records 282 aliases and 17 actual
cluster values, despite the guide advertising 18 clusters. Its fine-grained tags
are much more complete than OpenAPI: 286 OpenAPI operations have no tags at all,
including web-search. Using the live spec with a cluster filter would silently
omit useful operations.

The bundled request-only `openapi.json` preserves full OpenAPI request schemas,
descriptions and payment extensions, unions OpenAPI and registry tags plus cluster,
and records registry `aliasOf` in `x-agentutility-alias-of`. Registry input/output
schemas never replace the fuller OpenAPI contracts. The Rust
`snapshot_agentutility` join checks actual inventory, origin, method and alias
consistency; it has no recommendation or research-tag responsibility.

The full supported catalog, including compatible aliases, is available by default.
The former fifteen research tools plus help are advisory `recommended_tools`;
operators can explicitly opt in or select source cluster tags such as `web-probe`,
`wordmint` or `edge-finance`. Listener filters can only narrow loaded sources.
Aliases are distinct original HTTP paths and retain their own request, price and
name identities; no automatic alias rewriting or recommendation prefix rewriting
occurs. Catalog tags describe the input document, including excluded operations.

```sh
target/debug/x402_treazury catalog tools --provider providers/agentutility/provider.toml
target/debug/x402_treazury catalog tools --provider providers/agentutility/provider.toml --tags wordmint
target/debug/x402_treazury catalog tools --provider providers/agentutility/provider.toml --tags synthforge
```

A deployment source can set `extends = "../providers/agentutility/provider.toml"`
and `tags = ["edge-finance"]`. Provider files assign no wallet; choose the wallet in the deployment.

### Payment compatibility

A single unsigned POST to `/web-search` returned x402 v2 `PAYMENT-REQUIRED` with
Base USDC exact pricing of 6000 atomic units ($0.006), alongside a Solana offer.
The guide's `X-PAYMENT` wording is stale for that sample. Treazury retains its
Base USDC asset/network policy; it does not add Solana support.

The sampled challenge includes `bazaar` and `builder-code` extensions. Managed
`zcash_rotation` payments strip top-level extensions before signing, warn with
extension names only, and attempt one ordinary Base USDC payment. Providers
requiring extension echoes may reject it. Failure messages explain the omission;
submitted authorizations remain reserved until chain reconciliation and are never
automatically replayed. Stripping does not prevent the seller/facilitator from
adding its own settlement attribution. Static SDK extension handling is unchanged.
The captured challenge is tested against local static and managed payer/seller
fixtures, including Base selection, one signed retry and managed extension removal.
Rejection/restart tests verify retained reservations. These are not live funded
settlement qualification or proof about every endpoint.

Startup pricing probes are disabled. Prices are frozen vendor estimates, while
live challenges and the existing per-payment cap remain authoritative. Some
routes exceed the normal $1 cap or 90-second timeout. Missing responses/receipts
must not cause automatic paid retries. Registry `requiredEnvVars` describes
upstream service dependencies; this integration does not ask callers to supply
Venice/Fal/CloudConvert credentials or initialize a separate vendor wallet/client.

### Output and workflow caveats

- Hosted `image_url`, `audio_url`, `video_url`, `pdf_url` and similar results stay
  JSON metadata. Treazury does not download those assets or convert a returned URL
  into an inline image. Completed media-conversion results can therefore be exposed
  without implementing artifact storage; a tracing `job_id` alone is not evidence
  of a client polling workflow.
- Four satellite routes use explicit base64 image mappings: satellite-address,
  satellite-bbox and satellite-tile produce one attachment; satellite-change
  produces before/after attachments. MIME comes from `content_type`; attribution,
  license, dates, geography and remaining metadata are preserved. Existing inline
  byte/count limits apply, with explicit errors after a potentially paid response.
- One-shot browser-session returns HTML/title JSON with an optional screenshot;
  IPFS-fetch returns JSON metadata plus text/JSON/base64; QR-code-generate returns
  JSON format/content-type/base64 fields. Optional screenshot or SVG content does
  not make these JSON envelopes unsupported. These and their compatible aliases are exposed without inventing
  automatic image decoding, file adapters or a stable browser-session lifecycle.
- Watch-page is excluded because it stores caller-selected watch IDs and server-side
  state for up to 365 days; its ownership/access semantics need review. The
  browser-workflow cluster, in contrast, analyzes supplied traces/HTML and returns
  advisory plans/diffs; those operations do not themselves execute browser actions.
- Six audit routes are excluded: db-migration-risk, dep-risk-summary,
  deploy-config-risk, production-readiness-score, prompt-injection-surface, and
  secrets-exposure-check. Their complete `oneOf` branches declare alternative
  repo/files properties; the current converter succeeds but emits an empty-properties object, losing the
  complete repo/files alternatives. Each captured operation is independently
  exercised offline to establish that precise loss; these are not converter errors.
  Do not replace them with the registry's empty object schemas. No captured aliases
  target these six operations or watch-page, so no alias exclusions are invented. Required-only
  alternatives on keyword-suggest, wallet-label and company-intel-pack are retained.
- Compose routes are synchronous bundles executed upstream in a single request. Preserve
  component failures and degraded markers rather than presenting partial reports
  as complete. Web-search can return an explicitly marked recent-news fallback;
  `include_text` is a compatibility input and does not enable page-text retrieval.

The large [llms.txt](https://agentutility.ai/llms.txt) is loaded only when help is
called and cached for the process. It includes excluded tools, aliases, other MCP
clients and pricing that may differ from this snapshot; provider instructions
explain these boundaries without truncating the guide.

### Refreshing the catalog

Fetch the public OpenAPI and registry JSON to temporary local files, then run:

```sh
scripts/zcash.sh run --offline --example snapshot_agentutility -- \
  /tmp/agentutility-openapi.json /tmp/agentutility-registry.json \
  providers/agentutility/openapi.json
scripts/zcash.sh test --offline --example snapshot_agentutility
```

Review request/description/pricing changes, new aliases, tags and response envelopes.
Review actual supported boundaries and focused transformation/request regressions
alongside updates; do not restore a fixed inventory or research-default gate. The snapshot does not refresh at runtime;
no custom registry adapter is introduced into serving or agent source registration.


## BlockRun and Claw402

BlockRun exposes its supported synchronous catalog plus help; compact chat/search
recommendations are advisory.
The spec uses `/api/v1`, while the guide often uses `/v1` aliases; keep the spec
paths on the wire. Omit chat `stream` or set false and supply a small `max_tokens`.
Music and image generation/editing use a hybrid lifecycle: they may return inline
or hand off a 202 job requiring same-wallet/payment-header polling, with no
force-inline option. Video is asynchronous. These contracts remain excluded.
Speech and sound-effects return synchronous JSON containing hosted audio URLs;
they remain available without fetching those links. Phone/sandbox ownership, voice
calls and trade submission have distinct lifecycle/authority boundaries. Valid
fixed BTC-USD, XAU-USD, EUR-USD and Binance BTCUSDT paths remain usable with their
literal targets and generated names. Do not invent symbol parameters or rewrite
wire paths. Zero-address/token placeholders retain an unverified intended-target
contract, not a general claim that every fixed target is invalid. Published
vendor prices remain advisory; startup discovery is disabled.

Claw402 exposes its full catalog plus help, with advisory market-read
recommendations and exact case-sensitive tags. RootData POST bodies are sparse open `type: object` schemas. They accept
undocumented-by-schema arguments in the JSON body: search uses `query` and optional
`precise_x_search`; project uses `project_id` or `contract_address`, with optional
`include_team`/`include_investors`; org uses `org_id` with optional
`include_team`/`include_investments`; funding accepts optional Unix-second
`start_time`/`end_time`, USD `min_amount`/`max_amount`, `project_id`, `page` and
`page_size`. Search supplies profile IDs. Preserve this prose and open-body
routing rather than inventing required schemas or calling the requests unusable. Account/model diagnostics and AI/media remain available; image result envelopes
are not independently qualified. Startup pricing discovery is disabled because
prices are embedded. Help includes broader routes and manual retry suggestions;
Treazury retains its own payment uncertainty/no-replay rules.

Both selected unsigned challenges used Base USDC v2 exact with `bazaar` metadata.
Catalog/help/challenges passed through the actual strict HTTP/2 + TLS 1.3 Tor
network factory. No payment or settlement was tested, and no reliability tag is
justified by this small successful sample.

## Reviewed but unavailable: Apollo

Apollo has a frozen request schema for future work. It has no active TOML definition:
request-ID retry/recovery handling needs verification. Catalog/quote availability
through Tor does not resolve this application-level limitation.

## Payment extension and offer boundaries

Additional extension semantics:

- `payment-identifier`: request/payment deduplication, advertised `required:false`
  in both samples. Omitting it forgoes that mechanism; it does not justify replaying
  a signed request. Future support must distinguish a per-logical-operation key
  from a persistent user identifier. [Specification](https://github.com/x402-foundation/x402/blob/main/specs/extensions/payment_identifier.md).
- `agentkit`: Exa advertised a World-chain (`eip155:480`) EIP-191/EIP-1271 proof
  challenge and a 100-use free-trial option. This is a separate authentication
  option in the sample, not evidence that every paid call requires human verification.
- `locus.freePreflight`: vendor coverage-check/tool-directory hints. These are
  untrusted guidance; do not automatically execute suggested commands or tool calls.
- `offer-receipt`: Locus and Otto include seller-signed offer records with accepted
  offer indexes, payment facts and validity. Their presence is not proof that the
  buyer paid, nor that Treazury verified the seller signature.
- `otto-content-receipt`: empty advertisement in the sampled challenge; no successful
  response was purchased, so its receipt contents and verification remain unqualified.
- `sign-in-with-x`: Otto advertises wallet ownership authentication. Its full guide
  documents selected read-only pay-once re-access for one hour. Stripping it does
  not implement that benefit or same-wallet job polling. [SIWX specification](https://github.com/x402-foundation/x402/blob/main/specs/extensions/sign-in-with-x.md)
  and [Otto full guide](https://x402.ottoai.services/llms-full.txt).

Managed mode strips all top-level extensions and attempts a supported ordinary
payment. It does not sign authentication challenges, verify signed offers, add
idempotency keys, or promise that servers accept omission. Static SDK handling is
separate. Ten raw unsigned challenges are pinned in
[provider_payment_challenges.json](../tests/fixtures/provider_payment_challenges.json).
They contain public seller offers/challenges, not buyer keys or payment signatures.
The managed regression checks local signing or refusal, not live settlement.

### Offer metadata is a separate compatibility boundary

Offer metadata is distinct from top-level extensions:

| Provider | Fields | Current managed result |
| --- | --- | --- |
| Agent402 | `accepts[].outputSchema` on the exact Base offer | Accepted as opaque object/boolean JSON Schema and echoed unchanged. Its contents never affect payment terms; the alternative Base `upto`/Permit2 offer remains unsupported. |
| Exa | `extra.breakdown`, `extra.totalUsd`, `extra.acceptId` | Reviewed informational fields accepted and echoed unchanged on ordinary Base EIP-3009 offers. GatewayWalletBatched remains unsupported. Search and Contents challenges are pinned. |
| Google Trends | `extra.merchant`, `extra.tier` | Reviewed string labels accepted and echoed unchanged. Challenges for trend, interest-by-region, related-queries and related-topics are pinned. |
| Locus | Ordinary EIP-3009 plus GatewayWalletBatched alternatives | Local fixture selects ordinary USDC and strips top-level extensions. |
| Otto | Ordinary EIP-3009 plus Permit2 alternatives | Local fixture selects ordinary USDC and strips top-level extensions. |

Managed admission accepts the five reviewed informational fields with type checks:
string labels, nonnegative numeric total and a map of nonnegative numeric price
components. These annotations never determine spending limits or signing; atomic
`amount` remains authoritative. The SDK must preserve the accepted offer exactly.
Unknown fields and unsupported mechanisms still fail closed. Local signing regressions do not prove live settlement.
OpenAPI `x-*` annotations (pricing, discovery, lifecycle, etc.) are yet another
category; they are not themselves x402 challenge extensions.

## Curl HTTP Request

`curl/provider.toml` pins the vendor's public OpenAPI schema for `POST /curl`.
It advertises $0.01 per call on Base USDC and returns origin status, headers and
body text. Redirects are automatically followed by the vendor; discovery relay
use rejects nonempty redirect chains. The documented request cannot forward
conditional headers, and binary bodies, body-size limits and completeness are not
qualified. No paid delivery or Tor qualification has been performed. The discovery
adapter accepts only unsigned target GETs; enabling the provider as an MCP source
also exposes the vendor's other target-method choices. See
[relay configuration](../docs/configuration.md#paid-discovery-relay).

## relayshield.toml — RelayShield

The provider exposes supported pay-per-call operations and public checks from the
vendor OpenAPI, plus lazy `llms.txt` help. Compact recommendations are advisory. Embedded
`x-payment-info` prices are advisory; pricing probes are disabled. Account and
metered routes are excluded. Public keyless `POST /v1/email-check`,
`POST /v1/link-check` and `POST /v1/wallet-risk` are compatible despite lying
outside `/v1/payg`; a root fallback preserves their full `v1_*` names without
renaming payg tools. File scanning is excluded because its result-polling schema
is absent from OpenAPI, although root discovery advertises the route. This is
a missing integration contract, not proof the API lacks it. URL scanning retains immediate heuristic
signals, and its description identifies the unavailable asynchronous verdict.
Bulk identity risk ($2.00) and campaign scanning ($5.50) exceed the default $1.00
per-payment cap. The help document uses metered paths and includes extra routes;
only selected MCP tools are callable.

`relayshield/phone-reputation.toml` is an optional companion for the phone endpoint
listed in vendor root discovery and help but absent from OpenAPI. Its local request
schema is explicitly authored: the documented phone argument and unsigned x402 v1
challenge input example inform the contract, rather than a vendor JSON Schema.
The companion shares the `relayshield` prefix and omits help to coexist with the
main source on one listener. Add both sources to expose phone reputation alongside
the main catalog. The unsigned phone challenge uses x402 v1; the vendor OpenAPI
advertises `x402_version: 2` for its payg operations. Live challenges determine
the protocol. Offline catalog and signing regressions establish local
compatibility only; no funded settlement or live Tor qualification is claimed.

## ipintel.toml — IPIntel.ai

Uses vendor [OpenAPI](https://tools.ipintel.ai/openapi.json) and lazy
[help](https://tools.ipintel.ai/llms.txt), without freezing the captured operation
inventory. Recommendations cover DNS, IP, domain health and WHOIS lookups plus
help; satellite and developer utilities remain available by default. Structured
prices disable probing. Captured schemas retain nullability, enums and arbitrary
validator JSON without a local repair. Public document retrieval was unavailable
in this audit; local tests do not qualify current availability, settlement or Tor.

Observed documentation oddities and interpretation boundaries:

- The guide and schema overview advertise $0.001–$0.17, but operation prices
  currently span $0.001–$0.09; satellite is $0.09. Live challenges govern payment.
- Satellite returns JSON with hosted PNG links, not image bytes/base64. Links
  remain metadata; Treazury does not automatically fetch them. Nullable `date`
  means latest when null, and default `fallback = true` can select a nearby scene.
  Preserve scene timestamps, fulfillment status, warnings and Copernicus attribution.
- HTTP latency prose calls it a HEAD check, but its request schema also permits
  `method = "GET"`. Measurements come from the handling Cloudflare Worker edge,
  not the agent's location, and are not ICMP ping.
- The domain-health example scores 100 with DMARC `p=none`. Treat scores as the
  vendor's configuration summary, not evidence of enforced anti-spoofing policy
  or company legitimacy. Email validation documents syntax/DNS/MX/provider checks,
  not confirmed mailbox reachability; availability checks are probabilistic.
- JWT decode explicitly does not verify signatures. Base64 and JSON Schema
  utilities are paid remote computations often better performed locally. The
  validator's `data` property deliberately accepts any JSON value.


## newsgurus/provider.toml — News Gurus Intel API

Uses `newsgurus/openapi.json`, a local OpenAPI projection of the vendor's
`https://api.newsgurus.ai/x402/catalog`, with lazy vendor `llms.txt` help. It
preserves all 49 catalog GET request contracts, descriptions, published prices
and response examples. The provider selects 48 task-facing reads plus help;
`/x402/agents/status` is omitted as a diagnostic. Prices use `x-payment-info`
without startup probes. The vendor advertises x402 v2, USDC on Base; the payment
challenge remains authoritative. No API key is required by this integration.

The vendor's `/openapi.json` is already x402-focused, rather than a separate
key-only inventory, but has only 39 paths: it substitutes sample `NVDA`, token
and condition IDs for path templates, omits the eight fantasy and three
`/api/v1` paid routes, and includes the catalog diagnostic. The x402 catalog is
therefore the request source of truth. Its three `/api/v1` resources are
explicitly sold via x402 despite also supporting subscriber credentials.
The local projection copies Bazaar `pathParams`/`queryParams` properties and
required lists into OpenAPI parameters; it uses each resource's actual GET
method, never the generic Bazaar boilerplate's HEAD/DELETE alternatives. Refresh
from `tests/fixtures/newsgurus_x402_catalog.json` and review the projection and
focused transformation contract together; the regression compares every parameter
schema, price, description and vendor output example without inventing a response
schema.

The guide still describes the legacy `X-PAYMENT` header alongside the v2 catalog;
Treazury owns negotiation/signing. Free `/public/regime` and `/public/narratives`
alternatives are advertised outside the selected paid catalog. Paid prices span
$0.002–$5, with several sports/fantasy calls above the default $1 payment cap.
No cap is raised for those tools. This is primarily derived trading intelligence,
not a general news wire; keep confidence estimates, source dates and vendor
opinions separate from primary evidence. The scoops example marks Business Wire
coverage as `stub`; examples do not establish current freshness or completeness.

Company candidates are `newsgurus_institutions_symbol` ($0.10: congressional
trades and institutional 13F changes), `newsgurus_primary_scoops` ($0.02: primary
filing/release snippets, but no company/ticker filter), and optionally
`newsgurus_spaces_intel` ($0.02: ticker-filtered X Spaces summaries and quotes,
with separately labeled `our_opinion`). Existing Otto/x402stock sources overlap
filings/news; the $0.50 tearsheet is mainly a trading-signal bundle.

## vaaya.toml — Vaaya

Uses vendor `https://vaaya.ai/openapi.json` with specific support-boundary
exclusions, not a fixed research inventory. Synchronous research, catalog,
validation and JSON/image contracts remain available, including compatible
aliases. Compact research recommendations are advisory. Optional MPP offers do
not block a route when x402 is also available; MPP-only video routes are excluded.

Firecrawl crawl/extract declare successful JSON wrappers; the underlying vendor
lifecycle alone does not establish that Vaaya returns an asynchronous handoff.
These remain exposed with service-specific result uncertainty and no automatic
follow-up, polling route, completeness promise or paid replay.

Excluded families require unsupported async result retrieval, uploads, persistent
account/memory/sandbox ownership, message/purchase authority, or payment
forwarding. The $5 `vaaya/fetch` forwarder is labeled fixed price despite prose
promising variable upstream settlement. Do not infer executable x402 support
from the broad account-oriented `llms.txt`. Hosted assets remain metadata links.
Captured request contracts and cached primary documentation establish the schema
basis, not current availability, funded settlement or Tor behavior.

Input guidance and data caveats:

- Many schemas permit extra JSON fields; do not globally close the provider.
  TheirStack jobs requires a date or company condition in prose, and
  technographics requires an identifier despite omitted schema required lists.
  TheirStack companies is $0.60, above the privacy example company's $0.50 cap.
- Fundable company/company-search say supply one identifier but have no formal
  `required`/`oneOf`. Preserve that schema and follow the tool guidance. The vendor
  documents a sampled company mismatch (1/10) and lagging total funding values;
  verify identity and favor dated source/deal evidence. Follow-up deals/investor-deals
  actions mentioned in prose are absent from this x402 schema.
- EDGAR autocomplete cannot prove an entity never registered, despite vendor
  prose claiming it can. CourtListener RECAP has incomplete coverage; no hits
  cannot establish absence of litigation. Preserve coverage and source dates.
- Returned hosted assets remain links; the integration does not automatically
  fetch them. Preserve the vendor's `ok`/`data`/`charged_cents` response envelope
  and degraded/partial signals; uncertain paid submissions are not replayed.

Strong company candidates are `vaaya_akta_headcount_trends` ($0.18),
`vaaya_akta_product_reviews` ($0.11), `vaaya_courtlistener_cases` and
`vaaya_courtlistener_dockets` ($0.01 each), and `vaaya_fundable_company_search` /
`vaaya_fundable_company` ($0.01 / $0.10). Raw EDGAR and USPTO records add evidence
beyond existing company summaries; Signalbase funding/acquisitions ($0.25 each)
add event feeds. OneSearch ($0.05) is convenient but overlaps existing Exa search.

The example company listener in `examples/deployments/privacy.toml` selects 26
Vaaya tools: Fundable company search/profile; CourtListener opinions/dockets;
Akta headcount/product reviews; TheirStack technographics/keyword lookup;
Signalbase funding/acquisitions; all six EDGAR reads; USPTO assignees/patents;
Wayback snapshots/availability/fetch; Tako search/answer/contents; and RecallRadar
search/get. They share its existing company wallet and payment cap. These
sources are not restricted to listed companies: coverage depends on indexed
profiles, products or public records. CourtListener's `q` remains required even
when supplying `party_name`; EDGAR XBRL concepts principally serve public-company
financial reporting.

Tako research and RecallRadar have no known synchronous JSON/x402 compatibility
blocker; their original omission was a compact catalog choice. The provider
includes the complete search/result research chains. Tako JSON chart creation remains available alongside research reads. RecallRadar search advertises `q` in prose
("Query params") but publishes an open JSON request body with no declared
properties. Pass `q` in the POST JSON body; the provider deliberately preserves
the upstream schema rather than closing it. Its get route requires a returned
`id`. Tako search/answer require `query`, and contents requires `url` from a result.
No paid result or completeness qualification is implied by these contracts.

Description prices are inconsistent with structured prices: Tako search/answer/
contents prose quotes about $0.012/$0.018/$0.002, while `x-payment-info` advertises
$0.03/$0.03/$0.05; RecallRadar search/get prose quotes $0.10/$0.025 while structured
prices are $0.12/$0.03. Use the structured estimates and live payment challenge.

## agosint.toml — AgOSINT

The configured [OpenAPI](https://agosint.shveik.dev/openapi.json) and
[guide](https://agosint.shveik.dev/llms.txt) distinguish `/x402` from duplicate
MPP routes. Exclude the `/mpp` subtree, keep `/x402` as a naming anchor and
retain a root fallback for public `/endpoints` discovery and new public reads. The root `base_url` repairs the vendor server's
extra `/v1` prefix. Company/due-diligence requests require exactly one lookup key;
id/name also require country. Network uses ip or asn; sanctions uses name or
wallet. Invalid inputs may still receive a quote before a paid validation error,
so preserve input guidance. Long registry fan-out is synchronous work, not proof
of a polling lifecycle. Public documents could not be retrieved during this audit;
there is no full offline provider capture or new settlement qualification.

## datakoot.toml — Datakoot

The [x402 OpenAPI](https://x402.datakoot.com/openapi.json) supplies public-data
and Koot synthesis requests. `/api` is a naming anchor with a root fallback;
the former exact inventory no longer freezes newly supported operations. Help
uses the [apex guide](https://datakoot.com/llms.txt), rather than the x402 host's
historically missing guide. Preserve alternate CVE/dependency input guidance,
US-only location coverage and operation-level estimates that differ from the
top-level summary. No recommendation list is needed for its coherent supported
catalog. Current document retrieval was unavailable and no full offline capture
is bundled; definitions are not evidence of funded delivery.

## telesint.toml — TeleSINT

Wallet-owned `/watch` state is excluded because managed payer rotation cannot
preserve its ownership. Public `/status` and new stateless intelligence routes
remain available. Recommendations select the established stateless intelligence
queries and their artifact/result flow plus help; opt-in deliberately omits status
without hiding it in the reusable default. `preparation` and `velocity` need a
scope parameter; `claim-check` needs id or text. Their documented HTTP 200 guidance
errors can still consume payment, so retained overrides explain the actual inputs.

Help uses [SKILL.md](https://telesint-api.onrender.com/SKILL.md) because the captured
`llms.txt` was truncated. Follow OpenAPI names/enums where guide prose differs.
The [schema](https://telesint-api.onrender.com/openapi.json) and guide were
unavailable to fresh inspection, and no complete provider capture is bundled.
Synthetic selection tests establish local policy only, not vendor delivery.

## hirescrape.toml — HireScrape

The primary [schema](https://hirescrape.com/openapi.json), retrieved through its
homepage documentation link, supplies six paid scraper contracts and specific
SIWX workflows: `/api/skills/search`, `/api/skills/runs` and its request-ID child,
`/api/snapshots` and its ID child, `/api/reports` and its ID child, and
`/api/scraper-requests`. Those actual authentication/lifecycle boundaries are
excluded explicitly; `/api/tools` remains a naming anchor with a root fallback.
The overview advertises free skills discovery and synchronous marketplace runs
without publishing their corresponding path schemas. Do not invent those tools
or classify every future marketplace route as incompatible. Scraper modes that
hand off SIWX snapshots remain unsupported even when small requests usually finish
inline. Cached primary documentation and captured contracts do not qualify paid
completion or current availability.

## netintel.toml — NetIntel

The complete retrieved [primary schema](https://netintel.dev/openapi.json) has
eighteen same-path GET/POST pairs with matching structural inputs, requireds,
response declarations, security and payment metadata. POST keeps passwords/tokens
out of URLs and arrays in JSON; vendor GET deprecation prose reinforces this
choice. This is schema comparison, not actual-charge qualification. Compatible
`/api/v1/chat/completions` and `/api/v1/embeddings` aliases retain full path names
alongside canonical `/v1` tools. The sole documented image path,
`/ai-image/generate`, retains its explicit PNG data-URI mapping; no alias mapping
is inferred. Research recommendations are advisory.

## x402-list.toml — public directory

The reusable provider exposes public directory reads, synchronous `/assess` and
completed `/suggestions` submissions. Later human review does not turn the
submission acknowledgment into an unsupported result lifecycle. Assessment may
perform a seller probe within its single quoted nonrefundable charge; payment
caps and uncertainty rules still apply. `/submit` is excluded because its full
top-level `oneOf` request alternatives are lost by the generic converter;
request-update, verify-ownership and reissue-token require domain/email proof or
owner tokens. This is distinct from the embedded directory's intentional three
read operations. Recommendations contain the familiar five directory reads and
help without constraining reusable defaults.
