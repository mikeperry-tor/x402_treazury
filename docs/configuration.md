# Configuration reference

For a Zcash-and-Tor walkthrough, start with the [README](../README.md).
This reference covers TOML composition, tool selection, transport and pricing.
Commands run from the repository root. The executable requires an explicit
subcommand; running it without arguments displays help and never starts serving.

| Command | Purpose |
| --- | --- |
| `serve --config FILE` | Start a multi-listener deployment and its configured funding workers |
| `serve --provider FILE` | Serve one provider, using stdio by default |
| `catalog tools --config FILE` or `--provider FILE` | Print selected tool inventories as JSON; optional `--discover-pricing` runs unsigned probes |
| `catalog warm --config FILE` | Warm catalogs and optional pricing; configured relay may pay, `--direct` stays unsigned |
| `catalog tags --config FILE` or `--provider FILE` | Print available OpenAPI tags |
| `catalog route TOOL --provider FILE --args '{...}'` | Preview an HTTP request without executing it; standalone providers only |
| `config show --config FILE` or `--provider FILE` | Resolve file settings offline, including wallet bindings for deployments |
| `config check --config FILE` or `--provider FILE` | Fetch catalogs and validate selected tools without signing, probing prices or funding |
| `wallet bootstrap --config FILE` | Fund initial managed pairs before discovery, without listeners; requires `auto_fund=true` |
| `wallet ...` | Administer the treasury |
| `sources inspect --config FILE` | Read persisted agent-added source records |
| `build-info` | Print build identity and provenance |

Use `x402_treazury COMMAND --help` for the options available to each operation.

## Configuration file roles

`--config FILE` selects a deployment: sources, listeners, wallets and network
policy. Wallet commands accept the same file without loading API sources.
`--meta-config` remains a deployment alias. `--provider FILE` selects a reusable
provider for standalone serving and catalog inspection. `--network-config FILE` is an optional network-only
file for standalone operations, not an override for deployments.

Ordinary deployment examples live in `examples/deployments/`, standalone network policies in
`examples/network/`, and reusable APIs in `providers/`. Paths resolve from the file
that declares them. Existing user-owned local copies are not moved automatically.

Managed serving confirms initial wallet pairs before discovery, then runs automatic
replacement funding. `wallet bootstrap --config FILE` performs initial funding
without catalogs or listeners. Explicitly set
`funding.auto_fund = false` to pause it. Merely inspecting a config or compiling
Zcash support does not spend funds. Source limits remain required. See the
[wallet defaults and commands](wallet-cli.md#one-configuration-for-wallet-commands).

## Multiple MCP ports from one configuration

Deployment catalogs load concurrently through a rolling queue. The default is 2
active loads; set a value from 1 to 64 in the deployment TOML:

```toml
[startup]
catalog_concurrency = 2
```

Two catalog slots are a provisional default. The working theory is that fewer
simultaneous large downloads leave headroom when Tor’s prebuilt Conflux capacity
is unavailable. Current measurements do not establish that mechanism or an optimal
limit; see [startup qualification](../tests/STARTUP.md). Pricing concurrency is separate.

A free slot starts the next source immediately. Serving still waits for every
declared catalog and startup pricing discovery; a catalog failure aborts startup
and cancels unfinished loads. Tool ordering and wallet bindings do not depend on
completion order. `config check`, `catalog tools` and `catalog tags` use the same loader;
`config show` remains offline and reports the effective limit. Progress, source
fetch/parse and generation timings, and pricing timings go to stderr; a ten-second
waiting message identifies remaining work. No response content or source URL is
included in these progress messages. Remote spec aliases share a download and parsed document within one deployment
load when the exact URL, requested timeout, byte limit, transport compatibility flags and discovery identity match.
Each alias retains its own tools, filters, base URL and wallet binding. Local files
are read independently. Compatible remote responses can also use the disk cache described below.
Pricing discovery rolls across sources and endpoints with a shared 16-request cap.


[examples/deployments/servers.toml](../examples/deployments/servers.toml) exposes PDL and LoneStar through
separate research and company MCP listeners sharing one static wallet profile:

```sh
target/debug/x402_treazury config check \
  --config examples/deployments/servers.toml
target/debug/x402_treazury catalog tools \
  --config examples/deployments/servers.toml
target/debug/x402_treazury serve \
  --config examples/deployments/servers.toml --env-file .env
```

Serving this example requires `EVM_PRIVATE_KEY`, `RESEARCH_MCP_TOKEN` and
`COMPANIES_MCP_TOKEN` in the environment or env file. Each listener serves
stateless Streamable HTTP at `/mcp` and authenticates with its own token.
Meta-config listeners currently require loopback addresses. There is no hot
reload; restart the process to apply configuration changes.

The version-1 TOML schema separates named sources, wallets and servers, with
optional wallet templates and an automatic assignment policy:

- `sources`: provider settings written inline or imported with
  `extends = "../providers/pdl.toml"`. Every generic catalog setting is
  available here, including `spec`, `base_url`, `prefix`, `timeout` (default 30),
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
  `include_tools`, `exclude_tools`, `tags`, `exclude_tags`, and `max_response_chars`.
  Resource identifiers use lowercase letters, digits and `_`, starting with a letter.

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
complete unlinkability. Every additional managed pool requires its own active
and standby funding. Referencing one profile from more bindings does not create
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
automatic assignment policy. A complete deployment is in
[servers-auto-wallets.toml](../examples/deployments/servers-auto-wallets.toml):

```toml
[wallet_templates.small]
mode = "zcash_rotation"
funding_amount_usdc = "5.00"
max_funding_amount_usdc = "6.00" # explicitly bound bridge-minimum increases
max_api_payment_usdc = "1.00"
max_funding_spend_zec = "0.02"
max_conversion_overhead_percent = 5

[wallet_assignment]
scope = "source"
template = "small"
```

Sources and servers can then omit `wallet`; `[wallets]` may also be omitted.
See [funding limits and fees](wallet-rotation.md#funding-limits-and-fees) for
treasury-wide USDC budgets and optional ZEC withdrawal safeguards.
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

Template changes update settings for existing generated pools. In particular,
`funding_amount_usdc` changes only future address allocations, preserving existing keys
and their captured targets. Changing scope or renaming a source/server may create
new pools. Managed startup disables pools absent from the resolved set and keeps
their keys and history; restoring the previous scope/name resumes them. Static-only
serving does not unlock or modify treasury state.

For automatic bindings, `config show` and inventory additionally report
`template` and `scope`. Configuration inspection includes `resolved_wallets`
(explicit plus generated profile settings), `generated_wallets` (generation
metadata), and `wallet_summary` (distinct managed/generated pool counts and their
combined active-plus-standby target in atomic units and decimal USDC). `config check`
also prints that summary. The total counts each managed pool once, including
explicit unreferenced profiles; it excludes unused templates and static wallets.
It is a configuration target, not a live balance or a quote for additional funds:
existing addresses retain their original targets, and fees and retired balances
are not included. None of these inspection commands allocates keys or funds.

Rust configuration is TOML-only. [providers/](../providers/README.md) contains
all 27 reusable provider definitions. AgentUtility, Glassnode, Concordance,
Straits and Locus have a `provider.toml` and a locally curated `openapi.json` in their own directory.
JSON remains the format for API documents, fixture data and inventory output.
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
`--provider` or `--config`, plus optional `--network-config` for a standalone
provider; CLI/environment runtime overrides are outside
this file inspection command. Wallet/token fields show environment variable
names, never their values.

`config check` validates and reports inventories; `catalog tools` prints per-server
JSON including source attribution. `catalog tags` reports unfiltered source tag
counts. These commands do not create signers, bind ports, or fetch help documents.
Pricing probes require the explicit `catalog tools --discover-pricing` combination.
Remote specs still require network access. Deployment operations accept `--env-file`
except offline `config show`; `--discover-pricing` belongs only to `catalog tools`.
Standalone CLI overrides cannot accompany a deployment `--config`; configure
those values in its TOML instead. Serving/authentication flags belong only to
`serve`.

Startup validates credentials and binds every listener before serving any.
A bind failure releases listeners already acquired. Both standalone HTTP and
meta-config serving handle SIGINT/SIGTERM by stopping listeners and allowing
in-flight calls up to ten seconds to finish. A stderr message announces the drain
and asks you to wait so pending payments can finish safely. Exceeding the deadline
returns an error reporting that paid outcomes may be unknown; signed requests are
never replayed during shutdown. A listener failure also stops its
siblings. Managed profiles retain durable payment uncertainty through shutdown;
automatic NEAR funding is controlled by `funding.auto_fund`.

## Download and display limits

Sources and reusable provider TOML files accept these positive byte limits:

| Setting | Default | Applies to |
| --- | --- | --- |
| `max_response_bytes` | 16 MiB (16777216) | API response bodies, including payment challenges and errors |
| `max_help_bytes` | 4 MiB (4194304) | Lazy help documents |
| `max_spec_bytes` | 32 MiB (33554432) | Static specs fetched from URLs; local operator-authored files are unaffected |

Standalone CLI flags `--max-response-bytes`, `--max-help-bytes` and
`--max-spec-bytes` override provider settings. Meta-config sets them per source;
`config show` reports resolved values and origins. For agent-added sources,
`[source_management]` sets the response/help caps and its existing spec import cap.
Agents cannot raise these operator-controlled limits.

A declared Content-Length over the cap is rejected immediately. Streaming reads
also enforce the cap when Content-Length is absent. An oversized document
is rejected entirely: the agent gets an error naming the limit and setting, and
stderr logs the rejection. No partial content is returned or cached as success.
A paid response may already have settled before its size is known; an oversize
response does not trigger another payment attempt.

`max_response_chars` and `max_description_chars` are separate display controls.
When they shorten text, the text includes a truncation marker and stderr records
the event; the marker is additional to the configured character allowance. They
do not control download size. MCP HTTP requests have a 4 MiB incoming-body limit;
exceeding it returns HTTP 413 with the limit in its message and a stderr warning.

## Implemented behavior

- Composed TOML provider settings and local/remote JSON OpenAPI specs or digests;
  local `$ref` resolution, path/tag selection, deterministic names, flattened
  schemas and routing, vendor prices, description overrides and explicit
  `max_description_chars`. Vendor text has no default truncation.
- `help_url` adds a lazy, cached documentation tool. A failed fetch can retry.
  `instructions_text` and `name` populate the initialize response.
- MCP stdio and Streamable HTTP. The HTTP bearer gate runs before MCP dispatch;
  no session header or sticky session is required. The SDK's default Host checks
  remain enabled, so the demonstrated setup is localhost. Deployment behind a
  different hostname needs an explicit allowed-host configuration change.
- Base canonical USDC payments: v1/v2 `exact` and v2 `upto`, signed by the SDK.
  V2 challenge descriptions are capped at 500 characters for facilitator
  compatibility. The accepted requirements themselves remain intact.
- Asset/network filtering and an exact decimal per-payment cap before signing.
  `--max-api-payment-usdc` overrides `X402_MAX_PRICE_USD`, default `1.00`.
  `none`, `off`, or an empty value removes the amount cap **but retains the
  Base USDC restriction**.
- One signed retry only. A final 402 surfaces its payment error; a paid request
  is not retried again after an ambiguous failure. Redirects are disabled.
- `PaidClient::replace_payer` swaps the signer for subsequent requests. A
  request captures its payer before the unpaid attempt and retains it until
  completion. This API is for static profiles. Managed profiles acquire an
  immutable signer lease after challenge validation and durable admission.

The CLI supports `--spec`, `--provider`, `--base-url`, `--prefix`, `--include`,
`--exclude`, `--tags`, `--exclude-tags`, `--timeout`, `--max-response-chars`,
`--transport`, `--host`, `--port`, `--bearer-token`, and `--env-file`.
Filters accept comma-separated values. Supported
`X402_MCP_GENERIC_*` overrides are `SPEC`, `BASE_URL`, `PREFIX`, `NAME`,
`PRICING_KEY`, `INCLUDE`, `EXCLUDE`, `TAGS`, `EXCLUDE_TAGS`, `INSTRUCTIONS_TEXT`,
`HELP_URL`, and `MAX_DESCRIPTION_CHARS`. Precedence is CLI > environment > config
> default; explicit env-file values override inherited environment values.
Use `--help` for the complete CLI. `catalog route NAME --args '{...}'` prints
method, URL, query and JSON body without making a request or loading a signer.

## Optional agent API discovery

Authorize agents to register public OpenAPI sources in a running multi-server
HTTP deployment with `[source_management]` and `source_management = true` on each participating endpoint. The
[static-wallet example](../examples/deployments/agent-sources.toml) and
[managed-wallet example](../examples/deployments/agent-sources-managed.toml) use one named
`agent_shared` wallet for all added APIs. At a $5 managed deposit size, that is
one $10 active-plus-standby target regardless of how many APIs are registered.
Registration does not create wallet pools or trigger funding.

| Arrangement | Configuration | Payment identity sharing |
| --- | --- | --- |
| All agent-added APIs share one wallet (recommended) | Global `source_management.wallet = "agent_shared"` | Across APIs and participating listeners |
| Listener-specific wallets | Listener `wallet` overrides | Across added APIs on that listener |
| Operator-selected listener groups | Several listener overrides reference the same named profile | Within the configured group |

A listener's bearer token defines its authority. Registrations remain local to that
endpoint; they cannot be published to another endpoint. The operator selects persistence
by configuring `source_management.registry_file`; agents cannot choose scope or lifetime.
Existing listener filters still constrain imported API tools. Parallel callers on the
same endpoint share one import slot. Concurrent additions publish complete snapshots;
repeat a busy or lost add with the same specification URL. Duplicate URLs return the
existing registration on that endpoint. See the [agent source guide](agent-sources.md).

The stable `x402_treazury_tools_search` and `x402_treazury_tool_call` tools let clients use new
APIs even when they cache their initial tool list. HTTP remains stateless and does
not advertise list-change notifications. Imports require public HTTPS, use the same
direct/Tor network factory, and never run pricing sweeps. Persistent registrations
use a separate owner-only SQLite registry; they do not require a wallet encryption key.

`providers/x402-list.toml` supplies five curated read-only directory tools. Directory
reads may become paid after a shared-IP quota; ordinary spend caps apply. Its exact
`include_operations` allowlist prevents new upstream write routes from appearing.
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
| `slow_pricing` | Unsigned pricing discovery has been slow and may delay startup. |
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
catalog. It exposes platform routes, excludes `Auth`, `Monitors` and `System`,
and disables pricing probes because credit prices are embedded in the spec.
Each selected tool retains credit terms and a USDC estimate calculated with the
provider’s explicit `credit_pricing` rate, independently of server instructions.

```sh
target/debug/x402_treazury catalog tags --provider providers/socialfetch.toml
target/debug/x402_treazury catalog tools --provider providers/socialfetch.toml --tags Twitter,YouTube
target/debug/x402_treazury serve --config examples/deployments/socialfetch.toml --env-file .env
```

[examples/deployments/socialfetch.toml](../examples/deployments/socialfetch.toml) configures a selected
platform set. TOML sources accept `tags` and `exclude_tags`: each specified list
replaces the corresponding provider list; omission inherits it, and `[]`
clears it. Matching is exact and case-sensitive. Include tags match any listed
tag; exclude tags win. Tag selection combines with source path filters, then
listener tool-name selection. Untagged operations cannot match an include tag.

`catalog tags` prints JSON counts from the unfiltered source (grouped by source
in meta-config mode). It loads no credentials, fetches no help, and performs no
pricing probes. A remote spec is still fetched once. Use `--spec tests/fixtures/socialfetch_openapi.json` on the single-source command for an
offline inventory. It is mutually exclusive with other inventory commands.

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
request_timeout_seconds = 240
```

Tor defaults to 120 seconds for connection establishment and a 240-second floor
for each complete HTTP request, including its response body. A source `timeout`,
pricing `probe_timeout`, or import `fetch_timeout_seconds` above this floor still
wins; smaller values cannot cut short Tor setup. The network request floor also
applies to application-owned treasury RPC deadlines and managed admission
`wait_seconds`. Explicit network values override these defaults; request timeout
must be at least the connection timeout. Direct-mode budgets are unchanged.
These settings govern our client, not the external Tor daemon. They do not extend
payment authorization expiry, NEAR quote expiry or experiment deadlines.

For standalone operations or wallet commands, use the same table in a separate file:

```sh
x402_treazury catalog tools --provider providers/socialfetch.toml \
  --network-config examples/network/tor.toml
x402_treazury wallet init --state-dir state/public-demo \
  --network-config examples/network/tor.toml
```

Paths above assume the repository root as the current directory. Meta-config wallet
commands take network policy from their deployment file; an external network policy
cannot override it. Provider files and MCP tool arguments cannot choose transport.
`config show` reports effective settings without reading keys or contacting Tor.
Offline wallet inspection, backup and explicit-birthday init remain offline.

The SOCKS endpoint must be a literal loopback IP and nonzero port. Tor Browser or a
Tor daemon must already be running; 9150 is an example, not an auto-discovered port.
A standalone daemon can use `SocksPort 127.0.0.1:9050 IsolateSOCKSAuth`. Optional
`socks_auth = "legacy"` uses username `x402_treazury` and an identity-derived password.
The default extended format uses username `<torS0X>0` and places the namespace and
token in the password. Both carry the same isolation boundaries. No authentication
downgrade or direct fallback is allowed. There is no additional key file or setup
command: tokens are deterministic, domain-separated SHA-256 hashes of the namespace
and existing identity. They provide circuit isolation, not secrecy from local Tor.

Paid challenge/retry traffic, Base address queries and NEAR funding jobs use the
actual EVM address identity. A rotated address gets a new pool of connections.
Shared treasury sync uses the treasury UUID; specs, pricing and help use discovery
origin identities. Pending operations keep their original recipient identity after
restart and retirement. All modes use these same pools and payment state machines.
A changed payer before signing gets one fresh challenge attempt for GET/HEAD;
non-idempotent tools return `payer_changed_before_payment` for an explicit caller
retry. Once a payment is signed/journaled, failures never trigger automatic replay
or a new payer.

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
  https://www.socialfetch.dev/openapi.json tests/fixtures/socialfetch_openapi.json

# Offline input and utility tests:
cargo run --locked --example snapshot_spec -- \
  /tmp/socialfetch-openapi.json /tmp/socialfetch-fixture.json
cargo test --locked --example snapshot_spec
```

Use it for JSON OpenAPI request fixtures from any provider. Review fixture diffs
when refreshing a vendor spec, including changes to the pinned catalog contracts.

## Provider HTTP and TLS policy

HTTPS provider traffic requires **HTTP/2 and TLS 1.3** by default. This applies to
catalogs, pricing discovery, lazy help and unsigned/signed API requests. Set
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

INFO logs report the actual response `http_version` and stage (catalog, pricing,
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

## Pricing discovery

Serving can discover missing prices once at startup through unsigned GETs.
Provider settings enable this by default (`probe_pricing = false` disables it).
Inline sources use the same defaults; a deployment source can override the
provider setting. Validation, ordinary inventory and `catalog route` never probe.
To inspect descriptions with unsigned discovery enabled, without wallet keys,
listener tokens, wallet state access or starting a server:

```sh
x402_treazury catalog tools --provider providers/botsmith.toml --discover-pricing
x402_treazury catalog tools --config examples/deployments/privacy.local.toml --discover-pricing
```

The flag requires `catalog tools` and respects `probe_pricing = false`, route
eligibility, caps, timeouts and the configured direct/Tor policy. Each invocation
is a new process, so repeated CLI invocations can repeat probes. Failed or skipped
discovery leaves prices explicitly unavailable; unpaid success does not establish
that an API is free.

Only tools selected by at least one listener are candidates. Routes with vendor
pricing, templated paths, help tools and non-GET methods are skipped. Candidates
are sorted by path/method and capped per source. TOML controls are
`probe_max_endpoints` (default 200; zero disables requests), `probe_concurrency`
(default 4), `probe_timeout` (default 5 seconds) and `probe_ttl_seconds` (default
3600 seconds). `probe_methods` accepts GET only. Process-wide concurrency is
capped at 16, even when sources request a higher limit. Up to 16 sources discover
prices concurrently; each source uses rolling slots at its own configured limit.
Completed slots are reused without waiting for a batch. All pricing completes
before serving, and descriptions are applied without changing inventory order.
Sources with no discovered prices reuse their original tool definitions, preserving
vendor prose and overrides. Kronos disables startup probing by default because
of its observed slow pricing discovery; a source can explicitly re-enable it.
Paid calls still validate the current challenge and enforce spend caps.

A process-wide cache keys GET requests by their full routed URL (including any
base URL path prefix), transport compatibility flags, disk-cache scope and opt-out. Sources and listeners
with the same policy share results; concurrent matching requests coalesce. Failures, malformed challenges, rate limits and
unpaid successes are cached too. Redirects are not followed. Nothing refreshes
a cached attempt during the process lifetime; TTL expiry makes its result
unavailable to later catalog builds without making another request. Tool
catalogs already built remain unchanged, so `tools/list` never performs network
requests. Restarting allows a new discovery attempt or reuse of a still-fresh disk estimate.

### Automatic discovery disk cache

Deployments with an **existing** `treasury.state_dir` automatically use its
`http-cache/discovery-v1.sqlite` for remote API catalogs and pricing estimates.
No wallet is opened and no state directory is created for caching. Standalone
providers and deployments without existing treasury state retain their in-memory
behavior. `config show` remains offline. A provider or source can opt out with
`http_cache_enabled = false`; this also prevents sharing a disk-enabled alias's
catalog load or pricing initialization.

Support is detected from ordinary unsigned GET response headers; there are no
extra HEAD probes. Catalogs use explicit `Cache-Control: max-age` or `Expires`
freshness, accounting for `Date`, `Age` and request duration. Stale catalogs with
`ETag` or `Last-Modified` use one conditional GET, accepting a matching 304 or
replacing the complete document with a new response. `no-cache` requires
revalidation. Failed requests never fall back to stale documents. Unsupported
responses continue through normal downloads. This conservative cache declines
`no-store`, `private`, all `Vary` variants, cookie-bearing responses and URLs with
embedded credentials; it does not invent heuristic lifetimes.

Pricing persists only the derived display estimate from a usable HTTP 402 with
explicit, positive freshness, capped by `probe_ttl_seconds`. Payment challenges,
headers and paid responses are never stored or reused for signing. An expired
estimate triggers the ordinary single unsigned GET on a later startup, with no
conditional pricing request. Within a process, one-shot success/failure caching
and expiry-without-refresh remain unchanged. Previously built descriptions are
still startup estimates. A disk hit retains its remaining lifetime rather than
receiving a new full TTL.

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
loads report fresh/revalidated catalog hits and pricing estimate hits on stderr.

#### Warming a selected source, including directly for a Tor deployment

```sh
x402_treazury catalog warm --config deployment.toml --source provider_id
x402_treazury catalog warm --config deployment.toml --source provider_id --direct --discover-pricing
```

`--source` selects authored source IDs from the deployment, not provider file paths;
repeat it or use a comma-separated list. Omitting it selects every declared source.
Only selected catalogs are fetched. `--discover-pricing` also warms eligible unsigned
GET estimates, respecting source opt-outs, endpoint caps and the union of listener
filters. The command requires an existing treasury state directory and rejects
`http_cache_enabled = false` on a selected source. Without `--direct`, it uses the
deployment's network policy. It starts no listeners or funding workers. It opens
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
`fresh_via_relay`, `requires_revalidation`, `not_stored` and `local_file`; `fresh_pricing_entries`
counts persistent, fresh estimates. Fetching successfully does not imply caching
is supported: validator-only catalogs still require revalidation, and entries
without explicit freshness cannot bypass a Tor-blocked origin. Normal HTTP storage
restrictions and limits still apply; `--direct` does not force persistence or
extend lifetimes. A failed multi-source warm can leave completed disposable entries.

#### Paid discovery relay

A deployment can opt into Curl HTTP Request as a fallback for catalog and pricing
GETs that fail, including HTTP errors, timeouts, connection resets, body/parse errors,
and missing or unusable pricing headers on a 402 response. Successful 2xx pricing
responses leave the price unknown and do not trigger paid fallback: no payment
challenge was observed, which does not establish that the API is free.
These discovery requests do not immediately
retry the origin: each failure gets one configured relay fallback. Local-file failures
remain local. This does not diagnose Tor blocking; the same policy applies on a direct
deployment. Warnings identify the source, catalog/pricing stage, fixed failure category
and HTTP status when available, without logging request URLs or upstream error text.

```toml
[discovery_relay]
provider = "../../providers/curl/provider.toml" # relative to this deployment file
# wallet = "web" # optional override; otherwise choose from each source's bindings
serve = true   # allow paid fallback during serving startup; defaults false
warm = true    # allow paid fallback during catalog warm; defaults false
# sources = ["social"] # optional authored source IDs; omitted/empty selects all
```

The relay provider must have a local bootstrap catalog exposing `POST /curl` and
an HTTPS `base_url`. The bundled provider pins the vendor schema and advertises
$0.01 per fetch; the live challenge and the wallet's existing `max_api_payment_usdc` are
authoritative. Its existing timeout, transport and `max_response_bytes` settings
bound the relay call; the target's existing `max_spec_bytes` bounds decoded catalogs.
No additional spending caps or download settings are introduced.

Without `wallet`, each source uses one of its effective assigned wallet profiles.
An explicit source wallet takes precedence through normal binding resolution. A
source shared across listeners chooses one candidate by the highest SHA-256 score
of the JSON tuple `["discovery-wallet-v1", source_id, wallet_name]` (wallet name
breaks ties). This is stable across restarts and listener ordering; duplicate
bindings to the same wallet do not add weight. Renaming a source or changing its
candidate wallets can change the selection. No keys, balances, timing or current
rotating addresses influence the choice. A failed wallet is never replaced by
another candidate to retry relay spending. An unbound source needs either its own
`wallet` or an explicit `discovery_relay.wallet` override.

`config show` reports the resulting `discovery_wallets` source-to-profile map.
The privacy example enables relay fallback without an override: `social` uses
`social`, `webinfo` uses `web`, and shared `exa` selects `company` from its `web` /
`company` candidates. Exa's discovered catalog/pricing remains shared by its
listeners; their paid API calls still use their original wallet bindings. Distinct
source aliases using different wallets keep separate relay requests and caches,
even if they fetch the same URL. Sharing a wallet intentionally retains that link;
this removes a mandatory common discovery payer, not all correlation channels.

Both modes use the resolved wallets through the ordinary paid client and the configured
network factory. Under Tor, the connection to Curl remains over Tor; Curl sees the
target URLs and its own egress reaches the targets. The target GET is unsigned.
The relay supplies origin content, headers and status: our TLS connection verifies
Curl, not the target's TLS policy. Never use it for authenticated discovery URLs.
The provider can also be exposed as ordinary MCP tools by adding it as a source;
the relay setting alone does not expose a tool to listeners.

With `funding.auto_fund=true`, ordinary managed serving completes initial wallet
bootstrap before catalogs and pricing, so paid fallback can use the selected wallets.
Static wallets require external funding. For paid warming, first use
`wallet bootstrap --config FILE`; warming opens only the wallets resolved for its selected
sources (including treasury ownership for managed wallets), denies new funding
and starts no workers. Export
static signing-key environment variables before warming. `--direct` always bypasses
the relay and remains unsigned. Ordinary `config check`, catalog inspection and
`catalog tools --discover-pricing` remain unsigned and never invoke the relay.
Qualification rejects paid relay startup.

Paid relay calls are serialized across wallets. Successful results are shared within
the same wallet/relay scope for each target URL. A valid, correctly attributed Curl
response reporting an origin HTTP error, connection failure, unusable catalog or
pricing response ends fallback for that target. Its failure is retained so aliases
cannot retry it, while other targets can still use Curl. No upstream error text is logged.
Payment failures, transport failures reaching Curl, or invalid/unattributable relay
envelopes disable further relay calls across all wallets for that run.
Cancellation also disables further calls, including after
a signed submission. Queued targets cannot spin on the wallet; there is no automatic
retry, alternate relay, or replay of an uncertain payment. A new explicit command or
process restart starts a new run and can spend again. Catalog failure still aborts
startup; pricing failures remain unknown estimates. Existing successful results and
fresh cache entries remain usable.

Relay responses must match the target URL and GET method and contain complete JSON
catalog text, or a usable 402 payment header for pricing. Redirects and explicitly
truncated results are rejected. Curl's documentation does not specify its body limit
or guarantee a truncation flag; paid delivery, completeness on large documents and
Tor reachability remain unqualified. Valid JSON alone cannot prove the relay has
preserved every origin field.

Only explicitly fresh origin metadata permits disk storage, under the existing
treasury cache directory. Resolved wallet, relay identity and target policy partition these entries;
relay-disabled inspection cannot reuse them. The relay's outer caching headers are
ignored. Curl does not expose conditional request headers, so expired relay entries
are fetched anew through the normal origin-first fallback path, never revalidated
with relay validators or served stale. Pricing persists only display estimates,
never payment challenges. `catalog warm` reports `fresh_via_relay` for cached catalogs;
a successful fetch without suitable origin headers reports `not_stored`.

Pricing suffixes use compact provenance labels:

- `Cost: ~$0.015/call [spec].` or `Cost: ~$0.01–$0.20/call [spec].`
- `Cost: ~$0.015/call [x402 probe].`
- `Max: $0.05/call [x402 probe].` for an `upto` offer.
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
Original credit prose remains alongside the estimate. Missing, malformed or unknown
metadata retains that prose and the configured rate, explicitly marks the per-call
estimate unavailable, and logs a warning.

SocialFetch enables this using the historical 0.014 USDC/credit estimate; it is not
a verified current x402 tariff. Every converted tool names the configured rate and
states that the payment challenge is authoritative. Source instruction overrides
and tag filtering (including `privacy.toml`) preserve these per-tool estimates.
An explicit tool description override still replaces the generated description.

MCP server instructions preserve authored guidance and include this caveat once:

> Prices are estimates or sampled payment offers, not guaranteed quotes. Costs may vary with arguments; metered charges may be below the displayed maximum. Unknown does not mean free.

Prices from v1/v2 challenge headers retain atomic units for unknown assets, while
recognized USDC uses six decimal places. Authored
description overrides apply last, and description limits remain explicit.
Cached prices are informational: each paid invocation still obtains a fresh
challenge and enforces its spend cap before signing.
