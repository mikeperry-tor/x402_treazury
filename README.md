# x402_treazury

[![CI](https://github.com/mikeperry-tor/x402_treazury/actions/workflows/ci.yml/badge.svg?branch=main&event=push)](https://github.com/mikeperry-tor/x402_treazury/actions/workflows/ci.yml)
[![Default line coverage](https://raw.githubusercontent.com/mikeperry-tor/x402_treazury/badges/coverage.svg)](docs/testing.md#github-actions-and-coverage)

**Privacy-enhanced x402 payments for AI agents.**

[x402 micropayments](https://x402.org/) have a privacy problem. When you reuse your
payment wallet across multiple agents, your wallet source address lists all their API calls on
the public Base blockchain. Connecting directly to API services also exposes your
network address to these API providers and to Coinbase infrastructure.

**x402_treazury** reduces that linkability with **Zcash-funded rotating wallets** and **Tor isolation tied to each payment identity**. You choose which providers share a wallet and which stay separate.

The `x402_treazury` cli tool loads x402 provider API catalogs, converts them into
MCP tools, handles x402 payments, and replenishes Base USDC wallets, without giving the agent spending keys. One Rust process can serve multiple authenticated MCP endpoints with different tools and wallet groups, all filled from the same Zcash treasury wallet.

You can also optionally grant your agent a set of tools to discover and add x402 providers themselves, and, as a separate additional option, to send and receive cryptocurrency on any blockchain supported by Near Intents, using shielded ZEC swaps.

You can set custom financial limits on all of these transactions.

## How it works

![Wallet funding and payment flow, with Low Balance Wallets stacked beside the Active USDC Wallet](docs/wallet-flow.svg)

> **NOTE: Payments are not invisible**
>
> `x402_treazury` reduces wallet and network linkability; it does not make payments
> invisible. The Zcash to USDC funding path uses **public NEAR swaps and public Base
> transactions**. These are visible, but unlinkable to your Zcash shielded
> address. Because of the Near Intents ~$2 minimum swap size, each wallet will make
> multiple API calls before rotation. Timing correlation and identifying request
> contents across wallet rotations can also link activity. x402_treazury schedules
> wallet rotation swaps to decouple their execution from prior transactions, but
> if Near Intents USDC swap volume is extremely low, this correlation will be easier.
> You can also bind independent wallet groups to specific API collections, to reduce
> content linkability.

Each managed wallet group uses one small (~$2.50 USDC) wallet pair, and an
optional custom-denomination large wallet pair.

Every wallet group always maintains as least two USDC wallet addresses to ensure
that no x402 calls are blocked or delayed by refill swaps: one active USDC
wallet, one fully-funded reserve USDC wallet, and a collection of previously
used low-balance wallets. Thus, even though Near Intents swaps can take some
minutes to settle, your agent never notices, nor do you experience any delays.

API catalog pricing data and cached paid API prices are used to choose the
smallest sufficient USDC wallet address from a wallet group for each paid x402
request. ZEC-to-USDC swaps are scheduled on a randomized timer (30-120 seconds) to
create a new wallet address when one of the wallets in a group drops below a
randomized threshold (5-25%) of its initial value.

If Tor is in use by x402_treazury, then payment requests, wallet-specific
balance checks, and swap jobs all use the current payer wallet address as the
Tor circuit isolation identity. Switching payers changes the Tor isolation
identity. API catalog retrieval and llm help text use separate Tor identities
isolated by the domain of the destination.  Zcash treasury synchronization has
its own Tor identity and isolation. Tor mode uses remote DNS and has no
direct-network fallback. If you [enable direct Zcash payments and swap tools](docs/configuration.md#direct-swap-control) for your agent, each swap is
isolated to a separate Tor circuit.

The CLI uses explicit commands: `serve` starts MCP endpoint serving, `catalog
tools` inspects API inventories, `catalog warm` caches the API catalog, `wallet`
manages the ZEC treasury and USDC wallet groups, `sources inspect` reads
persisted agent-added sources. Running `x402_treazury` without a command shows
help. See the [command reference](docs/configuration.md).

## Choose a deployment

| Path | Best for | Network | Starting cost |
| --- | --- | --- | --- |
| [Managed starter](examples/deployments/managed.toml) | Learning the model and making a first paid call | Direct | One $5 ZEC allocation |
| [Research desk](examples/deployments/research_desk.toml) | Multi-domain research with task-oriented endpoints | Tor | One $5 ZEC allocation to start; fully funding all four independent wallet groups takes ~$20. |
| [Private Hermes](https://github.com/mikeperry-tor/private-hermes-agent) | A complete packaged private Hermes Agent stack | Tor-routed TEE-verified private agent use | Same as Research Desk (uses this config by default) |

The managed starter is the smallest readable configuration: one listener, one
wallet group and one source. The research desk is the full selection described in
[README_ADVANCED.md](README_ADVANCED.md). `comprehensive.toml` remains a smaller
multi-listener example documented there.

[Private Hermes](https://github.com/mikeperry-tor/private-hermes-agent) is
maintained separately. It packages Hermes Agent, Tor-routed egress,
validated TEE confidential inference, a code executor, and this project.
It adds its own requirements (Docker Engine 28+ with Linux containers, Compose
2.36+, make, Python 3), and it pins the versions of each component. Stack and
packaging issues belong there, and x402_treazury issues belong here.

For the research desk, `$5 to start` means one initial bootstrap allocation; all APIs will work, but all will share an initial bootstrap wallet.

## Quickstart: Z-Cash Wallet with Dynamically Managed Tools

The [managed starter](examples/deployments/managed.toml) exposes Exa web search,
page contents and provider-management tools on `http://127.0.0.1:8000/mcp`, backed
by one rotating wallet pool. Agents can
[add public APIs](docs/agent-sources.md#agent-tools) that use that same pool;
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

The example listeners require `X402_TREAZURY_MCP_TOKEN`; missing, empty and
whitespace-only values fail startup. To allow unauthenticated access explicitly,
set `auth = false` and omit `bearer_token_env` on that listener. Host header
checking must remain enabled when authentication is disabled (this ensures
that your browser's web pages can't access your MCP endpoints).

Send $5 worth of ZEC to the shielded receive address printed by initialization to cover
the $3 upper initial nominal USDC bound plus source fees and bridge costs. Add
additional funds for future USDC wallet replacement rotations. Each x402 API payment is capped at
$0.50. Funding starts after the listener binds, so tools remain visible while
funding is pending. Connect your MCP client with
`Authorization: Bearer <your X402_TREAZURY_MCP_TOKEN>`.

To serve over Tor instead, start Tor Browser with its SOCKS listener at
`127.0.0.1:9150` and pass [the Tor and discovery-relay settings](examples/network/tor.toml):

```sh
target/release/x402_treazury serve \
  --network-config examples/network/tor.toml
```

`--network-config` replaces the complete `[network]` policy and, when present,
`[relay_fallback]` for that invocation.

For treasury initialization over Tor, pass the same option to
`wallet init`, or initialize using a deployment with the embedded policy.

This Tor config enables a paid relay fallback for catalog and x402 provider
directory searches using the deployment's existing payers. This means that
x402_treazury will use up to two x402 paid API calls to fetch catalogs and
perform directory searches, if it experiences a captcha, 429, 503, or other Tor
block by an API provider or directory service. Three separate Tor circuits will
be used before these payments, so this fallback is very rarely needed.

Actual API call data is not sent through these x402 relay providers; that
requires explicit `api_requests = true`. This is off by default to avoid
exposing API data to these paid relay providers. Additionally, no providers in
our catalog have ever required this option for actual API calls; they all
happily accept payment over Tor.

The wallet key file lives with treasury state at `state/managed/wallet.key`;
protect that directory because it contains both state and decryption material.

Run one only serving process per state directory.

`x402_treazury wallet status` reads saved wallet information, and can be used
when serve is either running or not; other wallet administration requires
stopping `serve`.

See [wallet administration](docs/wallet-cli.md) and the [example choices](examples/README.md)
for more details.

## API discovery and provider management

The `examples/deployments/managed.toml` configuration's `research` endpoint lets
the agent extend its own toolset at runtime. It enables local and remote
provider management and documented raw URL calls, so the agent can find bundled
providers, search public directories, register new APIs and call documented
endpoints directly. These tools are off by default on other endpoints and must
be enabled deliberately. In the
[advanced configuration example configs](./README_ADVANCED.md), these tools are
provided on the `toolbox` endpoint.

New registrations belong to the endpoint that added them and are invisible to
other endpoints. They last until the process exits unless you set
`source_registry_file`. Normal payment caps and endpoint tool filters still
apply.

| Tool | Enabled by | What the agent uses it for |
| --- | --- | --- |
| `x402_treazury_providers_list_local` | `provider_management_local = true` | Finds reviewed bundled provider definitions offline, with optional `q`, `page` and `per_page`. |
| `x402_treazury_providers_add_local` | `provider_management_local = true` | Registers a bundled provider by exact `provider_id`, with optional `name`. |
| `x402_treazury_providers_search_remote` | `provider_management_remote = true` plus `remote_provider_directories` | Searches every configured directory, with optional `q`, `limit` and `max_price_usd`. |
| `x402_treazury_providers_add_remote` | `provider_management_remote = true` | Registers a public HTTPS OpenAPI 3 JSON specification by `spec_url`, with optional `name`. |
| `x402_treazury_provider_details_remote` | `provider_management_remote = true` | Locates specifications or help from `base_url`, reads usage text from `documentation_url`, or looks up an `x402_list_id` when `x402-list` is selected. |
| `x402_treazury_raw_call` | `raw_url_calls = true` | Calls an exact documented public HTTPS URL with required `method` and `url`, plus optional `query`, `headers` and JSON `body`. |

Set `discover_on_demand = true` on an endpoint with a large catalog to keep full API
definitions out of the advertised tool list. The endpoint then adds two wrapper
tools, and the agent retrieves schemas only when it needs them:

| Tool | What the agent uses it for |
| --- | --- |
| `x402_treazury_tools_search` | Finds callable tools and their complete input schemas on this endpoint; optional `query`, `source_name`, `cursor` and `limit`. Makes no provider requests. |
| `x402_treazury_tool_call` | Invokes a discovered `tool_id` with matching `arguments`; ordinary payment checks apply. |

These two tools are also advertised whenever either provider-management capability
is enabled, because the agent needs them to find and call newly registered APIs.
On-demand discovery only changes tool advertising: catalogs still load and direct
tool calls still work, but most agent frameworks will require using these indirect
calls to non-advertized tools. See [agent source management](docs/agent-sources.md) for
directory behavior, registration names and persistence.

## Enabling Direct Cryptocurrency Transactions

Optionally let agents receive payments into your Zcash treasury and send ZEC or
foreign assets from it. These tools are disabled by default. Native ZEC transfers
use shielded Ironwood transactions; foreign assets use NEAR swaps.

Use an initialized `[treasury]` and an authenticated MCP endpoint (`auth = true`).
Add this to your deployment file, replacing `research` with the endpoint name
(`research` in the quickstart, `toolbox` in the research desk):

```toml
[servers.research.swap_control]
receive = true
send = true
max_send_zec = "0.05" # Optional per-send ZEC limit, including the network fee.
```

Enable `receive` and `send` independently. `max_send_zec` limits only sends and
outgoing swaps through these tools; it does not limit managed wallet funding or other ZEC
usage, nor does it limit incoming swaps. Omit it or set it to `"none"` for no total-debit limit. Native ZEC needs no
NEAR setup; foreign assets require the [NEAR swap settings](docs/configuration.md#direct-swap-control)
under `[funding]`. Start or restart `serve` to expose the enabled tools.

| Tool | Enabled by | What it does |
| --- | --- | --- |
| `x402_treazury_swap_assets` | Either flag | Search native ZEC and supported foreign assets by symbol, chain or keywords. |
| `x402_treazury_payment_request_create` | `receive` | Create a fresh shielded ZEC receive address or a single-use foreign swap deposit request. |
| `x402_treazury_swap_preview` | `send` | Preview an outgoing payment and whether its estimated total fits the send limit and spendable ZEC balance. |
| `x402_treazury_swap_send` | `send` | Send native ZEC or swap treasury ZEC for a specified foreign amount. |
| `x402_treazury_swap_status` | Either flag; retained for saved requests | Check sent payments or receive requests, including current UTC time and available expiry countdowns. |

The tool descriptions give the agent the usage rules for these tools in the tool
descriptions, to ensure correct usage within their limitations. In particular,
foreign payers must be told to send the **exact requested amount**, with network
fees and any required memo added, and agents must never to reuse a swap receive
address: another payment needs a new `x402_treazury_payment_request_create` call
and a fresh quote/address. The tool descriptions also explain that native ZEC
addresses may be reused and that partial receipts accumulate toward the
requested amount, and that saved requests remain accessible through status after
new sends or receives are disabled.

Even so, you may want to reinforce these requirements in your prompts for any
agent doing heavy cryptocurrency activity, to reduce the chance of tool
description compliance issues. LLMs are trained to place tool
instruction descriptions at lower trust and lower priority than prompts and
direct requests. See the [direct payment and swap
reference](docs/configuration.md#direct-swap-control) for parameters and timing
details.

> **NOTE**: It should go without saying that even with these tool descriptions,
> spend limits, and careful prompting, these tools are still
> [incredibly risky](https://github.com/mikeperry-tor/HostileShop/) to enable,
> due to prompt injection risk and cryptocurrency scams that even
> humans regularly fall for. Do NOT give your agent large quantities of ZEC,
> and have it regularly transfer any ZEC that it earns out of its wallet to
> an offline wallet or a wallet that you have exclusive control over.

## Going further

- [Advanced deployments](README_ADVANCED.md): configuration walkthroughs, Tor, bootstrap, relay details, adding new providers.
- [Command-line guide](docs/usage/cli.md): setup, wallet management, wallet inspection, and recovery workflows.
- [Configuration guide](docs/usage/configuration.md): operator choices for listeners, sources, wallets, Tor and relay fallback.
- [Large reserves and expensive APIs](docs/usage/configuration.md#small-and-large-reserves): sizing, fragmentation and when to opt into a large reserve.
- [Wallet sharing and automatic assignment](docs/usage/configuration.md#reusing-one-policy-across-many-groups): templates, scopes and examples.
- [Sharing a payer during first use](docs/usage/configuration.md#sharing-a-payer-during-first-use): bootstrap groups and deferred funding.

## Developer Documentation (Warning: Heavy LLM-isms)

| Guide | What it covers |
| --- | --- |
| [Documentation index](docs/README.md) | Architecture, demos, plans and further references |
| [Framework integration](docs/framework-integration.md) | Catalog preparation, degraded coverage and endpoint readiness contract |
| [Provider caveats](providers/CAVEATS.md) | Current provider support boundaries and uncertainty |
| [Wallet rotation](docs/wallet-rotation.md) | Admission, reserve claims, retained capacity and durable accounting |
| [Network isolation](docs/network-egress.md) | Tor identities, connection pools and egress boundaries |
| [Developer orientation](docs/development.md) | Platform support info, tests, diagnostic logs, code coverage, and complexity tools |

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
