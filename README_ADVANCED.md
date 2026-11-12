# Advanced deployments

This guide covers the multi-listener deployments. Start with the
[README quickstart](README.md#quickstart-z-cash-wallet-with-dynamically-managed-tools)
to learn the model, or use the
[research desk](examples/deployments/research_desk.toml) directly if you want the
full task-oriented selection.

| Example | Listeners | Wallet groups | Network | Treasury state |
| --- | --- | --- | --- | --- |
| [research_desk.toml](examples/deployments/research_desk.toml) | 12 | general, personal, health, financial | Tor | `state/research_desk` |
| [comprehensive.toml](examples/deployments/comprehensive.toml) | 4 | web, company | Tor | `state/privacy` |

The research desk is the broad selection. The comprehensive example is a smaller
multi-listener showcase with the same bootstrap, relay and shared-payer model.

## Prerequisites

Run these commands from the repository checkout. `make build` verifies the pinned
Rust/Cargo toolchain and supplies protoc. Make requires GNU Make 3.81+ and a POSIX
shell. Supported runtime validation covers Linux and macOS; Windows runtime work is
deferred. See [portability](docs/development.md#portability).

The Zcash treasury is unconditional in the application build; static-wallet
deployments simply omit treasury configuration. Only synthetic proving and local
consensus tests use opt-in Cargo features. Cargo downloads and Zcash proving
parameters are fetched outside Tor routing by default.

Both advanced examples use Tor. Start Tor Browser with its SOCKS listener at
`127.0.0.1:9150`, or use an installed Tor daemon. Tor is not bundled or started by
the application. The examples use SOCKS username/password isolation, so their
circuits do not match your web browsing traffic. For a dedicated daemon, use a
listener such as `SocksPort 127.0.0.1:9050 IsolateSOCKSAuth` and change
`network.socks_endpoint` in the local deployment. The endpoint accepts an IP
literal, a network name such as `tor:9050`, or a Unix path (`unix:/absolute/path`).
Keep Tor running throughout.

## Research desk walkthrough

### 1. Build, copy and start Tor

```sh
make build
cp examples/deployments/research_desk.toml examples/deployments/research_desk.local.toml
```

Start Tor Browser or your daemon at the configured SOCKS endpoint. If you prefer
not to embed network settings, pass the shared policy for one invocation:

```sh
target/release/x402_treazury serve \
  --config examples/deployments/research_desk.local.toml \
  --network-config examples/network/tor.toml
```

`--network-config` replaces the complete `[network]` policy and, when present,
`[relay_fallback]` for that invocation. The research desk already embeds Tor and
relay settings, so the option is needed only when you want a different policy.

### 2. Create the treasury

```sh
export X402_TREAZURY_CONFIG="examples/deployments/research_desk.local.toml"
target/release/x402_treazury wallet init
```

Initialization creates the configured `state/research_desk` directory, generates a
seed and discovers its birthday through Tor. It prints the treasury's shielded ZEC
receive address. The default encryption key is `wallet.key` inside that directory,
with no password or OS-keychain protection; anyone obtaining the directory obtains
both state and decryption material.

### 3. Start the server

```sh
export X402_TREAZURY_MCP_TOKEN="replace_with_a_secret_bearer_token"
target/release/x402_treazury serve
```

`serve` synchronizes the treasury, discovers API catalogs and funds managed USDC
wallets in the background after every listener binds. The research desk prepares
the general group first. Personal, health and financial activate only after a
validated API invocation in one of their endpoints, including a free response or
cache hit; calls never wait for swaps. Previously credited groups route directly on
restart, and depletion never returns them to general.

Run one `serve` process per treasury directory. `wallet status` is safe alongside
serving; other exclusive wallet administration requires stopping it.

### 4. Fund the treasury and connect

```sh
export X402_TREAZURY_CONFIG="examples/deployments/research_desk.local.toml"
target/release/x402_treazury wallet status
```

Send ZEC to the shielded receive address. Initial setup is general's one $2-3
nominal allocation plus fees and bridge costs. The eventual four-group initial
inventory is $8-12 nominal, before fees, relays and later replacements. A light
trial can start with about $5 of ZEC; continued staggered use is better covered by
roughly $20, including headroom. These are capital estimates, not guaranteed
spendable balances or lifetime costs.

| Group | Endpoints | Payment cap |
| --- | --- | --- |
| general | toolbox, web, science, shopping, world | $1.00 |
| personal | social, property, travel | $1.00 |
| health | health | $0.50 |
| financial | business, finance, security | $1.00 |

Ports and endpoint scopes are listed in [examples/README.md](examples/README.md).
Connect each MCP client to its endpoint with
`Authorization: Bearer <your X402_TREAZURY_MCP_TOKEN>`.

The example enables:

```toml
[wallet_bootstrap]
bootstrap_group = "general"
deferred_groups = ["personal", "health", "financial"]
```

Paid metadata and directory relay fees stay on general across API cutover and do
not activate a group. Relay fallback covers catalog and help reads by default;
actual API fallback remains disabled unless `api_requests = true`. Relay fees
depend on the provider and request. With bootstrap policy enabled,
`relay_fallback.wallet` is overridden for metadata and discovery.

Comment out the entire bootstrap table to fund all declared groups normally and
use separate payers from the beginning. Existing funds and earlier links are not
moved or erased. Removing a listener alone does not disable its declared managed
group.

Separate initial addresses avoid direct payer reuse, but closely spaced swaps
still provide a funding-time correlation signal. First-use activation and
randomized later opportunities can weaken it. Bootstrap payments use a visibly
common payer; timing, amounts, providers, request content and settlement can still
correlate activity. Delayed funding does not erase earlier links or provide an
unlinkability guarantee.

### 5. Stop

Press Ctrl-C once and allow the announced transaction-safety drain to finish.
Do not start a second serving process while work is draining.

## Comprehensive example walkthrough

The [comprehensive deployment](examples/deployments/comprehensive.toml) provides
four MCP endpoints and two intended wallet groups:

| Endpoint | Port | Initial group and cap | Selected scope |
| --- | --- | --- | --- |
| web | 6337 | web, $0.10 | Exa, Google Trends and web sources |
| social | 6338 | company (initially web), $0.10 until first company credit, then $0.50 | People and social platform research |
| company | 6339 | company (initially web), $0.10 until first company credit, then $0.50 | Company, public-record, security and phone intelligence |
| toolbox | 6340 | company (initially web), $0.10 until first company credit, then $0.50 | API discovery, direct calls and persistent agent-added tools |

All endpoints initially use web's payer and cap. After company's first positive
credit, social, company and toolbox use company's separate payer and $0.50 cap;
web keeps its own group. Previously credited company groups route directly on
restart, and later depletion never returns them to web.

```sh
make build
cp examples/deployments/comprehensive.toml examples/deployments/comprehensive.local.toml
export X402_TREAZURY_CONFIG="examples/deployments/comprehensive.local.toml"
target/release/x402_treazury wallet init
export X402_TREAZURY_MCP_TOKEN="replace_with_a_secret_bearer_token"
target/release/x402_treazury serve
```

Initial setup is web's one $2-3 nominal small allocation. Both groups' eventual
initial inventory totals $4-6 nominal. Separately, preparing one group's first small
wallet and its later replacement spans two allocations: $4-6 nominal, averaging
approximately $5 before bridge-floor adjustments and fees. That one-group estimate
is not the same as both groups' combined initial inventory. Allow randomization
headroom to $6. A nominal bridge minimum above the $3 ceiling can refuse a route
rather than silently increasing the bound.

The example enables `bootstrap_group = "web"` and `deferred_groups = ["company"]`.
If the treasury is already funded, you can run `wallet bootstrap` before `serve` to
prepare only web's enabled reserves, without catalogs or listeners. Stop serving
before running this exclusive command; it does not prepare company or activate
participant demand.

The company endpoint adds selected RelayShield and NetIntel security tools. Their
advertised prices can exceed web's initial $0.10 cap and some can be refused while
company prepares; their $0.05-$0.40 prices fit the later $0.50 cap, though larger
company or grant batches may still exceed it. NetIntel phone intelligence
complements RelayShield reputation but does not establish scam history or live
reachability. See [provider caveats](providers/CAVEATS.md) for current limits.

## Expensive APIs and large reserves

A per-payment cap authorizes one challenge, not a job budget. Increase
`max_api_payment_usdc` on the relevant wallet group for larger individual
payments; the live challenge determines the actual amount. For APIs costing more
than $0.50 per call, consider a large reserve:

```toml
# Add to a local comprehensive deployment.
[wallets.company.reserves.large]
# Default randomized nominal funding: $10-25.
```

A separate large wallet is preferred over enlarging the small slot because both
wallets then rotate after fewer calls, reducing usage tracking and linkability.
Size large to at least twice the largest expected payment, preferably four times
when prices vary. For a $2.50 maximum payment, $10 is the lower end of the default
large range. Ensuring payments stay below 25% of the wallet target reduces
fragmentation; retained balances can still serve smaller calls. See
[reserve sizing](docs/configuration.md#reserve-sizing-and-fragmentation).

## Packaged stack

[Private Hermes](https://github.com/mikeperry-tor/private-hermes-agent) packages
Hermes Agent, Tor-routed egress, confidential inference, a code executor and this
project with Docker Compose. It ships its own copy of `research_desk.toml` and its
own version pins. See the [README deployment choices](README.md#choose-a-deployment)
for its requirements and support boundary.

## Further references

- [Wallet administration](docs/wallet-cli.md): bootstrap, funding failures, refunds, backup restoration and RPC configuration.
- [Wallet rotation](docs/wallet-rotation.md): admission, reserve claims, retained capacity and recovery.
- [Configuration](docs/configuration.md): TOML composition, listeners, source distribution, relay and direct swap control.
- [Provider caveats](providers/CAVEATS.md): current provider support boundaries.
- [Live integration runbook](tests/live/INTEGRATION.md): bounded paid test runs.
