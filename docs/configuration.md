# Configuration reference

Start with the [managed-wallet quickstart](../README.md#quickstart-z-cash-wallet-with-dynamically-managed-tools);
the README also provides a comprehensive Zcash-and-Tor walkthrough.
This reference covers TOML composition, tool selection, transport and pricing.
Commands run from the repository root. The executable requires an explicit
subcommand; running it without arguments displays help and never starts serving.

| Command | Purpose |
| --- | --- |
| `serve --config FILE` | Start a multi-listener deployment and its configured funding workers |
| `serve --provider FILE` | Serve one provider, using stdio by default |
| `catalog tools --config FILE` or `--provider FILE` | Print selected tool inventories as JSON with authored prices |
| `catalog warm --config FILE` | Warm catalogs; configured relay may pay, `--direct` stays unsigned |
| `catalog tags --config FILE` or `--provider FILE` | Print available OpenAPI tags |
| `catalog route TOOL --provider FILE --args '{...}'` | Preview an HTTP request without executing it; standalone providers only |
| `config show --config FILE` or `--provider FILE` | Resolve file settings offline, including wallet bindings for deployments |
| `config check --config FILE` or `--provider FILE` | Fetch catalogs and validate selected tools without signing, probing prices or funding |
| `wallet bootstrap --config FILE` | Fund captured managed reserve claims, without listeners; requires `auto_fund=true` |
| `wallet sync --config FILE` | Discover ZEC and existing configured managed-USDC balances once, including outstanding funding recipients |
| `wallet ...` | Administer the treasury |
| `sources inspect --config FILE` | Read persisted agent-added source records |
| `build-info` | Print build identity and provenance |

Use `x402_treazury COMMAND --help` for the options available to each operation.
Keep these modes in the shared Clap command tree. Only `serve` accepts listener,
authentication and supervised-funding options; administration and inspection must
retain their distinct authority and side effects.

## Configuration file roles

`X402_TREAZURY_CONFIG` supplies the default path for every deployment `--config`
option, including wallet, catalog, config and source commands. An explicit
`--config FILE` (or `--meta-config FILE`) overrides it. This variable must be in
the process environment; `--env-file` is loaded after CLI parsing. The selected
deployment retains the same conflicts with standalone provider options and wallet
`--state-dir`, `--key-file` and `--treasury-id`; unset the variable to use those
options without a deployment.

`--config FILE` selects a deployment: sources, listeners, wallets and network
policy. Wallet commands accept the same file without loading API sources.
`wallet sync` requires exclusive administration ownership (stop `serve` first).
It resolves treasury and relevant managed-Base settings without provider catalogs,
NEAR credentials or automatic funding. Treasury-only deployments need no Base
configuration; independent per-asset results report partial failure. `wallet status`
reads saved observations without network traffic. See the
[CLI output and discovery scope](wallet-cli.md#one-configuration-for-wallet-commands).
`--meta-config` remains a deployment alias. `--provider FILE` selects a reusable
provider for standalone serving and catalog inspection. `--network-config FILE`
selects a file containing `[network]` and optionally `[relay_fallback]`. It
replaces the complete deployment `[network]` section for that invocation; omitted
fields use network defaults rather than inheriting deployment values. A supplied
relay section replaces the complete deployment relay section; an omitted section
preserves the deployment relay. Its provider paths resolve relative to the file
that declares them. Relay settings apply to deployment reads; standalone provider
and wallet commands accept the same file without loading relay providers. Other
standalone provider options still cannot be combined with `--config`.

Ordinary deployment examples live in `examples/deployments/`, reusable network policies in
`examples/network/`, and reusable APIs in `providers/`. Paths resolve from the file
that declares them. Existing user-owned local copies are not moved automatically.

Ordinary enabled managed serving starts initial and replacement funding after
listeners bind, without requiring an eligible funded payer before serving available tools. `wallet bootstrap --config FILE`
waits for its captured enabled reserve claims without catalogs
or listeners. Explicitly set
`funding.auto_fund = false` to pause it. Merely inspecting a config or compiling
Zcash support does not spend funds. Source limits remain required. See the
[wallet defaults and commands](wallet-cli.md#one-configuration-for-wallet-commands).

## Multiple MCP ports from one configuration

Deployment catalogs share download capacity. The default is 2 active downloads;
set a value from 1 to 64 in the deployment TOML:

```toml
[startup]
catalog_concurrency = 2
```

Two catalog slots are a provisional default. The working theory is that fewer
simultaneous large downloads leave headroom when Tor’s prebuilt Conflux capacity
is unavailable. Current measurements do not establish that mechanism or an optimal
limit; see [startup qualification](../tests/STARTUP.md).

A free slot starts the next ready download immediately. Logical loads waiting for
`Retry-After` release download capacity while retaining their retry sequence and
deadline. The same scheduling applies to inspection, warming and deferred serving;
inspection and warming still require their complete selected results. Relay work
reserves capacity for each HTTP attempt/body after serialization/cache/readiness
checks and releases it before retry delays. Accepted ownership retains publication. Synchronous relay fallback traverses its configured list once;
only deferred serving waits for discovery readiness. Ordinary deployment serving loads
local catalogs and binds every listener socket before funding and background remote
catalog discovery start. Each endpoint defaults to `wait_for_catalogs = true`:
it accepts MCP HTTP requests only after its configured `sources` have loaded and
their tools have been published. Unrelated sources and other endpoints do not
gate it. Set the endpoint's setting to `false` to serve available tools immediately
and publish remote tools as they arrive:

```toml
[servers.research]
listen = "127.0.0.1:8000"
sources = ["company_stableenrich"]
wallet = "research"
bearer_token_env = "X402_TREAZURY_MCP_TOKEN"
wait_for_catalogs = true # Default; false enables incremental availability.
```

Sockets are reserved early, so TCP connections can queue while MCP HTTP service
waits. `MCP listening at /mcp` is logged only when that endpoint starts serving.
There is no overall catalog-startup deadline: progress, server-directed retry
waits, Tor availability or relay payer readiness can keep a dependent endpoint
waiting. Funding and discovery workers run during this wait under their existing
authority. Shutdown still cancels unsigned discovery and drains accepted paid work.

A required remote catalog failure leaves its waiting endpoints unavailable,
closes their sockets and emits an ERROR identifying each failed endpoint and the
source. Other endpoints keep running, even if every waiting endpoint fails.
Restart to retry. `sources.NAME.optional_startup = true` lets waiting endpoints
start after that source's failure, with its tools omitted as a complete unit.
This includes failures discovered during remote parsing, tool generation or filter
selection. For `wait_for_catalogs = false`, any failed remote source is omitted,
regardless of `optional_startup`. Stderr and MCP instructions identify pending or
unavailable sources. Local configuration errors remain fatal.
An unresolved exact listener selector is deferred only if its name falls
exclusively under pending or unavailable sources' resolved prefixes (including inherited
prefixes). Unknown namespaces and names also attributable to a ready source
remain errors; an unavailable source cannot hide a ready-source typo.
Inspection, frozen inventories and qualification wait for complete catalogs and
remain strict. The synchronous unsigned-serving loader can omit remote failures
only with `sources.NAME.optional_startup = true`; required failures cancel its
unfinished loads. Ordinary deployment serving publishes remote catalogs as they
finish. Failed sources are not retried during the run;
restart to retry them. A source that keeps downloading occupies its normal slot
until completion. Tool ordering and wallet bindings do not depend on completion
order for unambiguous names. If loaded catalogs define the same tool name on a
listener after tag selection, ordinary serving omits every binding of that name
and reports the ambiguity in MCP instructions. Unique tools and other listeners
remain usable. A previously published name is withdrawn when a later catalog
reveals its ambiguity; calls that already captured a binding finish with that
original provider and wallet. Exact selectors still recognize ambiguous names.
Strict inspection rejects duplicate names. `config check`, `catalog tools` and
`catalog tags` use strict loading;
`config show` remains offline and reports the effective limit and each endpoint's
`wait_for_catalogs` setting. Progress, source
fetch/parse and generation timings go to stderr; a ten-second
waiting message identifies remaining work. No response content or source URL is
included in these progress messages. Remote spec aliases share a download and parsed document within one deployment
load when the exact URL, requested timeout, byte limit, transport compatibility flags and discovery identity match.
Each alias retains its own tools, filters, base URL and wallet binding. Local files
are read independently. Compatible remote responses can also use the disk cache described below.


[examples/deployments/managed.toml](../examples/deployments/managed.toml) exposes
Exa web search, page contents and provider-management tools through an authenticated
research listener and one managed pool. Exa's catalog loads from its public
specification. Agent-added APIs share the research pool; registrations last until
exit unless a registry file is configured. Optional `config show` inspection stays
offline; `config check` and `catalog tools` load the remote catalog.

Initialize the treasury explicitly and fund its shielded address before paid use;
see the [quickstart](../README.md#quickstart-z-cash-wallet-with-dynamically-managed-tools). Serving needs
the initialized treasury and binds the listener before
background funding. The [comprehensive deployment](../examples/deployments/comprehensive.toml)
adds multiple research listeners, Tor, relay and persistent source registrations.

Each listener serves stateless Streamable HTTP at `/mcp`. Meta-config listeners
require loopback addresses. There is no hot reload; restart to apply changes.
Static wallet profiles remain supported and use `mode = "static"` with
`private_key_env`; they do not require treasury configuration. Static serving
loads only the keys used by selected tools.

The version-1 TOML schema separates named sources, wallets and servers, with
optional wallet templates and an automatic assignment policy:

- `sources`: provider settings written inline or imported with
  `extends = "../providers/pdl.toml"`. Every generic catalog setting is
  available here, including `spec`, `name`, `description`, `base_url`, `prefix`, `read_timeout_seconds` (inherits the network default of 60),
  path/tag filters, `include_tools`, `exclude_tools`, pricing options, overrides
  and guidance. A deployment source
  can also set `wallet` to override the server default; this field is forbidden
  in reusable provider files.
- `wallets`: `mode = "static"`, `private_key_env`, and `max_api_payment_usdc`
  (decimal string, default `"1.00"`), or a managed `zcash_rotation` profile
  described below. Bindings referencing the same profile share payer state.
  The cap applies per payment; it is not an aggregate budget.
- `servers`: `listen`, `sources`, and `bearer_token_env` when `auth = true` (the default), with an optional default
  `wallet` and optional
  `include_tools`, `exclude_tools`, `tags`, and `exclude_tags`.
  Resource identifiers use lowercase letters, digits and `_`, starting with a letter.

Provider/source `description` is optional searchable capability metadata. It inherits
and replaces as an ordinary whole field and appears in `config show` with its
origin. It does not replace tool descriptions or `instructions_text`. Shipped
providers require nonblank descriptions; see [bundle membership](../providers/README.md#bundled-provider-metadata).

Provider/source `include_operations` and `exclude_operations` match exact original
`METHOD path` identities (uppercase GET, POST, PUT, PATCH or DELETE), independently
of generated names and prefixes. Paths use the specification's original path or
absolute HTTP(S) URL. Exclusions win over inclusions before method preference and
naming. Unlike subtree `exclude`, an exact parent exclusion preserves supported
child routes. Absent operation entries do not fail inventory validation; an
independent source tag/path filter can remove them. Lists replace on inheritance;
`[]` clears the inherited list. Ordinary exact generated-name validation remains
unchanged. Catalog selection does not constrain explicitly authorized raw calls.

Provider/source `prefer_post` defaults to true. After path/operation/tag
compatibility filters, exact original-path GET/POST pairs retain POST before
collision counting and naming. Other methods, different paths and GET-only routes
remain. Name filters and recommendations cannot restore a removed GET; explicitly
set `prefer_post = false` before selecting its collision-resolved name. This is an
internal integration escape hatch, inherited like any scalar; a source can override
either value. It is not a CLI option or ordinary provider boilerplate. Review
request, response and payment contracts before collapsing meaningful differences.
The full default catalog includes this collapse; disabling preference exposes the
preference-free method inventory. Exact selections using removed `_get`/`_post`
suffixes must migrate to canonical names; no hidden aliases are installed. Raw calls
retain their requested method, and selection grants no retry authority.

Provider and source `include_tools` / `exclude_tools` select generated tool names,
using the same case-sensitive `*` (any characters) and `?` (one character) patterns
as server filters. For example:

```toml
[sources.company]
extends = "../../providers/otto.toml"
include_tools = ["otto_equity_intel", "otto_financial_statements", "otto_help"]
```

Names are assigned after path, operation and tag selection, before tool-name
filtering, so removing a tool by name does not rename its remaining siblings.
Tool-name filters intersect the earlier filters; exclusions win. Help tools also
participate: include their name or a matching pattern to keep them in an allowlist.
Unknown exact names fail catalog loading; unmatched wildcard patterns emit a
warning, and an empty final toolset fails. Server filters further narrow each
listener's inventory and cannot restore excluded source tools.

Provider/source `recommended_tools` is advisory metadata using the same generated-name
selector syntax. The full compatible catalog remains visible unless the source
explicitly sets `use_recommended_tools = true` (default false):

```toml
[sources.research]
extends = "../../providers/otto.toml"
recommended_tools = ["otto_equity_intel", "otto_financial_statements", "otto_help"]
use_recommended_tools = true
```

The [comprehensive deployment](../examples/deployments/comprehensive.toml) explicitly adopts
TeleSINT's stateless research recommendations and excludes its broad help. Its
company and person sources retain their narrower purpose-specific selections.

Recommendation, inclusion and exclusion selectors all validate against the same
compatible inventory after names and help are generated. Opt-in intersects advice
with the ordinary source filters; exclusions win, and listener filters narrow the
result further. Selection never renames siblings or changes source/wallet identity.
Include help deliberately. An opted-in missing, empty or entirely unmatched list
fails; unmatched exact recommendations fail when opted in, while unmatched wildcards
warn. Unused stale advice warns without hiding compatible tools. Malformed selector
syntax or field types remain configuration errors even when advice is unused.

Lists replace on inheritance, and `recommended_tools = []` clears advice. Names are
not rewritten when a source changes its prefix: replace inherited advice with the
new generated names or disable opt-in. A provider that recommends its entire
compatible catalog can omit advice and leave opt-in disabled rather than enumerate
today's inventory.

`catalog tools` JSON marks every tool with `recommended: true` or `false`; these
markers are operator metadata and never modify MCP schemas or descriptions.
`catalog tools --all-tools` bypasses only recommendation opt-in, after typed
configuration resolution. Ordinary path,
operation, tag, source-name and listener filters remain effective. Clear those
filters explicitly in a separate inspection configuration to inspect a broader
compatible inventory; set `prefer_post = false` to inspect both method twins.
This inspection never initiates extra pricing probes or changes serving selection.
Offline `config show` reports effective advice, opt-in and field origins without
catalogs or secrets; inventory-dependent errors (including empty opt-in) are
reported when loading a catalog.

Composition follows the ordinary replacement rule: omitted lists inherit,
present lists replace, and `[]` clears. To select tools outside an inherited
`include_operations` allowlist, explicitly clear or replace that allowlist too.

Host validation defaults to `localhost`, `127.0.0.1` and `::1`, independently of
the listener's bind address. Replace that allowlist per listener when using a
custom hostname or reverse proxy:

```toml
[servers.research]
allowed_hosts = ["mcp.example.com", "localhost"]
# Alternatively, omit allowed_hosts and disable the allowlist:
# disable_host_check = true
```

Entries are exact hostnames or IP addresses, optionally with a port. A hostname
without a port permits any port; `"mcp.example.com:8443"` permits only that port.
URLs and wildcards are not supported. The list replaces rather than extends the
defaults. `allowed_hosts = []` also disables the allowlist. Do not combine
`allowed_hosts` with `disable_host_check = true`.

Standalone HTTP serving accepts `--allowed-hosts mcp.example.com,localhost` or
`--disable-host-check`, both requiring `--transport http`. Deployments configure
these settings in TOML. Disabling the allowlist emits a startup warning and removes
this DNS-rebinding defense; it does not change authentication. Disabling both
authentication and Host checking is permitted. Malformed/missing Host headers
and HTTP/2 authority validation still follow the MCP library's protocol rules.
`config show` exposes each server's settings without starting listeners.

To reduce the initial tool context for a listener, add `discover_on_demand = true`
to its `[servers.NAME]` table (default: `false`). It advertises only search/call
wrappers plus independently enabled provider-management and raw-call tools.
Search returns complete tool descriptions and schemas, 25 results at a time by
default. Filters, payment routing, source permissions, and startup catalog loading
are unchanged. Queries use BM25 keyword ranking over names, descriptions and
parameter names, with literal substring matches prioritized. Omit the query to
enumerate the catalog; `available` and `total` distinguish scope size from matches.
See [tool search matching and results](agent-sources.md#tool-search-matching-and-results)
for query semantics, diagnostics and pagination.

Endpoint `instructions_text` overrides composed source guidance (an empty string
suppresses it), while generated pricing and operational notices remain.
`discovery_search_tool_instructions_text` appends to the local tool-search description;
`providers_search_remote_tool_instructions_text` appends to configured remote-directory search help.
Neither setting enables discovery or provider management. See
[endpoint instruction placement](agent-sources.md#endpoint-instruction-placement)
for defaults, client caching and guidance placement.


Listeners configured with `bearer_token_env = "X402_TREAZURY_MCP_TOKEN"` use
bearer authentication when that variable is set. If it is absent, they serve
without authentication and log a WARN. Empty or whitespace-only values are
rejected. Custom `bearer_token_env` names still require a nonempty value.
Existing local configurations using `TREAZURY_MCP_TOKEN` must update the reference
to adopt the new optional variable.

To disable HTTP authentication for one listener, set `auth = false` and omit
`bearer_token_env` in its `[servers.NAME]` table. Other listeners retain their own
authentication policy. Standalone HTTP uses `--transport http --no-auth`; this
explicit flag ignores `X402_MCP_BEARER_TOKEN` and conflicts with `--bearer-token`.
Startup warns when a listener allows clients to call tools without authentication.
Stdio needs no authentication. Live integration qualification still requires
authenticated listeners and fresh runner-generated tokens.

Authenticated HTTP clients send `Authorization: Bearer <token>`; `bearer_token_env` names the
environment variable containing only the token. HTTP 401 responses distinguish
missing headers, malformed or duplicate authorization headers, and incorrect token
values. Warnings identify the listener and failure category without including
credentials. Each category logs at most once per listener per 30 seconds; warnings
state this limit and the next warning reports the suppressed count. Every rejected
request receives the error response even when its warning is suppressed.

Payment identity sharing is controlled by named wallet references. For every
server/source binding, precedence is **source `wallet` → server default `wallet`
→ automatic assignment**. A server may omit its default if every listed source
assigns a wallet or an automatic policy supplies the fallback. Missing assignments
and unknown explicit wallet names are configuration errors, even when
a source is filtered out or a default would be overridden.

| Arrangement | Wallet sharing |
| --- | --- |
| All bindings reference one wallet | Shared across the deployment |
| Each server has its own default, with no source overrides | Shared within each server |
| Each source specifies its own wallet | Shared across listeners using that source |
| Several sources reference one wallet | Shared within an operator-defined group |

For example, these deployment excerpts use a dedicated `social` wallet for
SocialFetch and the listener's `general` default for PDL. Both names must be
defined in `[wallets]`; this works with static or managed profiles:

```toml
[sources.socialfetch]
extends = "../providers/socialfetch.toml"
wallet = "social"

[sources.pdl]
extends = "../providers/pdl.toml"

[servers.research]
listen = "127.0.0.1:8000"
bearer_token_env = "RESEARCH_MCP_TOKEN"
wallet = "general"
sources = ["socialfetch", "pdl"]
```

A source override retains its payment identity across listeners. For finer
isolation, create separate source aliases extending the same provider and assign
different wallet names; provider files describe APIs, while deployments choose
payment identities. Sharing scope controls wallet reuse, not a guarantee of
complete unlinkability. Every additional managed pool requires its own configured reserve funding. Referencing one profile from more bindings does not create
additional pools or funding jobs.

`config show` includes `wallet_bindings.<server>.<source>` with the effective
`wallet` and its `origin`, such as `sources.socialfetch.wallet` or
`servers.research.wallet`. Resolved sources display their optional `wallet`
separately from provider `settings`. Inventory (`catalog tools`) includes each
server's `default_wallet` and the same `wallet_bindings` map; each tool's `source`
identifies its binding. `config check` prints these mappings as well. Mappings cover
all declared server/source bindings, including sources whose tools are filtered
out; only effective static wallets used by selected tools load private keys.
Inspection never reads wallet secrets or opens treasury state.

To avoid repeating managed wallet definitions, define one template and an
automatic assignment policy. To adapt the managed starter, replace its
`[wallets.research]` definition and reserve with this template/policy and remove
`wallet = "research"` from the listener. For this variant, also remove the
listener's provider-management/raw-call capabilities and any `source_registry_file`:
these capabilities require an explicitly named wallet. Keep its treasury, funding, source and
other listener settings. This substitutes automatic pools for the explicit pool:

```toml
[wallet_templates.small]
mode = "zcash_rotation"
max_api_payment_usdc = "1.00" # Required explicit group per-payment authority.
max_retirement_remainder_usdc = "0" # Default; only eligible empty retained wallets.
max_funding_spend_zec = "0.02"
max_conversion_overhead_percent = 5

[wallet_templates.small.reserves.small]
funding_amount_usdc = "2.00"
max_funding_amount_usdc = "3.00" # Explicit bridge-minimum headroom, not payment authority.

[wallet_templates.small.reserves.large]
funding_amount_usdc = "10.00"

[wallet_assignment]
scope = "source"
template = "small"
```

Sources and servers can then omit `wallet`; `[wallets]` may also be omitted.
See [funding limits and fees](wallet-rotation.md#funding-limits-and-fees) for
actual spendable treasury capacity and per-operation ZEC/fee safeguards.
The singleton `[treasury]` and `[funding]` settings are still required when the
resolved deployment has managed pools. Explicit source/server wallet references
continue to take priority and must refer to entries in `[wallets]`, not templates.
They provide custom sharing groups and exceptions to the automatic policy.

| Automatic scope | One generated pool per | Sharing |
| --- | --- | --- |
| `deployment` | Deployment/state owner | All otherwise-unassigned bindings |
| `server` | Server name | Otherwise-unassigned sources within that server |
| `source` | Source name | That source across otherwise-unassigned listeners |
| `binding` | Server/source pair | Each otherwise-unassigned binding separately |

Templates accept the same settings and defaults as a named `zcash_rotation`
profile; static templates, unknown fields/scopes, missing templates and invalid
risk limits fail validation. A template never allocates a pool by itself.
Automatic pools are generated only for declared server/source bindings that need
the fallback. Unreferenced sources and unused templates create none. Tool filters
do not change these identities or suppress a declared binding's generated pool;
inspection can therefore resolve the exact set before fetching API catalogs.
All explicitly declared managed profiles still request pools, even if unreferenced.

Generated names use the reserved `auto_v1_` prefix: `auto_v1_deployment`,
`auto_v1_server_<server>`, `auto_v1_source_<source>`, and
`auto_v1_binding_<server-name-length>_<server>_<source>`. The length prefix avoids
ambiguous pairs containing underscores. Explicit `[wallets]` entries cannot use
this namespace. Pool identity belongs to the initialized treasury/state directory;
it does not depend on the configuration file path, ordering, template name or
template contents. Separate deployments require separate initialized treasuries
as described in the managed-wallet section.

Configuration changes take effect when the deployment is reopened. Managed
profiles require an explicit `max_api_payment_usdc`; the static-wallet default
remains unchanged. The cap applies to every active, reserve and retained wallet,
independently of original funding size. Explicit unlimited spellings retain their
existing meaning for payment authority only. A cap above every target is valid,
and targets above the cap are valid; neither arrangement authorizes extra funding.

`reserves.small` is required. `reserves.large` is optional and must have a target
at least as large as small; equal targets are valid. Each slot requires a positive
exact micro-USDC target. Its optional output ceiling must be at least its target;
omission permits no increase above target. New groups fund only the enabled slots,
with no separate active allocation. The first journaled payment promotes its READY
payer to ACTIVE and opens that slot for refill. Vacancies allocate small-first,
independently of which wallet can pay.
Consumed reserve wallets become active, previous active wallets become retained,
and retained wallets remain usable without automatic top-ups or slot reassignment.
First positive external credit permanently consumes the initial funding claim,
including partial credit. Disabling a slot retains accepted source ownership.

`max_retirement_remainder_usdc` is a finite nonnegative micro-USDC amount, default
`"0"`. It authorizes eligible retained-wallet retirement on the canonical-publication
path, independently of `auto_fund`, vacancies and treasury capacity. Ordinary
advisory refresh does not collect those canonical proofs or retire wallets. Retirement leaves
funds and keys at their address, stops routine polling/selection, transfers nothing
and never releases financial obligations. Positive tolerance trades purchasing
power for less observation work; it does not bound cumulative stranded capital.

Funding capital comes from actual spendable treasury funds after confirmations,
reservations, liabilities and fees. Displayed total balance is not spendable
capacity. Adding spendable ZEC restores capacity without another aggregate-budget
setting. `auto_fund=false` pauses new funding. Enabled unfilled reserve claims
keep discovering later ZEC deposits and confirmations through the shared treasury
owner; fully funded idle serving does not watch for incoming funds. Ordinary scalar
maintenance does not invoke permanent retirement. Keep group
payment caps, per-operation ZEC/output/fee safeguards, explicit funding controls
and separately authorized qualification-run limits.

`funding.poll_seconds` defaults to 5 (accepted range 1 through 60). It controls
local funding inventory/owner scheduling and existing per-job jitter. Each
recurring source, provider-status or Base credit observation starts at least
60 seconds after the previous observation for that work completes; configured
polling does not shorten that floor or delay successful local preparation and
submission handoffs. `funding.swap_timeout_seconds` defaults to 1,800 seconds
from the first durable broadcast intent and reports degraded health without
abandoning accepted work or granting replay authority. See the
[wallet observation policy](wallet-rotation.md#wallet-observation-policy) for
startup, explicit sync and requested-refresh scopes.

The old managed top-level `funding_amount_usdc` and `max_funding_amount_usdc` must
move into named reserve tables. `limit_payments_to_funding_target` is removed:
choose an explicit payment cap preserving the intended authority. The aggregate
settings `daily_funding_limit_usdc`, `total_funding_limit_usdc` and
`daily_treasury_spend_limit_zec` are rejected with migration guidance. Tier/retention
aliases and `max_live_wallets` are unsupported. Updating configuration never
converts wallet state. Unsupported legacy schemas are rejected without mutation.
Existing keys, balances, transactions and liabilities
must survive. There is no general migration/abandonment framework or manual
retirement command.

Reserve settings change future authorized allocations, not existing accepted
transactions or captured targets. Changing assignment scope or a source/server
name may create new groups. Startup disables absent groups while retaining keys
and history; restoring the prior identity resumes that group. Static-only serving
does not unlock or modify treasury state.

For automatic bindings, `config show` and inventory additionally report
`template` and `scope`. Configuration inspection includes `resolved_wallets`
(explicit plus generated profile settings), `generated_wallets` (generation
metadata), and `wallet_summary` (distinct managed/generated pool counts and their
combined initial-inventory target in `initial_inventory_target_atomic` and
`initial_inventory_target_usdc`). `config check`
also prints that summary. The total counts each managed pool once, including
explicit unreferenced profiles; it excludes unused templates and static wallets.
It is a configuration target, not a live balance or a quote for additional funds:
the fresh-group formula is `S + L`, or `S` without large, before accepted
bridge minimum increases. With $2/$10 reserves it is $12. Existing addresses keep
their captured targets; accumulated retained holdings, fees and retirement amounts
are separate. Offline inspection cannot infer migration cost or usable capacity.
None of these inspection commands allocates keys or funds.

### Reserve sizing and fragmentation

Size large to at least twice the largest expected payment, preferably four times
when prices vary. This is guidance, never an admission restriction. Larger sizes
reduce some remainders but can keep more calls on one payer address. Small-funded
wallets provide more frequent rotation opportunities for cheap calls while a large
reserve supports occasional expensive calls. Selection may still use large-funded
wallets for cheap calls; retained reuse, request content, funding and network
observations can link activity. These settings do not promise unlinkability or
strict price-class separation.

A $10 wallet settles two $4 calls and retains $2; two $3.34 calls retain $3.32.
A later smaller call can use retained capacity, but fragmented balances cannot be
summed to authorize one payment. Cold selection uses the default payer before
learning prices. A sufficient bound payer remains stable. Otherwise, selection
uses the first estimated-affordable existing wallet in ACTIVE, RETAINED, READY
order, with creation order within each role. Unknown or low estimates remain
fallbacks. It does not sort wallets by balance; cheap calls can keep using a
large-funded active wallet.

With small $2 and large $10, fresh requested inventory is $12. Repeated $6
calls can leave $4 at each large-funded address: after nine settled calls and
eight replacements, $92 gross allocation has funded $54 of calls and leaves $38
in holdings. Those holdings cannot collectively authorize another $6 payment.
This illustrates sizing below the recommendation, not a wallet-count limit.
Learned cheap calls can drain compatible smaller balances; cold calls, expired
hints or unavailable evidence can keep using a sufficient large active payer.
Neither case establishes a quantitative privacy guarantee.

For target `L` and repeated settled payments `p`, the unusable remainder for
another same-price call is `L - floor(L/p)*p`. At a maximum payment of 25% of the
wallet target, the remainder is below 20% for identical prices and below 25% for
mixed prices. These bounds assume settled payments, no other liabilities or
unexpected credits, and replacement because the next payment no longer fits.
They describe one wallet's remainder, not total deployment holdings; pending
payment authorizations can cause earlier reserve consumption.

### Startup readiness and configuration changes

Ordinary enabled serving binds listener sockets before starting funding;
MCP availability follows each endpoint's `wait_for_catalogs` setting independently
of wallet funding readiness.
Small and large reserve funding may finish in either order. An empty managed pool
returns typed unavailability until an eligible funded payer exists. No ACTIVE wallet is
required: a funded READY reserve can serve immediately. Unsigned requests do not
promote wallets or create replacement vacancies. `wallet bootstrap` instead
captures the enabled slot generations and waits for those original
destination obligations, without chasing replacement generations or source-only
settlement backlog. A stopped or cancelled waiter preserves accepted work.
`auto_fund=false` still observes accepted operations but cannot allocate or prepare
new funding. The first positive credit—even $3 toward a $10 target—consumes its
initial funding claim permanently and publishes the funded role. No top-up follows;
existing source liabilities remain owned even after disablement. Insufficient accepted source work remains owned for reconciliation,
not automatically abandoned or replaced.

Reopen applies changed caps, targets and slot enablement. Existing accepted terms,
job identities and original targets remain immutable; current policy is rechecked
before new quote acceptance/preparation. Configuration edits do not convert state.
Ordinary startup rejects unsupported legacy schemas without mutation. Configuration
edits and `wallet recover` cannot supply missing immutable destination-readiness
evidence or convert unsupported state. Preserve complete backups and all accepted
financial obligations; restoring an older backup cannot undo subsequent work.
See [Wallet selection and network isolation](architecture.md#wallet-selection-and-network-isolation)
for the bounded selector and its accepted limitations.

Rust configuration is TOML-only. [providers/](../providers/README.md) contains
the reusable provider definitions. AgentUtility, Glassnode, Concordance,
Straits and Locus have a `provider.toml` and a locally curated `openapi.json` in their own directory.
JSON remains the format for API documents, fixture data and inventory output.
Dynamic bundled provider catalogs retain accepted provider configuration with their schema
bytes; restart and offline source inspection rebuild that accepted definition.
Explicit source refresh adopts current bundled settings and schema together.
Source-management byte limits and wallet/origin policy remain authoritative; its
explicit read-inactivity setting overrides the provider value, then the network
default applies. See [source persistence and maintenance](agent-sources.md#persistence-and-operator-maintenance).
Small PNG/JPEG/WebP results are returned as MCP image attachments; provider TOML
can also extract explicitly mapped base64/data-URI images from JSON while keeping
metadata. See [inline image configuration](../providers/README.md#inline-image-results)
for byte/count limits and supported formats. Asset URLs are not automatically fetched.

Composition is one level: a source may extend a provider file, but that provider
must not contain `extends`. Local fields replace inherited fields in full;
arrays are not concatenated, and an `overrides` table replaces the whole inherited
table. An empty list or table clears the inherited value. Omitted fields inherit,
then use defaults. Unknown fields are errors.

Paths resolve where declared: `extends` is relative to the deployment file;
a provider's local `spec` is relative to the provider file; a deployment's local
`spec` override is relative to the deployment file. There is no global `root`.
URLs are unchanged. A provider's explicit tool prefix is preserved regardless of
its source identifier; otherwise the source ID becomes the prefix. Two aliases
with overlapping names cannot be mounted in the same listener.

Source path/tag filters and description overrides run first. Each listener then
filters the generated catalog by tags and tool names. Tag inclusion matches any
listed tag, exclusions win, and matching is case-sensitive; untagged tools
(including help) do not match a positive tag filter. Tool-name patterns support
`*` and `?`; unmatched exact selectors are errors and unmatched wildcards warn.
Duplicate tool names and empty server inventories are errors. Calls enforce
the same selection as listing. Sources referenced by multiple listeners are
loaded once and retain one catalog; selected instructions are combined without
hidden truncation.

`config show` prints the composed file settings and per-source field origins,
including defaults, without reading secrets or loading specs. It accepts
`--provider` or `--config`, plus optional `--network-config` to inspect the selected network policy; CLI/environment runtime overrides are outside
this file inspection command. Wallet/token fields show environment variable
names, never their values.

`config check` validates and reports inventories; `catalog tools` prints per-server
JSON including source attribution. `catalog tags` reports unfiltered source tag
counts. These commands do not create signers, bind ports, or fetch help documents.
Remote specs still require network access. Deployment operations accept `--env-file`
except offline `config show`.
Standalone CLI overrides cannot accompany a deployment `--config`; configure
those values in its TOML instead. Serving/authentication flags belong only to
`serve`.

Startup validates credentials and binds every listener before serving any.
A bind failure releases listeners already acquired. Both standalone HTTP and
meta-config serving handle SIGINT/SIGTERM by stopping listeners and allowing
accepted calls to finish without a fixed drain deadline. Stderr announces the drain
and reports stages, call counts and elapsed time every ten seconds. A second
SIGINT/SIGTERM forces exit with an unknown-outcome warning; it never releases
reservations or replays signed work. MCP client disconnect and stdio EOF stop
response delivery without abandoning application-owned calls. Funding and
reconciliation remain available until accepted deployment calls finish. A listener failure also stops its
siblings. Catalog startup failure instead disables only dependent waiting endpoints
and emits ERROR; their peers and financial owners keep running. Managed profiles
retain durable payment uncertainty through shutdown;
automatic NEAR funding is controlled by `funding.auto_fund`.

## Download and display limits

Sources and reusable provider TOML files accept these positive byte limits:

| Setting | Default | Applies to |
| --- | --- | --- |
| `max_response_bytes` | 16 MiB (16777216) | API response bodies, including payment challenges and errors |
| `max_help_bytes` | 4 MiB (4194304) | Lazy help documents and exact provider-management usage-document reads |
| `max_spec_bytes` | 32 MiB (33554432) | Static specs fetched from URLs; local operator-authored files are unaffected |

Standalone CLI flags `--max-response-bytes`, `--max-help-bytes` and
`--max-spec-bytes` override provider settings. Meta-config sets them per source;
`config show` reports resolved values and origins. Generic agent-added sources, raw calls and exact usage-document reads use the
shared defaults. Bundled imports preserve the provider's configured limits.
Agents cannot change these limits.

A declared Content-Length over the cap is rejected immediately. Streaming reads
also enforce the cap when Content-Length is absent. An oversized document
is rejected entirely: the agent gets an error naming the limit and setting, and
stderr logs the rejection. No partial content is returned or cached as success.
A paid response may already have settled before its size is known; an oversize
response does not trigger another payment attempt.

Completed tool output is returned in full, including JSON and image metadata.
The listener setting `max_response_chars` and CLI flag `--max-response-chars` have
been removed and reject by name; remove them from existing configurations/commands.
`max_description_chars` remains an explicit provider-description display control:
shortened descriptions include a marker and stderr records the event. It does not
control download size. MCP HTTP requests have a 4 MiB incoming-body limit;
exceeding it returns HTTP 413 with the limit in its message and a stderr warning.

## Implemented behavior

- Composed TOML provider settings and local/remote JSON OpenAPI specs or digests;
  local `$ref` resolution, path/tag selection, deterministic names, flattened
  schemas and routing, vendor prices, description overrides and explicit
  `max_description_chars`. Vendor text has no default truncation.
- `help_url` adds a lazy, cached documentation tool. Proven unpaid fetch failures
  can retry; failures with possible relay charges remain cached.
  `instructions_text` and `name` populate the initialize response.
- MCP stdio and Streamable HTTP. The HTTP bearer gate runs before MCP dispatch;
  no session header or sticky session is required. The SDK's default Host checks
  remain enabled, so the demonstrated setup is localhost. Deployment behind a
  different hostname needs an explicit allowed-host configuration change.
- Base canonical USDC payments: v1/v2 `exact` and v2 `upto`, signed by the SDK.
  V2 challenge descriptions are capped at 500 characters for facilitator
  compatibility. The accepted requirements themselves remain intact.
  Static Permit2/`upto` requires an externally provisioned allowance; never
  automatically approve one. Managed profiles support v2 `exact` only.
- Asset/network filtering and an exact decimal per-payment cap before signing.
  `--max-api-payment-usdc` overrides `X402_MAX_PRICE_USD`, default `1.00`.
  `none`, `off`, or an empty value removes the amount cap **but retains the
  Base USDC restriction**.
- Static profiles perform one signed payment attempt after the unsigned challenge.
  Managed profiles may traverse existing eligible wallets after recognized
  insufficient-funds headers; see the [best-effort wallet rules](wallet-rotation.md#best-effort-wallet-rules).
  Other ambiguous failures grant no paid replay. Redirects are disabled.
- `PaidClient::replace_payer` swaps the signer for subsequent requests. A
  request captures its payer before the unpaid attempt and retains it until
  completion. This API is for static profiles. Managed profiles acquire an
  immutable signer lease after challenge validation and durable admission.

The CLI supports `--spec`, `--provider`, `--base-url`, `--prefix`, `--include`,
`--exclude`, `--tags`, `--exclude-tags`, `--read-timeout-seconds`,
`--transport`, `--host`, `--port`, `--bearer-token`, and `--env-file`.
Filters accept comma-separated values. Supported
`X402_MCP_GENERIC_*` overrides are `SPEC`, `BASE_URL`, `PREFIX`, `NAME`,
`PRICING_KEY`, `INCLUDE`, `EXCLUDE`, `TAGS`, `EXCLUDE_TAGS`, `INSTRUCTIONS_TEXT`,
`HELP_URL`, and `MAX_DESCRIPTION_CHARS`. Precedence is CLI > environment > config
> default; explicit env-file values override inherited environment values.
Use `--help` for the complete CLI. `catalog route NAME --args '{...}'` prints
method, URL, query, declared headers and JSON body without making a request or
loading a signer. Previewed idempotency keys belong only to that preview; an actual
tool invocation generates its own key.

All MCP listeners return their complete advertised inventory in one `tools/list`
response, without `nextCursor`; supplied listing cursors are ignored.
Tool search defaults to 25 results per page; explicit positive `limit` values are
supported without a local ceiling. Search cursors bind the listener and catalog
snapshot; publication requires restarting search enumeration. Remote directory
browse defaults to 25 and retains its upstream maximum of 100. Pagination selects
pages and does not reject or trim the full inventory.

## Operation URL extensions

`operation_url_extension = "x-direct-url"` opts a provider or deployment source
into absolute request URLs from that operation-level OpenAPI extension. Each URL
must use public HTTPS without credentials or a fragment, with a fixed authority.
Declared path parameters are substituted using the normal escaping rules; query,
header and body arguments retain their OpenAPI routing. Fixed query strings are
preserved. An absent extension retains ordinary `base_url` plus path routing;
a present invalid value fails catalog construction for that operation.

The original OpenAPI path still controls tool names, method selection, path/tag
filters and response mappings. URL validation follows path/method/tag selection
and POST-twin preference, before tool-name and recommendation filters. Generic
imports ignore vendor URL extensions. Accepted bundled definitions preserve their
explicit opt-in across registration, refresh and restart, permitting declared
cross-origin targets through the existing public-destination network policy.
This setting does not enable HTTP redirects.

Optional cover profiles remain bound to the configured `base_url` origin. A call
to a different operation URL origin emits a warning and continues without cover;
see [cover configuration](cover-traffic.md#enable-and-inspect). Disable cover for
sources whose operations span origins, as the bundled Lonestar provider does.

## OpenAPI header parameters and idempotency

OpenAPI `in: header` business parameters become ordinary tool arguments. Header
names are case-insensitive on the wire and for operation overrides of inherited
parameters. Argument names preserve the declared spelling; a collision with a
query/body argument adds `_header`, retaining the original wire name. An
unresolvable alias collision is an explicit catalog error. Required headers must
be supplied before dispatch; absent/null optional values are omitted.

Header serialization supports OpenAPI `simple` style: scalar values become text,
arrays join with commas, and flat objects become `key,value` pairs or `key=value`
pairs with `explode = true`. Unsupported styles, content-based header parameters,
nested values and invalid HTTP names/values fail explicitly. Values are never
included in validation error messages. As required by OpenAPI, header parameters
named Accept, Content-Type and Authorization are ignored. Declarations of x402
protocol headers (`PAYMENT-SIGNATURE`, `X-PAYMENT`, `PAYMENT-REQUIRED`,
`PAYMENT-RESPONSE`, `X-PAYMENT-RESPONSE`) are also ignored, case-insensitively,
even when marked required: they describe the payment protocol, which Treazury
handles automatically. They never become tool arguments or business headers.
Direct or persisted routing metadata cannot supply these runtime-owned headers.
Cookies and HTTP framing/connection/proxy header declarations are rejected.
Generated and manual raw requests share fallible HTTP header construction and
runtime-header headroom checks before inventory or network activity. Sets that
exceed the HTTP library's representation capacity fail visibly rather than panic;
there is no additional local header-count quota.
Explicit custom API-key headers are ordinary declared arguments; existing
relay policy still forbids credential forwarding and skips adapters unable to
preserve all business headers.

A header named `Idempotency-Key` is managed automatically, even when optional in
the schema. It is omitted from tool arguments. Each invocation generates a random
UUID string and preserves it through the existing unsigned retries, signing and
recognized insufficient-funds wallet traversal. Identical arguments in two calls
produce different keys and can intentionally buy twice. Automatic generation
accepts unconstrained string declarations, optional `format = "uuid"` and length
bounds admitting 36 characters; incompatible formats or other validators fail
visibly at catalog construction rather than inventing a vendor key format.
Pure local schema references are followed. References with sibling constraints,
unresolved references and cycles remain unsupported and fail explicitly; the
general tool-schema projection cannot erase constraints to permit generation.

Accepted in-process work retains the request after waiter cancellation. No HTTP
key/request recovery journal or cross-call deduplication is added; interrupted
calls are not automatically replayed after restart. A key neither proves vendor
deduplication nor prevents a second x402 debit, and ambiguous signed outcomes
still grant no automatic replay authority. This behavior applies to static and
registered OpenAPI tools, including on-demand calls. The separate `raw_call`
interface accepts caller-owned headers and keys without generating or remembering
them; see [manual raw headers](agent-sources.md#manual-raw-headers). Every nonempty
manual set disables paid relays and cross-payer handoff. It does not change
generated-tool idempotency or provide automatic credential lookup. Manual raw
Authorization is explicitly caller-supplied, without OpenAPI registration, storage
or automatic injection; generated tools and public routed business headers retain
their Authorization restriction. A future managed OpenAPI-auth mode must reject
mixed manual/managed input before credential lookup or I/O.

## Optional agent API discovery

Authorize agents to register public OpenAPI sources in a running multi-server
HTTP deployment with independent `provider_management_local`,
`provider_management_remote` and `raw_url_calls` booleans (all default `false`),
and an explicit existing `wallet` on each participating endpoint. Local management
authorizes bundled listing and ID registration; remote management authorizes
remote details and public HTTPS OpenAPI registration; raw permission authorizes
documented URL calls. None allocates or funds a wallet.

`remote_provider_directories` defaults to `[]` and accepts any subset of
`coinbase`, `coinbase_curated` and `x402-list`. Unknown values reject; repeated
identical entries select a directory once. `coinbase` includes curated and
non-curated resources; `coinbase_curated` restricts the same search to curated
resources. The list has no authority with remote management disabled. Empty
selection omits remote search while permitting URL details and registration. It
is not a destination allowlist. Retired `source_management` and
`sources_search_tool_instructions_text` settings reject without aliases.

The [comprehensive example](../examples/deployments/comprehensive.toml)
uses its existing company pool on the toolbox listener; the managed starter uses
its research pool. The [agent source guide](agent-sources.md#configuration-and-authority)
explains persistence and wallet sharing. Registered APIs share their pool's
existing reserve claims regardless of registration count.
Registration does not create wallet pools or trigger funding.
Tool counts do not gate registration, restoration, publication or refresh.
A `tools/list` request with more than 500 advertised definitions produces an advisory
WARN whose message states the endpoint name and current tool count and suggests
explicit `discover_on_demand = true`. Each listing may warn again. The advisory
counts filtered API definitions plus wrappers (only wrappers in on-demand mode)
and never changes access.

| Arrangement | Configuration | Payment identity sharing |
| --- | --- | --- |
| All agent-added APIs share one wallet | Each listener sets `wallet = "agent_shared"` | Across APIs and participating listeners |
| Listener-specific wallets | Each listener names its own `wallet` profile | Across added APIs on that listener |
| Operator-selected listener groups | Several listeners reference the same named `wallet` profile | Within the configured group |

A listener's bearer token defines its authority. Registrations remain local to that
endpoint; they cannot be published to another endpoint. The operator selects persistence
with each endpoint's `source_registry_file`; agents cannot choose scope or lifetime.
Omit it for registrations that last until exit. Persistent endpoints need distinct
registry files: exclusive ownership prevents sharing one file, and saved records
owned by another endpoint fail restoration without modifying the registry.
Existing listener filters still constrain imported API tools. Parallel callers on the
same endpoint share one import slot. Concurrent additions publish complete snapshots;
repeat a busy or lost add with the same selector. Repeated provider IDs and generic
specification URLs return the accepted registration on that endpoint. See the [agent source guide](agent-sources.md).

`x402_treazury_providers_list_local` lists or searches bundled reviewed definitions
offline, including compatibility repairs and filters. Optional `q` uses BM25 and
literal-priority matching across ID, name, description and API base. Blank queries
enumerate in provider-ID order. `page` defaults to 1 and `per_page` to 25, with any
positive page size. Listing performs no wallet or network work. Metadata describes
addable providers; live `tools_search` returns callable tools on the endpoint.
An absent `api_base` means import uses the schema's API server. Unknown cost does
not mean free, and registration/help do not establish live paid qualification.

`providers_add_local` takes an exact `provider_id`; `providers_add_remote` takes a
public HTTPS OpenAPI 3 JSON `spec_url`. Both accept optional `name`. A unique exact
canonical bundled specification URL, including its query, selects that definition
even with local management disabled, unless an accepted generic registration
already owns the URL. Ambiguity returns available local choices when permitted,
otherwise operator-resolution guidance; remote import has no ID override. Base,
hostname and inferred resource paths do not match specifications. Bundled settings
and embedded schemas are preserved, and names default to short IDs (`exa` yields
`dyn_exa__search`). Repeated registration returns accepted state without refresh.

`providers_search_remote` queries every selected directory concurrently with `q`,
`limit` (default 5, range 1–20 per directory), and `max_price_usd`. Either Coinbase
selection imposes its 400-character query bound; x402-list alone does not. Coinbase
uses the selected curated scope and Base mainnet USDC/exact constraints; x402 List
uses `/best`, Base, `prefer=balanced`, `require_verified=false`. Price filters cover
Coinbase resources or x402 List minimum service prices, not spending authority.
Results form a dictionary keyed by configured identifiers: each success is the
complete upstream JSON object. Local failures use `local_error` and preserve
successful siblings; attribution, relay advisories and typed possible-payment facts
remain outside successful objects. One call can authorize multiple charges. There
is no automatic federation replay, whole-search deadline, merged output or combined
truncation. See [remote search contracts](agent-sources.md#external-directory-matching).

Either management capability exposes `tools_search` and `tool_call`, independently
of `discover_on_demand`; raw-only permission does not. Disabled operations fail
before identity lookup, I/O and side effects. Saved registrations retain existing
visibility, filters and payer bindings when either management capability remains
enabled. Disabling both disables restored dynamic registrations; raw permission
alone does not restore them. Operator inspection/removal remains
available regardless of capabilities; refresh requires either management capability
before loading the existing payer, for generic and bundled records alike.

Given a resource URL such as `https://api.example.com/weather/v2/forecast?city=London`,
try `provider_details_remote(base_url="https://api.example.com/")` when no API base
is documented. This probes fixed document locations at the supplied root; it does
not call the resource or prove that the root is the API base. Prefer an explicitly
documented base path: root discovery can miss documentation beneath another path
or on another host. Bundled matching remains exact by origin and base path.
`x402_list_id` is the exact native `slug` from x402 List, not a registration ID
or Coinbase resource URL. A Coinbase usage-document URL can be passed directly
as `documentation_url`.

`x402_treazury_provider_details_remote` resolves exact bundled provider matches before
probing specification/documentation URLs. Exactly one `x402_list_id`, HTTPS
`base_url` or `documentation_url` is required, and every mode requires remote
management. ID mode is advertised and accepted only with `x402-list` selected;
URL modes remain available with Coinbase-only or empty directory lists. Matches return IDs and schema provenance, skip speculative provider
requests and make no live reachability/qualification claim. A slug causes one
ordinary potentially paid directory lookup; a base URL skips it. Results retain the listing alongside URL-discovery status, even when a missing
or rejected API base prevents probing. Each candidate gets one validated GET,
with four candidates in flight and existing spec/help limits. Available wallet
identity is pinned; without it the URL's discovery identity keeps origin retrieval
available. No wallet is allocated or funded for probes. Configured on-demand
fallback can charge for blocked document retrieval.

Alternatively, `provider_details_remote(documentation_url=...)` reads one exact public HTTPS
usage-document URL, allowing a query. This selector is exclusive with `x402_list_id`/`base_url`.
It uses the shared unsigned origin sequence with provider-management identity,
transport, public-destination policy, shared help-byte default and network response inactivity. Configured
on-demand fallback can acquire the document through the GET relay list. Complete accepted UTF-8 text is returned with caller-supplied
provenance; failures are visible without partial text. See the
[document contract](agent-sources.md#listing-and-url-discovery) for accepted MIME
types and HTML rejection. Reading help grants no execution or payment authority.

With `raw_url_calls = true`, `x402_treazury_raw_call` calls a documented public HTTPS x402 API URL without
registering an OpenAPI catalog. It takes a required HTTP `method`, `url`, optional
`query`, string-valued `headers` and optional JSON `body`; omitted body and explicit JSON null are distinct.
It uses the endpoint's wallet, ordinary payment admission, public-destination
policy, shared response-byte default, network timeout and network factory. It is independent of API
tool-name filters and accepts no agent wallet or payment settings. Nonempty
[manual headers](agent-sources.md#manual-raw-headers) disable relay forwarding and
cross-payer handoff; header-free calls retain ordinary behavior.

Registration returns a readable name, prefix, visible tool count, status and next step.
Disabled registrations direct operator repair/refresh; enabled registrations with
zero visible tools direct inspection of source selections and listener filters.
Repeating registration does not refresh it or make hidden tools callable.
An omitted name defaults to its provider ID for bundled sources or its specification
hostname for generic sources, with collision suffixes.
Search filters use optional `source_name`; calls use stable readable `tool_id`
names resolved against the current endpoint catalog. Internal source UUIDs,
revisions and operator registry inspection remain available. Legacy registration
receipt fields are ignored; source records remain durable before publication.

The stable `x402_treazury_tools_search` and `x402_treazury_tool_call` tools let clients use new
APIs even when they cache their initial tool list. HTTP remains stateless and does
not advertise list-change notifications. Imports require public HTTPS, use the same
direct/Tor network factory, and never run pricing sweeps. Persistent registrations
use a separate owner-only SQLite registry; they do not require a wallet encryption key.

`providers/x402-list.toml` exposes the supported public directory catalog with
advisory research recommendations. Directory reads may become paid after a
shared-IP quota; ordinary spend caps apply. Specific owner-token workflows and
an unrepresentable submission schema are excluded. The embedded directory keeps
its separate intentional three-read contract.
Directory results are leads, not automatically imported schemas.

See [agent source management](agent-sources.md) for the complete configuration,
agent workflow, limits, wallet/privacy behavior and recovery procedures.
`config show` inspects grants and wallet bindings offline;
`x402_treazury sources inspect --config FILE` inspects saved registrations.

## Provider reliability observations

Provider TOML files may carry `reliability_tags` and a concise `reliability_note`.
These describe observed behavior, independently of OpenAPI tool-selection
`tags`. They do not hide tools, block payments, or enable retries.

| Tag | Startup warning |
| --- | --- |
| `upstream_timeout` | Provider reported upstream timeouts, possibly inside an otherwise successful response. |
| `upstream_rate_limited` | An upstream dependency rate-limited the provider. |
| `intermittent_response_body` | Response delivery failed after payment; a failed download can still be charged. Do not automatically replay it. |
| `catalog_http_403` | Repeated catalog HTTP 403 responses can prevent live catalog startup. |
| `intermittent_catalog_connection` | Catalog connections have intermittently timed out. |
| `intermittent_help_connection` | Help retrieval has intermittently timed out. |
| `degraded_upstream_data` | Successful responses have contained failed or missing upstream inputs. |
| `questionable_result_relevance` | Result metadata did not match the query; check relevance. |

Serving startup writes a warning to **stderr** for each unique tag, including the
source name and evidence note, before fetching that source's catalog. A deployment
warns once per listener-bound source/tag, even when several listeners share it.
Declared unbound sources do not warn; bound sources warn even if later tool filters
remove all their tools. Standalone serving also warns. Inspection commands
(`config show`, `config check`, `catalog tools`, `catalog tags`, `catalog route`)
remain quiet; `config show` exposes the annotations and their origins.

Annotations follow normal TOML composition: omission inherits, a supplied list
replaces the entire list, and `reliability_tags = []` suppresses its warnings.
`reliability_note` is replaced independently. Unknown tags are configuration
errors. Describe the affected stage and revise notes when new evidence warrants it;
keep dated measurements in private local reports.

The initial annotations cover AgentFund, AgentUtility, Arkham, OneShot, Kronos,
RegimeShift and Concordance. These observations were made through Tor, but do
**not** establish Tor causation or long-term failure rates; warnings therefore
apply in both network modes. See the [provider caveats](../providers/CAVEATS.md) and
[qualification scope](testing.md#qualification-status).

## SocialFetch and tag selection

`providers/socialfetch.toml` uses the vendor's live OpenAPI tags through the generic
catalog. It exposes platform routes, excludes `Auth`, account/monitor ownership
and webhook lifecycle operations, and deliberately omits `GET /health`.
Other public `System` operations remain available. Credit prices are embedded in the spec.
Each selected tool retains credit terms and a USDC estimate calculated with the
provider’s explicit `credit_pricing` rate, independently of server instructions.

```sh
target/debug/x402_treazury catalog tags --provider providers/socialfetch.toml
target/debug/x402_treazury catalog tools --provider providers/socialfetch.toml --tags Twitter,YouTube
```

The [comprehensive deployment](../examples/deployments/comprehensive.toml) selects
separate social and web/community platform sets. TOML sources accept `tags` and `exclude_tags`: each specified list
replaces the corresponding provider list; omission inherits it, and `[]`
clears it. Matching is exact and case-sensitive. Include tags match any listed
tag; exclude tags win. Tag selection combines with source path filters, then
listener tool-name selection. Untagged operations cannot match an include tag.

`catalog tags` prints JSON counts from the unfiltered source (grouped by source
in meta-config mode). It loads no credentials, fetches no help, and performs no
pricing probes. A remote spec is still fetched once. Use `--spec tests/fixtures/socialfetch_openapi.json` on the single-source command for an
offline sampled inventory. This scoped capture is not the complete vendor API.
It is mutually exclusive with other inventory commands.

SocialFetch's `include: ["/v1", ""]` uses the longest matching naming anchor:
`/v1` strips that segment; the empty fallback preserves other paths. Consequently
`socialfetch_twitter_profiles_handle` routes to `/v1/twitter/profiles/{handle}`,
while LinkedIn v2 names retain `socialfetch_v2_linkedin_*`. Both API versions
have the `LinkedIn` tag. Setting `--include /v1` selects v1 only. Naming never
changes the HTTP path or adds a version to `base_url`.

## Optional Tor routing

Direct networking is the default. Add this table to a deployment meta-config to
route all application outbound HTTP and gRPC through an existing Tor SOCKS port:

```toml
[network]
mode = "tor"
socks_endpoint = "127.0.0.1:9150"
isolation_namespace = "x402_treazury"
socks_auth = "tor_extended"
connect_timeout_seconds = 120
read_timeout_seconds = 60
tor_isolation_retries = 2
```

`tor_isolation_retries` defaults to 2 and counts fresh owner-bound scopes after
an initial unsigned Tor attempt. Zero allows one origin attempt. Nonnegative
values are accepted when their total attempt count fits the platform's `usize`;
overflow is rejected. Inspection exposes the effective value. Direct mode and
explicit direct warming use one origin attempt (Base's existing direct transport
recovery remains separate). This is an attempt count, not a timer or permission
to replay a signed request. Whole-section network-file replacement also resets
an omitted count to its default. See [retry policy](network-egress.md#retry-ownership)
for participating callers and transient versus local setup failures.

`read_timeout_seconds` defaults to 60 seconds in **both direct and Tor modes**.
It limits inactivity while waiting for headers or reading the body. Every
successful read renews the budget; catalogs and API responses have no total HTTP
download deadline. The same setting on a provider/source
overrides the network default for that scope, including catalogs and help. Bundled
dynamic sources preserve that setting; generic imports and raw calls use the network default.

`connect_timeout_seconds` is separate, defaulting to 15 seconds direct and 120
seconds Tor. Both settings work in either mode; there is no Tor read-timeout floor.
Read timeout accepts 1..86400 seconds at network level and
positive fractional seconds at provider level. Connection timeout accepts 1..300.
Base RPCs, chain views and receipt verification have no separate total deadline;
they use these connection and read-inactivity limits. Tor Base reads exhaust the
selected scope plus `tor_isolation_retries` fresh scopes per endpoint before configured fallback;
direct mode retains its two-attempt transport recovery. See [Base policy](wallet-cli.md)
and [network policy](network-egress.md) for status eligibility, validation and
provider evidence boundaries. Protocol transaction expiry
and qualification deadlines remain independent. Managed payment
admission has no total timeout; legacy wallet `wait_seconds` is ignored with a
warning and omitted from effective configuration.

Legacy network `request_timeout_seconds` is accepted as an alias with its literal
value now meaning read inactivity. Provider `timeout`, import
`fetch_timeout_seconds`, and CLI `--timeout` remain compatibility spellings. Use
only `read_timeout_seconds` in new configs. Do not combine it with `timeout`. The bundled comprehensive example now
uses 60 seconds; user-owned local configs retain their existing explicit values.

For deployment, standalone or wallet commands, use a separate network file:

```sh
x402_treazury catalog tools --provider providers/socialfetch.toml \
  --network-config examples/network/tor.toml
x402_treazury wallet init --state-dir state/public-demo \
  --network-config examples/network/tor.toml
```

Paths above assume the repository root as the current directory. Deployment commands,
including wallet commands, use their embedded policy unless `--network-config` is
provided. The external file replaces the entire network policy and any supplied
relay section before validation or client creation; it does not rewrite the
deployment or change wallet identities. This also applies to `catalog warm` and
`sources refresh`. `examples/network/tor.toml` configures ordered paid relay fallback for
deployment catalog discovery and Coinbase search. Standalone provider and
wallet commands use only the transport policy and never load the relay provider.
`catalog warm --direct` retains its
explicit direct-fetch behavior even when a network override is supplied.
Qualification snapshots, bound/supervised serving and supervised wallet sync keep
the policy in their pinned deployment and reject an external override; select the
policy in that deployment before preparation. Provider files and MCP tool arguments
cannot choose transport.
`config show` reports effective settings without reading keys or contacting Tor.
Offline wallet inspection, backup and explicit-birthday init remain offline.

The SOCKS endpoint must be a literal loopback IP and nonzero port. Run Tor Browser
or a Tor daemon on that endpoint; 9150 is an example, not an auto-discovered port.
If that port is unreachable during ordinary serving startup, an ERROR names the
configured SOCKS endpoint, with a reminder on the next failed probe after each
minute while startup discovery is waiting. Finite startup discovery retries the local
TCP connection every five seconds, then resumes automatically without a restart.
Ready listeners, fresh cached catalogs and independent financial work remain
available. This checks TCP reachability only, not Tor bootstrap or destination
connectivity, and stops after the first successful connection. Explicit sync,
inspection and qualification retain their normal failure behavior.
A standalone daemon can use `SocksPort 127.0.0.1:9050 IsolateSOCKSAuth`. Optional
`socks_auth = "legacy"` uses username `x402_treazury` and an identity-derived password.
The default extended format uses username `<torS0X>0` and places the namespace and
token in the password. Both carry the same isolation boundaries. No authentication
downgrade or direct fallback is allowed. There is no additional key file or setup
command: tokens are deterministic, domain-separated SHA-256 hashes of the namespace
and existing identity. They provide circuit isolation, not secrecy from local Tor.

Paid challenge/retry traffic, Base address queries and NEAR funding jobs use the
actual EVM address identity. A rotated address gets a new pool of connections.
Shared treasury sync uses the treasury UUID; specs and help use discovery
origin identities. Pending operations keep their original recipient identity after
restart and retirement. All modes use these same pools and payment state machines.
Paid traversal follows the [Best-effort wallet rules](wallet-rotation.md#best-effort-wallet-rules):
a recognized structured insufficient-funds 402 after a signed attempt can visit
each captured eligible wallet once. Earlier authorizations remain chargeable;
other failures and restarts do not grant a logical-call replay.

Tor mode sends destination DNS through SOCKS, preserves TLS validation, and ignores
ambient proxy/NO_PROXY settings. Direct mode also ignores those settings: the explicit
network policy is authoritative. Local MCP listeners remain local. Tor outage yields
bounded errors and retains financial journals; it never permits direct egress. Tor
can reject private-address destinations, and vendors can reject exit traffic.
Existing operation and quote deadlines still apply. Changing mode, namespace or
authentication format requires restart.

Isolation prevents different identities from sharing a circuit when the configured
Tor listener honors it; it does not guarantee different exit IPs or hide public-swap
links, shared API credentials or identifying request contents. Identical identities
and namespaces intentionally derive the same token across deployments. Use distinct
namespaces for deployment separation. Build-time dependency/parameter downloads are
outside runtime routing. See [runtime egress and verification](network-egress.md)
for the constructor inventory, tests, optional live smoke command and OS-enforced
egress boundary.

## Creating offline OpenAPI fixtures

The `snapshot_spec` development utility accepts a local JSON file or an HTTP(S)
URL and a required output path. It removes operation responses and
`components.responses`, preserving request schemas, tags, descriptions and
vendor extensions. Output has recursively sorted keys, compact JSON, ASCII
Unicode escapes and a trailing newline. It replaces the destination only after
successful download, parsing and serialization. The destination directory must
already exist. No wallet, pricing probes or Python installation is needed.

```sh
cargo run --locked --example snapshot_spec -- \
  https://www.socialfetch.dev/openapi.json /tmp/socialfetch-requests.json

# Offline input and utility tests:
cargo run --locked --example snapshot_spec -- \
  /tmp/socialfetch-openapi.json /tmp/socialfetch-fixture.json
cargo test --locked --example snapshot_spec
```

Use it for JSON OpenAPI request fixtures from any provider. Review fixture diffs
when refreshing a vendor spec, including changes to the pinned catalog contracts.

## Provider HTTP and TLS policy

Participating catalog, unsigned API and Base reads may recover TLS
failures on `network.tor_isolation_retries` fresh Tor scopes after the selected
scope (default 2). Every attempt
retains certificate, hostname and configured protocol verification. This grants
no replay authority to signed requests and does not change direct-mode recovery.

HTTPS provider traffic requires **HTTP/2 and TLS 1.3** by default. This applies to
catalogs, lazy help and unsigned/signed API requests. Set
compatibility flags in the provider TOML or override them on a deployment source:

```toml
allow_http1 = true  # permit HTTP/1.1 fallback; still offer HTTP/2
allow_tls12 = true  # permit TLS 1.2; still prefer TLS 1.3
```

Both default to `false`, inherit through `extends`, and appear in `config show`.
They are independent: an HTTP/1.1-only server with TLS 1.3 needs only
`allow_http1`. There is no automatic downgrade retry on a strict connection
failure. These flags do not relax certificate or hostname checks. Existing
cleartext `http://` URLs retain their previous behavior; these settings do not
upgrade them to HTTPS. Treasury/NEAR/RPC and lightwalletd transports keep their
own existing policies. Agent-added providers use the strict defaults; agents
cannot supply compatibility overrides.

Concurrent requests may multiplex over one HTTP/2 connection when origin,
wallet/discovery identity, timeout and transport policy match. Different wallets
and discovery traffic retain separate pools and SOCKS identities. A wallet
rotation never reuses the previous wallet's TLS connection. Server limits and
connection closure can require additional connections. Protocol changes do not
add signed-payment retries.

DEBUG logs report the actual response `http_version` and stage (catalog,
help, agent import, payment challenge/result), without URLs or payment credentials.
This is HTTP version telemetry, not negotiated TLS-version telemetry; a successful
strict HTTPS handshake enforces the TLS 1.3 minimum. Provider compatibility has
not been surveyed live; use the flags only for demonstrated compatibility needs.
Cover traffic defaults on with Tor and off in direct mode. Same-origin HTTPS
catalog/help documents are bounded range candidates; explicit profiles can add
randomized padding. Set `network.cover_traffic_enabled = false` globally, or
`cover_traffic_enabled = false` in a provider/source to opt out. Explicit profiles
can set `fallback_url` to a same-origin `llms.txt`. These requests use the same pools. See
[cover traffic](cover-traffic.md) for configuration, five bounded distributions,
scoped status and experimental limitations. Periodic/randomized PINGs are not implemented.

## Pricing metadata and runtime hints

Tool descriptions retain authored provider/schema prices and configured credit-to-USDC
estimates. Missing structured prices remain unknown. Startup, inspection and warming
never send endpoint requests to discover prices. Actual calls still acquire a fresh
unsigned x402 challenge and enforce the selected payer's payment authority.

Validated challenges from actual managed calls can teach the passive runtime price
hint cache. Hints advise existing wallet selection; they never authorize signing,
initiate HTTP, refresh catalogs or alter tool descriptions. Receipt age, identity
partitioning and expiry are described in [architecture](architecture.md#wallet-selection-and-network-isolation)
and [resource limits](local-resource-limits.md).

## Automatic discovery disk cache

Deployments with an **existing** `treasury.state_dir` automatically use its
`http-cache/discovery-v1.sqlite` for remote API catalogs.
No wallet is opened and no state directory is created for caching. Standalone
providers and deployments without existing treasury state retain their in-memory
behavior. `config show` remains offline. A provider or source can opt out with
`http_cache_enabled = false`; this also prevents sharing a disk-enabled alias's
catalog load.

Catalogs are local snapshots, using a provider/source setting:

```toml
catalog_cache_ttl_seconds = 86400 # default: 24 hours; 0 disables catalog disk caching
```

This default shares a constant with the runtime API price-hint lifetime. A catalog
override does not change that process-wide hint lifetime; see
[local resource limits](local-resource-limits.md).

A deployment source can override its provider's value. Any positive integer sets
local retention regardless of upstream `Cache-Control` or `Expires`, including
`no-store`, `no-cache` and `max-age=0`. DEBUG logs record
`cache_policy="local_override"`, `ttl_seconds`, and a fixed `origin_policy` label.
`http_cache_enabled=false` disables disk caching. Changing the catalog TTL separates disk entries and shared startup loads.

Caching uses the normal unsigned GET; there are no extra HEAD probes. The TTL
starts after a complete catalog download. Once expired, catalogs with `ETag` or
`Last-Modified` use one conditional GET; a matching 304 reuses the body and starts
a new local lifetime. Without validators, the catalog is downloaded again.
Failed refreshes never fall back to expired documents.

This owner-only private cache permits `private` responses and responses carrying
`Set-Cookie`; cookies are neither saved nor sent. Ordinary `Vary` is supported by
pinning the unsigned discovery request profile (`Accept: */*`,
`Accept-Encoding: identity`, no cookies, credentials or custom request headers).
The profile version and URL are part of the cache key. `Vary: *`, conditional-header
variants and credential-bearing URLs remain excluded. A relay's effective target
request headers are not established by origin transport, so relay responses with `Vary` are not persisted.
Relay catalogs use the same local TTL, measured from the original completed relay
response so sharing it cannot renew its lifetime.

The local-policy cache key causes one initial miss for catalogs written by older
versions; those disposable entries are left for eviction.

Keys separate exact URLs, discovery network/isolation policy, transport flags,
timeouts, limits and resource kind. The owner-only SQLite cache holds at most
4,096 entries and 128 MiB of payload/metadata, evicting oldest entries with a log.
Its database is capped at 144 MiB; rollback journals can temporarily add up to
another database-sized file. Cache errors log a fixed warning and fall back to
normal origin loading, without retrying a failed origin request. Oversized origin
catalogs still fail with the normal `max_spec_bytes` error. Cache contents may
reveal provider interests; treat the directory as private disposable state. To
clear it, stop the application and remove only `http-cache/`.

Live qualification and captured qualification catalog loads bypass disk caching,
so a historical cache hit cannot qualify a fresh network observation. Ordinary
loads report source-specific cache decisions on stderr at DEBUG: catalog
`miss`, `disk_hit`, `revalidate`, `revalidated`, `shared_load`, and successful
`stored` events. Configuration logs distinguish disabled caching from a missing
cache directory. Response policy labels explain `local_override`,
`requires_revalidation`, `no_store`, `vary_star`, or invalid/unsupported policies
without exposing raw headers or validators. A fresh policy is eligibility;
`stored` confirms a successful disk write. Relay cache reuse and directly warmed
cache provenance remain explicit.

Catalogs with `max-age=0, must-revalidate` reuse local snapshots until the configured
TTL expires; then a 304 can reuse the saved body. Missing entries alone do not prove a provider forbids
caching: expiration, eviction, network/timeout policy changes, and failed writes
also affect reuse.

### Warming a selected source, including directly for a Tor deployment

```sh
x402_treazury catalog warm --config deployment.toml --source provider_id
x402_treazury catalog warm --config deployment.toml --source provider_id --direct
```

`--source` selects authored source IDs from the deployment, not provider file paths;
repeat it or use a comma-separated list. Omitting it selects every declared source.
Only selected catalogs are fetched. The command requires an existing treasury state directory and rejects
`http_cache_enabled = false` or `catalog_cache_ttl_seconds = 0` on a selected source. Without `--direct`, it uses the
deployment's network policy, replaced by `--network-config` when supplied. It starts
no listeners or funding workers. It opens
only the resolved discovery wallets when paid relay warming is enabled below;
otherwise it remains unsigned and opens no wallet.

`--direct` is an explicit exception for this dedicated warming process. Its
unsigned discovery requests use direct egress through the normal network factory,
so those providers can observe the machine's direct IP address. The deployment
file and serving policy remain unchanged. Entries have separate direct-warm
provenance and are bound to the original configured network/isolation policy,
URL, transport flags, timeouts and limits. Ordinary direct-mode cache entries do
not gain this cross-policy permission.

Normal serving/inspection can reuse explicitly warmed entries only while fresh,
with a stderr warning identifying their direct origin. It never sends a directly
fetched validator over Tor. Once entries expire, normal loading uses the configured
network; it does not fall back to direct access. Repeat the explicit warm command
to refresh directly. Paid requests, real payment challenges, lazy help, agent
imports and treasury traffic always retain their configured policy. Qualification
continues to bypass all disk caches and cannot invoke this warm path.

The command prints a JSON summary. `catalog_cache` distinguishes `fresh`,
`fresh_via_relay`, `requires_revalidation`, `not_stored` and `local_file`. Catalogs use the configured local TTL,
including when upstream requests no storage or immediate revalidation. Variant,
size and provenance restrictions still apply; `--direct` does not override them
or extend lifetimes. A failed multi-source warm can leave completed disposable entries.

## Relay fallback

A deployment can configure ordered paid relay fallback for catalogs, document
reads, built-in directory searches and explicitly enabled API requests, in its
own configuration or through
`--network-config`. Origin acquisition has one attempt in direct mode, or the
initial attempt plus `network.tor_isolation_retries` fresh owner-bound Tor scopes.
Eligible origin statuses are `400`, `403`, `408`, `429`, `500`, `502`, `503` and
`504`; eligible transport/body failures share that sequence. Valid `Retry-After`
on `429`/`503` applies before another attempt and before a configured relay handoff.
No origin sequence restarts between relay entries. Local destination, authority,
setup and download-limit rejections cannot purchase fallback. Complete catalogs
reject unsolicited `206` or `Content-Range` responses without paid fallback.
Static remote HTTP and JSON-parse failures retain their broader fallback contract;
Coinbase also preserves remote-status/JSON fallback. Successful empty or partial
search-result metadata is terminal success.

```toml
[relay_fallback]
source_discovery = true # directory/resource searches; default true
api_requests = false # actual API data; default false
provider_metadata = true # catalogs/help/imports/probes; default true
# wallet = "web" # optional static payer override
# sources = ["social"] # optional static source selection; empty selects all

[relay_fallback.relays]
GET = ["../../providers/curl/provider.toml", "../../providers/agproxy.toml"]
POST = ["../../providers/agproxy.toml"]
HEAD = ["../../providers/agproxy.toml"]
```

Method keys are exactly `GET`, `HEAD` and `POST`, selected by the original target
method. A missing or empty list means origin-only for that method; lists do not
inherit from GET. Supported consumers are static discovery, Coinbase searches,
source imports, help, explicit usage documents, URL-discovery probes and
representable unsigned API, directory and raw GET/HEAD/POST requests. Ordering is
operator-authored, with no implicit vendor ranking or extra provider. Each listed
provider declares `relay_format = "curl"`, `"agproxy"` or `"horizonpulse"` in its
own TOML. Ordinary providers need no format field. Unsupported request bodies,
bytes or required headers skip the entry before paid I/O; headers and bodies are
never silently discarded to fit another adapter. Target payment signatures and
other credentials are not forwarded.

The section is named `relay_fallback`, with provider lists under `relays`.
The old `discovery_relay` section, `fallbacks` lists and single `provider` shorthand
are rejected. Every listed provider requires explicit `relay_format` metadata.
Inspection emits canonical method lists without reading
provider files. Paths in lists resolve relative to the declaring deployment/network
file; schemas/help paths inside providers resolve relative to their TOML.

Runtime bootstrap validates every configured adapter, operation and filter using
local schemas before payment. The bundled Curl schema, `agproxy-openapi.json` and
`horizonpulse-openapi.json` are complete pinned runtime assets; bootstrap fetches
no schema or help document and rejects nonlocal schema references.
[Provider caveats](../providers/CAVEATS.md) own provenance and update instructions.

Relay preferences classify each request by trusted caller purpose, independently
of HTTP method or destination:

| Preference | Eligible fallback | Default |
| --- | --- | --- |
| `source_discovery` | Built-in Coinbase resource search and x402 List browse, recommendation and service-detail requests | `true` |
| `api_requests` | Ordinary generated/configured API tools and `raw_call`, including application search APIs or direct calls to directory URLs | `false` |
| `provider_metadata` | Static schemas at serving startup and explicit warming, imports/refreshes, help, usage documents and provider URL/specification probes | `true` |

These defaults apply only when the section exists. Without it no paid relay is
created. Disabled preferences leave origin retrieval available but authorize no
relay request or signature. `serve`, `warm`, `on_demand` and `search` are rejected
configuration keys. Method lists, formats and static `wallet`/`sources` bindings
remain separate settings; runtime reads retain the authenticated endpoint/source's
payer. Enabling metadata permits paid catalog fallback during ordinary serving and
explicit non-direct warming. Inspection remains unsigned, `--direct` warming
bypasses relays, and no warming starts automatically.

A mixed `provider_details_remote` call applies discovery permission to its directory lookup
and metadata permission to each later provider document/probe. Help remains
metadata when invoked through a tool wrapper. No agent argument can override a
category, and an application search API remains API work. Metadata contains no API
argument map, but its URL and query string can still disclose sensitive information.
Every enabled relay sees the request it handles and supplies its response plaintext.

No runtime read allocates wallets, starts funding or waits for funding. For document reads, missing
payment authority leaves unsigned origin retrieval available and relay fallback
unavailable. Runtime relay exchange adds no sticky response cache, readiness queue
or provider breaker; existing help/import coalescing remains with those owners.
Repeated Coinbase searches are fresh operations and may authorize new payments.
The caller's response bound covers the outer envelope; its spec/help/response
bound applies to decoded content. Inactivity precedence is source management,
relay provider, then network. Provider transport permissions govern the relay leg.

Unsigned API/directory/raw requests use the original method's list after the
common eligible origin sequence. Origin `402` retains normal payment handling;
relay-supplied target `402` is terminal and never signed. Any earlier signed target
attempt suppresses generic fallback, including later wallets' unsigned failures.
Requests preserve their built URL/query, bytes and required headers. Unsupported
entries skip before payment; complete invalid output cannot purchase another read.
Legitimate API `206`/`Content-Range` responses remain allowed. Curl's text envelope
cannot establish binary response bytes and rejects such API responses terminally;
AGProxy/Horizon decode only their documented binary encodings. HEAD returns empty
text regardless of representation MIME. Relay fees stay distinct from a target
payment, with possible fees retained through output and directory processing.

Runtime document readers use the common unsigned origin sequence and ordered GET
relay list after blocking transport failures or HTTP `400`, `403`, `408`, `429`
or `5xx`. Complete invalid schemas/text, unsupported content, unsolicited `206`
or `Content-Range`, local policy/byte bounds, redirects and other `4xx` are terminal.
A relayed `402` never reaches target signing. Origin HTTP remains no-follow;
configured relays can follow redirects under the trust boundary below. Accepted
work retains fetching, validation and cache/source publication after disconnect.
Shared cache identities retain the exact payer and effective document policy;
possible charges survive parsing, publication and cleanup errors.

After exhausted origin attempts, each authored position has at most one logical
acquisition. Its unsigned relay-service requests use the same configured Tor
count, or one attempt in direct mode; signed requests never gain generic replay.
A proven pre-payment provider failure or complete attributable failed target
result can advance according to the consumer contract. Validated relay-declared
truncation can advance; malformed/unattributable envelopes after possible payment
stop. Delivered error/truncated results may already be billed. A refund promise,
timeout or vendor prose cannot prove nonpayment. Every actual authorization retains
ordinary per-payment authority and possible liability; no cumulative spending
counter is added. Local bounds/policy failures are terminal.

A relayed challenge never authorizes target signing.

Without `wallet`, each static source uses an effective assigned wallet profile.
An explicit source wallet takes precedence through normal binding resolution.
A shared source chooses the highest SHA-256 score of the JSON tuple
`["discovery-wallet-v1", source_id, wallet_name]`, with wallet name breaking ties.
This remains stable across restarts/listener ordering; balances, keys and rotating
addresses do not select it. A failed wallet does not select another profile to
repeat relay spending. An unbound source needs its own wallet or the explicit
relay override. `config show` reports `discovery_wallets`; distinct payer scopes
keep separate requests/caches even for the same URL.

Every relay connection uses the configured network factory and existing payer.
Tor never falls back to direct egress. The relay sees target URLs and reaches them
from its own network. Origin HTTP remains no-follow; configured relays may follow
redirects remotely. Requested and reported final destinations must satisfy the
caller's public-destination policy, and every Curl-reported hop is checked.
AGProxy/Horizon Pulse expose only a final URL: intermediate destinations remain
unknown. Missing chain evidence does
not exclude an adapter. This explicitly trusts remote redirect/public-destination
policy and the relay's plaintext content, headers and status; our TLS verifies
the relay, without establishing target TLS or content authenticity. A relay service
endpoint is separate from the requested target. Never use this path for
authenticated target requests. Adding a relay does not expose it as an MCP tool.

Ordinary managed serving binds listeners before funding. Static fallback waits
on existing eligible-payer notifications without allocating funds or consuming
network attempts. Endpoints with `wait_for_catalogs = false` keep available tools
callable during discovery. Paid warming opens only its
resolved wallets, denies new funding and starts no workers; bootstrap/fund them
first. `--direct` warming stays unsigned and bypasses the relay. Ordinary
`config check` and catalog inspection never
pay through the relay. Qualification rejects paid relay startup.

Static paid fetches serialize per provider across wallets through decoding and
publication. Waiter cancellation does not release accepted ownership or duplicate
same-provider I/O. Shared state locking is brief, so cached results and independent
providers remain available during another provider's exchange. Initial
origin-to-relay delay precedes accepted relay ownership/provider gates;
between-provider delays retain the logical call owner without a provider gate.
This partitions existing serialization without adding a resource cap. Static aliases
with the same wallet, catalog contract, authored provider position and
target URL share their first process acquisition and outcome. A terminal decoded
body-bound failure from that acquisition remains shared even if a later alias
allows a larger body; changing the bound does not authorize a second fee. Reusing
a successful cached response still checks the requesting alias's bound. An uncertain
paid failure stops that provider for the run, without disabling an independent
provider; malformed paid envelopes likewise stop the sequence. There is no separate
three-attempt counter or one-second backoff. Independent targets retain access after
a definitive failure. Strict inspection/warming still require complete selected
results; ordinary serving can expose a failed catalog as unavailable. Successful cached results remain usable.

Outer provider and decoded target bounds are separate allocations. JSON escaping
or base64 can enlarge the envelope; raising only the target limit cannot enlarge
the provider's `max_response_bytes`. Adapter external request limits are checked
against actual encoded bytes before payment. A caller's larger accepted-response
maximum alone does not make the request incompatible. Neither local size rejection
permits another paid acquisition. Explicit vendor truncation is distinct from a
local limit or a target's unsolicited partial-document response.

Relay catalogs use the configured local catalog lifetime and the existing private cache, rejecting
relay `Vary` metadata. Disk slots partition adapter/provider, authored position,
payer, target URL and configured bounds, alongside ordinary
HTTP-cache transport policy; their bound-specific keys differ from process outcome
sharing. Relay-disabled inspection cannot reuse them. The outer relay's cache
headers are ignored. Expired relay entries return to ordinary origin-first
acquisition, without origin validator promotion or stale delivery. Warming scans all configured GET provider slots and reports
`fresh_via_relay` for a fresh catalog; responses excluded by cache policy report
`not_stored`. No live paid delivery or Tor qualification is implied by offline tests.

Pricing suffixes use compact provenance labels:

- `Cost: ~$0.015/call [spec].` or `Cost: ~$0.01–$0.20/call [spec].`
- `Cost: free [spec].` or `Cost: unknown.`

An explicit vendor `unit` is preserved instead of `/call`. Dynamic fixed estimates
use `[spec, dynamic]`; unrecognized vendor pricing structures retain their details
with `[spec]`. Unknown assets retain their denomination and network information.
Dollar amounts elsewhere in prose do not suppress the suffix: only an identical
suffix already at the end is omitted.

Providers with structured credit tariffs can opt into display conversion:

```toml
pricing_key = "x-socialfetch-credits-pricing" # preserve original billing prose
[credit_pricing]
credit_cost_key = "x-socialfetch-pricing"
usdc_per_credit = "0.014"
```

The decimal rate must be positive, with at most six decimal places. Conversion uses
exact integer arithmetic and changes descriptions only, never payment challenges,
spending caps or signing. The supported version-1 metadata describes `baseCredits`,
`maxCredits`, `normalizationFailureCredits`, `surcharges`, optional URL batches and
per-returned-record metering. Conditional surcharge amounts are shown as upper
bounds, since the metadata may describe a maximum across multiple assets.
The `includes` surcharge condition names a value in a query parameter's array.
Optional `billableDomainErrors` entries name an error `code`, `httpStatus` and
`credits`; estimates show each error's charge separately from successful-request
costs. Error charges cannot exceed `maxCredits`.
Recognized `x402ExactPolicy` hints (`always`, `fixed_price_only`) leave credit
estimates unchanged; payment negotiation still uses the actual challenge.
Original credit prose remains alongside the estimate. Missing, malformed or unknown
metadata retains that prose and the configured rate, explicitly marks the per-call
estimate unavailable, and logs one WARN summary per catalog build. The summary
identifies the provider prefix, configured metadata key, affected-operation count
and failure-category counts. Only tools retained by the provider's final name
filters count. DEBUG diagnostics identify each affected tool and method with a
fixed reason; raw tariff metadata and parser error text are never logged. Separate
source builds or catalog reloads each report their own summary. Payment limits are
unchanged.

SocialFetch enables this using the historical 0.014 USDC/credit estimate; it is not
a verified current x402 tariff. Every converted tool names the configured rate and
states that the payment challenge is authoritative. Source instruction overrides
and tag filtering (including `comprehensive.toml`) preserve these per-tool estimates.
An explicit tool description override still replaces the generated description.

MCP server instructions preserve authored guidance and include this caveat once:

> Prices are estimates or sampled payment offers, not guaranteed quotes. Costs may vary with arguments; metered charges may be below the displayed maximum. Unknown does not mean free.

Authored description overrides apply last, and description limits remain explicit.
Display prices and runtime hints are informational: each paid invocation still
obtains a fresh challenge and enforces its spend cap before signing.

`treasury.max_sync_age_seconds` and `funding.base_max_block_age_seconds` were removed.
Remove these keys even if their old values were zero. Balances are advisory; SDK
input selection and protocol deadlines govern spending. Treasury discovery is
requested at available-capability startup, explicit sync, enabled funding demand
and validated pending source/shielding work. Recurring observations have a
60-second completion floor and independent error backoff up to 300 seconds; idle
treasury performs no balance polling. Local configured polling/health thresholds do
not shorten network cadence or delay valid preparation/submission handoffs. See the
[wallet observation policy](wallet-rotation.md#wallet-observation-policy).
