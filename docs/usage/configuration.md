# Configuration guide

This guide explains the deployment TOML in ordinary operator terms. It covers the
settings most people need to change and the choices that affect cost, privacy, and
which tools an agent can see. The complete field-by-field reference remains in the
[configuration reference](../configuration.md); use it when you need exact
composition rules or uncommon provider settings.

Start from an [example deployment](../../examples/deployments/) rather than an empty
file. Copy it to a local name, edit the copy, and keep the original as a reference.

## How configuration is selected

One deployment file describes sources, listeners, wallets, funding, and network
policy. Use the same file for serving, wallet commands, catalog inspection, and
configuration checks.

| Method | Use it for |
| --- | --- |
| `X402_TREAZURY_CONFIG=path.toml` | Default file for every `--config` command in the current shell |
| `--config FILE` | One command with a specific deployment file |
| `--provider FILE` | Standalone provider serving or catalog inspection without a deployment |
| `--network-config FILE` | Replace the deployment's `[network]` and optional `[relay_fallback]` for one command |
| `--env-file FILE` | Load environment values for serving and catalog/check commands |

`--config` and standalone provider options cannot be combined. Unset
`X402_TREAZURY_CONFIG` before using `--state-dir`, `--key-file`, or `--treasury-id`.
Relative paths resolve from the file that declares them.

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

The listener is authenticated by default. The environment variable named by
`bearer_token_env` must contain a nonempty token before startup. To allow
unauthenticated access deliberately, set `auth = false`, omit `bearer_token_env`,
and keep Host checking enabled.

| Listener setting | What it controls |
| --- | --- |
| `listen` | Address and port for this MCP endpoint |
| `allowed_hosts` | Exact hostnames or IPs accepted in the HTTP Host header |
| `bearer_token_env` | Environment variable holding the bearer token |
| `auth` | Whether the endpoint requires authentication (default `true`) |
| `wallet` | Default wallet group for this listener's sources |
| `sources` | Source names exposed on this listener |
| `discover_on_demand` | Advertise search/call wrappers instead of individual API schemas |
| `wait_for_catalogs` | Wait for this endpoint's configured catalogs before serving (default `true`) |
| `instructions_text` | Endpoint guidance shown to the agent |

Host checking defaults to `localhost`, `127.0.0.1`, and `::1`. Add the hostname
clients actually use when serving behind a reverse proxy or from a container. Do
not combine `allowed_hosts = []` with disabled authentication.

## Choosing APIs

A source points at a reusable provider file or a public OpenAPI specification.

```toml
[sources.my_api]
spec = "https://api.example.com/openapi.json"

[sources.company]
extends = "../../providers/otto.toml"
include_tools = ["otto_equity_intel", "otto_financial_statements"]
```

Filters select which generated tools reach the agent. Use them to keep tool lists
focused and context small.

| Setting | Effect |
| --- | --- |
| `include_tools` / `exclude_tools` | Select generated tool names with `*` and `?` patterns |
| `tags` / `exclude_tags` | Select operations by OpenAPI tag |
| `recommended_tools` | Advisory subset defined by the provider |
| `use_recommended_tools = true` | Make that advisory subset active |
| `include_operations` / `exclude_operations` | Select exact `METHOD path` operations |

Name filters run after tag and path selection, so removing one tool does not rename
its siblings. Exclusions win over inclusions. An allowlist that removes every tool
leaves no tools from that source; ordinary serving reports reduced coverage, while
`catalog tools` and `config check` reject the empty selection.

The separate `send_tools` setting distributes one source's tools to named listeners
instead of filtering them. See [source tool distribution](../configuration.md#source-tool-distribution).

When the catalog is large, enable on-demand discovery instead of advertising every
schema:

```toml
[servers.research]
discover_on_demand = true
```

Agents then use `x402_treazury_tools_search` to find a tool and
`x402_treazury_tool_call` to invoke it. Filtering, wallet routing, and payment
limits still apply.

### Keeping tools through catalog outages

By default, an endpoint that waits for a required remote catalog also waits for
that source. If the source cannot be loaded, the endpoint stays unavailable until
you restart it. For sources that may be temporarily unreachable, let the endpoint
start with the remaining tools:

```toml
[sources.flaky]
spec = "https://api.example.com/openapi.json"
allow_catalog_failure = true
```

The failed source is omitted as a unit, and the endpoint reports it in its MCP
instructions. Tools from healthy sources remain usable. With
`wait_for_catalogs = false`, failed remote sources are omitted regardless of this
setting. Local configuration and parsing errors remain fatal.

Ordinary serving also keeps endpoints available when authored selections produce
no tools, such as a missing tag or an empty filtered inventory. It reports reduced
coverage instead of widening the selection. `catalog tools`, `config check`, and
qualification remain strict and report the incomplete inventory. Framework authors
should follow the [framework integration contract](../framework-integration.md)
when treating catalog preparation and endpoint readiness as separate results.

### Letting the agent add providers

Provider management is off by default. Enable the capabilities you want on a
listener that already names a wallet:

```toml
[servers.research]
provider_management_local = true
provider_management_remote = true
remote_provider_directories = ["coinbase", "x402-list"]
raw_url_calls = true
source_registry_file = "../../state/agent-sources.sqlite"
```

Local management searches bundled provider definitions offline. Remote management
searches public directories and registers public OpenAPI specifications. Raw URL
calls invoke documented HTTPS endpoints without registration. Omit
`source_registry_file` if registrations should disappear when the process exits.

Agent-added APIs use the listener's wallet and normal payment caps. They cannot
change configured sources, wallet settings, or spending limits, and they are
visible only on the listener that added them.

## Choosing wallet groups

A wallet group is a payment identity. APIs in the same group share a payer until
rotation; separate groups reduce address reuse across different kinds of research.

Set the per-payment cap on each managed group:

```toml
[wallets.research]
mode = "zcash_rotation"
max_api_payment_usdc = "0.50" # Per payment, not a total budget.
```

`max_api_payment_usdc` authorizes one payment. It is independent of how much a
wallet holds. Use `"none"` or `"off"` only when you intentionally want no amount
cap; the Base USDC restriction still applies.

### Small and large reserves

A managed group funds a small reserve by default:

```toml
[wallets.research.reserves.small]
# Nominal random target between $2 and $3.
```

For occasional expensive calls, opt into a large reserve:

```toml
[wallets.research.reserves.large]
# Nominal random target between $10 and $25.
```

To choose exact amounts, use a fixed target or an inclusive range:

```toml
[wallets.research.reserves.small]
funding_amount_usdc = "2.50"

[wallets.research.reserves.large]
min_funding_amount_usdc = "10"
max_funding_amount_usdc = "15"
```

Aim for large reserves at least twice the largest expected payment, and preferably
four times when prices vary. A payment should stay below 25% of the wallet target
to reduce unusable remainders. A larger wallet settles fewer calls before rotating:
a $10 wallet covers two $4 calls and leaves $2, and fragmented balances cannot be
combined to authorize one payment. Keeping a separate small reserve lets cheap
calls rotate while the large reserve covers occasional expensive calls. These
settings do not guarantee unlinkability. Reserves authorize nominal USDC sizing
only; actual spendable ZEC and standard network and bridge fees govern funding.

The first wallet and its first replacement in a small-only group draw $4–6 nominal,
averaging about $5 before bridge minimums and fees. That is a capital estimate across
two allocations, not a promise of $5 spendable at once. Funding starts after
listeners bind, so tools remain visible while a wallet is being prepared.

### Reusing one policy across many groups

Templates avoid repeating wallet settings and an assignment policy decides where
separate groups are created:

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

Each generated group gets its own wallets and payment identity. A template never
allocates anything by itself. Explicit source or listener wallet names take
priority over automatic assignment.

## Sharing a payer during first use

Bootstrap policy lets several listeners share one wallet group until each has
served real API traffic:

```toml
[wallet_bootstrap]
bootstrap_group = "general"
deferred_groups = ["personal", "health", "financial"]
```

A validated API call records demand for its deferred group. That group then gets
its own funding opportunity on a randomized 30–120-second timer. Until its first
positive credit, its calls use the bootstrap group's wallet and cap.

This reduces the number of initial wallets you must fund. It does not make the
groups unlinkable: bootstrap calls share a visible payer, and timing, amounts, and
request contents can still correlate activity. Removing the table funds all
declared groups normally.

## Tor and relay fallback

Direct networking is the default. To send all outbound application traffic through
Tor, add:

```toml
[network]
mode = "tor"
socks_endpoint = "127.0.0.1:9150"
isolation_namespace = "x402_treazury"
socks_auth = "tor_extended"
```

Start Tor Browser or a Tor daemon at that endpoint first. Tor mode uses remote DNS,
keeps TLS validation, and never falls back to direct networking. `--network-config`
can supply this policy for one command without editing the deployment.

Optional paid relay fallback helps when a provider or directory blocks Tor:

```toml
[relay_fallback]
source_discovery = true
provider_metadata = true
api_requests = false

[relay_fallback.relays]
GET = ["../../providers/readfetch.toml"]
POST = ["../../providers/readfetch.toml"]
```

`source_discovery` covers built-in directory searches, `provider_metadata` covers
catalogs and help, and `api_requests` covers actual API data. API relay fallback is
off by default because the relay sees the request and response. Each relay attempt
can be charged.

## Direct cryptocurrency tools

Agents can optionally receive ZEC or request foreign payments, and send or swap
funds from the treasury. See
[Enabling Direct Cryptocurrency Transactions](../../README.md#enabling-direct-cryptocurrency-transactions)
for the listener settings and tool list.

## Output and download limits

Provider and source tables accept byte limits for downloads:

| Setting | Default | Applies to |
| --- | --- | --- |
| `max_response_bytes` | 16 MiB | API response bodies |
| `max_help_bytes` | 4 MiB | Help and usage documents |
| `max_spec_bytes` | 32 MiB | Remotely fetched OpenAPI specifications |

Oversized downloads are rejected as a whole and reported to the agent. Completed
tool output is otherwise returned in full. Providers may omit selected verbose JSON
fields; each omission is disclosed in the result.

## Validate before serving

Inspect configuration without starting listeners, fetching secrets, or spending:

```sh
x402_treazury config show --config deployment.toml
x402_treazury catalog tools --config deployment.toml
x402_treazury config check --config deployment.toml
```

The [command-line guide](cli.md#inspecting-and-serving-apis) explains what each
command validates and when to use `--explain-routing`.

## Common problems

- **Startup fails for a missing token.** The environment variable named by
  `bearer_token_env` must exist and contain a nonempty value.
- **A listener is unreachable behind a proxy.** Add the public hostname to
  `allowed_hosts`; the default only accepts local hosts.
- **A wallet name change starts from zero.** Renaming a TOML group creates a new
  group. Use `wallet rename` to keep the existing identity, then update references.
- **An unknown field fails parsing.** The schema rejects obsolete and misspelled
  fields instead of ignoring them. Remove the field or use the current name.
- **A large catalog floods the agent.** Add filters or enable
  `discover_on_demand = true`.
- **A paid catalog fetch needs a wallet.** With bootstrap policy, metadata and
  directory relay fees use the bootstrap group.

For exact field behavior, composition rules, provider compatibility, and recovery
semantics, continue with the [configuration reference](../configuration.md) and
[wallet CLI reference](../wallet-cli.md).
