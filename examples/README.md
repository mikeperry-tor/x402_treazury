# Deployment examples

Start with [deployments/managed.toml](deployments/managed.toml): one authenticated
MCP listener, one managed wallet pool, Exa web search and page contents, and
source management for agent-added APIs. Exa's catalog loads from its public
specification. Added APIs share the research pool; registrations last until exit.
The [README quickstart](../README.md#quickstart-z-cash-wallet-with-dynamically-managed-tools) walks through
building, treasury setup and serving.

[deployments/comprehensive.toml](deployments/comprehensive.toml) combines curated
research sources, four listeners, two named pools, paid discovery relay and a
persistent endpoint for agent-added APIs. Its relationship map explains sharing.
It retains the `state/privacy` treasury, registry and operational identities of
the original privacy template; renaming the template does not migrate state.

[deployments/research_desk.toml](deployments/research_desk.toml) distributes
its research sources across eleven task-oriented endpoints using four topical
wallet profiles: web for web and science; social for health and social; financials for business, finance,
property and security;
and global for world, travel and the toolbox. Ordered source `send_tools` rules group
tool families, exclude editorial choices and express sharing with destination
arrays. Each source has one final catch-all that assigns or excludes unmatched
tools; listeners retain endpoint properties,
instructions, capabilities and wallet references. See
[source tool distribution](../docs/configuration.md#source-tool-distribution) for
matching, potential bindings and `config check --explain-routing` inspection.

The research desk prefers Exa for general/news search and retains Otto's topical
crypto news and linked market recaps as complementary coverage. Topical reports
lack citations; recap generation time can be newer than the underlying news window.
Otto and StableEnrich supply rendered page reads and cited answers. NetIntel's
page reader is excluded because it did not retrieve JavaScript-generated content.
AgentUtility supplies static HTML links and schema-based static-page JSON, with
Botsmith X search shared across web/social. Otto DNS, WHOIS and domain age remain
available alongside NetIntel DNS/RDAP on security; matching sampled data did not
establish an advantage from removing Otto. NetIntel's DKIM field is nullable and
does not establish complete selector discovery.
Exa retains explicit freshness/history controls. Vaaya and SocialFetch are fully
excluded in this deployment after cross-family settlement/lookup failures. On
2026-10-28, the latest Base settlements listed by
[402radar for Vaaya](https://402radar.io/vaaya.ai) were 18 days old and
[SocialFetch](https://402radar.io/api.socialfetch.dev) three days old. The operator
had observed at least 14 days without Vaaya payments. These are dated external
observations, not proof of universal outage or the settlement outcome of our
calls. Reusable provider definitions remain available for deliberate retesting.
Selections belong here, not in the reusable providers.

APEX's two rendered readers retrieved the JavaScript test page, and its research
search returned credited OpenAlex and Hacker News results. Research search is
shared with science/social; it does not replace full paper retrieval or discussion
threads. AgData company enrichment returned the requested LinkedIn company; Instagram posts,
TikTok videos, YouTube channel videos and Facebook page details also delivered
records. These narrow platform operations are enabled; separate comments,
transcripts and personal-profile modes remain unqualified.
AgentUtility closest-date archive lookup and 2s Hacker News search also succeeded.
2s scholarly search returned PubMed results with an explicit arXiv failure; preserve
per-source errors. These narrow workflows are enabled, rather than whole catalogs.

| Excluded research | Selected alternatives | Coverage limits |
| --- | --- | --- |
| Web/news and rendering | Exa, Otto, StableEnrich, APEX; AgentUtility JSON/links | Preserve source dates, final destination and partial results. |
| Papers | 2s scholarly search/citations, APEX OpenAlex search, existing science tools | No complete OpenAlex author/work API or guaranteed full text. |
| SEC, court records, rules, nonprofits and recalls | Existing x402stock and 2s APIs | Different query/pagination contracts; US recalls do not replace six-authority international coverage. |
| Archives | AgentUtility `archive_snapshot` | Closest snapshot lookup; no full CDX inventory or qualified archived-text read. |
| Social and company evidence | Botsmith X, StableEnrich Reddit, Agent402 GitHub repository metadata, 2s HN, AgData LinkedIn company/Instagram/TikTok/YouTube/Facebook reads, existing contact providers | No complete SocialFetch platform or endpoint parity. |
| Patents | Atlas Google Patents search/details, all seven 2s USPTO/EPO operations, Maxwell cached USPTO search, existing x402stock patent/company tools | Atlas returned 23 claims; 2s adds application detail, document indexes, bibliography, families and historical legal events. Neither a document index nor a legal-event list establishes full document retrieval or current legal status. |

Atlas Google Patents remains enabled. Its search mapping omits
`/results/*/figures` and discloses removals in the result advisory, preserving
patent records and pagination. A sampled 100-result CUDA/Nvidia response shrank
from 829,409 to about 96,306 characters after figure inventories were removed;
this is not a context-size guarantee. Patent details are unchanged. An unmapped
raw search can retrieve figure metadata as a separate explicit call that may be
paid. See [JSON response projection](../docs/configuration.md#json-response-projection).

For Google Patents, prefer quoted technical phrases joined with explicit `AND`,
structured assignee/office/date filters and verified classification restrictions.
Broad free-text ranking can be noisy; inspect ten results before broadening.
Google's [search guide](https://support.google.com/faqs/answer/7049475?hl=en)
documents title/abstract/claims fields and CPC descendants. These Google-native
operators differ from EPO CQL. Date sorting is not a substitute for a precise query.

Agent402 repository metadata succeeded. NetIntel and AgentUtility repository
health calls also succeeded, but their different scores and contracts are
candidates rather than silently interchangeable data. The 2s GitHub replacement
failed upstream authentication and remains excluded. HireScrape's one-record
Instagram profile, LinkedIn company, YouTube video and TikTok profile calls all
failed with upstream HTTP 401 `Token expired`; TikTok also reported settlement
before failure. Its social tools remain excluded. These failures do not prove
that every small mode is asynchronous. A separate Indeed-only, one-record jobs
request completed inline in 37 seconds at a reported $0.052; jobs stays selected.
Its `sd_sync_` snapshot identifier accompanied complete items and required no
polling. Other job boards remain unqualified. AgentUtility patent keyword search
failed upstream USPTO 403.
Maxwell returned ten cached USPTO records for its documented lithium-battery
query after the payment compatibility fix. A separate CUDA query returned
`not_cached` with queued retrieval; it was not automatically repeated. This is
not an empty search or independently verified free failure. Check `fetchedAt`.
All five additional 2s patent operations delivered records: application detail
and a 72-document index for US application 11627892, and bibliography, six family
publications and 50 legal events for EP1000000. The family included EP, AT, DE,
NL and US publications. Legal rows were unsorted and often lacked jurisdiction;
complete worldwide coverage and current status were not established.
Other gaps include GitHub profiles/trending, GDELT timelines, Glassdoor employer
reviews and customer-review sources beyond the qualified Trustpilot sample, TheirStack technographics and buying intent, licensed Tako data cards and
SocialFetch's long tail of social platforms. Do not silently substitute ordinary
web search for those structured datasets. The bundled FreeQ provider is available
for discovery: static Markdown worked, JavaScript-generated content did not; it is
not selected as another rendered-reader replacement.

The travel listener at port 3019 shares the existing global wallet. Qualified
2s calls supply destination advisories and community-maintained visa references.
Maxwell returned 354 Austrian resorts, 27 published 2026/27 ticket-price rows for
Kitzbühel and dated base/summit forecasts; lift and station components may be
unavailable. Preserve dates, seasons, price channels and source links. Published
prices are not live inventory; entry requirements require destination-authority
verification. Flight prices, operational lookups, airport references and route
matrices also belong on travel; aviation weather and accident research stay on world.

Business coverage includes Strale UK charity records, 2s NY/CO/CT state company
records and FCC proceeding filings. CompanyRecords supplies NY registry records,
Swedish weekly snapshots, known-ID Polish KRS extracts and CourtListener opinions.
Its court search matched all 20 sampled 2s records and added snippets at a lower
advertised price; 2s and Maxwell opinion searches remain explicitly excluded backups.
2s retains CO/CT coverage, Strale retains fresher direct Swedish lookups, and Maxwell
retains Polish name discovery. These broader contracts remain selected alongside
CompanyRecords; Maxwell's current payment-service failure does not qualify delivery. 2s adds HN
items/feeds/users, FCC spectrum geographies and FIGI/security resolution. Its
issuer-level ISIN list may include bonds rather than identify the input share.
Strale MiCA checks reported unavailable registers despite HTTP success; they
remain excluded. 2s HICP returned 2025-12 as latest in October 2026 and remains
unselected. Neither payment success nor a false match field proves data coverage.

FindMyDirectDoctor adds its full read-only healthcare-access catalog on health:
doctor and clinic directories, membership fees, hospital cash prices, HRSA community
health centers, financial assistance and uninsured-care guides. Doctor search,
hospital prices, financial assistance and health-center reads delivered records;
those samples do not qualify every route or independently verify listing accuracy.
See the [provider caveats](../providers/CAVEATS.md#findmydirectdoctor) for coverage
and price interpretation.

Seven further bundled providers add narrow, qualified selections. Atlas Biomedical
Grants supplies all seven NIH research operations on science, sharing project
search/details and organization funding with business. Atlas Legal Entities adds
GLEIF search, registration and accounting parents on business; this is gateway
redundancy, not independent ownership evidence. SmartPerps adds a free preview
and paid Hyperliquid cohort lite/detail/scan reads on finance. Public wallet
lookup remains unqualified and unselected. Preserve snapshot coverage and age.

SiteCheck ShopSocial adds Instagram/TikTok profiles, YouTube channel feeds,
transcripts/comments, Facebook Page Plugin facts, Threads profile/posts,
Pinterest pins and public Telegram channel previews on social. AgData remains
selected alongside it. CompanyRecords adds LinkedIn company facts shared with
social/business and Trustpilot reviews on business, alongside the registry and
court reads above. WebMD provider search and HealthCare.gov ACA plan data belong
on health. WebMD delivered family-medicine listings with missing contacts and
related specialties; dentistry failed upstream, and ADA searches returned no
records in two cities, so ADA remains unselected. ACA data preserves plan year,
published age and premiums before subsidies. These samples do not qualify every
reusable operation. Review disclosures and verification markers remain part of
the evidence.

HealthTravel adds Numbeo city-price observations and Rome2Rio intercity options
on travel, and EMA supply-shortage records on health. Preserve raw price text
when normalized currency is missing, partial route previews and resolved versus
active shortage statuses. Its Google Flights sample returned no fares; Wikivoyage
failed upstream 503. Both remain excluded. WebIntel adds news aggregation and
GDELT Context article matches on web; the sample had a Google timeout alongside
Bing/GDELT results. It does not supply general GDELT volume/tone timelines.

The four SiteCheck Workers catalogs share an operator contact and payment wallet,
so they do not establish four independent fallback vendors. Their primary schemas,
guides and sampled paid operations were live; the Workers hostname itself does
not establish durability. Radar's shared-recipient activity is not endpoint-level
paid-delivery proof. Full reusable catalogs stay available through local-provider
discovery. Research_desk also selects contract-reviewed travel companions after
representative delivery tests; this does not qualify every operation. No new
wallet profile is introduced. See [provider caveats](../providers/CAVEATS.md)
for source provenance, qualification boundaries and observed failures.

A full-schema travel inventory covered all 61 bundled providers. StableTravel
Google Flights returned 22 priced JFK–LHR itineraries for a one-way date, and its
FlightAware departure read returned 15 JFK records. 2s flight status, airport
board and route-schedule samples returned real flight records. These operations
now live on travel with 2s airport lookup/nearby and HALOWERK route matrices;
generic geocoding is shared with world. StableTravel and 2s flight operations
share FlightAware, so gateway redundancy is not independent upstream redundancy.
The fare search uses a separate Google Flights/SerpAPI workflow. A later award
search returned ten cached availability records; details for one record returned
12 itineraries and five booking links. All four Seats.aero reads and operational
FlightAware reads are selected on travel, including history, tracks, delays and
disruption counts. Intent writes and overlapping weather are excluded. Cash
booking remains excluded: the sampled search returned booking_token while its
booking route expects departure_token. Published observations do not hold tickets.

Representative travel qualification made nine calls across five providers:
StableTravel award search/details; HealthTravel dated hotel prices and Amtrak;
APEX cached flight deals; Maxwell resort lodging/access; ShopSocial hiking and FlixBus. All
returned useful records. Paris hotel search preserved the requested two-night
stay and returned three EUR prices; Amtrak returned two trains from eleven NYP
matches. FlixBus returned 21 Berlin–Munich trips for the requested date with
EUR fares and fees. APEX returned 30 cached London-origin deals. Maxwell returned 186
Kitzbühel lodging records and 40 lift/ticket entry points plus rail, bus and
parking context. Hiking returned the 153.45 km West Highland Way from OSM.
These are delivery observations, not independent settlement or booking guarantees.

Travel also selects contract-reviewed companions: hotel/hostel/vacation-rental
sources and reviews, regional rail/transit, FlixBus/FlixTrain, EV-charger
directories, Michelin and Grubhub restaurants, Rappi menus, ski operators and pass quotes, resort dining/menus, rentals, ski
schools, attractions and local mountain tips. Maxwell local tips advertise $1
per call and currently cover Slovakia; the name does not mean financial tips.
Resort proximity uses approximate map distances rather than routed walking time.
APEX deals are roughly 48-hour cached Aviasales fares with affiliate links;
hiking is Waymarked Trails/OpenStreetMap rather than AllTrails user content;
EV chargers are OSM/NREL directory records rather than PlugShare live occupancy.
FlixBus/FlixTrain searches cover one adult one way; Rappi menus require a known
store ID or URL and supply no store search. Grubhub discovery covers US locations.
SiteCheck hotel city-page samples are not date-specific availability; use dated
Google Hotels/Airbnb reads for stay dates. No hotel-loyalty API was found.

Only representative calls were tested, not every selected companion operation.
HealthTravel Google Flights (empty) and Wikivoyage (503) remain excluded; aviation
weather stays on world. Maxwell bulk prices/trip bundles repeat selected component
data and remain omitted. Non-travel namespace operations remain omitted as well.
The full local-catalog inventory is not an exhaustive paid test of provider health.

Otto's browser-dependent JSON/link tools remain available because static HTML
alternatives do not establish equivalent JavaScript coverage. Inspect freshness,
degradation, truncation and final destination before relying on a result; a
successful extraction or quality grade does not prove the intended page was read.
AgentUtility browser rendering and Strale structured extraction did not qualify;
Strale static link extraction subsequently delivered 26 links after the same
compatibility fix. No rendered/structured equivalence follows from that result. 2s
structured extraction returned unknown fields on the JavaScript test page.
Otto's remaining financial, owned-account, chain-specific and curated APIs retain
distinct contracts; overlapping endpoint names alone do not justify removing them.
Google Trends uses `trend_over_time` for all windows, including `2004_present`.
Concordance's catalog is shared with business/finance so series IDs remain
resolvable. Exa, Trends and Concordance vendor help tools are omitted here;
complete input schemas and inline guidance cover the selected calls without
HTTP setup instructions or unavailable follow-ups. Focused public-page probes
support the selected reading, rendering, static extraction, DNS/RDAP and first-page
X-search workflows; they do not establish universal availability, identical
outputs or complete pagination. Botsmith returned a cursor longer than its
declared input limit, so its continuation remains unqualified.

The desk also selects direct scientific and public-record datasets from 2s,
additional national registries and scoped ownership/insolvency evidence from Strale,
and international legislation, open-data catalogs and selected company records from
Maxwell. HALOWERK adds geocoding, route matrices, HS candidates and text-injection
checks; Daedalus adds static code and skill-content scans. These sources select
and route tools entirely through ordered `send_tools` rules.
Explicit destination selectors precede a final exclusion; comments name existing
alternatives or explain why the remaining workflows are outside this research selection.
Reusable provider definitions remain broader than these editorial selections.

Science groups academic search and metadata, citations, researcher/institution
identities, research funding and biological/chemical references; it shares the web
wallet. Health groups clinical trials, drug/device evidence, public-health and
nutrition datasets, healthcare access, care costs and insurance plans. Health
shares the existing social wallet and its per-payment authority. Investment-oriented
pharma catalysts and general product recalls remain on business. Legal/government
records retain their existing split across business, world, property and finance.

Travel groups trip planning: dated flight prices, operational flight data,
airport references, route matrices, destination references and ski/leisure data.
World retains general weather, aviation weather/hazards and accident research;
generic geocoding is shared. Sports data and generative/developer workflows
remain available through bundled provider selection.

The listener order and ports are web 3010, science 3011, health 3012, social 3013,
business 3014, finance 3015, property 3016, security 3017, world 3018, travel 3019
and toolbox 3020. Connect each client to its listed port. Local deployment overrides remain
operator-owned.

Published schemas and estimates do not establish comparative output quality,
Tor delivery or funded settlement.

Tool search reads the loaded catalog without provider requests. Prefer bundled
provider listing before remote discovery; remote discovery can incur several
charges. Wallet payment caps bound each challenge, not the sum of a research job.
Agent guidance keeps invocation prices and uncertain-payment warnings, while
transport fallback costs and vendor API path mappings remain operator concerns.

Copy a deployment beside its tracked template as `NAME.local.toml` before
customizing it. Relative paths resolve from the file declaring them. Existing
local configurations remain operator-owned; keep using their original state and
identities unless deliberately changing them.

The starter uses direct networking. The comprehensive example requires a running
Tor Browser SOCKS listener at `127.0.0.1:9150`; adjust its `[network]` for your own
Tor daemon. [network/tor.toml](network/tor.toml) can be passed with
`--network-config` to replace the whole network policy for deployment or standalone
commands, or copied into a deployment as its `[network]` table. Tor resolves destination names remotely and has no direct
fallback; separate circuits do not hide identifying request content.

Managed serving binds listeners before funding. Initialize the treasury explicitly
and fund its shielded receive address; available tools remain accessible during
funding or discovery delays. Reserve targets are funding preferences, payment
caps authorize individual signatures, and actual spendable ZEC limits ordinary
funding. See [wallet administration](../docs/wallet-cli.md),
[configuration](../docs/configuration.md) and
[agent source management](../docs/agent-sources.md).

Static-wallet deployments remain supported in the same executable; see the
[configuration reference](../docs/configuration.md). Operational Rust utilities
live in [tools/](../tools/README.md), and qualification deployments and runner
instructions live in [tests/live/](../tests/live/INTEGRATION.md).
