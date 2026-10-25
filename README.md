# x402_treazury

**Privacy-enhanced x402 payments for AI agents.**

[x402 micropayments](https://x402.org/) have a privacy problem. When you reuse your
payment wallet across multiple agents, your wallet source address lists all their API calls on
the public Base blockchain. Connecting directly to API services also exposes your
network address to these API providers and to Coinbase infrastructure.

**x402_treazury** reduces that linkability with **Zcash-funded rotating wallets** and **Tor isolation tied to each payment identity**. You choose which providers share a wallet and which stay separate.

The `x402_treazury` cli tool loads x402 provider API catalogs, converts them into MCP tools, handles x402 payments, and replenishes Base USDC wallets, without giving the agent spending keys. One Rust process can serve multiple authenticated MCP endpoints with different tools and wallet groups, all filled from the same Zcash treasury wallet.

You can also optionally grant your agent a set of tools to discover and add x402 providers themselves.

## How it works

![Wallet funding and payment flow, with Low Balance Wallets stacked beside the Active USDC Wallet](docs/wallet-flow.svg)

> **NOTE: Payments are not invisible**
>
> `x402_treazury` reduces wallet and network linkability; it does not make payments
> invisible. The Zcash to USDC funding path uses **public NEAR swaps and public Base
> transactions**. These are visible, but unlinkable to your Zcash shielded
> address. Because of the $2 minimum swap size, each wallet will make multiple API
> calls before rotation. Identifying request contents across these rotations can
> also link activity.

Each managed wallet group starts with one small Base USDC reserve and an
optional large reserve, without a separate active allocation. Funded retained wallets remain available for payments.
Validated recent price estimates can choose the smallest sufficient wallet before
the ordinary request. A reserve's first journaled payment makes it active and
leaves its slot vacant; the previous active becomes retained. Ordinary authorized
funding fills each vacancy independently of payment and retirement.
Funding, confirmation and payment uncertainty are persisted across restarts.

Configure each reserve's target explicitly. A group with a $2 small reserve and
no large reserve starts with one $2 wallet, plus
swap and network fees. An optional large reserve adds its configured target.
Bridge-minimum increases require explicit per-slot headroom. Actual spendable
Zcash treasury funds govern enabled funding; the API payment cap is separate.

Payment requests, wallet-specific balance checks and swap jobs use the current
payer wallet address as the Tor circuit isolation identity. Switching payers
changes the Tor isolation identity. API catalog
retrieval and llm help text use separate Tor identities
grouped by origin. Zcash treasury synchronization has its own identity. Tor mode
uses remote DNS and has no direct-network fallback.

The CLI uses explicit commands: `serve` starts MCP serving, `catalog tools` and
`catalog tags` inspect API inventories, and `config show` / `config check` inspect
and validate configuration. `wallet` manages the treasury; `sources inspect`
reads persisted agent-added sources. Running `x402_treazury` without a command
shows help. See the [command reference](docs/configuration.md).

## Quickstart: Z-Cash Wallet with Dynamically Managed Tools

The [managed starter](examples/deployments/managed.toml) exposes Exa web search,
page contents and source-management tools on `http://127.0.0.1:8000/mcp`, backed
by one rotating wallet pool. Agents can [add public APIs](#agent-api-auto-discovery-and-tool-addition) that use that same pool;
API registrations last until the process exits. Exa's catalog loads from its public
OpenAPI specification; no Exa API key is required.

```sh
make build
cp examples/deployments/managed.toml examples/deployments/managed.local.toml
```

Initialize its treasury explicitly, then start the authenticated listener:

```sh
export X402_TREAZURY_CONFIG="examples/deployments/managed.local.toml"
export X402_TREAZURY_MCP_TOKEN="replace_with_a_secret_bearer_token"
target/release/x402_treazury wallet init
target/release/x402_treazury serve
```

`X402_TREAZURY_CONFIG` supplies the deployment file for serving, wallet, catalog,
config and source commands. An explicit `--config FILE` overrides it. Export it
in each terminal where you run commands. Unset it for standalone `--provider`
commands or wallet commands using `--state-dir`.

`X402_TREAZURY_MCP_TOKEN` is optional: when absent, the example listeners allow
requests without authentication and log a WARN. Set it to require bearer auth;
an empty value is rejected. Existing local configs must change their
`bearer_token_env` references to `X402_TREAZURY_MCP_TOKEN` to use this behavior.

To serve over Tor instead, start Tor Browser with its SOCKS listener at
`127.0.0.1:9150`. Pass [the Tor and discovery-relay settings](examples/network/tor.toml):

```sh
target/release/x402_treazury serve \
  --network-config examples/network/tor.toml
```

Or embed those settings once in the fresh local deployment and use the basic command:

```sh
cat examples/network/tor.toml >> examples/deployments/managed.local.toml
target/release/x402_treazury serve
```

`--network-config` replaces the complete `[network]` policy and, when present,
`[relay_fallback]` for that invocation. This Tor example enables configured paid relay fallback for catalog,
document reads and built-in directory/Coinbase searches using the deployment’s
existing payers. Actual API fallback requires explicit `api_requests = true`. Without the option, the managed starter uses its own settings.
For treasury initialization over Tor, pass the same option to `wallet init`, or
initialize using the deployment with the embedded policy.

Send ~$5 ZEC to the shielded receive address printed by initialization. The pool
requests a $2 USDC reserve, accepting bridge-minimum increases up to $3 before
fees; allow funds for fees and future replacements. Each payment is capped at
$0.50. Funding starts after the listener binds, so tools remain visible while
funding is pending. Connect your MCP client with
`Authorization: Bearer <your X402_TREAZURY_MCP_TOKEN>`.

The default key file lives with treasury state at `state/managed/wallet.key`;
protect that directory because it contains both state and decryption material.
Run one serving process per treasury.

`x402_treazury wallet status`
reads saved status alongside serving;
other exclusive wallet administration requires stopping it. See
[wallet administration](docs/wallet-cli.md) and the
[example choices](examples/README.md).

## Comprehensive walkthrough: Zcash and Tor

The [comprehensive deployment](examples/deployments/comprehensive.toml)
provides four MCP endpoints backed by two rotating wallet pools. Social, company
and toolbox share a wallet and Tor payer identity; web uses a separate wallet:

| Research context | Selected APIs | MCP endpoint |
| --- | --- | --- |
| Web | Exa, SocialFetch web/community tools, Google Trends | `http://127.0.0.1:6337/mcp` |
| Social | SocialFetch social platforms, PDL people tools | `http://127.0.0.1:6338/mcp` |
| Company | PDL company search, Otto financials/news, Claw402 startup/investor data, Deepline profiles/ads, x402stock disclosures, GenuineGood grants, RelayShield security and phone reputation, NetIntel phone/IP/infrastructure intelligence | `http://127.0.0.1:6339/mcp` |
| Toolbox | API discovery, direct calls and persistent agent-added tools | `http://127.0.0.1:6340/mcp` |

SocialFetch's overlapping Reddit tools use different wallets on the web and social
endpoints. Company tools are explicitly selected by name to keep the agent's
context focused.

The company endpoint adds 12 selected RelayShield security tools: email breach and
infostealer checks, identity graphs, vendor supply-chain risk, IP intelligence,
ransomware risk, public secret exposure, domain identity risk scores, declared-stack
CVE checks, counterparty wallet risk, SIM-swap checks and phone reputation.
Phone reputation uses a [local companion catalog](providers/relayshield/phone-reputation.toml)
for the route missing from the vendor's main schema. Its SIM-swap signals are cached
for 24 hours; the separate carrier SIM-swap endpoint is included but vendor-documented
availability problems may prevent a result. Neither missing findings nor degraded
results establish safety. See [RelayShield caveats](providers/CAVEATS.md#relayshieldtoml--relayshield).

Seven selected NetIntel tools add phone validation and formatting, IP threat
reputation and combined reports, geolocation, WHOIS/RDAP registration data,
cloud-provider detection and website technology fingerprinting. NetIntel phone intelligence complements RelayShield reputation;
it does not establish scam history or live reachability. Advertised prices for
these tools are $0.002-$0.20 per call, within the shared company wallet's $0.50 cap.

The social endpoint enables `discover_on_demand = true` because of its large
number of tools. This [on-demand discovery mode](docs/agent-sources.md)
initially advertises search/call wrapper tools; agents retrieve full tool
descriptions and schemas as needed. The server's `instructions_text` mentions
the available networks and explains the discovery workflow to help the agent
find the tools without loading every definition into context.

The toolbox endpoint enables
[Agent API Auto-Discovery and Tool Addition](#agent-api-auto-discovery-and-tool-addition)
using the company wallet. Added APIs are saved in `state/privacy-sources.sqlite` and
restored after restart; their tools are available only on the toolbox endpoint.

### 1. Build x402_treazury and start Tor

Run these commands from the repository checkout directory:

```sh
make build

cp examples/deployments/comprehensive.toml examples/deployments/comprehensive.local.toml
```

The Makefile verifies the pinned Rust/Cargo toolchain and supplies protoc.
Builds require GNU Make 3.81+ and a POSIX shell, including on Windows; see
the [development entrypoints](scripts/README.md).
Zcash is always included; static-wallet deployments omit treasury configuration.
Only synthetic proving and local consensus tests use opt-in Cargo features.
See [build reproducibility](docs/reproducible-builds.md) for toolchain setup.

The cargo download dependencies and Zcash proving parameters are downloaded
outside of Tor routing by default.

The build produces `target/release/x402_treazury`. For incremental development
builds, use `make build-dev`; see
[build storage options](docs/development.md#build-storage-options).

Start Tor Browser, or use an installed Tor daemon. Tor is not bundled or started by
the application. The [comprehensive deployment](examples/deployments/comprehensive.toml)
connects to Tor Browser's `127.0.0.1:9150` SOCKS port. `x402_treazury` uses
SOCKS username+password isolation, so it will not use the same Tor circuits
as your web browsing traffic.

For a dedicated daemon, use a listener such as `SocksPort 127.0.0.1:9050
IsolateSOCKSAuth` and change `network.socks_endpoint` in your local deployment
file. That same file controls initialization, synchronization and serving.
Keep Tor running throughout.

### 2. Create the treasury

```sh
export X402_TREAZURY_CONFIG="examples/deployments/comprehensive.local.toml"
target/release/x402_treazury wallet init
```

Initialization creates the configured wallet directory (`./state/privacy`),
generates a seed, and discovers its birthday through Tor. It prints the treasury's
shielded ZEC receive address.

The default encryption key is `wallet.key` inside the wallet directory, with no
password or OS-keychain protection. Anyone obtaining that directory obtains both
state and decryption material.

### 3. Start the MCP server

Set `X402_TREAZURY_MCP_TOKEN` to require bearer authentication, then launch
using the exported `X402_TREAZURY_CONFIG` path. Leaving the token absent disables
authentication and logs a WARN:

```sh
export X402_TREAZURY_MCP_TOKEN="treazury_bearer_auth_key_here"
target/release/x402_treazury serve
```

`serve` synchronizes the treasury, discovers API catalogs, and funds
USDC wallets in the background after listeners bind. With no or insufficient
spendable ZEC, it keeps monitoring and retries funding automatically. You can
deposit ZEC before starting it or at any time afterward.

> **NOTE: Single Instance Only**
>
> Run only one `serve` process per treasury directory. `wallet status` is safe to
> run alongside it. See the [wallet reference](docs/wallet-cli.md) for other commands.

API payments are capped at
**0.10 USDC for web** and **0.50 USDC for company, social and toolbox**,
which share the company wallet. These are per-payment caps, not total budgets.

### 4. Fund the treasury and connect

In another terminal, display the shielded receive address, saved balances and sync
progress using the same deployment file:

```sh
export X402_TREAZURY_CONFIG="examples/deployments/comprehensive.local.toml"
target/release/x402_treazury wallet status
```

Send ZEC to that **shielded receive address**. The example's two wallet groups each request a $2 USDC reserve: **$4 USDC total before fees and accepted bridge-minimum increases**. Allow additional ZEC for those costs and future wallet replacements.

> **NOTE: All declared managed wallets are auto-funded**
>
> Removing an MCP server definition entry does not remove its wallet pool's funding setup. To reduce
> initial funding, remove an unused `[wallets.NAME]` definition and remove or
> rebind servers and sources that reference it before starting `serve`.

While enabled reserve claims need funds, `serve` discovers later ZEC deposits
and confirmations at its shared observation cadence, then funds the managed USDC
wallets when the SDK can spend them. Available tools remain accessible during
funding; paid calls use existing eligible payers under their payment authority.
Saved balance estimates do not prevent an otherwise permitted signature. A
wallet's first positive USDC observation consumes its initial claim, even below
its target, and never authorizes a top-up.

Startup performs finite balance discovery behind bound listeners. Fully funded
idle serving does not watch for incoming transfers. `wallet status` reads saved
observations without network traffic; an idle deposit may remain absent until
startup, explicit `wallet sync` with serving stopped, or another relevant trigger.
See the [wallet observation policy](docs/wallet-rotation.md#wallet-observation-policy)
for the discovery scopes and unavailable-treasury exception.

The comprehensive example uses a [paid discovery relay](docs/configuration.md#relay-fallback)
over Tor when catalog requests are blocked, rate limited or time out.
Relay fees depend on the configured provider and request; relay-dependent catalogs
appear when their payer wallet becomes usable.

Connect your MCP client to the endpoint(s) in the table above, authenticating with
`Authorization: Bearer <your X402_TREAZURY_MCP_TOKEN>`.

To stop, press Ctrl-C once and allow the announced transaction-safety drain to finish.

> **NOTE: Advanced Wallet Commands**
>
> For manual wallet bootstrapping, funding failures, refunds, backup restoration and RPC configuration, use the
> [wallet reference](docs/wallet-cli.md) and [rotation guide](docs/wallet-rotation.md).
> For bounded paid test runs, use the [live integration runbook](tests/live/INTEGRATION.md).

## Using Expensive APIs

The comprehensive example caps each web payment at $0.10 and each payment from the
shared company/social/toolbox wallet at $0.50. The selected RelayShield tools
advertise $0.05-$0.40 per call, within that shared cap. Larger company/grant
queries and result batches can still exceed it. Increase `max_api_payment_usdc`
on the relevant wallet group to authorize larger individual payments; live
challenges determine the actual amount.

If any of your APIs cost more than **$0.50 per call**, consider
adding a large reserve wallet to that wallet group.

> **NOTE: Why Separate Large Wallets?**
>
> We provide a separate large wallet option for expensive APIs (rather than simply increasing the small wallet `funding_amount_usdc`), because this uses fewer API calls per rotation of both wallets, which reduces usage tracking and linkability.

To enable a large reserve wallet for a wallet group, add a reserve block for that group name under a new stanza `[wallets.NAME.reserves.large]`, and configure the target amount as `funding_amount_usdc`.

When sizing the large wallet, aim for a **4x multiple your largest expected API payment**. For example, a $2.50 maximum API payment suggests a **$10 large reserve**:

```toml
# Add a large wallet to the company group in examples/deployments/comprehensive.toml
[wallets.company.reserves.large]
funding_amount_usdc = "10.00"
```

> **NOTE: Why 4X Sizing?**
>
> Ensuring that wallets only serve payments up to 25% of their initial allocation
> size reduces the risk of fragmentation: balances left across several wallets
> cannot be combined for one payment. Retained balances can still serve smaller
> calls, but selection may also keep cheap calls on a sufficient large wallet.
> See the [fragmentation examples and bounds](docs/configuration.md#reserve-sizing-and-fragmentation) for the detailed math and assumptions.

## Wallet Sharing and Automatic Assignment

A wallet group determines which API calls reuse a payment identity until rotation.
Grouping related tools saves funding overhead; separate pools reduce address reuse
across those groups. Sharing the same wallet profile also shares its current
payment-related Tor isolation identity.

| Arrangement | Wallet sharing |
| --- | --- |
| One wallet for the deployment | All selected APIs share one pool |
| One default wallet per MCP server | APIs on each server share a pool |
| One wallet per source | A source shares its pool across listeners |
| Several sources reference one wallet | Your own provider groups |
| One wallet per server/source binding | Separate pool for each binding |

You may want to create many independent and Tor-isolated wallet groups with the same settings, rather than using the same group for multiple MCP server endpoints. This is accomplished using **wallet templates**.

A **wallet template** defines reusable settings that can be applied to any wallet group to set
payment cap, reserve sizes, and funding safeguards to common values. The template itself creates
no wallets. A `wallet_assignment` policy chooses where to create separate groups
using those template settings: per deployment, MCP server, source or server/source binding.

Each generated group has its own wallets and payment identity; using the same
template does not make the groups share funds or addresses.

For example, this `research` template gives each listener its own group with a $2
small reserve and a $10 large reserve. It permits payments up to $2.50, following
the 4x large-wallet sizing guidance above:

```toml
[wallet_templates.research]
mode = "zcash_rotation"
max_api_payment_usdc = "2.50"
max_retirement_remainder_usdc = "0"
max_funding_spend_zec = "0.006" # adjust for the intended USDC target and ZEC price
max_conversion_overhead_percent = 5

[wallet_templates.research.reserves.small]
funding_amount_usdc = "2.00"

[wallet_templates.research.reserves.large]
funding_amount_usdc = "10.00"

[wallet_assignment]
scope = "server" # deployment, server, source, or binding
template = "research"
```

This is an alternative to explicit wallet assignments in either example. Remove
those explicit wallet group assignments where you want the `wallet_assignment` policy to apply; explicit references
retain priority.

Each generated group in this example starts with $12 of requested
USDC inventory, so three automatically assigned listeners would request $36 in total.
`config show` displays the resolved bindings and combined initial-inventory target
before allocating anything. That target excludes fees and bridge-minimum increases.

See the [automatic assignment reference](docs/configuration.md#multiple-mcp-ports-from-one-configuration)
and [comprehensive deployment](examples/deployments/comprehensive.toml) for
multiple listeners sharing explicit named pools.

## Adding API Providers

The [provider catalog](providers/README.md) contains reusable TOML definitions,
including curated catalogs for APIs that need adaptation. Providers carry specific
reliability tags for observed issues, including Tor-related failures; startup prints
corresponding warnings. Tags describe known observations, not current availability
or proof that Tor caused a failure.

For a compatible OpenAPI provider, a source can start with just its specification URL:

```toml
[sources.my_api]
spec = "https://api.example.com/openapi.json"
```

Add `my_api` to a server's `sources`. Some providers also need a `base_url`, tool
filters, schema overrides or payment compatibility settings. Reuse bundled settings
with `extends = "../providers/exa.toml"`; add a `help_url` for a single usage guide.
Use `catalog tags` and `catalog tools` to choose a useful subset instead of exposing an
entire large catalog to the agent.

For large catalogs, set `discover_on_demand = true` under `[servers.NAME]` to provide the agent with search+call wrapper tools instead of every API schema. Agents retrieve full
descriptions and schemas as needed via the `x402_treazury_tools_search()` tool, and can
make calls to discovered tools via the `x402_treazury_tool_call()` tool.

See [on-demand discovery](docs/agent-sources.md) for more details.

### Agent API Auto-Discovery and Tool Addition

Source management lets agents find bundled providers or APIs through
[x402 List](https://x402-list.com) and Coinbase curated search, then add tools to an
MCP endpoint themselves. Known provider IDs and specification URLs can be registered
directly; discovery and usage-help inspection are optional.

The feature is disabled by default.

Set `source_management = true` inside a `[servers.NAME]` section and name its
existing `[wallets.NAME]` profile with `wallet`. Optional `source_registry_file`
on that endpoint enables persistence.

The [comprehensive deployment](examples/deployments/comprehensive.toml) enables
this on its toolbox listener using the existing company pool. The
[managed starter](examples/deployments/managed.toml) enables it on the research
listener using its research pool. See the
[agent source guide](docs/agent-sources.md#configuration-and-authority) for persistence
and wallet sharing.

Here is an example excerpt:

```toml
[servers.research]
listen = "127.0.0.1:8000"
bearer_token_env = "RESEARCH_MCP_TOKEN"
wallet = "agent_shared" # defined elsewhere as [wallets.agent_shared]
source_management = true
source_registry_file = "../../state/agent-sources.sqlite" # optional; relative to this file
# discover_on_demand = true # keep API definitions out of the advertised tool list
```

Directory requests and added APIs use the endpoint's wallet. Existing payment
caps and endpoint tool filters apply.

Registrations are
shared by agents using the same MCP server endpoint, but are not visible on other MCP server endpoints.
Agent tools cannot change your configured API sources, wallet settings, or spending limits.

An enabled endpoint advertises these seven tools to the agent:

| Tool | How the agent uses it | Main inputs |
| --- | --- | --- |
| `x402_treazury_sources_search` | Searches bundled providers offline, x402 List, or Coinbase resources. | `local_only=true` with query/pagination; otherwise `directory` defaults to `x402_list`, or select `coinbase` for ranked curated search. |
| `x402_treazury_provider_details_remote` | Resolves bundled matches, locates specifications/help, or reads exact usage text. | Exactly one x402 List `x402_list_id`, public HTTPS API `base_url`, or `documentation_url`. |
| `x402_treazury_source_add` | Registers an API on this endpoint so its tools become available. | Exactly one bundled `provider_id` or public OpenAPI `spec_url`, plus optional `name`. |
| `x402_treazury_providers_list` | Enumerates bundled provider metadata offline. | Optional `page` and `per_page`. |
| `x402_treazury_tools_search` | Retrieves relevant tool descriptions, argument schemas and readable IDs. | Optional query, registered `source_name` and pagination. |
| `x402_treazury_tool_call` | Invokes a currently available tool with normal wallet payment checks. | Readable `tool_id` and arguments matching its schema. |
| `x402_treazury_raw_call` | Calls any documented public HTTPS x402 API URL with automatic payment handling. | HTTP `method`, `url`, optional query parameters and JSON body. |

For offline discovery, call `x402_treazury_sources_search` with
`{"local_only":true,"q":"web search"}`, or use `x402_treazury_providers_list`.
Pass a selected ID directly to `source_add`, for example `{"provider_id":"exa"}`.
Bundled sources preserve reviewed compatibility settings and default to short names
such as `dyn_exa__search`. Bundled metadata does not establish current reachability
or live qualification. Remote directory search remains the default.

For Coinbase resources, use `{"directory":"coinbase","q":"weather","limit":3}`.
Results preserve complete upstream metadata and partial-result indicators, with no
pagination. Sufficient documented HTTP metadata can go directly to `raw_call`;
an OpenAPI URL can go directly to `source_add`. A published usage link can optionally
be read with `provider_details_remote(documentation_url=...)`. Configured relay fallback
can charge for that read. Missing contracts remain missing: do not guess paid arguments.

Coinbase origin search needs no API key. Tor uses one origin attempt plus
`network.tor_isolation_retries` fresh attempts (default two). Configured
`[relay_fallback.relays]` method lists enable runtime fallback using the endpoint’s
source-management payer. `source_discovery` defaults true for built-in directories,
`provider_metadata` defaults true for schemas/help, and `api_requests` defaults false
for actual API data. Set the applicable preference false to disable its fallback.
With no relay section, requests use only the origin.
See the [relay bootstrap settings](docs/configuration.md#relay-fallback).
Connections to relays retain the configured Tor/direct policy. Relays fetch from
their own networks; their redirect policy, unreported hops and plaintext content
remain trusted. Requested and reported destinations are checked against the
caller’s policy. Every search is fresh and repeating fallback may authorize
another payment. Valid empty/partial results do not trigger fallback; never
automatically replay an uncertain paid search.

Supply a readable source name when registering: `name = "netintel"` creates tools
such as `dyn_netintel__ip_reputation_analyze`. Names are preserved exactly and are
case-sensitive; `__` is reserved as the separator. Search with
`{"source_name":"netintel"}` and call the returned `tool_id`.
Use `dyn_netintel__*` for source-specific tool filters. Existing ID-based filters
must be updated explicitly; they are not rewritten.

By default, registered API tools also appear in the endpoint's tool list; clients may cache that list. Tool IDs remain usable across turns and restarts while available.

Set `discover_on_demand = true` to advertise only the seven management tools and
let clients retrieve API schemas as needed, rather than fill the tool list
with all added APIs. Tool arguments and the API discovery
workflow are documented in [agent source management](docs/agent-sources.md#agent-tools).

With `source_registry_file` set, accepted specifications are saved and restored on restart;
without it, additions last only until the process exits.

To view saved API sources and each source's tool names, HTTP methods and paths:

```sh
export X402_TREAZURY_CONFIG="path/to/deployment_config.toml"
target/release/x402_treazury sources inspect
```

Each source's `tools` list is rebuilt offline from its saved specification and
accepted settings or generic selection, before current listener filters. Disabled sources still show their
saved tools; removed sources have an empty list. An unreadable saved catalog has
`tools: null` and a `tools_error` instead. For current callable tools and full
schemas, use `x402_treazury_tools_search` on the owning MCP endpoint.

Saved API specifications do
not automatically follow vendor updates; they must be updated manually.

First, stop the server, then:

```sh
target/release/x402_treazury sources refresh --server research --source SOURCE_ID_OR_NAME
```

To remove a source:

```sh
target/release/x402_treazury sources remove --server research --source SOURCE_ID_OR_NAME
```

Use an exact saved ID or name with `--source`. To remove every saved source on
that endpoint, use `sources remove --server research --all`.
Removal revokes API access without network or wallet work and preserves financial journals.

See the [configuration and authority reference](docs/agent-sources.md#configuration-and-authority)
for import restrictions, limits and endpoint permissions.

## Experimental HTTP/2 cover traffic

Tor mode enables bounded unsigned range traffic by default for eligible configured
sources. Explicit profiles can add randomized request-header padding and tune
concurrency and sampling distributions. This is a
research vehicle for traffic-shape experiments, **not a demonstrated defense against
traffic analysis**. It shares the real request's isolation scope and prioritizes paid
traffic.

Set `network.cover_traffic_enabled = false` to disable it globally, or
`cover_traffic_enabled = false` in a provider/source definition to disable it for that
provider. Some bundled providers already opt out for compatibility. Agent-added
sources do not automatically authorize cover traffic. See the
[cover traffic guide](docs/cover-traffic.md) for distributions, bounds and qualification.

## Documentation and status

| Guide | What it covers |
| --- | --- |
| [Configuration](docs/configuration.md) | Sources, listeners, filters, pricing and transport options |
| [Wallet CLI](docs/wallet-cli.md) | Treasury operations, funding limits and recovery |
| [Wallet rotation](docs/wallet-rotation.md) | Admission, reserve claims, retained capacity and durable accounting |
| [Network isolation](docs/network-egress.md) | Tor identities, connection pools and egress boundaries |
| [Development](docs/development.md) | Tests, coverage and complexity tools |
| [Documentation index](docs/README.md) | Architecture, demos, plans and further references |

Common development commands run from the repository root:

| Command | Purpose |
| --- | --- |
| `make` / `make build` | Optimized release build |
| `make build-dev` | Incremental development build |
| `make test` | Default test suite |
| `make cargo-check` | Fast compilation check |
| `make fmt` / `make fmt-check` | Apply / check formatting |
| `make clippy` | Clippy with warnings denied |
| `make check` | Full ordered development checks |
| `make coverage` | Default coverage report |
| `make help` | All entrypoints and argument examples |

See the [entrypoint reference](scripts/README.md) for prerequisites, focused tests
and optional coverage modes. Run `make check` for the full offline test suite.
Live tests are separately funded and opt-in; ordinary tests use temporary state and unfunded fixture keys.

## Related projects

`x402_treazury` exposes existing Base USDC x402 APIs as MCP tools, with
Zcash-funded wallet rotation and optional Tor isolation. API Providers receive ordinary
x402 payments without integrating a new privacy scheme; the Base payments remain
public.

These projects explore related approaches to private API access:

| Project | Approach and comparison |
| --- | --- |
| [CipherPay](https://github.com/atmospherelabs-dev/cipherpay-x402) | Middleware for accepting shielded ZEC payments through x402 and MPP. API Providers must integrate Zcash payment acceptance to be used with it; x402_treazury uses Zcash to fund rotating Base USDC wallets for existing x402 providers. |
| [zimppy](https://github.com/betterclever/zimppy) | Zcash wallet and Rust/TypeScript SDKs implementing the Machine Payments Protocol (MPP), including prepaid sessions and an MCP example. Related through Zcash and agent tooling, it requires API servers supporting its Zcash payment method. |
| [SecretPay](https://github.com/DVB-ANRS/SecretPay) | A hackathon prototype using Unlink privacy pools and disposable wallets to make ordinary x402 payments on Base Sepolia. Its gateway and wallet approach is close to x402_treazury's, but uses a different funding mechanism; its documented direct-transfer funding fallback can expose the funding link. |
| [Galactica aztec-x402](https://github.com/Galactica-corp/aztec-x402) | Client and server tooling for private x402 settlement on Aztec, with a public-testnet demo. API Providers must integrate the Aztec payment flow to be used; x402_treazury retains existing Base USDC settlement and reduces linkability through funding, wallet rotation and network isolation. |
| [PrivAgent](https://github.com/Himess/privagent) | A shielded-pool payment protocol on Base with x402 middleware and client tooling. It brings privacy into settlement through a custom protocol that API providers must integrate; x402_treazury keeps ordinary public Base USDC payments. |
| [zkAPI](https://blog.ethereum.org/2026/10/01/introducing-zkapi) | Private usage credits backed by zero-knowledge proofs, with temporary capped API keys for metered usage. It separates payment identity from API usage through a credit/session model. Network anonymity and request-content correlation remain separate concerns. |

## License

AGPL-3.0. See [LICENSE](LICENSE) and [CONTRIBUTING.md](CONTRIBUTING.md).

Dual licensing available.
