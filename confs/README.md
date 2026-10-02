# Generic launcher confs

Ready-to-use JSON conf files for `x402-mcp-generic`. Run one as its own MCP
server:

```bash
uv run x402-mcp-generic --config confs/botsmith.json          # stdio
uv run x402-mcp-generic --config confs/botsmith.json --list-tools   # inventory, no wallet/network
```

Wallet/spend-cap setup is shared with every launcher (`EVM_PRIVATE_KEY`,
`X402_MAX_PRICE_USD`, `--env-file`); see the repo README. Field semantics and
the full key list are documented in the repo README ("Generic launcher").
`tests/test_confs.py` pins the socialfetch, pdl, deepline, kronos, regimeshift, glassnode,
concordance, agentfund, otto, straits, brazilayer, locus, lonestar and
genuinegood confs against committed spec snapshots (`tests/fixtures/`) and
committed digests (`confs/glassnode/`, `confs/concordance/`,
`confs/straits/`).
LoneStarOracle is available through `lonestar.json` as well as its custom
launcher with group selectors.

## socialfetch.json — SocialFetch

Uses the live OpenAPI document's platform tags, with administrative `Auth`,
`Monitors` and `System` excluded. All platforms are included by default;
`--tags Twitter,YouTube` selects a subset. `LinkedIn` spans v1 and v2; add
`--include /v1` when only v1 is wanted. Tags are exact and case-sensitive.

`include: ["/v1", ""]` strips `/v1` from tool names while the empty root fallback
retains other API versions: `socialfetch_twitter_profiles_handle` and
`socialfetch_v2_linkedin_*`. Original request paths remain intact; keep the base
URL at `https://api.socialfetch.dev`. This naming differs from the custom
SocialFetch launcher. The config works with both Python generic and Rust.

Credit pricing comes from `x-socialfetch-credits-pricing`, supplemented by
credit-to-USDC guidance in `instructions_text`. Pricing probes are disabled;
no endpoint sweep is needed. Fetching the live spec still requires one startup
request. Use a 90-second timeout for slow search/transcript calls.

`tests/fixtures/socialfetch_openapi.json` pins the 2026-10-02 source: 259
operations, 237 selected tools including Yelp. It retains tags, descriptions,
request schemas and vendor extensions, omitting response documentation only.
Regenerate it with the Rust development utility (replace the URL with a local
spec path for offline operation):

```sh
cargo run --locked --manifest-path rust-prototype/Cargo.toml --example snapshot_spec -- \
  https://www.socialfetch.dev/openapi.json tests/fixtures/socialfetch_openapi.json
```

The custom launcher's committed digest is independent and unchanged.

## deepline.json — Deepline GTM (stable-deepline.dev)

GTM contact lookup: 11 POST tools mirroring the vendor's OpenAPI spec —
prices via `x-payment-info`, requireds, the ads `platform`/`media_type`
enums, waterfall-provider detail in summaries. Tool schemas are closed via
`additional_properties: false`. Waterfall endpoints are slow — run with
`--timeout 120`.

```bash
uv run x402-mcp-generic --config confs/deepline.json --timeout 120
```

## pdl.json — People Data Labs (stablepeopledata.dev)

Person/company enrich + search (4 POST tools). The spec carries prices
(dynamic `min`/`max` for the size-scaled search endpoints) and full param
docs; `overrides` restores the two semantic notes the spec lacks — "free on
no match" on the enrich tools and "free when the result set is empty /
recommended size 1-5" on the search tools — plus the curated `min_likelihood`
and `size` hints. Tool schemas are closed via `additional_properties: false`.

## botsmith.json — x402.botsmith.dev

Market-signal API for trading agents (X/Twitter intel, Polymarket, perp
funding/OI, stablecoin pegs). All routes are flat-price GETs ($0.015/call at
the time of writing), so no `pricing_key` is configured: the launcher probes
each route once at boot with an unpaid GET (expect `402` + challenge, $0
spent) and renders the live price into every tool description, cached for the
process lifetime.

## x402stock.json — x402stock.xyz

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

## google-trends.json — Google Trends SEO keyword data (x402 Atlas)

Four GET endpoints on [google-trends.use.x402atlas.com](https://google-trends.use.x402atlas.com):
interest over time ($0.05/call, worldwide), interest by region, related
queries and related topics ($0.03/call each, per-country). Prices surface
from the `x-payment-info` extension, so boots make zero probe requests.
Guidance: long vendor descriptions describe each route's limits (empty
result sets, established search terms) — never re-enable trimming for this
spec; `help_url` serves the vendor `llms.txt` via `google_trends_help()`.

## arkham.json — Arkham Intel x402 rail (api.arkm.com/x402)

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
  challenge instead of a payable x402 quote — the launcher conf **excludes**
  them, since this server cannot answer that challenge.
- The API host sits behind Cloudflare and 403s some non-browser clients
  (notably curl from datacenter IPs); the launcher's spec fetch has worked
  reliably, but if a boot fails with an HTML/403 spec error, snapshot the
  catalog from a browser and point `spec` at the local file.

## regimeshift.json — RegimeShift (regimeshift.xyz)

Agent-SOFR (decentralized USD short-rate benchmark), variance-aware max-LTV
and ETH/BTC volatility risk premium at **$0.001/call** each (`x-payment-info`
in the spec), plus a **free** on-chain RFQ clearinghouse: lend/borrow intents,
open intents, matches with EIP-712 signed quotes, active/liquidatable loans,
loan registry. The spec's `servers` block is relative, so the conf pins
`base_url` to `https://regimeshift.xyz/api`; free endpoints carry overrides
documenting that they cost nothing (the launcher would otherwise render its
default "paid per call" line). The `/v1/intent/{id}/match` tool long-polls up
to 300s — run with `--timeout 320`.

```bash
uv run x402-mcp-generic --config confs/regimeshift.json --timeout 320
```

## kronos.json — Kronos Crypto Data (kronossignals.com)

Crypto market data for trading agents: 36 GET tools under `/api/v1` — per-asset
signals/price/ohlc/volatility/liquidation maps (`{asset}` paths, 16 spot
assets + HYPE derivatives-only), market-wide overview/scan/funding extremes,
BTC/ETH options (GEX, IV surface, implied probabilities), ML forecasts
($0.05) with a **free** `kronos_track_record` accuracy audit, macro context,
and the $0.10 all-in-one `briefing`. The include anchors on `/api/v1` so the
`v1` path segment never leaks into tool names (the vendor's free `/api/stats`,
`/api/health`, `/api/methodology` top-level routes are not exposed). Prices
live in vendor prose (the spec's `x-x402` extension has the wrong shape for
`pricing_key`), so boot probes price the untemplated routes live; an override
marks `kronos_track_record` free.

## concordance.json — Concordance (concordancehq.duckdns.org)

Cross-source research aggregator: 18 refreshed-on-schedule free and keyed
sources (SEC EDGAR, FRED, World Bank, UN Comtrade, GDELT news, CourtListener,
govinfo, arXiv/CORE, Hacker News, Stack Exchange, Wikipedia, weather,
DefiLlama, finnhub/twelvedata/polygon quotes) surfaced as six paid POST tools
($0.01–$0.05/call, USDC on Base or Solana). The vendor publishes both an
OpenAPI spec and an `llms.txt`; the spec's request bodies are FastAPI `$ref`s
(these deref fine since the flatteners learned local refs) but the spec
carries **no pricing at all** and POSTs are never probed — so the conf points
at a derived digest, `confs/concordance/concordance_digest.json` (refs
inlined, prices embedded as `pricing` blocks taken from the vendor llms.txt /
live 402 gate). `tests/fixtures/concordance_openapi.json` snapshots the vendor spec
and the test rebuilds the digest from it, so regeneration is pinned. Boot
makes zero probe requests (all paid routes are POST, and the probe is
GET-only by design). The free utility routes (`/healthz`, `/catalog`,
`/llms.txt`) are not exposed: `/catalog` would name `concordance_root` under
the include-anchor naming rules, so the source slugs and example series ids
it discovers live in the series tool's param notes instead, and `help_url`
serves the vendor `llms.txt` as `concordance_help()`.

## agentfund.json — AgentFund US Economic, SEC & On-Chain Data (x402.agentfund.net)

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
digest, no overrides, and makes zero probe requests. No vendor `llms.txt`;
the spec's top-level `x-guidance` is condensed into `instructions_text`
instead. (The vendor also serves these tools natively over MCP at `/mcp`;
this conf exists so the wallet/spend-cap story stays in our launcher.)

## otto.json — Otto AI x402 swarm (x402.ottoai.services)

The largest spec in this directory: 92 operations → **87 exposed tools**
(four routes ship GET+POST variants and get `_get`/`_post` names), an
autonomous agent swarm selling market/token intelligence, DeFi & markets
data, web/domain intelligence, real-world data (FX, weather), AI creative
tools (image/video gen, research, tx explainer), portfolio reads, a
meta-intelligence router ($0.001) and a feedback/refund route (needs the
original payment tx hash; vendor-capped at $0.01). Every op carries a
correct-shape `x-payment-info` (fixed and dynamic min/max entries), so
prices render from the **live vendor spec** with zero probes and zero
overrides; where the vendor prose already states its own price the rendered
line dedupes away (31 tools at time of writing) — every tool still shows a
price exactly once. Absolute `servers` URL, inline POST bodies. No llms.txt
on the API host (`docs.useotto.xyz/llms.txt` is a docs-site index whose
links the agent cannot follow), so the spec's `x-guidance` is condensed
into `instructions_text`. Caveats:

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

## brazilayer.json — Brazilayer registry & market data (api.brazilayer.com)

Official Brazilian registry and market feeds: Receita Federal CNPJ registry
(registration, partners, sanctions/integrity screening, Central Bank and CVM
checks), PNCP public tenders, EUDR deforestation profiles, and
commodity/macro/news datasets — **53 GET tools**; response keys use Portuguese
legal terms (`razao_social`, `situacao_cadastral`, …). Every paid route states
its price in prose ($0.001–$0.10); the ten routes sharing one
`components/parameters` `$ref` for `{cnpj}` get that path argument through the
digest flattener's op-level `$ref` parameter deref. The sixteen free routes
(`/v1/health`, CNPJ/CPF validators, twelve `*_amostra` dataset samples) carry
free-semantics overrides; `overrides` also restores the price the vendor's
long integridade description drops and completes two `nome` param descriptions
the vendor spec truncates with malformed keys. Pinned against
`tests/fixtures/brazilayer_openapi.json`.

## genuinegood.json — Genuine Good Grants (genuinegood.online)

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
closed via `additional_properties: false`. Caveats:

- `/v1/grants/pass` ($15 non-renewing 30-day pass) is **excluded on
  purpose**: the pass buys bearer-token (`Authorization` header) access,
  which this server cannot attach to vendor calls (headers are not
  agent-settable; x402 replaces auth), so activating it would be $15 for a
  token no tool can use.
- preflight ($5.00) exceeds the default `X402_MAX_PRICE_USD=1.00` spend cap
  and is rejected pre-sign at the default — raise the cap to use it.
- The vendor guidance prefers the POST forms; the GET twins accept the same
  input as query parameters, but the launcher sends array query values as
  repeated keys while the vendor documents comma-separated values — treat
  the POST twins as canonical.
- No llms.txt overflow to worry about: `help_url` serves the vendor's compact
  `llms.txt` (per-route prices, payment contract, free preflight sample) as
  `genuinegood_help()`, and `instructions_text` condenses the spec's
  top-level `x-guidance` — noting that the 402/sign/retry dance the vendor
  describes for raw HTTP clients is automatic here, plus the research-aid
  disclaimer. Pinned against `tests/fixtures/genuinegood_openapi.json`.

## locus.json — Locus US property & public-records context (api.locus.report)

110-operation committed digest (`confs/locus/locus_digest.json`, rebuilt by
`scripts/build_locus_digest.py`, pinned byte-identical against
`tests/fixtures/locus_openapi.json`): **108 paid POST tools** with prices
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
  `webhookUrl` jobs an MCP agent cannot poll, and the retrieval route is not
  in the spec. Response headers (X-Locus-Report-*) are not surfaced over MCP.
- Slow: place-report can take tens of seconds, environmental-context 60–90s.

```bash
uv run x402-mcp-generic --config confs/locus.json --timeout 120
```

## straits.json — Straits.live Hormuz monitor (straits.live)

Strait of Hormuz crisis data in two tiers, exposed as **33 tools tagged
`Free` / `Premium` in a hand-built digest** (`confs/straits/straits_digest.json`)
so the launcher's tag selection mounts tiers per server (`--tags Premium`,
`--tags Free`; `--list-tags` inventories them). The vendor spec
(`straits.live/openapi.json`, snapshot in `tests/fixtures/`) covers only the
premium side — 13 ops, all tagged `Premium`, correct-shape `x-payment-info`
($0.01–$0.50) — from which the digest derives the 10 GET data routes; the
digest hand-authors the 23-route free tier (`/status`-style summaries,
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
descriptions that Coinbase's facilitator rejects; `payment.py` trims
challenge descriptions client-side, so they settle like the rest.

## lonestar.json — LoneStarOracle catalog (lonestaroracle.xyz)

A pure-conf alternative to the `x402-mcp-lonestar` launcher, reading the
vendor's **unified spec** (one fetch, `openapi.json`, root server =
`https://lonestaroracle.xyz/api`). The gateway proxies every service
payment-gated — verified 402-identical to the per-service subdomains — so no
per-op URL handling is needed (the spec's `x-direct-url` extension is
informational). 96 tools over 71 services; the vendor's `authMode: free`
markers render as "Free — no payment required (vendor spec)." via the
launcher's pricing-renderer, and the audit POSTs' `$ref` bodies deref through
the shared flatteners. Zero boot probes (every op is spec-classified).
Exclusions: the digest build's curated suffix rules (demos, previews,
per-service `/feed` pages, subscription management, unpriced LeaseEdge
extras) are spelled out as 31 concrete paths — new vendor preview/feed routes
would appear as tools until the list is extended. Group selection
(`--groups macro,crypto`) remains a launcher-only feature; use
`x402-mcp-lonestar` when you want curated subsets, this conf when you want
the whole catalog off the live spec. `help_url` serves the vendor `llms.txt`
(one-fetch catalog, per-service prices, subscribe notes) as
`lonestar_help()`.

## glassnode/ — Glassnode x402 gateway (x402.glassnode.com)

Glassnode publishes no OpenAPI spec; this pair works around it:

- `glassnode_digest.json` — a small **hand-authored digest** of the four
  gateway routes (the gateway mirrors `api.glassnode.com` paths one-to-one):
  one templated metrics tool (`/v1/metrics/{category}/{metric}`, **$0.05**)
  plus the metadata discovery trio (assets / metrics / metric, **$0.01**
  each). Prices are embedded as `pricing` blocks, so boots make zero probe
  requests.
- `glassnode.json` — the conf: pins `base_url` + `prefix`, carries the
  discover→metadata→fetch workflow in `instructions_text`, and serves the
  vendor's x402 doc page via `glassnode_help()` (`help_url`).

The agent flow is: list metric paths → fetch one metric's allowed
parameters/interval values → pull data. Metric paths are two-segment
(category/metric); deeper sub-path variants (e.g. `/bulk`) are not addressable
through the templated tool.
