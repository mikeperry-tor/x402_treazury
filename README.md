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

You can also optionally grant your agent a set of tools to discover and add x402 providers themselves, and as a separate additonal option, to send and receive cryptocurrency on any blockchain supported by Near Intents, using shielded ZEC swaps.

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

## Going further

- [Advanced deployments](README_ADVANCED.md): configuration walkthroughs, Tor, bootstrap, relay details, adding new providers.
- [Large reserves and expensive APIs](docs/configuration.md#reserve-sizing-and-fragmentation): sizing, fragmentation and the separate large-wallet rationale.
- [Wallet sharing and automatic assignment](docs/configuration.md#multiple-mcp-ports-from-one-configuration): templates, scopes and examples.
- [Agent Zcash payments and swaps](docs/configuration.md#direct-swap-control): optional receive/send tools and limits.
- [Agent-added API providers](docs/agent-sources.md#agent-tools): discovery, registration and direct calls.
- [Wallet CLI](docs/wallet-cli.md): treasury operations, funding limits and recovery.

## Documentation and status

| Guide | What it covers |
| --- | --- |
| [Advanced deployments](README_ADVANCED.md) | Research desk, comprehensive example, Tor and bootstrap walkthroughs |
| [Examples](examples/README.md) | Deployment choices, wallet groups, ports and source selections |
| [Configuration](docs/configuration.md) | Sources, listeners, filters, pricing and transport options |
| [Wallet CLI](docs/wallet-cli.md) | Treasury operations, funding limits and recovery |
| [Wallet rotation](docs/wallet-rotation.md) | Admission, reserve claims, retained capacity and durable accounting |
| [Network isolation](docs/network-egress.md) | Tor identities, connection pools and egress boundaries |
| [Provider caveats](providers/CAVEATS.md) | Current provider support boundaries and uncertainty |
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
