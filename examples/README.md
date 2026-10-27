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
its research sources across eight task-oriented endpoints using four topical
wallet profiles: web; social; financials for business, finance, property and security;
and global for world and the toolbox. Ordered source `send_tools` rules group
tool families, exclude editorial choices and express sharing with destination
arrays. Each source has one final catch-all that assigns or excludes unmatched
tools; listeners retain endpoint properties,
instructions, capabilities and wallet references. See
[source tool distribution](../docs/configuration.md#source-tool-distribution) for
matching, potential bindings and `config check --explain-routing` inspection.

The research desk prefers Exa for general search, Otto for ordinary page reads,
and Vaaya for Firecrawl and Serper News; duplicate StableEnrich wrappers are
excluded here, not in the reusable provider. Exa retains explicit freshness and
historical controls; SocialFetch retains selector, scrolling and cache controls.
Google Trends uses `trend_over_time` for all windows, including `2004_present`.
Concordance's catalog is shared with business/finance so series IDs remain
resolvable. Exa, Trends and Concordance vendor help tools are omitted here;
complete input schemas and inline guidance cover the selected calls without
HTTP setup instructions or unavailable follow-ups. These are capability-based selections, not claims of identical
upstream results or comparative live qualification.

The desk also selects direct scientific and public-record datasets from 2s,
additional national registries and scoped ownership/insolvency evidence from Strale,
and international legislation, open-data catalogs and selected company records from
Maxwell. HALOWERK adds geocoding, route matrices, HS candidates and text-injection
checks; Daedalus adds static code and skill-content scans. These sources select
and route tools entirely through ordered `send_tools` rules.
Explicit destination selectors precede a final exclusion; comments name existing
alternatives or explain why the remaining workflows are outside this research selection.
Reusable provider definitions remain broader than these editorial selections.

Biomedical datasets currently route to business, bibliographic identities to web,
and aviation/trade/environment observations to world. A dedicated science/health
endpoint would group biological/chemical references, drug/device evidence, hospital
quality, surveillance and research funding. A travel/mobility endpoint could combine
2s aviation with Maxwell ski/menu/trip tools and the separately bundled StableTravel
provider. Legal/government research could also move from business into its own
endpoint if legislative, case-law and political-record work becomes frequent.
These are optional reorganizations; this template keeps eight listeners and its
existing wallet assignments. Sports data and generative/developer workflows remain
available through bundled provider selection rather than new default listeners.
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
