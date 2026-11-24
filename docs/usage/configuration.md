# Configuration guide

This guide explains the deployment TOML in plain operator terms, covering the
settings most operators change and the choices that affect cost, privacy, and
tool visibility. See the [configuration reference](../configuration.md) for the
complete field-by-field reference, composition rules, and uncommon provider
settings.

Start from an [example deployment](../../examples/deployments/) rather than an
empty file. Copy it to a local filename (such as `*.local.toml`), edit the
copy, and keep the tracked template as a reference.

## How configuration is selected

One deployment file defines sources, listeners, wallets, funding, and network
policy. Use the same file for `serve`, `wallet`, `catalog`, `config`, and
`sources` commands.

| Method | Use it for |
| --- | --- |
| `X402_TREAZURY_CONFIG=path.toml` | Default deployment file for `--config` commands in the current shell |
| `--config FILE` | Override the deployment file for one command |
| `--provider FILE` | Serve or inspect one standalone provider without a deployment |
| `--network-config FILE` | Replace `[network]` (and `[relay_fallback]` when present) for one command |
| `--env-file FILE` | Load environment variables for `serve`, `config check`, and catalog inspection commands (`tools`, `tags`, and `route`) |

`--config` cannot be combined with `--provider`. Unset `X402_TREAZURY_CONFIG`
before passing `--state-dir`, `--key-file`, or `--treasury-id`. Relative paths
resolve from the file that declares them.

## A minimal listener

```toml
version = 1

[treasury]
state_dir = "../../state/managed"

[funding]
confidentiality = "public"

[wallets.research]
mode = "zcash_rotation"
max_api_payment_usdc = "0.50"

[wallets.research.reserves.small]

[sources.exa]
extends = "../../providers/exa.toml"

[servers.research]
listen = "127.0.0.1:8000"
bearer_token_env = "X402_TREAZURY_MCP_TOKEN"
wallet = "research"
sources = ["exa"]
```

Listeners require bearer authentication by default, and the environment
variable named by `bearer_token_env` must be set to a nonempty token before
startup. To allow unauthenticated access, set `auth = false`, omit
`bearer_token_env`, and keep Host header checking enabled.

| Listener setting | What it controls |
| --- | --- |
| `listen` | Bind address and port for this MCP endpoint |
| `allowed_hosts` | Exact hostnames or IPs accepted in the HTTP `Host` header |
| `bearer_token_env` | Environment variable holding the bearer token |
| `auth` | Whether bearer authentication is required (default `true`) |
| `wallet` | Default wallet group for this listener's sources |
| `sources` | Source names exposed on this listener |
| `discover_on_demand` | Advertise search and call wrappers in place of individual provider API schemas |
| `wait_for_catalogs` | Wait for this endpoint's catalogs to settle before serving (default `true`) |
| `instructions_text` | Custom guidance included in the endpoint's MCP instructions |

Host header checking defaults to `localhost`, `127.0.0.1`, and `::1`. Add the
hostname clients use when serving behind a reverse proxy or inside a container.
Never combine `allowed_hosts = []` with `auth = false`.

## Choosing APIs

A source points at a bundled provider file or a public OpenAPI specification:

```toml
[sources.my_api]
spec = "https://api.example.com/openapi.json"

[sources.company]
extends = "../../providers/otto.toml"
include_tools = ["otto_equity_intel", "otto_financial_statements"]
```

Filters control which generated tools reach the agent, keeping tool lists
focused and context usage small:

| Setting | Effect |
| --- | --- |
| `include_tools` and `exclude_tools` | Match generated tool names with `*` and `?` patterns |
| `tags` and `exclude_tags` | Match operations by OpenAPI tag |
| `recommended_tools` | Advisory subset suggested by the provider |
| `use_recommended_tools = true` | Activate the provider's advisory subset |
| `include_operations` and `exclude_operations` | Match exact `METHOD path` operations |

Name filters run after tag and path selection, so excluding one tool does not
rename its siblings. Exclusions take precedence over inclusions. If filters
remove every tool from a source, `serve` reports reduced coverage while
`catalog tools` and `config check` fail on the empty selection.

To route one source's tools across multiple listeners instead of filtering them
out, use `send_tools`. See
[source tool distribution](../configuration.md#source-tool-distribution).

Enable on-demand discovery to show provider tool schemas when agents search
for them. Enabled built-in tools appear in the initial list.

```toml
[servers.research]
discover_on_demand = true
```

Agents then call `x402_treazury_tools_search` to inspect matching schemas and
`x402_treazury_tool_call` to invoke a tool. Filtering, wallet routing, and
payment caps still apply.

### Keeping tools through catalog outages

By default, an endpoint waiting on a required remote catalog stays unavailable
until you restart `serve` if that source fails to load. To let the endpoint
start with the remaining healthy sources when a remote source is flaky, set:

```toml
[sources.flaky]
spec = "https://api.example.com/openapi.json"
allow_catalog_failure = true
```

When a source fails, it is omitted as a whole and listed in the endpoint's MCP
instructions, while tools from healthy sources remain usable. If an endpoint
sets `wait_for_catalogs = false`, failed remote sources are omitted regardless
of `allow_catalog_failure`. Local configuration and parsing errors still fail
startup.

`serve` also keeps endpoints up when filters produce no tools (for example, a
missing tag or empty filtered list), reporting reduced coverage without
widening the filter. `catalog tools` and `config check` remain strict and fail
on incomplete inventories. Framework authors should follow the
[framework integration contract](../framework-integration.md).

### Letting the agent add providers

Provider management is off by default. Enable the flags you want on a listener
that already names a `wallet`:

```toml
[servers.research]
provider_management_local = true
provider_management_remote = true
remote_provider_directories = ["coinbase", "x402-list"]
raw_url_calls = true
source_registry_file = "../../state/agent-sources.sqlite"
```

- `provider_management_local`: searches and adds bundled provider definitions
  offline.
- `provider_management_remote`: searches public directories and registers
  public OpenAPI specifications.
- `raw_url_calls`: calls documented HTTPS URLs directly without registering a
  catalog.
- `source_registry_file`: persists agent-added sources across restarts (omit it
  for ephemeral registrations).

Agent-added APIs use the listener's wallet and payment caps, are visible only
on the listener that registered them, and cannot modify configured sources,
wallet settings, or spending limits.

## Choosing wallet groups

A wallet group is a payment identity. APIs in the same group share a payer
address until rotation; separate groups limit address reuse across unrelated
workloads.

Set a per-payment cap on each managed group:

```toml
[wallets.research]
mode = "zcash_rotation"
max_api_payment_usdc = "0.50" # Caps one payment, not total spend.
```

`max_api_payment_usdc` limits a single payment challenge independently of the
wallet's balance. Set `"none"` or `"off"` only if you want no per-payment cap
(payments still require Base USDC).

### Small and large reserves

Managed groups enable a small reserve by default:

```toml
[wallets.research.reserves.small]
# Default randomized nominal target: $2-$3.
```

Add a large reserve for occasional expensive calls:

```toml
[wallets.research.reserves.large]
# Default randomized nominal target: $10-$25.
```

You can set a fixed target or an inclusive range:

```toml
[wallets.research.reserves.small]
funding_amount_usdc = "2.50"

[wallets.research.reserves.large]
min_funding_amount_usdc = "10"
max_funding_amount_usdc = "15"
```

Size a large reserve to at least 2x your largest expected payment, and
preferably 4x when prices vary. Keeping each payment below 25% of the wallet
target reduces unusable remainders: for example, a $10 wallet covers two $4
calls and leaves $2, and balances across separate wallets cannot be combined
for one payment. Keeping a separate small reserve lets cheap calls continue
rotating while the large reserve handles expensive calls. Reserve targets set
nominal USDC sizing only; spendable ZEC and network and bridge fees govern
transfers.

In a small-only group, the first wallet plus its first replacement draw $4-6
nominal total (about $5 on average before bridge minimums and fees). Funding
runs in the background after listeners bind, so tools stay visible while a
wallet prepares.

### Reusing one policy across many groups

Wallet templates avoid repeating settings, and `[wallet_assignment]` controls
how separate groups are generated:

```toml
[wallet_templates.research]
mode = "zcash_rotation"
max_api_payment_usdc = "2.50"

[wallet_templates.research.reserves.small]

[wallet_templates.research.reserves.large]

[wallet_assignment]
scope = "server" # deployment, server, source, or binding
template = "research"
```

Each generated group gets its own wallets and payment identity. A template
allocates nothing on its own, and explicit `wallet` names on a source or
listener take precedence over automatic assignment.

## Sharing a payer during first use

A bootstrap policy lets multiple listeners share one wallet group until each
deferred group handles API traffic:

```toml
[wallet_bootstrap]
bootstrap_group = "general"
deferred_groups = ["personal", "health", "financial"]
```

The first validated API call on a deferred group's endpoint records demand for
that group, which then receives a funding slot on a randomized 30-120 second
timer. Until that group receives its first credit, its calls use the bootstrap
group's wallet and cap.

Bootstrap lowers upfront funding costs, though calls made during bootstrap
share a visible payer and timing, amounts, and request contents can still link
activity. Remove `[wallet_bootstrap]` to fund all declared groups from the
start.

## Tor and relay fallback

Direct networking is the default. To route all outbound application traffic
through Tor, add:

```toml
[network]
mode = "tor"
socks_endpoint = "127.0.0.1:9150"
isolation_namespace = "x402_treazury"
socks_auth = "tor_extended"
```

Start Tor Browser or a Tor daemon at `socks_endpoint` first. Tor mode uses
remote DNS, validates TLS certificates, and never falls back to direct
connections. You can also supply this block for a single command with
`--network-config`.

Optional paid relay fallback recovers when a provider or directory blocks Tor:

```toml
[relay_fallback]
source_discovery = true
provider_metadata = true
api_requests = false

[relay_fallback.relays]
GET = ["../../providers/readfetch.toml"]
POST = ["../../providers/readfetch.toml"]
```

- `source_discovery`: fallback for built-in directory searches.
- `provider_metadata`: fallback for catalog and help document fetches.
- `api_requests`: fallback for API calls (off by default because the relay sees
  the request and response).

Each relay attempt may incur an x402 charge.

## Direct cryptocurrency tools

Agents can optionally receive ZEC or foreign assets and send or swap funds from
the treasury. See
[Enabling Direct Cryptocurrency Transactions](../../README.md#enabling-direct-cryptocurrency-transactions)
for the listener settings and tool list.

## Output and download limits

Provider and source tables accept per-document byte limits:

| Setting | Default | Applies to |
| --- | --- | --- |
| `max_response_bytes` | 16 MiB | API response bodies |
| `max_help_bytes` | 4 MiB | Help and usage documents |
| `max_spec_bytes` | 32 MiB | Remotely fetched OpenAPI specifications |

Oversized downloads are rejected whole and reported to the agent. Completed
tool responses are returned in full; if a provider definition omits verbose
JSON fields, the result discloses each omission.

## Validate before serving

Check a deployment without starting listeners, reading wallet keys, or spending
funds:

```sh
x402_treazury config show --config deployment.toml
x402_treazury catalog tools --config deployment.toml
x402_treazury config check --config deployment.toml
```

See the [command-line guide](cli.md#inspecting-and-serving-apis) for what each
command checks and when to pass `--explain-routing`.

## Common problems

- **Startup fails with a missing token:** Export a nonempty value for the
  environment variable named by `bearer_token_env`.
- **A listener is unreachable behind a proxy:** Add the client-facing hostname
  to `allowed_hosts`; the default allows only local hosts.
- **Renaming a wallet group in TOML starts a fresh wallet:** Renaming in TOML
  creates a new group. Run `wallet rename` to keep the same identity and funds,
  then update your TOML references.
- **An unknown field fails parsing:** The schema rejects misspelled and
  obsolete fields rather than ignoring them. Remove or rename the field.
- **A large catalog floods the agent:** Add tool filters or set
  `discover_on_demand = true`.
- **A paid catalog fetch needs a wallet:** When `[wallet_bootstrap]` is set,
  metadata and directory relay fees use the bootstrap group.

For field-level rules, composition semantics, provider compatibility, and
recovery behavior, see the [configuration reference](../configuration.md) and
[wallet CLI reference](../wallet-cli.md).
