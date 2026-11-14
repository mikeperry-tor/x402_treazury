# Command-line guide

`x402_treazury` has a small set of top-level commands. Most users only need
`wallet init`, `serve`, and `wallet status`; the rest inspect configuration,
manage the treasury, or recover interrupted financial work.

Run the executable with no command to print help. Run
`x402_treazury COMMAND --help` for the options of a specific command.

## Command overview

| Command | What it does |
| --- | --- |
| `serve --config FILE` | Start all configured MCP listeners and background funding |
| `serve --provider FILE` | Serve one provider, using stdio by default |
| `catalog tools` | Print selected tool inventories as JSON |
| `catalog tags` | Print the tags available in a provider or deployment |
| `catalog warm` | Download and cache selected catalogs |
| `catalog route` | Preview an HTTP request without sending it; standalone providers only |
| `config show` | Resolve configuration and wallet bindings offline |
| `config check` | Load catalogs and validate selections without signing or funding |
| `wallet init` | Create a new treasury or import an existing mnemonic |
| `wallet status` | Show saved treasury and managed-wallet state |
| `wallet addresses` / `wallet address` | List saved receive addresses or derive a new one |
| `wallet sync` | Run one ZEC and managed-USDC observation pass |
| `wallet notes` | Inspect retained Zcash notes as JSON |
| `wallet backup` | Copy the treasury, keys, and pending transaction records to a new directory |
| `wallet rename` | Rename a saved managed wallet group without changing its identity |
| `wallet preview` | Compare configuration with saved wallet groups |
| `wallet bootstrap` | Fund captured managed reserve claims without listeners |
| `wallet recover` | Assess and run one recovery pass for existing funding work |
| `wallet reconcile` | Reconcile one saved transaction |
| `sources inspect` | Read saved agent-added API source records |
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

`wallet init` prints a shielded ZEC receive address. Send ZEC there before paid use.
`serve` binds the listener before background funding starts, so tools are visible
while a wallet is being prepared. Connect your MCP client to the configured address
with `Authorization: Bearer <your token>`.

Stop with a single Ctrl-C and allow the announced drain to finish. Do not start a
second serving process for the same treasury directory.

## Selecting configuration

`X402_TREAZURY_CONFIG` supplies the default deployment path for every `--config`
command. An explicit `--config FILE` overrides it. Unset the variable before using
standalone `--state-dir`, `--key-file`, or `--treasury-id` options.

`--network-config FILE` replaces the deployment's network policy, and its relay
section when present, for that invocation. `--env-file FILE` loads environment
values after CLI parsing.

Use `--provider FILE` for standalone provider serving and inspection. Standalone
provider options cannot be combined with `--config`.

## Managing the treasury

### Create or import

```sh
x402_treazury wallet init --config deployment.toml
```

Initialization creates the configured state directory, generates a seed, and prints
the primary shielded receive address. To import an existing mnemonic, pass
`--birthday HEIGHT --mnemonic-file FILE`. The birthday must be at or before the
wallet's first use so older funds are found.

The default key file is `wallet.key` inside the state directory. Anyone with the
complete directory can decrypt the wallet; owner-only permissions are not a
password or OS keychain.

### Inspect

```sh
x402_treazury wallet status --config deployment.toml
x402_treazury wallet addresses --config deployment.toml
```

`wallet status` reads saved observations without network traffic or exclusive
ownership, so it is safe while `serve` is running. It shows the shielded receive
address, confirmed and pending shielded balances, spendable balance, managed wallet
groups, and any funding problems. Saved balances are observations, not a promise of
current spending capacity.

`wallet addresses` returns saved receive addresses as JSON. `wallet address`
derives a new address and requires exclusive ownership.

### Sync

```sh
# Stop serve first.
x402_treazury wallet sync --config deployment.toml
```

`wallet sync` performs one ZEC scan and one pass over configured managed-USDC
wallets plus outstanding funding recipients. It reports unresolved work without
waiting for another observation interval. Rerun it or resume serving to continue.
A failed ZEC pass does not suppress the Base pass.

Ordinary serving already performs background observation. Use explicit sync when
you need to refresh saved state, especially after depositing while the server was
idle.

### Notes

```sh
x402_treazury wallet notes --config deployment.toml
x402_treazury wallet notes --config deployment.toml --txid TRANSACTION_ID
```

`wallet notes` returns JSON describing saved shielded notes, including spent
history. It can run while serving owns the wallet and never changes state. Absence
from the retained snapshot does not prove that no payment arrived.

### Back up and restore

```sh
# Stop serve first.
x402_treazury wallet backup --config deployment.toml --destination secrets/backup
```

Backup copies the SQLite database, encryption key, and pending transaction records into a new
owner-only directory. It refuses to overwrite an existing destination. Protect the
whole directory as spending material.

To restore, stop the process and point `state_dir` and `key_file` at the backup.
Use the same treasury ID and never run the original and restored copies at the same
time. A mnemonic alone cannot restore EVM keys, pending payment authorizations, or
pending transaction records.

### Rename a managed group

```sh
x402_treazury wallet backup --config deployment.toml --destination secrets/backup
x402_treazury wallet rename research general --config deployment.toml
# Update wallet declarations and wallet_bootstrap references.
```

`wallet rename` changes only the saved group's name. Its internal identity, keys,
balances, accepted work, and pending obligations stay with the same group. Renaming a TOML
group without this command creates a new group instead.

### Compare and bootstrap

```sh
x402_treazury wallet preview --config deployment.toml
x402_treazury wallet bootstrap --config deployment.toml
```

`wallet preview` compares validated wallet assignments with saved public state
without changing anything. Use it before restarting after a configuration change.

`wallet bootstrap` funds the reserve allocations it records without starting
listeners or catalogs. It requires `funding.auto_fund = true`. With `[wallet_bootstrap]`, it
selects only the configured bootstrap group; without that table, it captures every
declared managed group.

## Inspecting and serving APIs

```sh
x402_treazury catalog tools --config deployment.toml
x402_treazury catalog tags --config deployment.toml
x402_treazury catalog warm --config deployment.toml --source exa
x402_treazury config show --config deployment.toml
x402_treazury config check --config deployment.toml
```

`catalog tools` prints the selected tools and authored prices. `catalog tags` helps
choose tag filters. `catalog warm` downloads and caches catalogs; add `--direct` to
fetch without the configured Tor policy for that dedicated warming process.

`config show` resolves file settings and wallet bindings offline. `config check`
loads catalogs and validates selections without signing, probing prices, or
funding. Add `--explain-routing` to see how source tools were distributed to
listeners.

For a single provider:

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

Stop serving before refreshing or removing saved agent-added sources. `sources
inspect` shows saved source records and selected tools, plus any loading error.
Removal revokes API access and preserves financial records.

## Recovery

Recovery commands are for interrupted funding or submission work. They do not
create replacement transactions without explicit authority and saved bytes.

```sh
# Stop serve first.
x402_treazury wallet recover --config deployment.toml
x402_treazury wallet reconcile --config deployment.toml --operation-id UUID
x402_treazury wallet reconcile --config deployment.toml --operation-id UUID --rebroadcast
```

`wallet recover` reports work as `recovered`, `waiting`, `operator_required`, or
`check_failed`. `wallet reconcile` checks one saved transaction; `--rebroadcast`
explicitly permits resubmitting the same saved bytes within their signed deadline.
Additional low-level commands exist for advanced use and are not part of ordinary
help.

## Logs and exit status

All logs go to stderr; stdout carries the MCP protocol or requested JSON. A failed
command prints a concise error and cause chain and returns a nonzero exit status.
Failed `serve` startup or runtime failures log ERROR; other failed CLI work logs a
terminal WARN. Detailed progress and diagnostics use the configured log levels.

For exact command options and recovery rules, see the
[wallet CLI reference](../wallet-cli.md). For configuration fields, see the
[configuration guide](configuration.md) and
[configuration reference](../configuration.md).
