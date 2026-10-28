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
its research sources across nine task-oriented endpoints using four topical
wallet profiles: web for web and science; social; financials for business, finance,
property and security;
and global for world and the toolbox. Ordered source `send_tools` rules group
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
threads. AgData company enrichment returned the requested LinkedIn company.
AgentUtility closest-date archive lookup and 2s Hacker News search also succeeded.
2s scholarly search returned PubMed results with an explicit arXiv failure; preserve
per-source errors. These narrow workflows are enabled, rather than whole catalogs.

| Excluded research | Selected alternatives | Coverage limits |
| --- | --- | --- |
| Web/news and rendering | Exa, Otto, StableEnrich, APEX; AgentUtility JSON/links | Preserve source dates, final destination and partial results. |
| Papers | 2s scholarly search/citations, APEX OpenAlex search, existing science tools | No complete OpenAlex author/work API or guaranteed full text. |
| SEC, court records, rules, nonprofits and recalls | Existing x402stock and 2s APIs | Different query/pagination contracts; US recalls do not replace six-authority international coverage. |
| Archives | AgentUtility `archive_snapshot` | Closest snapshot lookup; no full CDX inventory or qualified archived-text read. |
| Social and company evidence | Botsmith X, StableEnrich Reddit, HireScrape TikTok, 2s HN, AgData LinkedIn company, existing contact providers | No complete SocialFetch platform or endpoint parity. |
| Patents | Existing x402stock patent/company tools | AgentUtility keyword search failed upstream 403; broad keyword coverage remains a gap. |

The 2s GitHub replacement failed upstream authentication and remains excluded.
Other gaps include GitHub profiles/trending, GDELT timelines, employer/customer
reviews, TheirStack technographics and buying intent, licensed Tako data cards and
SocialFetch's long tail of social platforms. Do not silently substitute ordinary
web search for those structured datasets. The bundled FreeQ provider is available
for discovery: static Markdown worked, JavaScript-generated content did not; it is
not selected as another rendered-reader replacement.

Otto's browser-dependent JSON/link tools remain available because static HTML
alternatives do not establish equivalent JavaScript coverage. Inspect freshness,
degradation, truncation and final destination before relying on a result; a
successful extraction or quality grade does not prove the intended page was read.
AgentUtility browser rendering and Strale extraction did not qualify, and 2s
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

Travel/mobility stays on world, whose discovery keywords cover the selected flight,
airport, geocoding and routing tools. Ski/menu/trip-planning tools remain outside
this selection. Sports data and generative/developer workflows remain available
through bundled provider selection.

The listener order and ports are web 3010, science 3011, social 3012, business 3013,
finance 3014, property 3015, security 3016, world 3017 and toolbox 3018. Existing
client connections after web need their port updated when adopting this template.
Local deployment overrides remain operator-owned.

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
