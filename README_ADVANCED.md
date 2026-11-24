# Advanced deployments

This guide walks through the multi-listener deployments. Start with the
[README quickstart](README.md#quickstart-z-cash-wallet-with-dynamically-managed-tools)
to learn the single-listener model, or jump straight into the
[research desk](examples/deployments/research_desk.toml) for a multi-domain
setup.

| Example | Listeners | Wallet groups | Network | Treasury state |
| --- | --- | --- | --- | --- |
| [research_desk.toml](examples/deployments/research_desk.toml) | 12 | `general`, `personal`, `health`, `financial` | Tor | `state/research_desk` |
| [comprehensive.toml](examples/deployments/comprehensive.toml) | 4 | `web`, `company` | Tor | `state/privacy` |

The research desk is the full multi-domain setup. The comprehensive example is a
smaller four-listener deployment that uses the same bootstrap, relay, and
shared-payer model.

## Prerequisites

Run commands from the repository root with GNU Make 3.81+ and a POSIX shell.
`make build` uses the selected Rust toolchain and supplies `protoc`. See the
[build commands](scripts/README.md#common-entrypoints) for toolchain selection
and [portability](docs/development.md#portability) for platform support.

Zcash support is always compiled in; static-wallet deployments simply omit
`[treasury]`. Only synthetic proving and local consensus tests use opt-in Cargo
features. Cargo downloads and Zcash proving parameters fetch directly rather
than through Tor.

Both advanced examples require a running Tor SOCKS listener, because
`x402_treazury` does not bundle or start Tor. Start Tor Browser (which listens
on `127.0.0.1:9150`) or run a dedicated Tor daemon with
`SocksPort 127.0.0.1:9050 IsolateSOCKSAuth` and update `network.socks_endpoint`
in your local deployment file.

`socks_endpoint` accepts an IP and port, a hostname and port such as
`tor:9050`, or a Unix socket path (`unix:/absolute/path`). Keep Tor running for
the full session. Because `x402_treazury` isolates circuits with SOCKS
credentials, its traffic does not share circuits with Tor Browser tabs.

## Research desk walkthrough

### 1. Build, copy, and start Tor

```sh
make build
cp examples/deployments/research_desk.toml examples/deployments/research_desk.local.toml
```

Start Tor Browser or your Tor daemon at the configured SOCKS endpoint. The
research desk already embeds Tor and relay settings; pass `--network-config`
only when you want to override `[network]` (and `[relay_fallback]` when present)
for one command:

```sh
target/release/x402_treazury serve \
  --config examples/deployments/research_desk.local.toml \
  --network-config examples/network/tor.toml
```

### 2. Create the treasury

```sh
export X402_TREAZURY_CONFIG="examples/deployments/research_desk.local.toml"
target/release/x402_treazury wallet init
```

`wallet init` creates `state/research_desk`, generates a seed, discovers the
wallet birthday over Tor, and prints the shielded ZEC receive address. By
default, the decryption key is stored at `state/research_desk/wallet.key` with
no password or OS-keychain wrapper, so anyone who can read that directory has
both the wallet state and its decryption key.

### 3. Start the server

```sh
export X402_TREAZURY_MCP_TOKEN="replace_with_a_secret_bearer_token"
target/release/x402_treazury serve
```

`serve` binds all listeners first, then syncs the treasury, loads catalogs, and
funds USDC wallets in the background. The research desk funds `general` first;
`personal`, `health`, and `financial` begin funding only after a validated API
call hits one of their endpoints (including a free call or cache hit). API calls
never wait for swaps: until a deferred group receives its first credit, its
endpoints pay from `general`. Once a group has been credited, it uses its own
wallet permanently across restarts even if its balance later runs low.

Run only one `serve` process per treasury directory. Read-only wallet commands
(`wallet status`, `wallet addresses`, `wallet notes`, and `wallet preview`)
work while `serve` is running; exclusive commands (such as `wallet sync`,
`wallet backup`, `wallet rename`, and `wallet bootstrap`) require stopping
`serve` first.

### 4. Fund the treasury and connect

```sh
export X402_TREAZURY_CONFIG="examples/deployments/research_desk.local.toml"
target/release/x402_treazury wallet status
```

Send ZEC to the shielded receive address. Initial setup needs one $2-3 nominal
allocation for `general` plus network and bridge fees; funding all four groups
eventually needs $8-12 nominal before fees, relays, and replacements. Start with
about $5 of ZEC for a light trial, or around $20 for sustained use across all
groups.

| Group | Endpoints | Payment cap |
| --- | --- | --- |
| `general` | `toolbox`, `web`, `science`, `shopping`, `world` | $1.00 |
| `personal` | `social`, `property`, `travel` | $1.00 |
| `health` | `health` | $0.50 |
| `financial` | `business`, `finance`, `security` | $1.00 |

See [examples/README.md](examples/README.md) for ports and source selections.
Connect each MCP client with
`Authorization: Bearer <your X402_TREAZURY_MCP_TOKEN>`.

The research desk uses this bootstrap table:

```toml
[wallet_bootstrap]
bootstrap_group = "general"
deferred_groups = ["personal", "health", "financial"]
```

Paid catalog and directory relay fees stay on `general` even after deferred
groups switch to their own wallets, and relay fees never trigger deferred-group
funding. Relay fallback covers catalogs and help documents by default; API
request relay remains off unless `api_requests = true`. When `[wallet_bootstrap]`
is set, `general` overrides `relay_fallback.wallet` for metadata and discovery.

To fund all four groups separately from the start, comment out the entire
`[wallet_bootstrap]` table. Removing a listener alone does not stop its declared
wallet group from being funded.

> **Privacy note:** Separate addresses prevent direct payer reuse, though
> closely spaced initial swaps can still correlate by funding time. First-use
> activation and randomized funding timers weaken that signal without
> guaranteeing unlinkability. Calls made during bootstrap share `general`'s
> wallet, and timing, amounts, providers, and request contents can still link
> activity.

### 5. Stop

Press Ctrl-C once and wait for the announced transaction-safety drain to finish.
Do not start another `serve` process while work is draining.

## Comprehensive example walkthrough

The [comprehensive deployment](examples/deployments/comprehensive.toml) exposes
four MCP endpoints backed by two wallet groups:

| Endpoint | Port | Group and cap | Scope |
| --- | --- | --- | --- |
| `web` | 6337 | `web`, $0.10 | Exa, Google Trends, and web sources |
| `social` | 6338 | `company` (starts on `web`: $0.10 until first `company` credit, then $0.50) | People and social platform research |
| `company` | 6339 | `company` (starts on `web`: $0.10 until first `company` credit, then $0.50) | Company, public-record, security, and phone intelligence |
| `toolbox` | 6340 | `company` (starts on `web`: $0.10 until first `company` credit, then $0.50) | API discovery, raw URL calls, and persistent agent-added tools |

All four endpoints start on `web` with a $0.10 per-payment cap. After `company`
receives its first credit, `social`, `company`, and `toolbox` switch permanently
to `company`'s wallet and $0.50 cap while `web` keeps its own wallet.

```sh
make build
cp examples/deployments/comprehensive.toml examples/deployments/comprehensive.local.toml
export X402_TREAZURY_CONFIG="examples/deployments/comprehensive.local.toml"
target/release/x402_treazury wallet init
export X402_TREAZURY_MCP_TOKEN="replace_with_a_secret_bearer_token"
target/release/x402_treazury serve
```

Initial setup funds one $2-3 nominal small wallet for `web`. Funding both
`web` and `company` takes $4-6 nominal total before fees, and a single group's
first wallet plus its first replacement also spans $4-6 nominal (around $5 on
average; allow up to $6 for randomization and fees). If a bridge minimum
exceeds the $3 ceiling, `x402_treazury` refuses the route rather than silently
raising the ceiling.

Because `[wallet_bootstrap]` sets `bootstrap_group = "web"` and
`deferred_groups = ["company"]`, you can optionally run `wallet bootstrap` after
funding the treasury (with `serve` stopped) to pre-fund `web` without starting
listeners or loading catalogs. `wallet bootstrap` leaves `company` deferred.

The `company` endpoint includes RelayShield and NetIntel security tools whose
prices ($0.05-$0.40) can exceed `web`'s initial $0.10 cap until `company` is
funded; large company or grant batches can also exceed the $0.50 cap. See
[provider caveats](providers/CAVEATS.md) for current provider limits.

## Expensive APIs and large reserves

`max_api_payment_usdc` caps one payment challenge, not a multi-call workflow.
Raise it on a wallet group to allow larger individual calls; the live x402
challenge determines the actual charge. For APIs costing more than $0.50 per
call, add a large reserve alongside the small reserve:

```toml
# Add to a local deployment file.
[wallets.company.reserves.large]
# Default randomized nominal funding: $10-25.
```

Adding a large reserve is better than enlarging the small reserve because cheap
calls continue rotating on small wallets while expensive calls draw from the
large wallet. Size the large reserve to at least 2x your largest expected
payment, and preferably 4x when prices vary (keeping payments under 25% of the
wallet target reduces unusable remainders, though leftover balances can still
cover smaller calls). For example, $10 is a sensible floor for a $2.50 maximum
payment. See
[reserve sizing](docs/configuration.md#reserve-sizing-and-fragmentation).

## Adding API providers

The [provider catalog](providers/README.md) contains reusable TOML definitions,
including curated catalogs for APIs that need schema adaptation. Providers may
carry reliability tags (including Tor-related tags) that trigger startup
warnings; tags record past observations rather than live status.

A compatible OpenAPI provider can start with just its specification URL:

```toml
[sources.my_api]
spec = "https://api.example.com/openapi.json"
```

Add `my_api` to a listener's `sources` list. Some providers also need
`base_url`, tool filters, schema overrides, or payment compatibility flags.
Extend a bundled definition with `extends = "../../providers/exa.toml"` (paths
resolve relative to the deployment file), and add `help_url` for a single usage
guide. Run `catalog tags` and `catalog tools` to pick a focused subset rather
than exposing a huge catalog.

For large catalogs, set `discover_on_demand = true` under `[servers.NAME]` to
advertise `x402_treazury_tools_search` and `x402_treazury_tool_call` in place of
individual provider API schemas. See [on-demand discovery](docs/agent-sources.md).

Provider management lets agents search bundled providers and
[operator-selected remote directories](docs/agent-sources.md#external-directory-matching),
then register APIs on their endpoint. It is off by default. Enable local management,
remote management, and raw URL calls independently on any endpoint that names a
`wallet`, and set `source_registry_file` if registrations should survive
restarts. Agent tools cannot modify configured sources, wallet settings, or
spending limits. See
[configuration and permissions](docs/agent-sources.md#configuration-and-authority)
and the [agent tool table](docs/agent-sources.md#agent-tools).

## Packaged stack

[Private Hermes](https://github.com/mikeperry-tor/private-hermes-agent) packages
Hermes Agent, Tor-routed egress, confidential inference, a code executor, and
`x402_treazury` for Docker Engine 28+ or rootless Podman 5.4+ on Linux with
Compose 2.36+. It builds from a pinned `x402_treazury` revision and renders
upstream `examples/deployments/research_desk.toml` into local operator config,
adjusting only `socks_endpoint`, `state_dir`, and `allowed_hosts`. See
[README deployment choices](README.md#choose-a-deployment) for requirements and
issue boundaries.

## Further references

- [Wallet administration](docs/wallet-cli.md): bootstrap, funding failures,
  refunds, backups, and RPC settings.
- [Wallet rotation](docs/wallet-rotation.md): payment selection, funding,
  rotation, and recovery.
- [Configuration](docs/configuration.md): TOML composition, listeners, source
  distribution, relay fallback, and direct swap control.
- [Provider caveats](providers/CAVEATS.md): current provider support boundaries.
- [Live integration runbook](tests/live/INTEGRATION.md): bounded paid test runs.
