# Provider behavior and caveats

Use these notes alongside [provider definitions](README.md). Prices and vendor
behavior describe the committed configuration and fixtures; live challenges remain
authoritative. Tests pin 840 tool contracts across 20 providers and settings for all
24 providers without contacting vendors.

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

Uses the live OpenAPI document's platform tags, with administrative `Auth`,
`Monitors` and `System` excluded. All platforms are included by default;
`--tags Twitter,YouTube` selects a subset. `LinkedIn` spans v1 and v2; add
`--include /v1` when only v1 is wanted. Tags are exact and case-sensitive.

`include = ["/v1", ""]` strips `/v1` from tool names while the empty root fallback
retains other API versions: `socialfetch_twitter_profiles_handle` and
`socialfetch_v2_linkedin_*`. Original request paths remain intact; keep the base
URL at `https://api.socialfetch.dev`.

Credit pricing comes from `x-socialfetch-credits-pricing`, supplemented by
credit-to-USDC guidance in `instructions_text`. Pricing probes are disabled;
no endpoint sweep is needed. Fetching the live spec still requires one startup
request. Use a 90-second timeout for slow search/transcript calls.

`tests/fixtures/socialfetch_openapi.json` pins the 2026-10-02 source: 259
operations, 237 selected API tools including Yelp plus lazy help. It retains tags, descriptions,
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
treazury --provider providers/deepline.toml --timeout 120
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

US stocks/ETFs/options, forex, crypto & on-chain perps, energy commodities,
macro (Fed, CPI, Treasury, World Bank), SEC filings, congressional trades —
142 GET tools. 140 ops publish a flat USD price via the `x-payment-info`
extension ($0.01–$0.75 at time of writing), surfaced by `pricing_key`; two
genuinely free endpoints (`market-status`, `market-holidays`) answer `200`
unpaid and are the only ops probed at boot. The spec's second server
(`agents.x402stock.xyz`) is an MPP/Tempo pay-host — the conf pins `base_url`
to the direct x402/USDC-on-Base host. `POST /api/v1/feedback` is excluded
(a vendor feedback form, not data). Guidance: the vendor's 27KB `llms.txt`
is served via `x402stock_help()` (`help_url`) — far too long for
`instructions_text`, which instead summarizes path-placeholder handling,
the `{ticker, data, source, as_of}` response shape, and the 402/retry flow.

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
treazury --provider providers/regimeshift.toml --timeout 320
```

## kronos.toml — Kronos Crypto Data (kronossignals.com)

Crypto market data for trading agents: 36 GET tools under `/api/v1` — per-asset
signals/price/ohlc/volatility/liquidation maps (`{asset}` paths, 16 spot
assets + HYPE derivatives-only), market-wide overview/scan/funding extremes,
BTC/ETH options (GEX, IV surface, implied probabilities), ML forecasts
($0.05) with a **free** `kronos_track_record` accuracy audit, macro context,
and the $0.10 all-in-one `briefing`. The include anchors on `/api/v1` so the
`v1` path segment never leaks into tool names (the vendor's free `/api/stats`,
`/api/health`, `/api/methodology` top-level routes are not exposed). Prices
live in vendor prose (the spec's `x-x402` extension has the wrong shape for
`pricing_key`). Startup probes are disabled (`probe_pricing = false`) because
pricing discovery was repeatedly slow; a deployment source may explicitly
re-enable them. Vendor prose and the free `kronos_track_record` override remain.
Actual paid calls still use the live challenge and enforce spend caps.

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
GET-only by design). The free utility routes (`/healthz`, `/catalog`,
`/llms.txt`) are not exposed: `/catalog` would name `concordance_root` under
the include-anchor naming rules, so the source slugs and example series ids
it discovers live in the series tool's param notes instead, and `help_url`
serves the vendor `llms.txt` as `concordance_help()`.

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

92 operations → **87 API tools plus help**
(four routes ship GET+POST variants and get `_get`/`_post` names), an
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
  with `--tags` (eight tags, `--list-tags` inventories them).
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
GET+POST twins that share a path and so get `_get`/`_post` names (10 tools
plus `genuinegood_help()`). Every op publishes a correct-shape
`x-payment-info` ($0.05 / $0.08 / $0.20 / $0.50, preflight $5.00 per call,
USDC on Base), so prices render from the **live vendor spec** and boots make
zero probe requests; the vendor prose states no dollar amounts, so the
rendered price line survives the dedupe and each tool shows its price exactly
once. The include anchors on `/v1/grants` so the path prefix never leaks into
tool names; POST bodies are inline with real constraints and the schemas are
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
two-tool free gateway and a help tool. Prices are
embedded from the spec's `x-payment-info.price` blocks ($0.01–$2.49 per call),
so boots make zero probe requests (`probe_pricing` is false), plus a **two-tool
free gateway** mirroring the vendor's own remote-MCP shape: `locus_list`
(catalog of the 114 free tools with `compact`/`category` query knobs) and
`locus_call` (wrapped `{name, arguments}` body; the `name` enum pins the 114
valid tools, and the spec's oneOf-of-114-wrappers body — which the flattener
cannot expose — is rewritten into that flat form). Design notes:

- The spec's `/api/locus_*` underscore routes are free REST duals of the same
  tools, but their slugs collide with the paid `/api/locus-*` hyphen twins
  under path-derived naming, so the free tier rides the gateway instead.
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
treazury --provider providers/locus/provider.toml --timeout 120
```

## straits.toml — Straits.live Hormuz monitor (straits.live)

Strait of Hormuz crisis data in two tiers, exposed as **33 tools tagged
`Free` / `Premium` in a curated OpenAPI catalog** (`providers/straits/openapi.json`)
so the server's tag selection mounts tiers per server (`--tags Premium`,
`--tags Free`; `--list-tags` inventories them). The vendor spec
(`straits.live/openapi.json`, snapshot in `tests/fixtures/`) covers only the
premium side — 13 ops, all tagged `Premium`, correct-shape `x-payment-info`
($0.01–$0.50) — from which the local catalog derives the 10 GET data routes; it
also defines the 23-route free tier (`/status`-style summaries,
short-window series, curated indicators) from the human `/api` catalog, so
agents get zero-cost context and pay only for depth. Excluded on purpose:
`/api/premium/stream` + `/stream/session` and `/webhooks`(+DELETE) —
session-token/subscription-secret auth, not x402-payable, and webhooks
need a callback URL an MCP agent cannot receive; the retired frozen feeds
(`/api/v1/hormuz-snapshot`, `/api/v1/cape-snapshot`); and the free
`/api/v1/vessels` count summary (name collision with premium `vessels`;
`ports`/`transits` cover its data). Free routes answer `200` unpaid — the
boot probe would mislabel them, so each carries an override restating
"Free — no payment required" semantics. Rate limit: 30 req/60 s shared
across all routes. `help_url` serves the vendor `llms.txt` (which carries
the exact free-vs-premium comparison) as `straits_help()`. On a tier-tagged
mount (`--tags Premium`) the free-tier overrides warn-and-skip at boot —
expected, they do not apply to that subset. Three premium routes
(hormuz/flags, hormuz/risk-screen, risk-premium) inline >500-char challenge
descriptions that Coinbase's facilitator rejects; `src/payment.rs` trims
challenge descriptions client-side, so they settle like the rest.

## lonestar.toml — LoneStarOracle catalog (lonestaroracle.xyz)

The provider reads the vendor's **unified spec** (one fetch, `openapi.json`, root server =
`https://lonestaroracle.xyz/api`). The gateway proxies every service
payment-gated — verified 402-identical to the per-service subdomains — so no
per-op URL handling is needed (the spec's `x-direct-url` extension is
informational). 96 API tools plus help over 71 services; the vendor's `authMode: free`
markers render as "Free — no payment required (vendor spec)." via the
server's pricing-renderer, and the audit POSTs' `$ref` bodies deref through
the shared flatteners. Zero boot probes (every op is spec-classified).
Exclusions: curated suffix rules (demos, previews,
per-service `/feed` pages, subscription management, unpriced LeaseEdge
extras) are spelled out as 31 concrete paths — new vendor preview/feed routes
would appear as tools until the list is extended. Select subsets with source
path filters or listener `include_tools`/`exclude_tools` patterns; legacy curated
`--groups` aliases are not supported. `help_url` serves the vendor `llms.txt`
(one-fetch catalog, per-service prices, subscribe notes) as
`lonestar_help()`.

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

The [vendor OpenAPI spec](https://win.oneshotagent.com/openapi.json) describes
147 operations in the 2026-10-03 fixture. The default definition exposes only
`oneshot_search` and `oneshot_help`. `POST /v1/tools/search` is explicitly
synchronous and returns the result inline. Exact method/path selection excludes
new methods and subroutes as well as health/status/stats, account/balance tools,
marketplace administration, payments/credits, messaging and other workflows.
`include = ["/v1/tools"]` removes that prefix from the tool name while preserving
the original HTTP path and root base URL.

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

The [vendor spec](https://stableenrich.dev/openapi.json) has 38 operations in the
2026-10-03 fixture. The default exact allowlist selects 32 JSON/image operations plus
`stableenrich_help`. It covers CompanyEnrich, Exa, Firecrawl, FullEnrich, Google
Maps/Solar metadata, Minerva, PDL, Reddit, Serper and Whitepages. No basic health
or diagnostic routes appear in this spec; newly added methods/routes are excluded
until reviewed. `include = ["/api"]` shortens names without changing HTTP paths.

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

The [vendor OpenAPI](https://agent402.tools/openapi.json) captured on 2026-10-03
contains 607 operations and 24 lowercase tags. This provider defaults to `web`,
with 48 API operations plus lazy `agent402_help`; it does not load the entire
catalog into an agent's tool context. An exact method/path allowlist contains
480 eligible operations across the reviewed groups. Changing `tags` selects
from this inventory; it does not restore excluded operations. Source field/list
replacement semantics are unchanged.

Common selections (counts include help):

| Source/CLI tags | Tools in fixture | Intended use |
| --- | ---: | --- |
| `web` (default) | 49 | Search, extraction, browser-rendered text, document inspection and social reads |
| `data,crypto` | 215 | Broad public-data and on-chain catalog; narrow further for ordinary agent use |
| `llm` | 48 | Model gateways, reports and three image-generation endpoints |
| Empty source `tags = []` | 481 | Entire eligible catalog; avoid as the normal context-size default |

Other vendor groups include `network`, `encoding`, `identifiers`, `conversion`,
`text`, `time`, `date-time`, `validation`, `math`, `research`, `wallet`, `payments`,
`chain`, `ai`, `api`, and `x402`. Tags such as `data` are broad: they include some
local utilities and OCR as well as live public data. `time` and `date-time` are
distinct. `memory`, `agent`, `skill-pack` and `workflows` have no eligible operations
in this definition. `--list-tags` reports the unfiltered vendor catalog, so it
also lists those tags and counts routes removed by the allowlist. Selecting only
an excluded group fails with no matching operations rather than exposing it.

```sh
target/debug/treazury --provider providers/agent402.toml --list-tags
target/debug/treazury --provider providers/agent402.toml --tags network --list-tools
target/debug/treazury --provider providers/agent402.toml --tags data,crypto --list-tools
target/debug/treazury --provider providers/agent402.toml --tags llm --list-tools
```

In a deployment, override tags on the source to widen beyond the web default;
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

- Wallet-keyed memory and account history/receipts/feedback are excluded. Their
  remote identity follows the paying address; rotation changes that identity and
  can strand state. A future deliberate stable-wallet integration needs its own
  contract rather than silently treating these as ordinary rotating-wallet tools.
- Delegation, attestation, composite skill packs and workflow-prompt helpers are
  excluded. They need separate review of composed actions, state and output
  contracts. This is an integration boundary, not a claim that all such tools
  are fundamentally incompatible. Funding/onramp helpers are also excluded.
- Screenshots, QR, image crop and six image-generation endpoints are enabled.
  PNG/JPEG/WebP bodies become MCP image blocks. Explicit mappings extract
  `/image`, `/data/0/b64_json` or `/dataUri` from reviewed JSON envelopes while
  preserving metadata with attachment markers. Image generation's `n` is fixed
  to one in the vendor contract. No media URLs are automatically fetched.
- PDF/audio/video generation and conversion remain excluded pending artifact
  storage. Resize/convert/thumbnail may return BMP and remain excluded; favicon
  output can be ICO/SVG, missing, or URL-only and also remains excluded. JSON
  metadata, OCR, image-search results, text conversions and base64 string utilities
  remain eligible. Image crop describes binary output by default but documents a
  JSON `dataUri` alternative; both supported response forms are handled.
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

The bundled request-only `openapi.json` preserves OpenAPI request schemas and
full descriptions, unions OpenAPI tags with registry tags and cluster, and records
registry `aliasOf` in `x-agentutility-alias-of`. It adds the explicitly authored
`treazury-research` tag to 15 reviewed operations. It does not use the registry's
less complete input/output schemas as replacements. The Rust
`snapshot_agentutility` example generates this catalog from two locally fetched
files and refuses mismatched inventories, unexpected methods/origins, or missing
default routes. The provider's exact allowlist includes 525 eligible operations;
new routes and registry-declared aliases do not appear automatically.

Default: **15 research operations plus lazy help**—web search, cited answers,
research briefs, scraping, structured scraping, links, archive snapshots, arXiv
search/summaries, PubMed, Hacker News, Wikipedia, GitHub README and YouTube/PDF text.
Basic DNS/status/developer utilities are outside this default. Source tags replace
the default selection; listener filters can only narrow what a source has loaded.
Use cluster tags for practical subsets rather than browsing all 2,076 detailed tags:

| Cluster tag | Eligible API operations (excluding help) |
| --- | ---: |
| `web-probe` | 85 |
| `wordmint` | 68 |
| `edge-finance` | 48 |
| `edge-market` | 47 |
| `mediakit` | 38 |
| `locale` | 27 |
| `rollforge` | 26 |
| `synthforge` | 13 |
| `prooflayer` | 11 |
| `matchpoint` | 7 |
| `bestiary` | 6 |
| `retail` | 5 |
| `agentops`, `model-router` | 4 each |
| `browser-workflow` | 3 |
| `statline` | 1 |
| `compose` | 132 |

`tags = []` selects 525 API operations plus help. Clusters and finer tags can
overlap; the table counts each service's registry cluster. The full `--list-tags`
report includes aliases/exclusions because it describes the input document.
The registry does not mark every semantically similar endpoint as an alias, so
some overlap remains even after excluding all declared aliases.

```sh
target/debug/treazury --provider providers/agentutility/provider.toml --list-tools
target/debug/treazury --provider providers/agentutility/provider.toml --tags wordmint --list-tools
target/debug/treazury --provider providers/agentutility/provider.toml --tags synthforge --list-tools
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
- Browser-session is excluded because its optional screenshot mode is not described
  by a stable image envelope in the spec. QR-code-generate is excluded because it
  can return SVG as well as PNG. IPFS-fetch is excluded because its arbitrary-file
  response can switch between text, JSON and base64. Their aliases are also absent.
- Watch-page is excluded because it stores caller-selected watch IDs and server-side
  state for up to 365 days; its ownership/access semantics need review. The
  browser-workflow cluster, in contrast, analyzes supplied traces/HTML and returns
  advisory plans/diffs; those operations do not themselves execute browser actions.
- Six audit routes are excluded: db-migration-risk, dep-risk-summary,
  deploy-config-risk, production-readiness-score, prompt-injection-surface, and
  secrets-exposure-check. Their complete `oneOf` branches declare alternative
  repo/files properties; the current flattener cannot preserve those full branches.
  Do not replace them with the registry's empty object schemas. Required-only
  alternatives on keyword-suggest, wallet-label and company-intel-pack are retained.
- Compose routes are opt-in bundles executed upstream in a single request. Preserve
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
Update the exact allowlist only after reviewing new routes, and review golden
contracts alongside any fixture update. The snapshot does not refresh at runtime;
no custom registry adapter is introduced into serving or agent source registration.


## BlockRun and Claw402

BlockRun defaults to six synchronous chat/Exa/Grok search operations plus help.
The spec uses `/api/v1`, while the guide often uses `/v1` aliases; keep the spec
paths on the wire. Omit chat `stream` or set false and supply a small `max_tokens`.
Async image/video/voice jobs, binary audio, phone/sandbox ownership, trading and
paths containing literal example addresses/IDs are excluded. Do not reinterpret
those example-valued paths as templates without an authored schema. The selected
operations carry vendor prices; startup discovery is disabled.

Claw402 defaults to twelve market overview GETs plus help. Its larger catalog has
useful exact case-sensitive tags, but clearing/changing tags does not expand the
operation allowlist. Several RootData POST bodies are only `type: object` despite
parameter requirements in prose; do not expose these as correctly typed tools
without request-schema corrections. Account credits/model diagnostics and AI/media
are outside this compact default. Startup pricing discovery is disabled because
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
