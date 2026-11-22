# Command-line guide

`x402_treazury` has a small set of subcommands. Most operators only need
`wallet init`, `serve`, and `wallet status`; the remaining commands inspect
configuration, administer the treasury, or recover interrupted financial work.

Run `x402_treazury` with no arguments for usage help, or
`x402_treazury COMMAND --help` for a subcommand's options.

## Command overview

| Command | What it does |
| --- | --- |
| `serve --config FILE` | Start configured MCP listeners and background funding |
| `serve --provider FILE` | Serve one standalone provider (stdio by default) |
| `catalog tools` | Print resolved tool inventories as JSON |
| `catalog tags` | Print available OpenAPI tags for a provider or deployment |
| `catalog warm` | Download and cache catalogs |
| `catalog route` | Preview an HTTP request without sending it (standalone providers only) |
| `config show` | Resolve configuration and wallet bindings offline |
| `config check` | Load catalogs and validate selections without signing or funding |
| `wallet init` | Create a new treasury or import a mnemonic |
| `wallet status` | Show treasury and managed-wallet state |
| `wallet addresses` and `wallet address` | List receive addresses or derive a new one |
| `wallet sync` | Run one ZEC and Base USDC observation pass |
| `wallet notes` | Inspect Zcash notes as JSON |
| `wallet backup` | Copy treasury state, key file, and transaction journals to a new directory |
| `wallet rename` | Rename a managed wallet group without changing its identity |
| `wallet preview` | Compare deployment wallet assignments against stored state |
| `wallet bootstrap` | Fund enabled managed reserves without starting listeners |
| `wallet recover` | Assess and run one recovery pass for pending funding work |
| `wallet reconcile` | Reconcile one transaction by operation ID |
| `sources inspect`, `refresh`, and `remove` | Inspect, refresh, or remove persisted agent-added API sources |
| `build-info` | Print build identity and provenance |

## First run

```sh
make build
cp examples/deployments/managed.toml examples/deployments/managed.local.toml

export X402_TREAZURY_CONFIG="examples/deployments/managed.local.toml"
export X402_TREAZURY_MCP_TOKEN="replace_with_a_secret_bearer_token"

target/release/x402_treazury wallet init
target/release/x402_treazury serve
```

`wallet init` prints a shielded ZEC receive address; send ZEC there before
making paid calls. `serve` binds listeners before starting background funding,
so tools remain visible while a wallet prepares. Connect your MCP client with
`Authorization: Bearer <your token>`.

Stop `serve` with a single Ctrl-C and wait for the announced drain to finish.
Never start a second `serve` process against the same treasury directory.

## Selecting configuration

`X402_TREAZURY_CONFIG` sets the default deployment path for `--config`
commands, and `--config FILE` overrides it. Unset `X402_TREAZURY_CONFIG` before
using `--state-dir`, `--key-file`, or `--treasury-id`.

`--network-config FILE` replaces the deployment's `[network]` table (and
`[relay_fallback]` when present) for one command. `--env-file FILE` loads
environment variables after CLI parsing for `serve`, `config check`, and
catalog inspection commands (`tools`, `tags`, and `route`).

Use `--provider FILE` for standalone provider serving and inspection; it cannot
be combined with `--config`.

## Managing the treasury

### Create or import

```sh
x402_treazury wallet init --config deployment.toml
```

`wallet init` creates the treasury state directory, generates a seed, and
prints the primary shielded receive address. To import a mnemonic, pass
`--birthday HEIGHT --mnemonic-file FILE` with a birthday height at or before
the wallet's first use so earlier transactions are scanned.

By default, the decryption key lives at `wallet.key` inside the state
directory. Owner-only file permissions are not a password or OS keychain;
anyone who can read the directory can decrypt the wallet.

### Inspect

```sh
x402_treazury wallet status --config deployment.toml
x402_treazury wallet addresses --config deployment.toml
```

`wallet status` reads local state without network calls or an exclusive lock,
so it works while `serve` is running. Human-readable output opens the wallet
key to report the shielded receive address, ZEC balances (confirmed, pending,
and spendable), managed wallet groups, and any funding errors. `wallet status
--json` reads SQLite metadata only without opening the key; use human output or
`wallet addresses` to verify encrypted snapshot readability. Displayed balances
reflect the latest recorded sync rather than a live RPC query.

`wallet addresses` prints known receive addresses as JSON. `wallet address`
derives a new receive address and requires stopping `serve` first.

### Sync

```sh
# Stop serve first.
x402_treazury wallet sync --config deployment.toml
```

`wallet sync` runs one ZEC scan and one Base USDC check across managed wallets
and active funding recipients, then reports any unresolved items without
waiting for another interval. A ZEC sync error does not block the Base check.
Run `wallet sync` again or resume `serve` to continue observing.

`serve` already runs background observation; use `wallet sync` when you want an
immediate refresh, such as after depositing ZEC while `serve` was stopped.

### Notes

```sh
x402_treazury wallet notes --config deployment.toml
x402_treazury wallet notes --config deployment.toml --txid TRANSACTION_ID
```

`wallet notes` prints JSON describing recorded shielded notes, including spent
history. It is read-only and works while `serve` is running. Because it reads
the last synced snapshot, run `wallet sync` (or wait for `serve` to sync) to
see newly arrived notes.

### Back up and restore

```sh
# Stop serve first.
x402_treazury wallet backup --config deployment.toml --destination secrets/backup
```

`wallet backup` copies the SQLite database, encryption key, and transaction
journals into a new owner-only directory and refuses to overwrite an existing
path. Protect the backup directory as spending material.

To restore, stop `serve`, point `state_dir` at the backup directory, set
`key_file` to the `key` file inside it, and keep the same treasury ID. Never run
the original state directory and a restored copy at the same time. A mnemonic
alone restores Zcash funds, not EVM wallet keys, pending payment authorizations,
or swap journals.

### Rename a managed group

```sh
x402_treazury wallet backup --config deployment.toml --destination secrets/backup
x402_treazury wallet rename research general --config deployment.toml
# Then update wallet declarations and wallet_bootstrap references in TOML.
```

`wallet rename` changes only the group's label on disk; its internal identity,
keys, balances, and pending operations stay attached to the same group.
Renaming a group in TOML without `wallet rename` creates a new empty group.

### Compare and bootstrap

```sh
x402_treazury wallet preview --config deployment.toml
x402_treazury wallet bootstrap --config deployment.toml
```

`wallet preview` compares your deployment's wallet assignments against public
wallet metadata without modifying state or unlocking keys, and works while
`serve` is running. Use it to check config edits before restarting.

`wallet bootstrap` funds enabled reserves without starting listeners or loading
catalogs. It requires `funding.auto_fund = true` and stopping `serve` first.
With `[wallet_bootstrap]`, it funds only the bootstrap group; without it, it
funds every declared managed group.

## Inspecting and serving APIs

```sh
x402_treazury catalog tools --config deployment.toml
x402_treazury catalog tags --config deployment.toml
x402_treazury catalog warm --config deployment.toml --source exa
x402_treazury config show --config deployment.toml
x402_treazury config check --config deployment.toml
```

- `catalog tools`: prints resolved tools and advertised prices.
- `catalog tags`: lists available OpenAPI tags to help write tag filters.
- `catalog warm`: downloads and caches catalogs. Pass `--direct` to fetch
  without Tor for that warming run. Warming never starts listeners or funding
  workers, though relay fallback may make paid calls if configured.
- `config show`: resolves TOML settings and wallet bindings offline.
- `config check`: loads catalogs and validates filters without signing,
  probing prices, or funding. Pass `--explain-routing` to see how tools route
  to listeners.

`catalog warm` prints one JSON summary with `outcome: "ok"` (all selected
sources ready) or `outcome: "degraded"` (some sources, filters, pricing, or
cache writes are incomplete). Both exit `0` when attempts complete, so inspect
`outcome` and each source's `status` rather than relying on exit status alone.
Invalid config, state errors, cancellation, and internal failures exit nonzero.

Partial compatible catalogs can be cached with omissions, whereas
`catalog tools` and `config check` stay strict. Framework wrappers should
follow the [framework integration contract](../framework-integration.md).

Standalone single-provider commands:

```sh
x402_treazury serve --provider providers/exa.toml
x402_treazury catalog tools --provider providers/exa.toml
```

## Agent-added sources

```sh
x402_treazury sources inspect --config deployment.toml
x402_treazury sources refresh --server research --source SOURCE --config deployment.toml
x402_treazury sources remove --server research --source SOURCE --config deployment.toml
```

Stop `serve` before refreshing or removing persisted agent-added sources.
`sources inspect` lists stored source records, their tools, and any load error.
Removing a source revokes API access while keeping financial records intact.

## Recovery

`wallet recover` inspects interrupted funding records and resets or closes
eligible entries so `wallet bootstrap` or `serve` can continue funding. Recovery
commands never create replacement transactions; only
`wallet reconcile --rebroadcast` can resubmit already-signed bytes before
expiry.

```sh
# Stop serve first.
x402_treazury wallet recover --config deployment.toml
x402_treazury wallet reconcile --config deployment.toml --operation-id UUID
x402_treazury wallet reconcile --config deployment.toml --operation-id UUID --rebroadcast
```

`wallet recover` classifies each item as `recovered`, `waiting`,
`operator_required`, or `check_failed`. `wallet reconcile` checks one
operation by ID; `--rebroadcast` resubmits the same signed transaction bytes
if they are still within their expiry window. Additional low-level recovery
subcommands are documented in the [wallet CLI reference](../wallet-cli.md).

## Logs and exit status

All logs go to stderr; stdout carries the MCP protocol or requested JSON.
Failed commands print a concise error cause chain and exit nonzero. Failed
`serve` startup or runtime errors log `ERROR`; other failed CLI commands log a
terminal `WARN`.

For exact options and recovery rules, see the
[wallet CLI reference](../wallet-cli.md). For configuration fields, see the
[configuration guide](configuration.md) and
[configuration reference](../configuration.md).
