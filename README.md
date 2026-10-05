# x402_treazury

**treazury** exposes x402-paid HTTP APIs as ordinary MCP tools. It loads OpenAPI
specs and provider guidance, generates tools, and handles payment inside the server.
Configure one or more MCP listeners with reusable TOML API sources, tool filters,
and static or managed wallets.

A managed wallet uses a Zcash treasury to fund rotating Base USDC addresses through
NEAR Intents, with a funded standby to reduce downtime. Optional Tor routing isolates
connections by payment identity. The implementation is Rust; Python is only used by
optional development tools.

See the [documentation index](docs/README.md) for architecture, wallet lifecycle,
network isolation, agent source management and testing guides.

Run commands from the repository root. Tested with Rust/Cargo 1.99.0 on macOS.
The executable is `target/debug/treazury` (or `target/release/treazury` with
`--release`). The library/package is `x402_treazury`. The normal build needs no
reference checkouts or Python installation. Zcash treasury support is enabled by
default and needs protoc: use `scripts/zcash.sh build` to supply Cargo-managed
protoc, or install protoc for plain `cargo build --locked`. Build without the
embedded wallet using `cargo build --locked --no-default-features`. Proving test
helpers and Docker consensus tests remain opt-in features.

Dependency lockfiles fix resolved Rust versions; `rust-toolchain.toml` and the
build wrappers also pin Rust/Cargo. For native toolchain constraints and two-build
release verification, see [reproducible builds](docs/reproducible-builds.md).

```sh
scripts/zcash.sh build

# Offline inventory: no wallet or pricing probes. Output is JSON.
target/debug/treazury \
  --config providers/pdl.toml --spec tests/fixtures/pdl_openapi.json --list-tools

# Serve over stdio; read EVM_PRIVATE_KEY from the specified file.
target/debug/treazury \
  --config providers/pdl.toml --env-file .env

# Streamable HTTP, stateless JSON responses at /mcp.
# The env file must also contain X402_MCP_BEARER_TOKEN.
target/debug/treazury \
  --config providers/pdl.toml --env-file .env --transport http --port 8000
```

MCP clients should spawn the binary directly, pass an absolute `--config` path
(or set their working directory), and provide keys through their environment or
`--env-file`. Relative paths in TOML resolve from the file declaring them;
`--spec` and environment overrides resolve local paths from the working directory. Logs go to stderr. Help fetches and API calls occur
only on tool invocation; loading a remote `spec` still requires a startup fetch.

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
limit; see [startup qualification](tests/STARTUP.md). Pricing concurrency is separate.

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


[examples/servers.toml](examples/servers.toml) exposes PDL and LoneStar through
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
[servers-auto-wallets.toml](examples/servers-auto-wallets.toml):

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

Rust configuration is TOML-only. [providers/](providers/README.md) contains
all 26 reusable provider definitions. AgentUtility, Glassnode, Concordance,
Straits and Locus have a `provider.toml` and a locally curated `openapi.json` in their own directory.
JSON remains the format for API documents, fixture data and inventory output.
Small PNG/JPEG/WebP results are returned as MCP image attachments; provider TOML
can also extract explicitly mapped base64/data-URI images from JSON while keeping
metadata. See [inline image configuration](providers/README.md#inline-image-results)
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
automatic NEAR funding is not implemented.

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
[static-wallet example](examples/agent-sources.toml) and
[managed-wallet example](examples/agent-sources-managed.toml) use one named
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
[agent source guide](docs/agent-sources.md#concurrent-agents).

The stable `treazury_tools_search` and `treazury_tool_call` tools let clients use new
APIs even when they cache their initial tool list. HTTP remains stateless and does
not advertise list-change notifications. Imports require public HTTPS, use the same
direct/Tor network factory, and never run pricing sweeps. Persistent registrations
use a separate owner-only SQLite registry; they do not require a wallet encryption key.

`providers/x402-list.toml` supplies five curated read-only directory tools. Directory
reads may become paid after a shared-IP quota; ordinary spend caps apply. Its exact
`include_operations` allowlist prevents new upstream write routes from appearing.
Directory results are leads, not automatically imported schemas.

See [agent source management](docs/agent-sources.md) for the complete configuration,
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
apply in both network modes. See the [provider caveats](providers/CAVEATS.md) and
[qualification scope](docs/testing.md#qualification-status).

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

[examples/socialfetch.toml](examples/socialfetch.toml) configures a selected
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
outside runtime routing. See [runtime egress and verification](docs/network-egress.md)
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
[cover traffic](docs/cover-traffic.md) for configuration, five bounded distributions,
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

## Verification

Run `scripts/check.sh` for both build configurations, all-feature Clippy and
compatibility checks. Use `scripts/check.sh --no-default-features` to check only
the build without the embedded wallet. See [development tools](scripts/README.md).
Individual commands:

```sh
scripts/zcash.sh test --all-targets -- --test-threads=1
scripts/zcash.sh clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --locked --no-default-features --all-targets
```

Tests cover local x402 handshakes and signatures, spend limits, managed admission
and rotation, HTTP authentication, actual stdio process framing, tool generation,
configuration, pricing/help caches, Tor isolation, encrypted persistence and
Zcash sync/recovery. They use localhost and public deterministic test keys;
normal suites do not spend funds. A sandbox must permit binding localhost.

Golden fixtures pin **860 tool contracts across 22 providers** and settings for
all 26 bundled providers. Names, full descriptions, schemas, methods, paths and
parameter routes are checked without Python or live vendor access. Independent
Python-generated Keccak/EIP-712 vectors are committed as test data. See
[fixture maintenance](tests/fixtures/catalogs/README.md),
[network verification](docs/network-egress.md), and
[consensus tests](tests/REGTEST.md).

### Local complexity metrics

Run `python3 scripts/complexity.py` for an optional local audit using pinned
`rust-code-analysis`. Open `target/complexity/report.md`; full JSON/CSV retain all
scores. Production callables, tests and examples are separated, parser failures
are explicit, and existing coverage is joined only for unchanged source files.
No complexity thresholds are enforced. See [definitions and findings](tests/COMPLEXITY.md)
for syntax qualification, macro/feature limitations and the independent tool build.

### Local coverage

Install `cargo-llvm-cov` (`cargo install cargo-llvm-cov --locked`) and LLVM tools
matching `rustc -vV` (or `rustup component add llvm-tools-preview` for rustup).
Run `scripts/coverage.sh` for the default Zcash build, all test targets and examples.
It supplies protoc and detects Homebrew LLVM; `LLVM_COV` and `LLVM_PROFDATA` can
select another matching installation. Rust 1.99.0 from Homebrew uses LLVM 23.1.2.

Open `target/coverage/html/index.html`; text and machine-readable summaries are
`target/coverage/summary.txt` and `target/coverage/summary.json`. The denominator
covers application source, excluding integration tests, examples and vendored
code; inline unit-test helpers can still count. Stable Rust reports line, region
and function coverage, not branch coverage. Optional proving/Docker and live
funded tests are outside this run. Reports contain local paths and are gitignored.

Region coverage is already included in these reports. Branch coverage needs a
fresh instrumented run with nightly Rust. With rustup installed and its binaries
on PATH:

```sh
rustup toolchain install nightly --profile minimal --component llvm-tools-preview
RUSTUP_TOOLCHAIN=nightly scripts/coverage.sh --branch
```

This writes HTML/JSON/text to `target/coverage-branch` and keeps its build/profiles
separate from the stable run. It uses nightly's matching LLVM tools; unset any
`LLVM_COV`/`LLVM_PROFDATA` overrides pointing at a different LLVM version. Production
builds continue to use the existing stable compiler. Branch results measure the
compiler's supported branch instrumentation, not every possible logical path.

## Treasury and managed pools

Build or test the embedded wallet with the shell wrapper, which supplies a
Cargo-managed protoc compiler. First builds may download public proving
parameters. The wallet dependency is pinned upstream at zingolib v6.0.0 commit
`c6381534f802b1022041beda4b01c106ad132329`; no reference checkout is needed.

```sh
scripts/zcash.sh build
scripts/zcash.sh test --all-targets
```

The binary supports these treasury commands:

```sh
# STATE_DIR must not exist; KEY_FILE must be new, with an existing parent directory.
# Generate a new seed; query the mainnet tip automatically.
treazury wallet init --state-dir /private/state/new-treasury \
  --key-file /private/keys/new-treasury.key

# Supply the correct birthday for your mnemonic when importing.
treazury wallet init --state-dir /private/state/treasury \
  --key-file /private/keys/treasury.key --birthday 2000000 \
  --mnemonic-file /private/import/mnemonic.txt

# Without --mnemonic-file, init generates a new seed inside zingolib.
# Status reads persisted metadata and needs neither key nor network access.
treazury wallet status --state-dir /private/state/treasury

# Use the treasury_id returned by init. This allocates two keys and queued jobs;
# it does not contact NEAR, transfer ZEC, or fund either EVM address.
treazury wallet pool --state-dir /private/state/treasury \
  --key-file /private/keys/treasury.key --treasury-id UUID \
  --name research --deposit-size 5.00

# Display existing receive addresses without derivation or network access.
# The treasury UUID is read from state; --treasury-id UUID is an optional check.
treazury wallet addresses --state-dir /private/state/treasury \
  --key-file /private/keys/treasury.key

# Derive another shielded receive address and save its snapshot before returning it.
treazury wallet address --state-dir /private/state/treasury \
  --key-file /private/keys/treasury.key --treasury-id UUID
```

Use `target/debug/treazury` or put the built binary on
PATH. Mnemonic files must be owner-only and are never passed as seed arguments.
`wallet addresses` reads the encrypted snapshot without changing address indices,
snapshot revision, or sync readiness. `wallet address` creates a new address and
requires the treasury UUID shown by init/status, not an account number such as `0`.
Expected CLI failures print a concise error and cause chain to stderr and return
a nonzero exit code, even when `RUST_BACKTRACE` is enabled.
Initialization refuses existing state/key paths; a partial initialization failure
is not automatically retried over those files. Wallet data stays in memory until
written as an authenticated encrypted snapshot. The wallet's internal path uses a
private temporary directory, separate from the durable database. Upstream plaintext
save tasks are never started. These commands never broadcast or fund anything.

New-wallet initialization queries `ZCASH_INDEXER_URL` when set, otherwise
`https://zec.rocks:443`, and records the mainnet tip minus 100 blocks as its
birthday. `--indexer-url-env NAME` selects another environment variable. Lookup
must succeed before any seed, key or state is created; it checks the network and
tip consistency and has a 30-second deadline. `--birthday HEIGHT` skips lookup
and keeps initialization offline. Importing with `--mnemonic-file` requires an
explicit birthday at or before the wallet's first use, so older funds are scanned.

Address JSON separates `receiver_capabilities` (`orchard_protocol`, `sapling`,
`transparent`) from balances. `orchard_protocol_pools` lists `orchard` and
`ironwood` when that receiver is present: both pools use the same receiver and
keys. It does not report where funds reside. After sync,
`state.sync.confirmed_pool_balances_zatoshis` reports `ironwood`, `orchard`, and
`sapling` individually, alongside the aggregate shielded balance. A null pool
report means no pool breakdown has been recorded (including older snapshots);
a null individual balance means the SDK did not provide it. These are cached
observations whose freshness is reported separately.

After NU6.3 activation, birthday discovery and sync require the indexer's tip
TreeState to contain a nonempty Ironwood frontier at the requested height. Sync
checks before scanning and again at the final tip before accepting readiness.
Missing data, RPC failures, and timeouts fail closed. This checks server
capability; it does not independently prove that a server supplies honest or
complete chain data. No check is required before activation.

Both reference Zodl apps default to `zec.rocks:443`. They also list regional
`na`, `sa`, `eu`, and `ap.zec.rocks` endpoints, and `us.zec.stardust.rest` and
`eu.zec.stardust.rest`. These public lightwalletd services need no partner key.
Serving and sync still use the explicitly configured indexer and submission
environment variables; initialization does not change them. For example:

```sh
export ZCASH_INDEXER_URL=https://zec.rocks:443
export ZCASH_SUBMISSION_URL=https://zec.rocks:443
```

The same service can handle birthday lookup, sync and transaction submission.
Using it for both roles lets that provider observe both sync queries and
submission traffic. The roles can point to separate providers or your own
servers. Selecting a Zodl endpoint does not implement its failover or privacy
transport. Full treasury sync and funded swaps against these public endpoints
remain part of live qualification.

To sync an existing treasury, export the environment variable named by
`treasury.indexer_url_env` and run:

```sh
treazury wallet sync --meta-config examples/servers-managed.toml
```

This command reads `[treasury]`, resolves state/key paths relative to the TOML,
and requires exclusive ownership: stop the managed server first. It does not load
API specs, require NEAR/submission credentials, or load `.env` automatically.
The indexer must serve mainnet over TLS; HTTP is permitted only on loopback for
local testing. Account 0's shielded funds are used, with `confirmations` (default
3) applied to zingolib's spendability calculation. Unconfirmed change is excluded.

Managed serving starts one shared background sync worker after all listeners bind.
It retries or refreshes after half `max_sync_age_seconds`, bounded to 1–60 seconds;
connection and tip requests have 15-second deadlines. Failed sync disables new
treasury spending while independently funded EVM pools keep serving. Initial
scans can take a long time. Sync checkpoints every 30 seconds and on completion,
failure or cancellation; each checkpoint atomically commits the encrypted wallet
and its observation under one snapshot revision. Checkpoints resume from saved
scan state. Old unreferenced snapshots are pruned, retaining the current snapshot
and every snapshot referenced by an outgoing operation.

`wallet status` reports the last persisted sync phase, checkpoint time, scan target
and scanned-block count, last successful tip height/time, confirmed and spendable
shielded balances in zatoshis, and any last sync error. `sync_fresh` also checks the
snapshot revision and `max_sync_age_seconds` (default 300). Cached balances remain
observations during sync/failure; they are not authorization to spend. New sends
must also pass the unresolved-outgoing gate and have sufficient spendable input
including fees. Reopening and clean shutdown mark readiness offline; a fresh sync
is required after restart. SIGINT/SIGTERM cancel the CLI sync and save its state.

Stop serving before running an offline backup:

```sh
target/debug/treazury wallet backup \
  --state-dir /path/to/state --key-file /path/to/key \
  --treasury-id <UUID> --destination /path/to/new-backup
```

The command requires exclusive ownership and never overwrites a destination.
It copies a consistent SQLite snapshot (including pending signed transactions,
EVM keys and all journals) and its encryption key into an owner-only directory.
`backup.json` is published atomically after the database, key and manifest are
fully written and synced. A failed destination cannot be reused. If a directory
sync fails after publication, the command returns an error even though a complete
backup may be present; durability is not assured until sync succeeds. Protect the
entire backup as spending material. To restore, stop the original process and use the backup directory as
`state_dir`, its `key` as `key_file`, and the same treasury ID. Do not run the
original and restored copies simultaneously. A Zcash mnemonic alone cannot restore
random EVM keys, pending payment authorizations or the rotation journal.

To retry a job that never produced signed bytes, stop serving and run:

```sh
target/debug/treazury wallet recover-unprepared \
  --state-dir /path/to/state --key-file /path/to/key \
  --treasury-id <UUID> --job-id <UUID-from-status>
```

This archives the encrypted quote/refund bindings, releases only an unconsumed
reservation, and assigns a new operation ID and refund derivation for the retry.
It refuses any outgoing transaction record, including resolved transactions.
Use `wallet reconcile` for possibly submitted deposits; neither a failed swap nor
an absent/expired transaction is enough to release source liability.

Managed serving reconciles each pool's Base balances and authorizations every five
seconds, sharing the admission gate with paid calls. This can resolve confirmed
payments and finish bootstrap; it never signs a payment or promotes the active
wallet. Changed canonical anchors remain blocked for explicit recovery.

Each sync cycle owns a separate Tokio runtime because pinned upstream sync can
leave helper tasks alive on early exit. Teardown stops those tasks and joins
blocking work before the final snapshot; shutdown must await that cleanup.
The treasury exposes a bounded, serialized command queue in `treasury/actor.rs`
for sync, preparation, saved-byte submission and reconciliation. Accepted commands
finish even when a reply receiver is dropped. `rotation/store/funding.rs` journals
immutable operation IDs, encrypted quotes, guarded phases and persistent fair
scheduling; status includes funding progress. Preparing a journaled funding job
commits its PREPARED phase together with the wallet snapshot and signed bytes.
Set `[funding].auto_fund = true` to dispatch funding commands while serving;
the default is false and performs treasury sync only. This opt-in can spend ZEC
within the configured limits. `near_api_key_env` names the partner credential;
`near_user_session_env` optionally names a separate user-session bearer token.
Tokens stay in environment variables and are sent only to the fixed NEAR origin.
Session issuance/refresh and authenticated access still require qualification.

The coordinator persists a unique transparent refund address and wallet derivation
range before obtaining a quote. Quote retries and expired unprepared-quote refreshes
share `max_attempts`; every refresh archives its old bindings and allocates a fresh
operation/refund identity. Signed operations never refresh. Insufficient shielded
funds or budget leave the job unprepared and retryable.

`swap_timeout_seconds` starts at the first durable broadcast intent. On timeout,
status shows `timed_out` and pool `funding_degraded`; reconciliation continues at
least 60 seconds apart. This never releases funds or stops a funded active wallet.
Status failures have their own persisted exponential backoff (up to 300 seconds,
plus per-job jitter), independent of quote retry counts. Successful observations
reset the error streak. Status errors contain fixed actionable categories, never
remote bodies, credentials or deposit addresses. It resumes phases without repeating ambiguous
submissions, reconciles source confirmation, polls the same deposit, and independently
checks Base credit. Partial deposits enter `recovery_required`; failed/refunded swaps
stay `refund_pending`. Confirmed refund outputs reduce the originating operation's
consumed exposure once, capped at its principal; source and shielding fees remain
consumed. Reorgs of credited refunds fail treasury readiness closed. API refund
status alone never credits funds.

With serving stopped, prepare shielding for exactly one refund address:

```sh
target/debug/treazury wallet shield-refunds \
  --meta-config servers.toml --job-id JOB_UUID
```

This calculates and journals one transaction without broadcasting. Submit its
operation ID with `wallet reconcile --meta-config servers.toml --operation-id UUID
--rebroadcast`. The command enforces `shield_max_fee_zec` and the daily budget;
only the fee counts as new expense. Each proposal selects one address, never
combining unrelated swaps. Both calculation and submission use the existing
restart-safe outgoing journal. Refund principal becomes shielded spendable only
after the shielding transaction confirms. For an expired deposit or shielding operation, use
`wallet recover-expired --meta-config servers.toml --operation-id UUID` with serving
stopped. It requires a fresh tip beyond expiry by the configured confirmation
count, no positive indexer inclusion, synced invalidation, and confirmed unspent inputs
matched against the immutable preparation snapshot. It then releases the reservation
and resets any unfinished funding job with a new operation ID. Signed bytes and
recovery evidence remain archived; the old operation cannot be broadcast again.
Missing or conflicting evidence keeps the reservation. A later contradictory reorg
fails treasury readiness closed.

`treasury/send.rs` implements explicit deposit preparation through zingolib's
calculate-only API. It accepts one mainnet transparent recipient, uses account 0's
shielded inputs, rejects multi-step transactions, and verifies the resulting
recipient, amount, actual fee and bounded expiry. The caller supplies the pool,
operation UUID, source/fee limits, aggregate daily limit and quote deadline.
Preparation and the first submission require at least 300 seconds of quote validity.
The funding worker supplies these from validated configuration/quotes,
never from MCP tool arguments.

Preparation reserves the maximum source cost, marks readiness `preparing`, then
atomically commits the encrypted post-calculation wallet and signed bytes with
transaction ID, expiry, amount and fee. The reservation shrinks to actual cost.
An error/cancellation after reservation poisons the in-memory owner; close and
reopen it before further preparation. Unprepared reservations remain conservative
until explicitly abandoned through the store's guarded recovery API. The opt-in funding coordinator calls this API through the serialized owner.

`rotation/near.rs` supports public and confidential foreign-chain EXACT_OUTPUT swaps.
The demo configurations explicitly select `funding.confidentiality = "public"`:
no NEAR key, account signup or application commission is needed. This keeps the
Zcash treasury shielded but uses public NEAR settlement. `basic` and `advanced`
remain explicit confidential options requiring separately qualified authentication.
Omitting the setting retains `basic` so existing configurations never silently
lose confidentiality. No mode falls back to another after a quote error.

Run the read-only $5 route demo (public test addresses, no credentials, no wallet,
no deposit allocation or transfer):

```sh
scripts/zcash.sh run --offline --example quote_near
```

Cargo's `--offline` disables dependency fetching; the example itself calls NEAR.
It validates the live public quote and prints the required ZEC input. Funded
execution requires an initialized treasury, chain endpoint settings and deliberate
`auto_fund = true`. Public funded execution has bounded
[qualification scope](docs/testing.md#qualification-status);
authenticated confidential execution remains unqualified.

Quote validation checks assets, request bindings, mainnet transparent deposits,
exact output and integer cost caps. Timestamp normalization is accepted only for
the same instant. An extended provider deposit deadline never extends the local
send deadline. The echoed platform fee is allowed only for the captured 1Click
collector, with its rate and USD overhead bounded by `max_fee_bps`; requests never
add application fees. Unknown collectors are rejected. Public quote fixtures and
confidential-auth rejection fixtures live in `tests/fixtures/near/`. Redirects are
disabled; optional credentials remain confined to the NEAR client.

`rotation/transaction.rs` makes submission consume a `BroadcastTransaction` handle
created only after `BROADCAST_REQUESTED` is durable. Each retry requires another
journaled intent. `treasury/submission.rs` uses the configured submission endpoint
for raw sends and the indexer endpoint for lookups, with mainnet/TLS validation,
15-second bounds, no fallback endpoints and no implicit retries. It verifies returned
transaction identity. Timeouts, cancellation, rejection and conflicting responses
retain unresolved exposure; successful acceptance alone does not release inputs.
Status includes `treasury_operations` with public transaction facts and attempt state.

Recovery requires exclusive ownership. Without `--rebroadcast`, this command only
syncs and looks up the saved transaction and needs no submission credential:

```sh
treazury wallet reconcile --meta-config servers.toml --operation-id UUID
# Explicitly allow resubmission of the SAME saved bytes, within deadline/expiry:
treazury wallet reconcile --meta-config servers.toml --operation-id UUID --rebroadcast
```

Confirmation releases the send gate only when fresh wallet sync and exact-byte
lookup agree at the configured depth; lookup also checks transaction inclusion in
a stable canonical compact block. An absent or expired transaction is quarantined,
not treated as unspent. Recovery never generates a replacement transaction.
SIGINT/SIGTERM preserve ambiguous submission state. Managed startup still only
syncs and queues funding jobs; it never sends ZEC automatically.

The adapters are experimental. Tests cover empty-wallet admission, synthetic gRPC
transport, journal recovery and sync. The optional proving suite exercises the
production preparation path with synthetic Orchard notes belonging to public test
keys, verifies the resulting proof, and restores identical pending bytes and wallet
state after reopening:

```sh
scripts/zcash.sh test --features zcash-testutils --all-targets
```

`zcash-testutils` opts into upstream test helpers and an Orchard verifier; normal
serving needs only `zcash`. The proof test uses a localhost info stub and never
broadcasts. Lookup tests reject a changing confirmation block, missing inclusion,
wrong heights, malformed hashes and failed rechecks. These synthetic tests do not
establish consensus acceptance or recovery after a mined transaction is reorged.
The separate [regtest suite](tests/REGTEST.md) uses pinned Zebra/Zaino containers
and mined shielded funds to exercise submission, confirmation, restart and competing
branches. It verifies reservation retention before confirmation depth, explicit
rebroadcast of identical bytes, and quarantine after a previously accounted spend
is orphaned. Sync rechecks accounted source spends; loss of confirmation depth
sets `treasury_confirmed_spend_reorg`, retains consumed budget and blocks new
preparation until the original transaction regains sufficient depth. There is no
automatic replacement transaction or repair command for this condition.

The regtest network variant exists only in the unit-test binary under
`zcash-regtest`; production constructors remain mainnet-only, including all-feature
builds. Refund shielding and authenticated route qualification remain required before
unattended funding.

`rotation/store.rs` provides one exclusively owned, owner-only SQLite database
with WAL and full synchronization. A separate random 32-byte key encrypts wallet
snapshots, EVM keys and prepared transaction bytes using ChaCha20-Poly1305 and
fresh nonces. Authenticated record context includes the treasury and pool where
applicable. Keep both state and encryption key: a Zcash seed alone cannot restore
EVM keys or pending funding work. `wallet backup` preserves both in one consistent backup directory; the database
and key are not interchangeable backups.

Named pools persist two distinct bootstrap candidates and their USDC funding
floors. New profiles default to a $2 floor. An unsigned NEAR quote can raise a
candidate's target to the bridge's current minimum; the validated quote and new
wallet/job target commit atomically before any preparation or transfer. Successful
quotes freeze that target. Source-input, fee and treasury budget caps still apply;
an unaffordable minimum leaves funding unavailable rather than raising those caps.
Minimum hints are accepted only from the specific bridge-minimum error with a
positive atomic Base USDC amount. Each quote negotiation allows at most three
requests; other failures do not trigger amount changes or payment retries. Repeating `wallet pool` resumes the existing pool; changing its
`deposit_size` affects future allocations only. Transactional promotion retires
one address, promotes the standby, and creates one new key/funding job. Pools
have independent generation checks and roles. Shared ZEC budget reservations
carry unresolved exposure across days; confirmed costs are charged conservatively
on their confirmation day. A single pending prepared outgoing operation gates
new preparation, and exact bytes plus its wallet snapshot commit together.

A sync failure before the transaction preparer is invoked leaves the same quote
eligible for a later attempt. Interrupted calculation or durable transaction/budget
effects still require recovery; absence of signed bytes alone never permits retry.
Recovery status retains its preceding failure instead of clearing it.

Managed profiles connect these transitions to the x402 request path using a
trusted Base RPC. [servers-managed.toml](examples/servers-managed.toml) contains
a complete deployment example. Inspect it without credentials or state access:

```sh
target/debug/treazury \
  --meta-config examples/servers-managed.toml --show-config
```

Serve with the Zcash-enabled binary using the same `--meta-config` and an
`--env-file` containing the referenced endpoints and listener tokens. Paths are
relative to the deployment file. `[treasury]` identifies an already initialized
wallet; serving never creates/imports a Zcash seed. All declared managed profiles
allocate or resume their own pool, including profiles without a listener.
Source/server bindings sharing a profile share its gate and reservations. Static profiles can
coexist; their `private_key_env` is forbidden in managed profiles. Removing a
profile while serving managed pools disables it without deleting its history.
A name with durable managed state cannot become static; retain the treasury
reference so this identity conflict can be detected. Re-adding a managed name
resumes its existing UUID, keys and allocation targets.

Base verification defaults to **PublicNode → dRPC → Base**. The primary can be
overridden through `funding.base_rpc_url_env` (default `BASE_RPC_URL`); when that
default variable is absent, PublicNode is used. An empty value or a missing
explicitly named custom variable is an error. RPC access is separate from
API-provider access: a successful NEAR swap can still wait for independent Base
verification if the RPC rejects an isolated Tor exit. Logs identify the verification
step, HTTP status or JSON-RPC error code, and elapsed time without printing wallet
addresses, endpoint credentials or upstream response bodies. Funding status retains
RPC rejection codes. NEAR quote failures identify the asset-catalog or quote stage
and a sanitized HTTP, transport or validation category. Confirmed destination
credit clears obsolete quote/credit errors; source-reconciliation failures remain
visible until resolved. Late credit checks leave an already-completed job's newer
balances and roles unchanged. Credit-persistence messages distinguish insufficient
confirmed credit and local state failures from RPC access failures.
An empty treasury pauses refill before preparation and
reports how to fund/sync it, while existing funded wallets remain usable. Background reconciliation backs off after failures from 30
seconds up to five minutes; successful polls return to the normal five-second
interval. Failed checks never release reservations or authorize payments.

Unsigned `eth_chainId`, `eth_getBlockByNumber` and `eth_call` requests may retry
once after a transport interruption, with a 200 ms delay inside the original
request timeout. Both attempts reuse the exact payload, endpoint and
identity-bound HTTP client, including canonical block pins. HTTP/RPC rejections,
malformed JSON/envelopes and identified TLS errors are not retried. The read-only
retry never resends a paid API request or changes provider/Tor identity. Warnings
record each failed attempt and retry decision, request phase, safe HTTP/IO/TLS
categories and numeric status codes; raw error chains and addresses stay private.
The diagnostic example emits the same warnings to stderr.

When `funding.base_rpc_fallback_url_envs` is omitted, dRPC and Base are fallbacks
(a default equal to the primary is skipped). Set it to `[]` to disable fallbacks,
or name up to two environment variables to replace the default fallback list.
Missing custom variables fail startup rather than reverting to public services.
`--show-config` includes a secret-free `base_rpc_policy` describing these defaults
and environment references, and startup logs whether default fallbacks are active.
All selected services may receive wallet-address queries; shared provider accounts
or API keys can additionally link wallets. Custom-list example:

```toml
[funding]
base_rpc_url_env = "BASE_RPC_URL"
base_rpc_fallback_url_envs = ["BASE_RPC_SECONDARY", "BASE_RPC_TERTIARY"]
```

Failover restarts the **entire read-only chain view** on one endpoint, including
chain ID, persisted anchor, confirmed/latest balances, authorization nonces and
final canonical-block checks. Partial evidence is discarded. HTTP 403/408/429,
500/502/503/504, availability transport failures and RPC -32005/-32016/-32601
permit fallback. Malformed data, wrong chain, conflicting/stale chain evidence,
TLS validation failures and other rejections fail closed. Paid requests and Zcash
submissions are never replayed by this mechanism. Payer SOCKS identities and the
configured Tor-only network policy remain unchanged.

Each provider gets a complete-view deadline of 15 seconds in direct mode or the
Tor request floor (240 seconds by default), including its bounded transport retry.
At most three providers are visited once per view; earlier caller/admission
deadlines still apply. Logs identify provider indices, failure categories,
deadlines and fallback success without logging URLs or credentials. Each new view
starts with the primary; there is no global circuit cycling or cross-wallet
provider-health preference. With no URL flags, `diagnose_credit` uses the three
public defaults below. An explicit `--rpc-url` selects only that endpoint unless
up to two `--fallback-rpc-url` flags are supplied; it never silently appends defaults
to an explicit diagnostic list.

Default public endpoints, in order (availability and Tor acceptance vary):

| Operator | HTTPS endpoint | Documentation |
| --- | --- | --- |
| PublicNode | `https://base-rpc.publicnode.com` | [Base gateway](https://base.publicnode.com/) |
| dRPC | `https://base.drpc.org` | [Base API](https://drpc.org/docs/base-api) |
| Base | `https://mainnet.base.org` | [Network details](https://docs.base.org/get-started/connect-to-base) |

This list provides availability failover, not a quorum. A successful view
still trusts one configured provider; availability failover does not protect
against that provider returning fabricated but internally consistent data.

The public `mainnet.base.org` endpoint returned HTTP 403 and 429 during Tor
qualification; Base documents its public RPC as [rate limited and unsuitable for
production](https://blog.base.org/base-mainnet-is-open-for-builders). Use an
explicitly selected endpoint that works with your Tor identities. The application
does not use providers outside the configured failover policy, bypass Tor, or
weaken confirmation checks.
For the Tor qualification deployment, explicitly select PublicNode:

```sh
export BASE_RPC_URL=https://base-rpc.publicnode.com
```

PublicNode passed concurrent canonical balance checks and application reconciliation
through the test's wallet isolation identities, but also returned two intermittent
HTTP 403 block-read responses. This is a qualified test choice,
not a guarantee of production availability or Tor acceptance. The RPC sees the
queried public addresses; a shared provider account/API key can additionally link
wallets. Credit verification remains mandatory: NEAR success alone does not mark
a wallet ready. Balance and authorization reconciliation also protects concurrent
payment reservations. RPC failures defer affected operations, never imply a zero
balance, and never justify replacement funding. These checks trust the configured
RPC; they are not an independent light-client proof.

A read-only diagnostic can exercise concurrent verification without keys or changes
to wallet state:

```sh
scripts/zcash.sh run --example diagnose_credit -- \
  --network-config examples/network-tor.toml --state-dir state/demo \
  --rpc-url "$BASE_RPC_URL" --rounds 3
```

The diagnostic sends the state's public EVM addresses to the selected RPC and
prints balances indexed by pool, without printing addresses, and exits unsuccessfully
if any view fails. Use the same trusted
RPC policy as the deployment; choosing another provider discloses those addresses
to that provider.

Managed `mode = "zcash_rotation"` requires `max_input_zec` and `max_fee_bps`.
Small deposits increase refill frequency; validate swap minimums and total fee overhead
before choosing sub-dollar targets. A payment larger than the configured deposit
target is refused rather than repeatedly rotating wallets. Explicit sizes in
existing configurations are unchanged.

`deposit_size` defaults to `"2.00"`, `max_price_usd` to `"1.00"`, `wait_seconds`
to 30 (range 1–3600), and `max_attempts` to 3. Money fields are decimal strings;
USDC permits six fractional digits and ZEC eight. Singleton treasury/funding
settings and all risk limits are validated by `--check`/`--show-config` without
unlocking state. Endpoint credentials remain environment references. Serving
requires those references; endpoints require HTTPS, with HTTP allowed only for
loopback fixtures. Zcash/NEAR endpoint settings are reserved for the funding
worker and are not contacted by this implementation.

Managed payments accept only v2 `exact` EIP-3009 on Base with canonical USDC,
the `USD Coin`/`2` signing domain, and an amount within both the cap and current
deposit target. Selection preserves the first compatible offer, even if earlier
offers are unsupported. Permit2, `upto`, v1, non-authorization flows, nested
extensions and unknown extra/offer fields are rejected. Top-level extension maps
are removed before signing, with warnings naming the omitted extensions (never
logging their values). Managed payments attempt one ordinary payment without
extension behavior or attribution echoing. Providers requiring extension echoes
may reject it; failed submitted requests report this omission and retain their
reservation until chain reconciliation. Never automatically retry a signed
payment. Malformed extension maps are rejected before admission. A seller or
facilitator may still add its own on-chain attribution. Static payment support
is unchanged. An ordinary non-402 response never enters admission.

The per-pool deadline includes waiting for its gate and chain verification;
Tor applies the network request-timeout floor to this deadline. The gate is
released after the signed authorization is durably journaled, before sending it
to the provider. A slow signed response therefore does not block another call
with sufficient unreserved balance in the same pool, or background reconciliation.
Chain verification still holds the pool gate; unrelated pools remain independent.
Readiness failures return immediately; funding runs asynchronously when enabled.
Both bootstrap addresses must hold their original targets before the first paid
call. RPC verification checks chain 8453, block freshness, and EIP-1898 canonical
block-hash queries for USDC balances and `authorizationState`. The RPC must support
these queries. Admission uses the lower of confirmed and latest balances minus
unresolved authorizations. The default confirmation depth is 12 and maximum
latest-block age is 120 seconds. A changed confirmed anchor blocks the pool with
`chain_recovery_required`; automated reorg recovery is not implemented.

Admission, promotion and the replacement key/job commit transactionally. A busy
wallet returns `payment_pending` instead of rotating. Actual depletion promotes
only a freshly verified standby able to cover that call. A signed authorization's
payer, payee, nonce, amount, validity, requirements hash, attempt and generation
are journaled before its single signed HTTP attempt. Signatures are not persisted.
A final 402, timeout, cancellation or HTTP success does not release its reservation.
A subsequent admission reconciles it against a common confirmed block: used
nonces or expired authorizations verified unused release exposure. This conservative
accounting can temporarily understate spendable funds. Reconciliation runs on demand
and every five seconds in the background.

Pools can spend independently verified Base balances and promote funded standbys.
Queued replacements are funded only with `funding.auto_fund = true`. Refund shielding
and source-expiry recovery require the explicit operator commands described above.

The [lifecycle qualification matrix](tests/LIFECYCLE.md) maps each recovery and
funding invariant to its offline or consensus test. The combined lifecycle test
uses real payment signing and the Base RPC adapter with a deterministic funding
backend; regtest separately verifies Zcash proofs and settlement.

Store tests cover encrypted recovery, ownership, snapshot/generation
conflicts, multi-pool targets, budget contention, failed-write rollback, cancelled
waiters, sync checkpoint atomicity/freshness, durable submission records, and recovery
after a child exits without destructors. The child helper
is marked ignored in normal enumeration and run explicitly by its parent test.
The managed tests cover offer filtering, journal failures, deadlines, independent
pools, restart/cancellation, expiration, stale/reorged evidence and promotion
rollback. Two `zcash` feature tests exercise encrypted zingolib restore and the
offline CLI; two more test managed multi-listener serving and generated-pool
identity across template edits and scope changes. Local gRPC fixtures exercise
real zingolib sync, wrong-network rejection, periodic checkpoints, cancellation,
and encrypted restart/resume without real funds or external indexers.

## Public-edition funded qualification

The [bounded live-demo runbook](docs/public-swap-demo.md) covers two options:
`examples/public-swap-demo.toml` funds one double-buffered pool from a shielded
Zcash treasury; `examples/public-payment-demo.toml` uses a funded static EVM key
to qualify seller payment acceptance. Both expose one tool with a $0.05 payment
cap and use an offline request schema. Automatic treasury funding is disabled in
the supplied profile. Stop before funded execution until the operator supplies
funds and approves the test. See [qualification scope](docs/testing.md#qualification-status)
for exercised boundaries and outstanding acceptance; it grants no spending authority.

## Scope and qualification

Payments support Base USDC: static wallets sign v1/v2 exact and v2 upto;
managed rotating wallets admit v2 exact EIP-3009 only. SVM payments and legacy
custom launcher/group aliases are not supported. Full receipt accounting is
incomplete. Tool generation handles JSON request bodies, not uploads.

The NEAR coordinator is opt-in. Public funded swaps, double buffering and selected
provider calls have live qualification evidence in the linked checkpoint.
Authenticated confidential swaps remain unqualified. Regtest covers deposits,
refund shielding, expiry and restart recovery. Deep finalized-chain rollback
requires operator review. Remaining extensions are described in
[managed wallet follow-ups](docs/plans/deferred/managed_wallet_followups.md).

`upto` signing is tested; each real wallet needs its own on-chain USDC → Permit2
allowance. Treazury does not provision approvals or gas-sponsoring transactions.
Real-funded operation requires an operator's funded wallet and explicit execution.

## Dependency compatibility

The combined build and **13 tests pass** with the exact `zingolib_v6.0.0` tag,
commit `c6381534f802b1022041beda4b01c106ad132329`, on Rust 1.98.1/macOS.
Run the reproducible combined suite with the reviewed vendored connector patch:

```sh
python3 vendor/verify.py
python3 scripts/check_compat.py
```

The checker verifies the vendored source hashes, obtains a Cargo-managed `protoc`
compiler, and runs the combined suite with its committed lockfile. First builds
fetch Cargo dependencies and zingolib's public Sapling proving parameters;
zingolib caches/copies those parameters during its build. Tests use temporary
offline wallets and localhost payment servers. No wallet sync, funded payment, approval or broadcast
occurs.

The [vendored patch](vendor/README.md) changes only the `sha3` dependency in
`alloy-primitives` and `alloy-sol-macro-expander` 1.7.3 from `0.11.0` to
`=0.10.9`. Both workspace roots apply it. No Rust source in those Alloy crates is modified. The separate
[Zingolib connector patch](vendor/ZINGO.md) adds injected-channel support for
the shared direct/Tor transport. The graphs can then retain:

```text
Alloy → sha3 0.10.9 → digest 0.10.7
zingolib → bip32 → hmac prerelease → digest 0.11.0-pre.9
```

`vendor/provenance.json` records upstream archive/file hashes; `vendor/verify.py`
checks that these are the only modifications. Upstream license texts are included.
The independent hash vectors are regenerated with
`uv run --script scripts/generate_crypto_vectors.py`; normal Rust
tests consume the committed vectors without Python.

The combined workspace must also repeat zingolib's **existing upstream**
`lightwallet-protocol` patch, pinned to
`9bdfdc77eb283f2a3d26c27100cc2ac90148cd93`. Cargo does not inherit a dependency
workspace's root patches. Without it, compilation fails on missing Ironwood
protobuf fields. This is separate from the Alloy compatibility patch.

The combined suite runs the 12 shared payment/MCP/crypto tests plus an offline
wallet test: restore a public mnemonic, derive shielded addresses, serialize and
restore wallet bytes, check the next derived address, and create an EVM payer
in the same process. Its lockfile resolves Alloy's upper stack to 2.5.0 and
Zcash primitives to 0.30.1; the application lock retains Alloy 2.1.1.
Both graphs are tested, with Alloy core 1.7.3 and the same two-line patch.

This focused compatibility suite tests dependency coexistence and offline wallet
round trips. The application suites separately cover sync, proving and consensus
recovery. Neither suite qualifies mainnet NEAR settlement. Both manifests use the
reviewed vendored patches; no reference checkout is required. The
[wallet architecture](docs/wallet-rotation.md) describes treasury ownership,
admission, funding, persistence and recovery.

`x402-chain-eip155` also requires its `telemetry` feature with `client` in this
release: the `upto` implementation otherwise references an unavailable
`tracing` dependency. The application enables both.
