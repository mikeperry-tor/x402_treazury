# Configuration reference

For a Zcash-and-Tor walkthrough, start with the [README](../README.md).
This reference covers TOML composition, tool selection, transport and pricing.
Commands run from the repository root.

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
completion order. `--check`, `--list-tools` and `--list-tags` use the same loader;
`--show-config` remains offline and reports the effective limit. Progress, source
fetch/parse and generation timings, and pricing timings go to stderr; a ten-second
waiting message identifies remaining work. No response content or source URL is
included in these progress messages. Remote spec aliases share a download and parsed document within one deployment
load when the exact URL, requested timeout, byte limit, transport compatibility flags and discovery identity match.
Each alias retains its own tools, filters, base URL and wallet binding. Local files
are read independently; documents are not cached across deployment loads.
Pricing discovery rolls across sources and endpoints with a shared 16-request cap.


[examples/servers.toml](../examples/servers.toml) exposes PDL and LoneStar through
separate research and company MCP listeners sharing one static wallet profile:

```sh
target/debug/treazury \
  --meta-config examples/servers.toml --check
target/debug/treazury \
  --meta-config examples/servers.toml --list-tools
target/debug/treazury \
  --meta-config examples/servers.toml --env-file .env
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
  path/tag filters, pricing options, overrides and guidance. A deployment source
  can also set `wallet` to override the server default; this field is forbidden
  in reusable provider files.
- `wallets`: `mode = "static"`, `private_key_env`, and `max_price_usd`
  (decimal string, default `"1.00"`), or a managed `zcash_rotation` profile
  described below. Bindings referencing the same profile share payer state.
  The cap applies per payment; it is not an aggregate budget.
- `servers`: `listen`, `bearer_token_env`, and `sources`, with an optional default
  `wallet` and optional
  `include_tools`, `exclude_tools`, `tags`, `exclude_tags`, and `max_response_chars`.
  Resource identifiers use lowercase letters, digits and `_`, starting with a letter.

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

`--show-config` includes `wallet_bindings.<server>.<source>` with the effective
`wallet` and its `origin`, such as `sources.socialfetch.wallet` or
`servers.research.wallet`. Resolved sources display their optional `wallet`
separately from provider `settings`. Inventory (`--list-tools`) includes each
server's `default_wallet` and the same `wallet_bindings` map; each tool's `source`
identifies its binding. `--check` prints these mappings as well. Mappings cover
all declared server/source bindings, including sources whose tools are filtered
out; only effective static wallets used by selected tools load private keys.
Inspection never reads wallet secrets or opens treasury state.

To avoid repeating managed wallet definitions, define one template and an
automatic assignment policy. A complete deployment is in
[servers-auto-wallets.toml](../examples/servers-auto-wallets.toml):

```toml
[wallet_templates.small]
mode = "zcash_rotation"
deposit_size = "5.00"
max_price_usd = "1.00"
max_input_zec = "0.02"
max_fee_bps = 500

[wallet_assignment]
scope = "source"
template = "small"
```

Sources and servers can then omit `wallet`; `[wallets]` may also be omitted.
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
`deposit_size` changes only future address allocations, preserving existing keys
and their captured targets. Changing scope or renaming a source/server may create
new pools. Managed startup disables pools absent from the resolved set and keeps
their keys and history; restoring the previous scope/name resumes them. Static-only
serving does not unlock or modify treasury state.

For automatic bindings, `--show-config` and inventory additionally report
`template` and `scope`. Configuration inspection includes `resolved_wallets`
(explicit plus generated profile settings), `generated_wallets` (generation
metadata), and `wallet_summary` (distinct managed/generated pool counts and their
combined active-plus-standby target in atomic units and decimal USDC). `--check`
also prints that summary. The total counts each managed pool once, including
explicit unreferenced profiles; it excludes unused templates and static wallets.
It is a configuration target, not a live balance or a quote for additional funds:
existing addresses retain their original targets, and fees and retired balances
are not included. None of these inspection commands allocates keys or funds.

Rust configuration is TOML-only. [providers/](../providers/README.md) contains
all 26 reusable provider definitions. AgentUtility, Glassnode, Concordance,
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

`--show-config` prints the composed file settings and per-source field origins,
including defaults, without reading secrets or loading specs. It accepts only
`--config` or `--meta-config`; CLI/environment runtime overrides are outside
this file inspection command. Wallet/token fields show environment variable
names, never their values.

`--check` validates and reports inventories; `--list-tools` prints per-server
JSON including source attribution. `--list-tags` reports unfiltered source tag
counts. These commands do not create signers, bind ports, probe prices, or fetch
help documents. Remote specs still require network access. Only `--env-file`,
`--check`, `--list-tools`, `--list-tags`, and `--show-config` may accompany
`--meta-config`; deployment settings do not inherit single-source CLI overrides.

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
`--show-config` reports resolved values and origins. For agent-added sources,
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
  `--max-price-usd` overrides `X402_MAX_PRICE_USD`, default `1.00`.
  `none`, `off`, or an empty value removes the amount cap **but retains the
  Base USDC restriction**.
- One signed retry only. A final 402 surfaces its payment error; a paid request
  is not retried again after an ambiguous failure. Redirects are disabled.
- `PaidClient::replace_payer` swaps the signer for subsequent requests. A
  request captures its payer before the unpaid attempt and retains it until
  completion. This API is for static profiles. Managed profiles acquire an
  immutable signer lease after challenge validation and durable admission.

The CLI supports `--spec`, `--config`, `--base-url`, `--prefix`, `--include`,
`--exclude`, `--tags`, `--exclude-tags`, `--timeout`, `--max-response-chars`,
`--transport`, `--host`, `--port`, `--bearer-token`, and `--env-file`.
Filters accept comma-separated values. Supported
`X402_MCP_GENERIC_*` overrides are `SPEC`, `BASE_URL`, `PREFIX`, `NAME`,
`PRICING_KEY`, `INCLUDE`, `EXCLUDE`, `TAGS`, `EXCLUDE_TAGS`, `INSTRUCTIONS_TEXT`,
`HELP_URL`, and `MAX_DESCRIPTION_CHARS`. Precedence is CLI > environment > config
> default; explicit env-file values override inherited environment values.
Use `--help` for the complete CLI. `--route-tool NAME --args '{...}'` prints
method, URL, query and JSON body without making a request or loading a signer.

## Optional agent API discovery

Authorize agents to register public OpenAPI sources in a running multi-server
HTTP deployment with `[source_management]` and per-listener grants. The
[static-wallet example](../examples/agent-sources.toml) and
[managed-wallet example](../examples/agent-sources-managed.toml) use one named
`agent_shared` wallet for all added APIs. At a $5 managed deposit size, that is
one $10 active-plus-standby target regardless of how many APIs are registered.
Registration does not create wallet pools or trigger funding.

| Arrangement | Configuration | Payment identity sharing |
| --- | --- | --- |
| All agent-added APIs share one wallet (recommended) | Global `source_management.wallet = "agent_shared"` | Across APIs and participating listeners |
| Listener-specific wallets | Listener `source_management.wallet` overrides | Across added APIs on that listener |
| Operator-selected listener groups | Several listener overrides reference the same named profile | Within the configured group |

A listener's bearer token defines its authority. Grants separately control source
creation, receiving sources, permitted targets, process visibility and persistence.
Visibility (`server`, `servers`, `process`) is independent of lifetime (`process`,
`persistent`). Existing listener filters still constrain imported API tools. Parallel
callers on the same listener share its authority, quotas and one import slot.
Concurrent additions publish complete catalog snapshots; busy errors can be retried
with the same arguments and idempotency key. See the concurrency rules in the
[agent source guide](agent-sources.md#concurrent-agents).

The stable `treazury_tools_search` and `treazury_tool_call` tools let clients use new
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
`--show-config` inspects grants and wallet bindings offline;
`treazury sources inspect --meta-config FILE` inspects saved registrations.

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
(`--show-config`, `--check`, `--list-tools`, `--list-tags`, `--route-tool`)
remain quiet; `--show-config` exposes the annotations and their origins.

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
The server instructions explain the credit unit and metering caveats.

```sh
target/debug/treazury --config providers/socialfetch.toml --list-tags
target/debug/treazury --config providers/socialfetch.toml --tags Twitter,YouTube --list-tools
target/debug/treazury --meta-config examples/socialfetch.toml --env-file .env
```

[examples/socialfetch.toml](../examples/socialfetch.toml) configures a selected
platform set. TOML sources accept `tags` and `exclude_tags`: each specified list
replaces the corresponding provider list; omission inherits it, and `[]`
clears it. Matching is exact and case-sensitive. Include tags match any listed
tag; exclude tags win. Tag selection combines with source path filters, then
listener tool-name selection. Untagged operations cannot match an include tag.

`--list-tags` prints JSON counts from the unfiltered source (grouped by source
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

For standalone serving or wallet commands, use the same table in a separate file:

```sh
treazury --config providers/socialfetch.toml \
  --network-config examples/network-tor.toml --list-tools
treazury wallet init --state-dir state/public-demo \
  --key-file secrets/public-demo.key \
  --network-config examples/network-tor.toml
```

Paths above assume the repository root as the current directory. Meta-config wallet
commands take network policy from their deployment file; an external network policy
cannot override it. Provider files and MCP tool arguments cannot choose transport.
`--show-config` reports effective settings without reading keys or contacting Tor.
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

Both default to `false`, inherit through `extends`, and appear in `--show-config`.
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
provider setting. Validation, inventory and `--route-tool` never probe.

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
base URL path prefix) and transport compatibility flags. Sources and listeners
with the same policy share results; concurrent matching requests coalesce. Failures, malformed challenges, rate limits and
unpaid successes are cached too. Redirects are not followed. Nothing refreshes
a cached attempt during the process lifetime; TTL expiry makes its result
unavailable to later catalog builds without making another request. Tool
catalogs already built remain unchanged, so `tools/list` never performs network
requests. Restarting the process resets discovery; there is no disk cache.

Prices from v1/v2 challenge headers appear in tool descriptions; unknown assets
retain atomic units, while recognized USDC uses six decimal places. Authored
description overrides apply last, and description limits remain explicit.
Cached prices are informational: each paid invocation still obtains a fresh
challenge and enforces its spend cap before signing.

