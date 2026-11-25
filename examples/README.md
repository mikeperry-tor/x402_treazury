# Deployment examples

These examples are curated deployments, not full provider inventories. Provider
definitions under `providers/` remain broader and reusable; see
[provider caveats](../providers/CAVEATS.md) for support boundaries and
[provider maintenance](../docs/provider-updates.md) for the update workflow.

When updating an example deployment, follow the
[deployment acceptance procedure](../docs/provider-updates.md#stage-and-accept-changes):
review catch-all assignments and stale exact selectors, find replacements for
removed APIs, compare coverage before price, and leave adjacent reversible
comments when displacing a tool. Refresh dynamic-endpoint search keywords and
endpoint summaries whenever capabilities or routing change. Two providers backed
by the same upstream add gateway redundancy rather than an independent data
source.

| Example | Shape | Network | Initial funding |
| --- | --- | --- | --- |
| [managed.toml](deployments/managed.toml) | One listener, one wallet group, Exa, and agent-added APIs | Direct | One $2-3 small wallet; first wallet plus replacement spans $4-6 nominal |
| [comprehensive.toml](deployments/comprehensive.toml) | Four listeners, two wallet groups, curated sources, relay fallback, and persistent agent-added APIs | Tor | `web` starts at $2-3; both groups eventually span $4-6 nominal |
| [research_desk.toml](deployments/research_desk.toml) | Twelve task-oriented listeners, four wallet groups, broad research selection, and relay fallback | Tor | `general` starts at $2-3; all four groups eventually span $8-12 nominal |

Nominal USDC targets are not ZEC quotes; budget extra for Zcash network fees,
bridge minimums, relay calls, and later replacements. See
[README.md](../README.md) for the first-run walkthrough and
[README_ADVANCED.md](../README_ADVANCED.md) for the multi-listener runbook.

## Managed starter

[deployments/managed.toml](deployments/managed.toml) exposes Exa web search,
page contents, and provider-management tools on one authenticated `research`
listener backed by one rotating wallet group over direct networking. Exa's
catalog loads from its public OpenAPI spec, and agent-added APIs share the
`research` group (ephemeral unless `source_registry_file` is set). See the
[README quickstart](../README.md#quickstart-z-cash-wallet-with-dynamically-managed-tools).

The listener requires `X402_TREAZURY_MCP_TOKEN`, binds before background
funding starts, draws a randomized $2-3 nominal reserve, and caps each payment
at $0.50.

## Comprehensive example

[deployments/comprehensive.toml](deployments/comprehensive.toml) combines
curated research sources, four listeners, two wallet groups (`web` and
`company`), paid discovery relay fallback, and a persistent `toolbox` endpoint
for agent-added APIs. Its treasury state lives at `state/privacy`.

`web` supplies the initial shared payer and $0.10 cap. The first API call on
`company`, `social`, or `toolbox` (including a cap refusal) schedules deferred
funding for `company`; after `company`'s first credit, those three endpoints
switch permanently to `company`'s wallet and $0.50 cap.

Initial setup funds one $2-3 nominal wallet for `web`. Funding both `web` and
`company` totals $4-6 nominal before fees, and a single group's first wallet
plus its first replacement also spans $4-6 nominal (about $5 on average; allow
headroom up to $6). If a bridge minimum exceeds the $3 ceiling, `x402_treazury`
refuses the route rather than silently raising the ceiling.

## Research desk

[deployments/research_desk.toml](deployments/research_desk.toml) distributes
sources across twelve task-oriented endpoints and four wallet groups:

- `general` ($1.00 cap): `toolbox`, `web`, `science`, `shopping`, and `world`
- `personal` ($1.00 cap): `social`, `property`, and `travel`
- `health` ($0.50 cap): `health`
- `financial` ($1.00 cap): `business`, `finance`, and `security`

`general` serves as the initial shared payer ($2-3 nominal start; $8-12 nominal
across all four groups before fees and replacements). First API use activates
each deferred group independently; once credited, each group keeps its own
wallet permanently across restarts even if its balance later depletes.

Remote sources set `allow_catalog_failure = true`, so endpoints wait for
catalog attempts and serve healthy tools even if one provider is unreachable,
keeping configured filters intact. See
[framework integration](../docs/framework-integration.md) for degraded warming
and activation behavior.

| Endpoint | Port | Group | Scope |
| --- | --- | --- | --- |
| `toolbox` | 3010 | `general` | Provider management, raw calls, and agent-added APIs |
| `web` | 3011 | `general` | Search, retrieval, news, archives, and images |
| `science` | 3012 | `general` | Academic research, papers, funding, and biology and chemistry references |
| `shopping` | 3013 | `general` | Marketplace products, prices, offers, and Lens |
| `world` | 3014 | `general` | Geopolitics, weather, shipping, and physical-world data |
| `social` | 3015 | `personal` | People, contacts, and platform content |
| `property` | 3016 | `personal` | Parcels, local records, zoning, and site context |
| `travel` | 3017 | `personal` | Flights, awards, stays, transit, and destinations |
| `health` | 3018 | `health` | Care access, trials, drugs, and public health |
| `business` | 3019 | `financial` | Organizations, filings, jobs, funding, and reviews |
| `finance` | 3020 | `financial` | Equities, macro, crypto, and derivatives |
| `security` | 3021 | `financial` | Threats, breaches, and infrastructure checks |

Wallets group endpoints by what upstream providers learn about the caller, not
by topic. Separate addresses avoid direct payer reuse, though closely spaced
initial swaps can still correlate by funding time; first-use activation and
randomized timers weaken that signal without guaranteeing unlinkability.

## Wallet bootstrap and funding

Both multi-listener examples use `[wallet_bootstrap]` with `bootstrap_group`
and `deferred_groups`. Deferred allocations wait for first API use and
randomized 30-120 second funding timers; API calls never wait for swaps. Paid
catalog and directory relay fees stay on the bootstrap group across cutover and
never trigger deferred-group funding.

Run `wallet init` to create the treasury, fund its shielded receive address,
and optionally run `wallet bootstrap` before `serve` to fund only the bootstrap
group without starting listeners or catalogs. Stop `serve` before running
exclusive wallet commands; see the [wallet CLI reference](../docs/wallet-cli.md).

Comment out the entire `[wallet_bootstrap]` table to fund all declared groups
separately from the start. Existing funds and earlier on-chain links stay as
they are.

## Selection notes

These notes summarize why each example selects specific sources. Provider
connectivity, payment, and response boundaries live in
[provider caveats](../providers/CAVEATS.md).

### Web

- **Search and answers:** Exa is the primary general and news search source.
  Use `lite` for indexed search when its restricted inputs fit the task.
  `auto` is the general-search default. Content options can add charges.
  [Exa pricing and limits](../providers/CAVEATS.md#exatoml--exa-search-and-contents)
- **Page retrieval and rendering:** StableEnrich Firecrawl handles rendered
  pages and PDFs, APEX `page_markdown` is the cheaper JavaScript fallback,
  AGProxy provides raw residential fetches for non-JavaScript pages, and Exa
  Contents provides indexed text with freshness and history controls. Otto adds
  static page extraction (inspect output for incomplete JavaScript or bot
  challenges). NetIntel's page reader is excluded because it skips
  JavaScript-rendered content.
- **Trends, books, and media:** GLIM covers X trends, and Duoleads covers
  Google Trends series, comparisons ($0.045 vs. OpenVerbs $0.03, with explicit
  granularity, as-of timestamps, and partial-period flags; note that Duoleads
  and OpenVerbs weekly labels differ by six days for matching values, so do not
  merge them without aligning buckets), regional interest, and related queries.
  Atlas retains timelines, regional interest, related queries, and subjects as
  alternatives; all-time and sub-day windows are unqualified. OpenWeb Ninja
  provides book metadata and preview links (not full text), and Strale inspects
  C2PA media credentials.
- **Unselected:** SocialFetch remains bundled for opt-in review without being
  enabled in either multi-listener example. Vaaya is unselected in
  `research_desk.toml` and on the `web` and `social` endpoints of
  `comprehensive.toml` (where a 26-tool subset runs on `company`).

### Science

- **Literature and references:** 2s supplies scholarly search, citations,
  PubMed metadata (without abstracts), and Crossref and OpenAlex references;
  APEX adds OpenAlex search. FastAF searches Google Scholar by keyword or
  citation cluster. Additional tools cover biomedical grants, medRxiv
  preprints (not peer-reviewed), and NCBI, UniProt, GBIF, and PubChem
  references.
- **Limits:** Coverage is not a full OpenAlex author or work API and does not
  guarantee full text. Preserve per-source errors and publication dates.

### Shopping

- **Marketplaces and discovery:** OpenWeb Ninja covers search, product details,
  offers, Google Shopping, and Google Lens and OCR across Amazon, Walmart,
  eBay, Home Depot, Wayfair, and Costco.
- **Reviews and social:** Google Shopping, Walmart, Home Depot, and Wayfair
  provide product reviews; eBay seller feedback covers merchant vetting; Google
  Shopping store reviews cover store service; and ShopSocial shares Instagram,
  TikTok, YouTube, Facebook, Threads, Pinterest, and Telegram reads with the
  `social` endpoint. Domain-scoped Trustpilot company details and reviews live
  on `business`.
- **Limits:** Marketplace prices, availability, delivery, and sponsorship
  fields reflect snapshots rather than guarantees, identifiers and currencies
  vary by marketplace, and user reviews are not representative surveys.

### World

- **Maritime and weather:** Straits supplies the Hormuz shipping index and
  paged history (use `before` and `nextBefore` for older ticks and preserve
  cadence and resolution). Otto and 2s cover current weather and forecasts over
  shared Open-Meteo data, alongside aviation weather and hazards.
- **Physical world:** Additional tools cover energy, agriculture, satellites,
  disasters, and FCC geographies. Preserve observation timestamps and separate
  current observations from historical baselines.
- **Trade and energy:** SiteCheck covers U.S. and European power grids, EIA
  petroleum reports, WTI and Brent history, Suez and Panama canal traffic,
  Rhine water levels, and ACR and Verra carbon registries.

### Social

- **Platforms and analysis:** Glim covers X search, Reddit search, and GitHub
  metadata (Glim follower listing is unqualified). Botsmith adds account
  dossiers, watchlist polling, sentiment pulse, an audience read, and Kalshi
  prediction-market reads. 2s covers Hacker News items, feeds, and users.
  API Direct adds Bluesky and Truth Social posts, Substack publication posts,
  and public LinkedIn profiles. See the
  [API Direct paging and pricing caveats](../providers/CAVEATS.md#apidirecttoml--api-direct).
- **YouTube:** FastAF provides keyword search, including Shorts. ShopSocial
  provides channel videos, transcripts, and comments. The
  [deployment file](deployments/research_desk.toml) explains the pricing choice
  and shows how to restore AgData search for its richer video metadata.
- **Company and contact reads:** AgData and ShopSocial cover narrow LinkedIn
  company, Instagram, TikTok, and Facebook reads. AgData's comments,
  transcripts, and personal profile modes have less test coverage.
  CompanyRecords adds LinkedIn company facts, alongside person and contact
  providers for profiles, work email, and phone lookups.
- **Limits:** SocialFetch's long tail and Vaaya remain unselected on `social`
  (Vaaya is enabled only on `company` in `comprehensive.toml`). Preserve
  platform, date, and partial-result markers.

### Property

- **Local and market data:** Locus provides the free list and call gateway,
  place reports, and groundwater monitoring-well history (monitoring wells, not
  parcel-level private wells). Zillow and Redfin supply listings, estimates
  (not appraisals), and market context.
- **Public records:** Additional tools cover NYC PLUTO, ACRIS, DOB, and HPD
  records, flood and environmental data, zoning, and local-government records.
  Locus feedback and coverage-participation write endpoints are excluded.

### Travel

- **Flights and airports:** StableTravel covers Google Flights cash fares,
  FlightAware operations, and cached Seats.aero awards. 2s covers flight
  status, airport boards, and route schedules over shared FlightAware data, plus
  OurAirports IATA and ICAO lookup (replacing the unavailable FlightAware
  canonical lookup). HALOWERK provides route matrices. For nearby airports,
  FlightAware offers operational links and geographic fields, 2s adds country
  and scheduled-service filters, and OpenVerbs offers a kilometer-radius
  alternative with a smaller result cap and fewer operational fields.
- **Fare search:** FastAF covers one-way and round-trip fares. StableTravel
  also supports multi-city itineraries and additional filters.
- **Stays, transit, and local guides:** HealthTravel adds hotel prices, Google
  Flights fare observations, Wikivoyage discovery, and Amtrak; ShopSocial adds
  OpenStreetMap hiking trails (not user reviews) and FlixBus and FlixTrain
  one-way single-adult searches; Maxwell adds resort lodging, lift access,
  menus, rentals, and local tips; APEX adds cached Aviasales deals; and OpenWeb
  Ninja adds Yelp search, details, and reviews plus local and virtual event
  search.
- **Limits:** EV charger entries are directory records rather than live
  occupancy, Rappi menus require a store ID or URL, published fares and
  schedules are not held inventory, and community visa references should be
  verified with official authorities.

### Health

- **Care access and pricing:** FindMyDirectDoctor supplies the read-only
  healthcare-access catalog: doctor and clinic directories, membership fees,
  hospital cash prices, HRSA centers, financial assistance, and uninsured-care
  guides. For statewide DPC monthly plan distributions, both practice-level
  price statistics and family plans with `view=stats` stay selected because
  they expose different statistics and billing breakdowns.
- **Providers, plans, and codes:** WebMD covers provider and dentistry search,
  ADA adds ZIP-based dentist lookup, HealthCare.gov adds ACA plan data, EMA
  adds drug shortage records, HealthTravel adds PubMed citation metadata, and
  2s adds NPI and NPPES lookups.
- **Limits:** Hospital facility prices, Medicare benchmarks, drug acquisition
  costs, and pre-subsidy premiums are distinct pricing bases. Directories can
  be incomplete, HCPCS codes do not imply insurance coverage, and LOINC codes
  omit units and reference ranges.

### Business

- **Company registries and filings:** Strale covers UK charity records,
  Canadian and Swiss company profiles, Polish and Slovak ID lookups, EU
  regulation search, and MiCA checks against ESMA registers (preserve register
  dates; no match is not clearance). 2s covers NY, CO, and CT company records
  and FCC proceedings. CompanyRecords covers NY registries, weekly Swedish
  snapshots, Polish KRS extracts by known ID, and CourtListener opinions.
  Maxwell adds broader company discovery (court search stays on CompanyRecords
  and 2s).
- **Reviews, apps, patents, grants, and jobs:** OpenWeb Ninja covers
  domain-scoped Trustpilot details and reviews (company-name search is excluded
  due to entity ambiguity), Google Play listings, reviews, install and rating
  counts, and top charts (use repeated totals rather than chart ranks for
  adoption trends), plus JSearch and Glassdoor employer data. FastAF adds
  public LinkedIn job listings and Google Patents keyword search. Atlas Google
  Patents adds details, claims, and filters. 2s covers USPTO and EPO, and
  Maxwell supplies cached patent search. Document indexes list filings.
  Retrieve the documents for their full text and check filing records for
  current legal status. GenuineGood and Atlas cover federal and NIH biomedical
  grants.
- **Limits:** Resolve entity identity before enrichment and compare metrics by
  their start and end periods.

### Finance

- **Rates and yields:** RegimeShift is disabled while unreachable (kept
  commented in TOML for re-enabling). Horizon Pulse supplies free official SOFR
  and EFFR rates, Degenscan adds supply-yield pools and Deribit IV and DVOL,
  and Market Intelligence adds on-chain protocol supply and borrow APYs,
  utilization, and sizes. These observations do not replace Agent-SOFR
  premiums, forward-risk or max-LTV models, RFQ matching, or signed loan
  quotes.
- **Macro series:** 2s FRED supplies ex-autos retail sales via series `RSFSXMV`
  (advance monthly retail and food-service sales excluding motor vehicles and
  parts, seasonally adjusted, in millions of dollars). Fetch enough history to
  match prior-month and prior-year dates when computing percentage changes;
  current-vintage observations can be revised. AgentFund's additional monthly
  summary needs `body={}`; its advertised ex-autos label is unreliable, so
  prefer FRED for that series. See the [AgentFund
  boundary](../providers/CAVEATS.md#agentfundtoml--agentfund-us-economic-sec--on-chain-data-x402agentfundnet).
- **Crypto and on-chain:** Otto covers crypto news, market briefs, token data,
  and funding and open-interest reads; APEX adds venue-normalized funding
  boards, stablecoin pegs, prediction odds, tokenized-stock quotes, and X1
  reads. Blockchain Money Map covers wallet, transaction, and saved-report
  research; Arkham adds labels and transfers; Onchaincheck adds native and
  priced ERC-20 holdings (with explicit dust and unpriced exclusions,
  complementing Blockchain Money Map without replacing NFT, activity, or risk
  data); SmartPerps covers Hyperliquid cohort positioning. APEX funding boards
  and Otto token details supply narrower working market reads beside the
  excluded Kronos overview without replacing its model or forecast fields.
- **Equities, macro, and compliance:** 2s covers FIGI security resolution and
  SEC company facts on the research desk (the comprehensive example uses
  Horizon Pulse for company facts; compare reporting periods). Concordance
  provides multi-source series lookups. Banking Compliance provides advisory
  rule summaries (which can contain citation errors; `business` keeps
  authoritative eCFR retrieval, and compliance checks and drafting are
  excluded).

### Security

- **Threat and identity checks:** RelayShield covers email breach and
  infostealer lookups, identity graphs, vendor supply-chain risk, IP
  intelligence, ransomware risk, public secret exposure, domain identity risk,
  declared-stack CVE checks, counterparty wallet risk, SIM-swap checks (cached
  signals; carrier availability can prevent a result), and phone reputation via
  its phone companion. Bulk identity risk and campaign scanning exceed typical
  per-payment caps.
- **Network and phone lookups:** NetIntel covers phone validation, IP
  reputation, geolocation, WHOIS and RDAP, cloud detection, and technology
  fingerprinting. IPIntel adds IP lookup, and Otto covers DNS, WHOIS, and
  domain age.

### Cross-provider coverage limits

| Domain | Selected alternatives | Coverage limits |
| --- | --- | --- |
| Web, news, and rendering | Exa, StableEnrich, APEX `page_markdown`, AGProxy raw fetch | Preserve source dates, final destination, and partial results. |
| Papers | 2s scholarly search and citations, APEX OpenAlex search, science tools | No full OpenAlex author or work API or guaranteed full text. |
| SEC, court records, rules, nonprofits, and recalls | x402stock and 2s APIs | Query and pagination contracts differ; US recalls do not replace international recall authorities. |
| Archives | Exa `snapshotAsOf` | Stored historical page versions; no full CDX index or guaranteed archived text. |
| Social and company data | Glim X, Reddit, and GitHub search; Botsmith social analysis; 2s HN; AgData and ShopSocial reads; CompanyRecords LinkedIn facts; and contact providers | No complete SocialFetch platform or endpoint parity. |
| Patents | Atlas Google Patents, 2s USPTO and EPO tools, Maxwell cached USPTO search, x402stock patent tools | Document indexes and legal-event lists are not full document retrieval or current legal status. |

For Google Patents, use quoted technical phrases joined with `AND`, structured
assignee, office, and date filters, and verified classification codes. Free-text
ranking can be noisy; inspect ten results before broadening, and do not rely on
date sorting instead of a precise query. Google's
[search guide](https://support.google.com/faqs/answer/7049475?hl=en) covers
title, abstract, and claims fields and CPC descendants, which differ from EPO
CQL.

## Maintaining selections

Ordered `send_tools` rules group tool families, exclude unwanted operations,
and share tools across multiple listeners using destination arrays. Each source
ends with one catch-all rule that routes or excludes unmatched tools while
keeping endpoint capability and wallet bindings intact. See
[source tool distribution](../docs/configuration.md#source-tool-distribution)
for matching rules and `config check --explain-routing`.

Prefer bundled provider listings before remote discovery, which can incur
multiple paid charges. Wallet caps bound a single x402 challenge rather than a
multi-call workflow. Keep invocation prices and uncertain-payment warnings in
agent instructions, and keep transport fallback costs and vendor API path
details in operator documentation.

## Operator notes

Copy a tracked template beside itself as `NAME.local.toml` before editing.
Relative paths resolve from the file that declares them.

The starter example uses direct networking. The comprehensive and research-desk
examples expect Tor Browser's SOCKS listener at `127.0.0.1:9150`; adjust
`[network]` for a standalone Tor daemon. You can pass
[network/tor.toml](network/tor.toml) via `--network-config` to override
`[network]` (and `[relay_fallback]` when present) for one command, or copy it
into a deployment file. Tor resolves DNS remotely with no direct fallback, and
separate circuits do not hide identifying request content.

The comprehensive deployment, research desk, and Tor network override all try
readFetch ($0.003), Curl ($0.01), then AGProxy ($0.0156-$0.0372) for eligible
`GET`, `POST`, and `HEAD` requests. These prices are estimates. The x402
challenge determines the charge. Curl handles public text fetches and
skips requests with required headers or nonempty bodies. All three examples
leave API request relay fallback off.

Managed serving binds listeners before background funding starts. Initialize
the treasury with `wallet init` and fund its shielded receive address; tools
stay visible while funding completes. Reserve targets size nominal USDC
allocations, payment caps govern each signed payment, and spendable ZEC governs
funding. See [wallet administration](../docs/wallet-cli.md),
[configuration](../docs/configuration.md), and
[agent source management](../docs/agent-sources.md).

Static-wallet deployments use the same binary; see
[configuration](../docs/configuration.md). Operational Rust helpers live in
[tools/](../tools/README.md), and live qualification deployments and runner
instructions live in [tests/live/](../tests/live/INTEGRATION.md).
