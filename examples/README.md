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
its research sources across ten task-oriented endpoints using four topical
wallet profiles: web for web and science; social; financials for business, finance,
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
Other gaps include GitHub profiles/trending, GDELT timelines, employer/customer
reviews, TheirStack technographics and buying intent, licensed Tako data cards and
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
verification. Flights and route matrices remain on world.

New business coverage includes Strale UK charity records, 2s NY/CO/CT state
company records and FCC proceeding filings, plus Maxwell CourtListener search as
a separately callable gateway to the same underlying legal source. 2s adds HN
items/feeds/users, FCC spectrum geographies and FIGI/security resolution. Its
issuer-level ISIN list may include bonds rather than identify the input share.
Strale MiCA checks reported unavailable registers despite HTTP success; they
remain excluded. 2s HICP returned 2025-12 as latest in October 2026 and remains
unselected. Neither payment success nor a false match field proves data coverage.

The bundled-provider catalog was audited for candidate alternatives; this was not
an exhaustive paid test of every provider or every endpoint. The selected narrow
workflows above have specific delivery evidence; similarly named untested APIs
remain candidates, not missing-by-assumption or qualified replacements.

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
identities, research funding, biological/chemical references, clinical trials,
drug/device evidence and health datasets. It shares the web wallet and its
per-payment authority. Investment-oriented pharma catalysts and general product
recalls remain on business. Legal/government records retain their existing split
across business, world, property and finance.

Flights, airports, geocoding and route matrices stay on world. Destination
references, ski-resort data and leisure trip research use travel. Sports data
and generative/developer workflows remain available through bundled provider
selection.

The listener order and ports are web 3010, science 3011, social 3012, business 3013,
finance 3014, property 3015, security 3016, world 3017, toolbox 3018 and travel
3019. Connect each client to its listed port. Local deployment overrides remain
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
