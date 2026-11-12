# Deployment examples

These examples are maintained selections, not provider inventories. Provider
definitions remain reusable and broader; see [provider caveats](../providers/CAVEATS.md)
for support boundaries and [provider maintenance](../docs/provider-updates.md)
for the acceptance procedure. Follow the
[deployment acceptance procedure](../docs/provider-updates.md#stage-and-accept-changes):
review new catch-all assignments and stale exact selectors, find catalog replacements
for deleted APIs, compare useful coverage before price, and retain adjacent reversible
replacement comments when displacing a tool. Refresh dynamic-endpoint search keyword
hints and endpoint summaries with capability/routing changes. A backup sharing the
same upstream adds gateway redundancy, not an independent data source.

| Example | Shape | Network | Initial funding |
| --- | --- | --- | --- |
| [managed.toml](deployments/managed.toml) | One listener, one wallet group, Exa and agent-added APIs | Direct | One $2-3 small allocation; first replacement spans $4-6 nominal |
| [comprehensive.toml](deployments/comprehensive.toml) | Four listeners, two intended groups, curated sources, relay and persistent agent-added APIs | Tor | Web's $2-3 start; eventual two-group range $4-6 nominal |
| [research_desk.toml](deployments/research_desk.toml) | Twelve task-oriented listeners, four intended groups, broad research selection and relay | Tor | General's $2-3 start; eventual four-group range $8-12 nominal |

Nominal USDC sizing is not a ZEC quote. Cover fees, bridge costs, relays and
later replacements; bridge floors can bias draws upward. `README.md` owns the
first-run walkthrough, and `README_ADVANCED.md` owns the multi-listener runbook.

## Managed starter

[deployments/managed.toml](deployments/managed.toml) exposes Exa web search, page
contents and provider-management tools on one authenticated research listener and
one managed pool. Exa's catalog loads from its public specification. Agent-added
APIs share the research pool; registrations last until exit unless
`source_registry_file` is set. It uses direct networking. See the
[README quickstart](../README.md#quickstart-z-cash-wallet-with-dynamically-managed-tools).

The listener requires `X402_TREAZURY_MCP_TOKEN`. Serving binds the listener before
background funding; tools remain visible while funding is pending. The pool draws
a randomized $2-3 nominal reserve and permits payments up to $0.50.

## Comprehensive example

[deployments/comprehensive.toml](deployments/comprehensive.toml) combines curated
research sources, four listeners, two intended groups, paid discovery relay and a
persistent endpoint for agent-added APIs. Web supplies the initial common payer
and $0.10 payment cap. First company/social/toolbox API use activates later company
preparation; its first positive credit switches those contexts permanently to
company's payer and $0.50 cap. A cap refusal still records demand. It retains the
`state/privacy` treasury, registry and operational identities; renaming the example
does not migrate state.

Initial setup is web's one default $2-3 nominal small allocation. Both groups'
eventual initial inventory totals $4-6 nominal. Separately, preparing one group's
first small wallet and its later replacement spans two allocations: $4-6 nominal,
averaging approximately $5 before bridge-floor adjustments and fees. The first
payment makes READY become ACTIVE and opens the replacement vacancy. Allow
randomization headroom to $6. A nominal bridge minimum above the $3 ceiling can
refuse a route rather than silently increasing the bound.

## Research desk

[deployments/research_desk.toml](deployments/research_desk.toml) distributes
sources across twelve task-oriented endpoints with four intended groups. General
covers web, science, shopping, world and toolbox; personal covers social, property
and travel; health covers care research; financial covers business, finance and
security. General supplies the initial common payer and $1 cap. First API use
activates each participating group independently; after that group's first credit
it uses its own payer permanently. Previously credited groups route directly on
restart; depletion never returns them to general. Personal also has a $1 cap;
health changes to its own $0.50 cap; financial uses $1. Initial general setup is
$2-3 nominal, while the eventual four-group initial range is $8-12, before fees,
headroom and later replacements.

| Endpoint | Port | Group | Scope |
| --- | --- | --- | --- |
| toolbox | 3010 | general | Provider management, raw calls and agent-added APIs |
| web | 3011 | general | Search, retrieval, news, archives and images |
| science | 3012 | general | Academic research, papers, funding and bio/chem references |
| shopping | 3013 | general | Marketplace products, prices, offers and Lens |
| world | 3014 | general | Geopolitics, weather, shipping and physical-world data |
| social | 3015 | personal | People, contacts and platform content |
| property | 3016 | personal | Parcels, local records, zoning and site context |
| travel | 3017 | personal | Flights, awards, stays, transit and destinations |
| health | 3018 | health | Care access, trials, drugs and public health |
| business | 3019 | financial | Organizations, filings, jobs, funding and reviews |
| finance | 3020 | financial | Equities, macro, crypto and derivatives |
| security | 3021 | financial | Threats, breaches and infrastructure checks |

Wallets group endpoints by what the upstream learns about the caller, not by
topic. All initially share general's payer and cap. Separate addresses avoid
direct payer reuse, but closely spaced initial swaps still reveal a funding-time
correlation signal. First-use activation and randomized later opportunities can
weaken that signal; they do not guarantee unlinkability.

## Wallet bootstrap and funding

Both multi-listener examples use `[wallet_bootstrap]` with explicit
`bootstrap_group` and `deferred_groups`. New participant allocations wait for
first use and randomized 30-120-second opportunities; API calls never wait for
swaps. Paid metadata and directory relay fees stay on the bootstrap group across
API cutover and do not activate a group. Run `wallet init` for treasury setup,
fund the shielded receive address, and optionally run `wallet bootstrap` before
`serve` to prepare only the bootstrap group's enabled reserves without
catalogs/listeners. `serve` binds listeners before background funding. Exclusive
wallet commands require stopping serving; see the
[wallet reference](../docs/wallet-cli.md).

Comment out the entire bootstrap table for ordinary funding and separate groups
from the beginning. Existing funds and earlier links remain in place. Separate
initial addresses avoid direct payer reuse, but closely spaced swaps reveal a
funding-time correlation signal. Shared bootstrap payments have a visibly common
payer, and timing, amounts, providers, content and settlement can still correlate
activity. Delayed funding cannot erase prior links or guarantee unlinkability.

## Selection notes

These notes explain deployment choices. Provider connectivity, payment and
response boundaries live in [provider caveats](../providers/CAVEATS.md).

### Web

Exa is the primary general/news search and cited-answer source. StableEnrich
Firecrawl supplies rendered page and PDF reads; APEX page_markdown is the cheaper
JavaScript-only fallback; AGProxy adds raw residential fetches for
non-JavaScript pages. Exa Contents supplies indexed text and explicit
freshness/history controls. GLIM supplies X trends, Google Trends supplies interest
timelines, and Strale inspects C2PA media credentials.

Exa mode choice is advisory. Lite is $0.004 for indexed search with restricted
inputs and no live fetching; auto is $0.007 and remains the general-search
default. Choose coverage, freshness and quality before price; content options add
charges. No example forces lite. NetIntel's page reader is excluded because it
does not retrieve JavaScript-generated content. SocialFetch and Vaaya are not
selected; their reusable definitions remain available for deliberate review.

### Science

2s supplies scholarly search, citations, PubMed metadata and Crossref/OpenAlex
references; APEX adds OpenAlex search. Science also selects biomedical grants,
medRxiv listings, NCBI/UniProt/GBIF and PubChem references. This is not a complete
OpenAlex author/work API or a guaranteed full-text source. PubMed metadata lacks
abstracts; preprints are not peer-reviewed. Preserve per-source errors and
publication dates.

### Shopping

OpenWeb Ninja supplies marketplace search, product details, offers, Google
Shopping and Google Lens/OCR across Amazon, Walmart, eBay, Home Depot, Wayfair and
Costco. ShopSocial supplies Instagram, TikTok, YouTube, Facebook, Threads,
Pinterest and Telegram reads shared with social. Prices, availability, delivery
and sponsorship fields are marketplace state, not guarantees; identifiers and
currencies differ by marketplace. OpenWeb Ninja's Trustpilot search remains
unselected because company-name queries can return ambiguous entities;
CompanyRecords supplies the selected Trustpilot reviews on business.

### World

Straits supplies the Hormuz shipping index and paged history; use `before` and
`nextBefore` for older ticks and preserve cadence and resolution. Otto and 2s
supply current weather and forecasts over shared Open-Meteo data; aviation weather
and hazards stay here. World also covers energy, agriculture, satellites, disasters
and FCC geographies. Preserve observation timestamps and
distinguish current observations from historical baselines.

### Social

Glim supplies X search, Reddit search and GitHub metadata. Botsmith adds account
dossier, watchlist polling, sentiment pulse and an audience read; Glim's follower
listing remains unqualified. Botsmith also supplies Kalshi prediction-market
reads. 2s supplies Hacker News items, feeds and users. AgData and ShopSocial
supply narrow LinkedIn company, Instagram, TikTok, YouTube and Facebook reads;
comments, transcripts and personal-profile modes are separate and less qualified.
CompanyRecords adds LinkedIn company facts. Person and contact providers cover
profiles, work email and phone research. SocialFetch's long tail and Vaaya remain
unselected; this is not complete platform parity. Preserve platform, date and
partial-result markers; missing posts do not prove absence.

### Property

Locus supplies the free list/call gateway, place reports and broader groundwater
monitoring-well history; groundwater describes monitoring wells, not a parcel's
well. Zillow and Redfin supply listings, estimates and market context; estimates
are not appraisals. Property also selects NYC PLUTO/ACRIS/DOB/HPD records,
flood/environmental data, zoning and local-government records. Preserve record
dates and coverage. Locus feedback and coverage-participation writes are excluded.

### Travel

StableTravel supplies Google Flights cash fares, FlightAware operations and cached
Seats.aero awards; 2s supplies flight status, airport boards and route schedules
over shared FlightAware data; HALOWERK supplies route matrices. HealthTravel adds
hotel prices and Amtrak; ShopSocial adds hiking and FlixBus/FlixTrain; Maxwell
adds resort lodging, lift access, menus, rentals and local tips; APEX adds cached
Aviasales deals. EV chargers are directory data rather than live occupancy;
hiking is OpenStreetMap rather than user reviews. Flix searches cover one adult
one way. Rappi menus need a store ID/URL and cannot discover stores. Published
prices and schedules are not held inventory. Verify community-maintained visa
references with the destination authority.

### Health

FindMyDirectDoctor supplies the full read-only healthcare-access catalog: doctor
and clinic directories, membership fees, hospital cash prices, HRSA centers,
financial assistance and uninsured-care guides. WebMD adds provider search;
HealthCare.gov adds ACA plan data; EMA adds shortage records; HealthTravel adds
PubMed citation metadata; 2s adds NPI/NPPES lookups. Hospital facility prices, Medicare benchmarks, drug acquisition
costs and pre-subsidy premiums are distinct bases. Directories can be incomplete;
dentistry delivery is unqualified. HCPCS and LOINC are reference codes: existence
does not establish coverage, and LOINC results omit units and reference ranges.

### Business

Strale supplies UK charity records, Canadian and Swiss company profiles, Polish
and Slovak ID lookups and EU regulation search; MiCA checks are excluded because
the registers are unavailable. 2s supplies NY/CO/CT state company records and FCC
proceedings.
CompanyRecords supplies NY registry records, weekly Swedish snapshots, known-ID
Polish KRS extracts, CourtListener opinions and Trustpilot reviews. Maxwell adds
broad company and registry discovery; court search is selected through
CompanyRecords and 2s, not Maxwell. Atlas Google Patents, 2s
USPTO/EPO operations and Maxwell cached search cover patents; neither a document
index nor a legal-event list establishes full document retrieval or current legal
status. GenuineGood and Atlas biomedical grants cover federal and NIH funding.
OpenWeb Ninja JSearch and Glassdoor employer data stay selected on business; other
job boards and social modes remain unqualified. Resolve entity identity before
enrichment; compare facts by their start/end periods. Missing records do not prove
absence.

### Finance

Otto supplies crypto news, market briefs, market and token data and funding/OI
reads; APEX adds venue-normalized funding boards, stablecoin pegs, prediction
odds, tokenized-stock quotes and X1 reads. Blockchain Money Map supplies wallet,
transaction and saved-report research; Arkham adds labels and transfers. SmartPerps
supplies Hyperliquid cohort positioning. 2s supplies FIGI/security resolution.
Concordance supplies multi-source series lookups; Banking Compliance supplies rule
lookup only, while business retains authoritative eCFR retrieval. Checks and
drafting are omitted on information-quality grounds. 2s supplies SEC company facts
to the research desk; the comprehensive example uses Horizon Pulse for company
facts. Compare each fact's actual period.
Banking rule summaries are advisory and can contain citation errors.

### Security

RelayShield supplies email breach and infostealer checks, identity graphs, vendor
supply-chain risk, IP intelligence, ransomware risk, public secret exposure,
domain identity risk, declared-stack CVE checks, counterparty wallet risk and
SIM-swap checks. Its phone companion supplies phone reputation. NetIntel supplies
phone validation, IP reputation, geolocation, WHOIS/RDAP, cloud detection and
technology fingerprinting. IPIntel adds IP lookup; Otto supplies DNS, WHOIS and
domain age. Missing
findings and degraded results do not establish safety. Bulk identity risk and
campaign scanning exceed ordinary payment caps; SIM-swap signals are cached and
carrier availability can prevent a result.

### Cross-provider coverage limits

| Excluded research | Selected alternatives | Coverage limits |
| --- | --- | --- |
| Web/news and rendering | Exa, StableEnrich, APEX page_markdown, AGProxy raw fetch | Preserve source dates, final destination and partial results. |
| Papers | 2s scholarly search/citations, APEX OpenAlex search, existing science tools | No complete OpenAlex author/work API or guaranteed full text. |
| SEC, court records, rules, nonprofits and recalls | Existing x402stock and 2s APIs | Different query/pagination contracts; US recalls do not replace six-authority international coverage. |
| Archives | Exa `snapshotAsOf` | Stored historical page versions; no full CDX inventory or guaranteed archived-text retrieval. |
| Social and company evidence | Glim X/Reddit/GitHub search, Botsmith derived social analysis, 2s HN, AgData and ShopSocial platform reads, CompanyRecords LinkedIn company facts, existing contact providers | No complete SocialFetch platform or endpoint parity. |
| Patents | Atlas Google Patents search/details, 2s USPTO/EPO operations, Maxwell cached USPTO search, existing x402stock patent/company tools | 2s adds application detail, document indexes, bibliography, families and historical legal events. Neither a document index nor a legal-event list establishes full document retrieval or current legal status. |

For Google Patents, prefer quoted technical phrases joined with explicit `AND`,
structured assignee/office/date filters and verified classification restrictions.
Broad free-text ranking can be noisy; inspect ten results before broadening.
Google's [search guide](https://support.google.com/faqs/answer/7049475?hl=en)
documents title/abstract/claims fields and CPC descendants. These Google-native
operators differ from EPO CQL. Date sorting is not a substitute for a precise query.

## Maintaining selections

Ordered source `send_tools` rules group tool families, exclude editorial choices
and express sharing with destination arrays. Each source has one final catch-all
that assigns or excludes unmatched tools; endpoint capability and wallet bindings
remain intact. See [source tool distribution](../docs/configuration.md#source-tool-distribution)
for matching and `config check --explain-routing` inspection.

Prefer bundled provider listing before remote discovery; remote discovery can
incur several charges. Wallet payment caps bound each challenge, not the sum of a
research job. Agent guidance keeps invocation prices and uncertain-payment
warnings, while transport fallback costs and vendor API path mappings remain
operator concerns. Published schemas and estimates do not establish comparative
output quality, Tor delivery or funded settlement.

## Operator notes

Copy a deployment beside its tracked template as `NAME.local.toml` before
customizing it. Relative paths resolve from the file declaring them. Existing
local configurations remain operator-owned; keep using their original state and
identities unless deliberately changing them.

The starter uses direct networking. The comprehensive and research-desk examples
require a running Tor Browser SOCKS listener at `127.0.0.1:9150`; adjust their
`[network]` for your own Tor daemon. [network/tor.toml](network/tor.toml) can be
passed with `--network-config` to replace the whole network policy for deployment
or standalone commands, or copied into a deployment as its `[network]` table. Tor
resolves destination names remotely and has no direct fallback; separate circuits
do not hide identifying request content.

Managed serving binds listeners before funding. Initialize the treasury explicitly
and fund its shielded receive address; available tools remain accessible during
funding or discovery delays. Reserve targets are funding preferences, payment caps
authorize individual signatures, and actual spendable ZEC limits ordinary funding.
See [wallet administration](../docs/wallet-cli.md),
[configuration](../docs/configuration.md) and
[agent source management](../docs/agent-sources.md).

Static-wallet deployments remain supported in the same executable; see the
[configuration reference](../docs/configuration.md). Operational Rust utilities
live in [tools/](../tools/README.md), and qualification deployments and runner
instructions live in [tests/live/](../tests/live/INTEGRATION.md).
