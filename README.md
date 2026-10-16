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
retrieval, llm help text, and pricing queries use separate Tor identities
grouped by origin. Zcash treasury synchronization has its own identity. Tor mode
uses remote DNS and has no direct-network fallback.

The CLI uses explicit commands: `serve` starts MCP serving, `catalog tools` and
`catalog tags` inspect API inventories, and `config show` / `config check` inspect
and validate configuration. `wallet` manages the treasury; `sources inspect`
reads persisted agent-added sources. Running `x402_treazury` without a command
shows help. See the [command reference](docs/configuration.md).

## Quickstart: Zcash and Tor

The [privacy example conf file](examples/deployments/privacy.toml)
provides three MCP endpoints backed by two rotating wallet pools. Social and
company share a wallet and Tor payer identity; web uses a separate wallet:

| Research context | Selected APIs | MCP endpoint |
| --- | --- | --- |
| Web | Exa, SocialFetch web/community tools, Google Trends | `http://127.0.0.1:6337/mcp` |
| Social | SocialFetch social platforms, PDL people tools | `http://127.0.0.1:6338/mcp` |
| Company | Exa, PDL company search, Otto financials/news, Claw402 startup/investor data, Deepline profiles/ads, x402stock disclosures, GenuineGood grants | `http://127.0.0.1:6339/mcp` |

SocialFetch's overlapping Reddit tools use different wallets on the web and social
endpoints; Exa likewise uses separate wallets on web and company. Company tools
are explicitly selected by name to keep the agent's context focused.

The social endpoint enables `discover_on_demand = true` because of its large
number of tools. This [on-demand discovery mode](docs/agent-sources.md)
initially advertises search/call wrapper tools; agents retrieve full tool
descriptions and schemas as needed. The server's `instructions_text` mentions
the available networks and explains the discovery workflow to help the agent
find the tools without loading every definition into context.

### 1. Build and start Tor

Run these commands from the repository checkout directory:

```sh
scripts/zcash.sh build

cp examples/deployments/privacy.toml examples/deployments/privacy.local.toml
```

The wrapper uses the pinned Rust/Cargo toolchain and supplies protoc.
See [build reproducibility](docs/reproducible-builds.md) for toolchain setup.

The cargo download dependencies and Zcash proving parameters are downloaded
outside of Tor routing by default.

> **NOTE: Incremental Builds**
>
> `scripts/zcash.sh build` defaults to an optimized release build with incremental
> compiler caches disabled, including for the protoc helper. The executable is
> `target/release/x402_treazury`. Use `scripts/zcash.sh build --developer` for an
> incremental development build at `target/debug/x402_treazury`, with file/line
> backtraces and reduced debug information. See
> [build storage options](docs/development.md#build-storage-options) for tradeoffs.

Start Tor Browser, or use an installed Tor daemon. Tor is not bundled or started by
the application. The [example privacy conf file](examples/deployments/privacy.toml)
connects to Tor Browser's `127.0.0.1:9150` SOCKS port. `x402_treazury` uses
SOCKS username+password isolation, so it will not use the same Tor circuits
as your web browsing traffic.

For a dedicated daemon, use a listener such as `SocksPort 127.0.0.1:9050
IsolateSOCKSAuth` and change `network.socks_endpoint` in your local deployment
file. That same file controls initialization, synchronization and serving.
Keep Tor running throughout.

### 2. Create the treasury

```sh
target/release/x402_treazury wallet init \
  --config examples/deployments/privacy.local.toml
```

Initialization creates the wallet directory specified by the config file (`./state/privacy`). It then generates a seed and discovers its birthday through Tor.

The default encryption key is `wallet.key` inside the wallet directory. No
password or OS-keychain protection is provided at this time, but the keyfile
design is amenable to this protection in the future.

Anyone obtaining the wallet directory obtains both state and decryption material.

### 3. Fund and bootstrap the wallets

Display the treasury’s receive addresses:

```sh
target/release/x402_treazury wallet addresses \
  --config examples/deployments/privacy.local.toml
```

Send $5 worth of ZEC to the treasury's **shielded receive address**.

The privacy example conf defines two wallet groups, each starting with one small reserve: **2 x $2 = $4 USDC before fees**.

Individual API payments are capped at **0.05 USDC for web** and **0.10 USDC
for social/company**, so even a $5 initial funding should get you at least 40
web search+page fetch calls, but more likely closer to 100. The social+company
calls tend to be a bit more expensive, but you will still have at least 20 of
those.

You can send additional ZEC to your treasury address at any time.

> **NOTE: All wallets are auto-funded**
>
> Every declared managed wallet pool initializes, even when no server uses it: removing MCP server definitions associated with a wallet pool does
> not eliminate that pool's funding setup. To reduce initial testing expense, **remove one of the `[wallets.NAME]` definitions**, then remove or rebind any servers/sources referencing them.

After sending ZEC, sync the treasury and check its confirmed spendable balance:

```sh
target/release/x402_treazury wallet sync \
  --config examples/deployments/privacy.local.toml

target/release/x402_treazury wallet status \
  --config examples/deployments/privacy.local.toml
```

Once `wallet status` shows sufficient confirmed spendable ZEC, fund the
configured reserve wallets with USDC via bootstap:

```sh
target/release/x402_treazury wallet bootstrap \
  --config examples/deployments/privacy.local.toml
```

Bootstrap uses ZEC treasury funds and waits for confirmed USDC credit to each wallet.

Once bootstrap completes successfully, the wallets are ready for paid API calls.

Repeating the bootstrap command displays current configured inventory and
resumes any unfinished funding work, including automatic recovery for any
failed USDC swap transactions.

### 4. Optionally warm the catalogs

To speed up the MCP server launch, you can pre-warm the API catalog and
pricing cache before serving. The optional `--direct` flag bypasses Tor
for these requests, if Tor is configured.

```sh
target/release/x402_treazury catalog warm --discover-pricing \
  --config examples/deployments/privacy.local.toml
```

**Non-Tor warming with `--direct` reveals your IP address to providers; treasury traffic and
paid API calls still use Tor**.

> **NOTE: Tor 403/429/503 fallback relay**
>
> The privacy example conf automatically uses a [paid discovery relay](docs/configuration.md#paid-discovery-relay)
> when API catalog or pricing requests are blocked, rate limited, or time out over Tor.
> This relay service is also contacted over Tor. Relayed requests cost $0.01
> each, paid by the wallet that is attached to the provider that it
> is contacting. The requests to this relay service use Tor circuit isolation derived from that
> same target wallet. If multiple wallets are assigned to a provider through
> multiple MCP servers (such as with the Exa provider in the privacy example conf),
> one wallet is always chosen deterministically for relayed API catalog requests.

### 5. Start serving

Review the wallet bindings and spending settings before starting the server:

```sh
target/release/x402_treazury config show \
  --config examples/deployments/privacy.local.toml
```

Set `TREAZURY_MCP_TOKEN` with secret bearer token, then launch:

```sh
target/release/x402_treazury serve \
  --config examples/deployments/privacy.local.toml
```

> **NOTE: Single Instance Only**
>
> Do not run multiple instances of x402_treazury from the same Zcash wallet file: cross-instance Zcash transactions will not be synchronized and USDC wallet rotation may fail. See the [wallet reference](docs/wallet-cli.md) for more details about wallet commands.

Serving handles API catalog download and price discovery and wallet replacement funding automatically.

If you skipped funding bootstrap, `serve` starts ZEC-to-USDC funding in the
background after listeners bind. Available tools remain accessible; managed paid
calls require confirmed capacity. Relay-dependent catalogs appear when their payer
becomes usable.

Connect your MCP client to the endpoint(s) in the table above, authenticating with
`Authorization: Bearer <your TREAZURY_MCP_TOKEN>`.

`wallet status` reports persisted progress.

Stop with **only one** Ctrl-C **once** and allow the announced
transaction-safety drain to finish.

> **NOTE: Advanced Wallet Commands**
>
> For funding failures, refunds, backup restoration and RPC configuration, use the
> [wallet reference](docs/wallet-cli.md) and [rotation guide](docs/wallet-rotation.md).
> For bounded paid test runs, use the [live integration runbook](tests/live/INTEGRATION.md).

## Using Expensive APIs

The max API payments of the privacy example are set relatively low, which
excludes some APIs in the set (especially company and grant data searches, which can run from $0.15 up to $5 each). Increse `max_api_payment_usdc` on the main wallet
group definitions to allow larger API payments.

If any of your APIs cost more than **$0.50 per call**, consider
adding a large reserve wallet to that wallet group.

> **NOTE: Why Separate Large Wallets?**
>
> We provide a separate large wallet option for expensive APIs (rather than simply increasing the small wallet `funding_amount_usdc`), because this uses fewer API calls per rotation of both wallets, which reduces usage tracking and linkability.

To enable a large reserve wallet for a wallet group, add a reserve block for that group name under a new stanza `[wallets.NAME.reserves.large]`, and configure the target amount as `funding_amount_usdc`.

When sizing the large wallet, aim for a **4x multiple your largest expected API payment**. For example, a $2.50 maximum API payment suggests a **$10 large reserve**:

```toml
# Add a large wallet to the company group in examples/deployments/privacy.toml
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

For example, this `research` template gives each source its own group with a $2
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

This is an alternative to explicit wallet assignments in the quickstart. Remove
those explicit wallet group assignments where you want the `wallet_assignment` policy to apply; explicit references
retain priority.

Each generated group in this example starts with $12 of requested
USDC inventory, so the three automatically assigned MCP servers wallets would request $36 in total.
`config show` displays the resolved bindings and combined initial-inventory target
before allocating anything. That target excludes fees and bridge-minimum increases.

See [automatic wallets](examples/deployments/servers-auto-wallets.toml),
[multiple managed listeners](examples/deployments/servers-managed.toml), and the
[composition reference](docs/configuration.md#multiple-mcp-ports-from-one-configuration).

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

Source management lets agents find APIs through built-in [x402 List](https://x402-list.com) directory service tools,
and add tools to an MCP endpoint themselves.

The feature is disabled by default.

Configure it in two places: the deployment-wide `[source_management]` section
sets the default wallet and optional persistence file; `source_management = true`
inside a `[servers.NAME]` section enables it for that MCP server endpoint. The wallet name
must reference an existing `[wallets.NAME]` profile.

For a complete configuration example,
start with the [managed-wallet example](examples/deployments/agent-sources-managed.toml)
or [static-wallet example](examples/deployments/agent-sources.toml).

Here is an example excerpt:

```toml
[source_management]
wallet = "agent_shared" # defined elsewhere as [wallets.agent_shared]
registry_file = "../../state/agent-sources.sqlite" # optional; path relative to this file

[servers.research]
listen = "127.0.0.1:8000"
bearer_token_env = "RESEARCH_MCP_TOKEN"
source_management = true
# discover_on_demand = true # keep API definitions out of the advertised tool list
```

Directory requests and added APIs use the shared wallet unless the `[source_management]` stanza has
an explicit `wallet` override. Existing payment caps and endpoint tool filters apply.

Registrations are
shared by agents using the same MCP server endpoint, but are not visible on other MCP server endpoints.
Agent tools cannot change your configured API sources, wallet settings, or spending limits.

An enabled endpoint advertises these eight tools to the agent:

| Tool | How the agent uses it | Main inputs |
| --- | --- | --- |
| `x402_treazury_sources_search` | Browses APIs or asks x402 List for ranked recommendations. | Search query, browse/best mode, filters and ranking preferences. |
| `x402_treazury_source_details` | Checks a candidate's advertised endpoints, pricing and reliability. | Directory `slug` returned by search. |
| `x402_treazury_raw_get` | Calls a documented API directly, paying through the source-management wallet if required. | Public HTTPS `url` and optional query parameters. |
| `x402_treazury_raw_post` | Sends a documented JSON request directly, with normal payment checks. | Public HTTPS `url`, JSON `body`, optional query parameters. |
| `x402_treazury_source_url_info` | Checks common OpenAPI, x402 directory and LLM documentation URLs without paying provider probes. | Exactly one directory `slug` or public HTTPS `base_url`. |
| `x402_treazury_source_add` | Registers an API on this endpoint so its tools become available. | Public OpenAPI `spec_url` and optional display name. |
| `x402_treazury_tools_search` | Retrieves relevant tool descriptions, argument schemas and callable references. | Optional query, registered `source_id` and pagination. |
| `x402_treazury_tool_call` | Invokes a discovered tool using normal wallet payment checks. | A returned `tool_ref` and arguments matching its schema. |

By default, registered API tools also appear in the endpoint's tool list (but this list will not be refreshed until a new conversation).

Set `discover_on_demand = true` to advertise only the eight management tools and
let clients retrieve API schemas as needed, rather than fill the tool list
with all added APIs. Tool arguments and the API discovery
workflow are documented in [agent source management](docs/agent-sources.md#agent-tools).

With `registry_file` set, accepted specifications are saved and restored on restart;
without it, additions last only until the process exits.

To view all saved API sources currently saved in `registry_file`:

```sh
target/release/x402_treazury sources inspect \
  --config path/to/deployment_config.toml
```

Saved API specifications do
not automatically follow vendor updates; they must be updated manually.

First, stop the server, then:

```sh
target/release/x402_treazury sources refresh --server research --source-id SOURCE_ID \
  --config path/to/deployment_config.toml
```

To remove a source:

```sh
target/release/x402_treazury sources remove --server research --source-id SOURCE_ID \
  --config path/to/deployment_config.toml
```

Removal revokes API access without deleting wallet balances or financial journals.

See the [configuration and authority reference](docs/agent-sources.md#configuration-and-authority)
for import restrictions, limits and endpoint permissions.

## Bare Bones Single-Wallet MCP Stdio Server

For a simple static-key stdio setup, provide `EVM_PRIVATE_KEY` through the environment
or a private `.env` file:

```sh
target/release/x402_treazury serve --provider providers/socialfetch.toml \
  --network-config examples/network/tor.toml --env-file .env
```

This uses Tor but does not rotate or fund the static wallet. MCP clients should spawn
the binary with absolute config paths or a repository working directory. Logs go to
stderr; stdout carries MCP output.

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

Run `scripts/check.sh` for the full offline test suite. Live tests are separately
funded and opt-in; ordinary tests use temporary state and unfunded fixture keys.

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
