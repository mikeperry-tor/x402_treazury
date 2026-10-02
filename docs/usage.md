# Rust generic MCP prototype

Runnable generic OpenAPI/digest → MCP server, using `rmcp` 3.5.0,
`x402-reqwest` 2.0.2 and `x402-chain-eip155` 2.0.2. The Python launchers
remain the supported implementation. This prototype tests whether their core
behavior can move to Rust. Managed EVM pools support durable payment admission
and funded-standby rotation; Zcash/NEAR funding is not implemented.

Run commands from the repository root. Tested with Rust/Cargo 1.98.1 on macOS.
The normal build needs no reference checkouts or Python installation. The optional
`zcash` feature fetches pinned upstream sources and uses Cargo-managed protoc.

```sh
cargo build --locked --manifest-path Cargo.toml

# Offline inventory: no wallet or pricing probes. Output is JSON.
target/debug/x402-mcp-prototype \
  --config providers/pdl.toml --spec tests/fixtures/pdl_openapi.json --list-tools

# Serve over stdio; read EVM_PRIVATE_KEY from the specified file.
target/debug/x402-mcp-prototype \
  --config providers/pdl.toml --env-file .env

# Streamable HTTP, stateless JSON responses at /mcp.
# The env file must also contain X402_MCP_BEARER_TOKEN.
target/debug/x402-mcp-prototype \
  --config providers/pdl.toml --env-file .env --transport http --port 8000
```

MCP clients should spawn the binary directly, pass an absolute `--config` path
(or set their working directory), and provide keys through their environment or
`--env-file`. Relative paths in TOML resolve from the file declaring them;
`--spec` and environment overrides resolve local paths from the working directory. Logs go to stderr. Help fetches and API calls occur
only on tool invocation; loading a remote `spec` still requires a startup fetch.

## Multiple MCP ports from one configuration

[examples/servers.toml](examples/servers.toml) exposes PDL and LoneStar through
separate research and company MCP listeners sharing one static wallet profile:

```sh
target/debug/x402-mcp-prototype \
  --meta-config examples/servers.toml --check
target/debug/x402-mcp-prototype \
  --meta-config examples/servers.toml --list-tools
target/debug/x402-mcp-prototype \
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
  `extends = "../../providers/pdl.toml"`. Every generic catalog setting is
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
extends = "../../providers/socialfetch.toml"
wallet = "social"

[sources.pdl]
extends = "../../providers/pdl.toml"

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

Rust configuration is TOML-only. [../providers/](../providers/README.md) contains
all 18 reusable provider definitions. Glassnode, Concordance, Straits and Locus
have a `provider.toml` and a locally curated `openapi.json` in their own directory.
JSON remains the format for API documents, fixture data and inventory output.
The Python `confs/` files remain inputs to the reference tests; Rust does not
load them as configuration.

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
A bind failure releases listeners already acquired. SIGINT/SIGTERM stops all
listeners and allows in-flight calls up to ten seconds to finish; timeout
reports that paid outcomes may be unknown. A listener failure also stops its
siblings. Managed profiles retain durable payment uncertainty through shutdown;
automatic NEAR funding is not implemented.

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
  Base USDC restriction**. This differs from Python's disabled SpendControls.
- One signed retry only. A final 402 surfaces its payment error; a paid request
  is not retried again after an ambiguous failure. Redirects are disabled.
- `PaidClient::replace_payer` swaps the signer for subsequent requests. A
  request captures its payer before the unpaid attempt and retains it until
  completion. This API is for static profiles. Managed profiles acquire an
  immutable signer lease after challenge validation and durable admission.

The CLI supports `--spec`, `--config`, `--base-url`, `--prefix`, `--include`,
`--exclude`, `--tags`, `--exclude-tags`, `--timeout`, `--max-response-chars`,
`--transport`, `--host`, `--port`, `--bearer-token`, and `--env-file`.
Comma-separated filters use the same semantics as Python. Supported
`X402_MCP_GENERIC_*` overrides are `SPEC`, `BASE_URL`, `PREFIX`, `NAME`,
`PRICING_KEY`, `INCLUDE`, `EXCLUDE`, `TAGS`, `EXCLUDE_TAGS`, `INSTRUCTIONS_TEXT`,
`HELP_URL`, and `MAX_DESCRIPTION_CHARS`. Precedence is CLI > environment > config
> default; explicit env-file values override inherited environment values.
Use `--help` for the complete CLI. `--route-tool NAME --args '{...}'` prints
method, URL, query and JSON body without making a request or loading a signer.

## SocialFetch and tag selection

`providers/socialfetch.toml` uses the vendor's live OpenAPI tags through the generic
catalog. It exposes platform routes, excludes `Auth`, `Monitors` and `System`,
and disables pricing probes because credit prices are embedded in the spec.
The server instructions explain the credit unit and metering caveats.

```sh
target/debug/x402-mcp-prototype --config providers/socialfetch.toml --list-tags
target/debug/x402-mcp-prototype --config providers/socialfetch.toml --tags Twitter,YouTube --list-tools
target/debug/x402-mcp-prototype --meta-config examples/socialfetch.toml --env-file .env
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
connect_timeout_seconds = 30
```

For standalone serving or wallet commands, use the same table in a separate file:

```sh
x402-mcp-prototype --config providers/socialfetch.toml \
  --network-config examples/network-tor.toml --list-tools
x402-mcp-prototype wallet init --state-dir state/public-demo \
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
cargo run --locked --manifest-path Cargo.toml --example snapshot_spec -- \
  https://www.socialfetch.dev/openapi.json tests/fixtures/socialfetch_openapi.json

# Offline input and utility tests:
cargo run --locked --manifest-path Cargo.toml --example snapshot_spec -- \
  /tmp/socialfetch-openapi.json /tmp/socialfetch-fixture.json
cargo test --locked --manifest-path Cargo.toml --example snapshot_spec
```

Use it for JSON OpenAPI request fixtures from any provider; it does not generate
operations digests or modify the custom SocialFetch launcher's committed digest.
Review fixture diffs when refreshing a vendor spec. Python remains a development
dependency for the cross-language comparison suite, not for fixture creation.

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
also capped at four, even when sources request a higher limit.

A process-wide cache keys GET requests by their full routed URL, including any
base URL path prefix. Sources and listeners share results; concurrent requests
for the same URL coalesce. Failures, malformed challenges, rate limits and
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

```sh
cargo test --locked --manifest-path Cargo.toml
cargo clippy --locked --manifest-path Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path Cargo.toml --check

uv sync
.venv/bin/python tests/compatibility.py
uv run pytest tests/ -q

# Initialize/list through the existing Python MCP client, using a throwaway key.
.venv/bin/python scripts/smoke_stdio.py \
  --env EVM_PRIVATE_KEY=0000000000000000000000000000000000000000000000000000000000000001 \
  -- target/debug/x402-mcp-prototype \
  --config providers/pdl.toml --spec tests/fixtures/pdl_openapi.json
```

Fifty-two default Rust tests cover local x402 v1/v2 handshakes, EIP-3009 signature recovery,
Permit2 `upto` fields, amount/asset/network rejection, bounded retries, final
insufficient-funds errors, concurrent signer replacement, schema/routing edge
cases, HTTP bearer authentication, stateless initialize/list/call and lazy help
caching, multi-listener selection/auth/routing, shared wallet profiles, atomic
startup, graceful in-flight shutdown and one-shot pricing discovery, plus independent Keccak/EIP-712 vectors generated by Python. They use localhost and deterministic unfunded test keys; no RPC or
settlement transaction is submitted. A sandbox must permit binding localhost.

The differential script compares **726 tool definitions across 14 configs**
with Python JSON configurations and Rust TOML providers: names, complete descriptions, schemas, HTTP methods, paths and
parameter route maps. It uses committed snapshots/digests and disables probes.
The Python regression suite has 88 passing tests. These checks establish local
protocol behavior; they do not validate actual vendor settlement.

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
x402-mcp-prototype wallet init --state-dir /private/state/new-treasury \
  --key-file /private/keys/new-treasury.key

# Supply the correct birthday for your mnemonic when importing.
x402-mcp-prototype wallet init --state-dir /private/state/treasury \
  --key-file /private/keys/treasury.key --birthday 2000000 \
  --mnemonic-file /private/import/mnemonic.txt

# Without --mnemonic-file, init generates a new seed inside zingolib.
# Status reads persisted metadata and needs neither key nor network access.
x402-mcp-prototype wallet status --state-dir /private/state/treasury

# Use the treasury_id returned by init. This allocates two keys and queued jobs;
# it does not contact NEAR, transfer ZEC, or fund either EVM address.
x402-mcp-prototype wallet pool --state-dir /private/state/treasury \
  --key-file /private/keys/treasury.key --treasury-id UUID \
  --name research --deposit-size 5.00

# Display existing receive addresses without derivation or network access.
# The treasury UUID is read from state; --treasury-id UUID is an optional check.
x402-mcp-prototype wallet addresses --state-dir /private/state/treasury \
  --key-file /private/keys/treasury.key

# Derive another shielded receive address and save its snapshot before returning it.
x402-mcp-prototype wallet address --state-dir /private/state/treasury \
  --key-file /private/keys/treasury.key --treasury-id UUID
```

Use `target/debug/x402-mcp-prototype` or put the built binary on
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
x402-mcp-prototype wallet sync --meta-config examples/servers-managed.toml
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
target/debug/x402-mcp-prototype wallet backup \
  --state-dir /path/to/state --key-file /path/to/key \
  --treasury-id <UUID> --destination /path/to/new-backup
```

The command requires exclusive ownership and never overwrites a destination.
It copies a consistent SQLite snapshot (including pending signed transactions,
EVM keys and all journals) and its encryption key into an owner-only directory.
`backup.json` is the completion marker. Protect the entire backup as spending
material. To restore, stop the original process and use the backup directory as
`state_dir`, its `key` as `key_file`, and the same treasury ID. Do not run the
original and restored copies simultaneously. A Zcash mnemonic alone cannot restore
random EVM keys, pending payment authorizations or the rotation journal.

To retry a job that never produced signed bytes, stop serving and run:

```sh
target/debug/x402-mcp-prototype wallet recover-unprepared \
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
target/debug/x402-mcp-prototype wallet shield-refunds \
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
`auto_fund = true`; live funded execution remains unqualified.

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
x402-mcp-prototype wallet reconcile --meta-config servers.toml --operation-id UUID
# Explicitly allow resubmission of the SAME saved bytes, within deadline/expiry:
x402-mcp-prototype wallet reconcile --meta-config servers.toml --operation-id UUID --rebroadcast
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

Named pools persist two distinct bootstrap candidates and their immutable USDC
targets. Repeating `wallet pool` resumes the existing pool; changing its
`deposit_size` affects future allocations only. Transactional promotion retires
one address, promotes the standby, and creates one new key/funding job. Pools
have independent generation checks and roles. Shared ZEC budget reservations
carry unresolved exposure across days; confirmed costs are charged conservatively
on their confirmation day. A single pending prepared outgoing operation gates
new preparation, and exact bytes plus its wallet snapshot commit together.

Managed profiles connect these transitions to the x402 request path using a
trusted Base RPC. [servers-managed.toml](examples/servers-managed.toml) contains
a complete deployment example. Inspect it without credentials or state access:

```sh
target/debug/x402-mcp-prototype \
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

Managed `mode = "zcash_rotation"` requires `max_input_zec` and `max_fee_bps`.
`deposit_size` defaults to `"5.00"`, `max_price_usd` to `"1.00"`, `wait_seconds`
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
offers are unsupported. Permit2, `upto`, v1, non-authorization flows, nonempty
extensions and unknown extra/offer fields are rejected. Static payment support
is unchanged. An ordinary non-402 response never enters admission.

The per-pool deadline includes waiting for its gate and chain verification.
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
funds and approves the test. Mainnet swaps and seller acceptance remain unqualified.

## Remaining gaps before migration

SVM payments, custom launchers/handlers and receipt accounting are not implemented. The NEAR funding coordinator
is opt-in; confidential route access and a funded swap remain unqualified. Regtest covers deposits, refund shielding and expiry recovery. Deep finalized-chain rollback requires operator review. Encrypted treasury persistence,
controlled sync and managed pool admission are available as described above. Some Python CLI/env options are absent. This is not yet a drop-in
replacement for all Python launchers.

`upto` signing is tested; its required on-chain USDC → Permit2 allowance still
needs provisioning for each real wallet. The prototype never submits approvals
or gas-sponsoring transactions. Real-funded operation remains opt-in by running
it with an operator's key and calling a paid tool.

Two deliberate catalog differences are covered by a targeted Rust test:
body/query collisions mark the renamed body argument as required and restore
the original body key on the wire. Python currently marks/routes these
collisions incorrectly. Recursive schema handling also preserves nested schemas
without Python's depth/combinator compaction bounds. The selected fixture
catalogs still match exactly. Neither implementation is a complete OpenAPI
validator; for example, this prototype handles JSON request bodies, not uploads.

## Zingolib compatibility experiment

The combined build and **13 tests pass** with the exact `zingolib_v6.0.0` tag,
commit `c6381534f802b1022041beda4b01c106ad132329`, on Rust 1.98.1/macOS.
Run the reproducible combined suite with the reviewed vendored connector patch:

```sh
python3 vendor/verify.py
python3 compat/check.py
```

The checker verifies the vendored source hashes, obtains a Cargo-managed `protoc`
compiler, and runs the combined suite with its committed lockfile. First builds
fetch Cargo dependencies and zingolib's public Sapling proving parameters;
zingolib caches/copies those parameters during its build. Tests use temporary
offline wallets and localhost payment servers. The reference checkout's tracked
files remain unchanged. No wallet sync, funded payment, approval or broadcast
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
`.venv/bin/python tests/generate_crypto_vectors.py`; normal Rust
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
Zcash primitives to 0.30.1; the standalone prototype lock retains Alloy 2.1.1.
Both graphs are tested, with Alloy core 1.7.3 and the same two-line patch.

This demonstrates that a single Rust process is feasible. It does not test
funded Zcash transaction construction, chain sync, proving/broadcast,
or NEAR swaps, and is not a wallet security audit. The local reference path is
an experiment-only dependency; the executable's optional `zcash` feature uses
the pinned upstream Git revision. The [rotation plan](../docs/plans/zcash_rotation.md)
specifies a single Rust process with embedded zingolib, one encrypted state
store, and challenge-aware wallet rotation using this dependency patch.

`x402-chain-eip155` also requires its `telemetry` feature with `client` in this
release: the `upto` implementation otherwise references an unavailable
`tracing` dependency. The prototype enables both.
