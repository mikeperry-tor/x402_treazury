# Wallet CLI and managed configuration

For an operator walkthrough, see the [command-line guide](usage/cli.md) and
[managed quickstart](../README.md#quickstart-z-cash-wallet-with-dynamically-managed-tools).
For pool invariants, payment selection, observation schedules, and recovery
rules, see [wallet rotation](wallet-rotation.md).

## One configuration for wallet commands

`X402_TREAZURY_CONFIG` sets the default deployment file for `wallet` commands,
and `--config FILE` overrides it. Unset `X402_TREAZURY_CONFIG` before passing
standalone `--state-dir`, `--key-file`, or `--treasury-id`. Wallet commands do
not load API catalogs or provider definitions.

```sh
x402_treazury wallet init --config examples/deployments/managed.local.toml
x402_treazury wallet addresses --config examples/deployments/managed.local.toml
x402_treazury wallet status --config examples/deployments/managed.local.toml
x402_treazury wallet notes --config examples/deployments/managed.local.toml
x402_treazury wallet backup --config examples/deployments/managed.local.toml --destination secrets/backup
x402_treazury wallet sync --config examples/deployments/managed.local.toml
```

- **Read-only commands (safe while `serve` is running):** `wallet status`,
  `wallet addresses`, `wallet notes`, and `wallet preview`.
- **Exclusive commands (stop `serve` first):** `wallet init`, `wallet address`,
  `wallet sync`, `wallet backup`, `wallet rename`, `wallet bootstrap`,
  `wallet recover`, `wallet reconcile`, and `wallet recover-expired`.
- **Network override:** Combine `--config` or standalone options with
  `--network-config FILE` to replace `[network]` for that command (any
  `[relay_fallback]` table in that file is ignored by wallet commands).

## Preview configuration against saved groups

```sh
x402_treazury wallet preview --config deployment.toml
x402_treazury wallet preview --config deployment.toml --json
```

`wallet preview` compares your deployment's validated wallet assignments
against stored public group metadata without modifying state, opening keys, or
loading catalogs, and works while `serve` is running. It reports matched,
added, and disabled groups, nominal initial-inventory bounds, and
`[wallet_bootstrap]` setup estimates.

## Rename a managed wallet group

Managed group names map to stable internal UUIDs; renaming a wallet in TOML
without `wallet rename` creates a new empty group and disables the old one.
Stop `serve`, back up, rename, update TOML, and verify:

```sh
x402_treazury wallet backup --config deployment.toml --destination secrets/backup
x402_treazury wallet rename research general --config deployment.toml
# Update [wallets], source and listener references, and [wallet_bootstrap] in TOML.
x402_treazury wallet preview --config deployment.toml
x402_treazury serve --config deployment.toml
```

- `wallet rename OLD NEW [--config FILE | --state-dir DIR] [--json]` changes
  only the group's lookup name under exclusive ownership, preserving its UUID,
  keys, balances, first-use demand, and pending operations.
- Disabled groups and groups with pending work can be renamed, including
  migrating to or from generated `auto_v1_` names (declaring `auto_v1_` names
  as explicit TOML profiles remains rejected). Renaming onto an occupied name
  is rejected.
- Rename allocates and funds nothing; updated policy applies on the next
  `serve` restart.

## Initialization, addresses, and status

### `wallet init`

`wallet init` creates a new owner-only treasury state directory, generates a
seed inside Zingolib, queries the mainnet tip (rewinding 100 blocks for the
birthday), writes the encryption key, and prints the primary shielded ZEC
receive address. It refuses to overwrite an existing treasury.

```sh
# Standalone creation (STATE_DIR must not exist; KEY_FILE parent must exist):
x402_treazury wallet init --state-dir /private/state/new-treasury \
  --key-file /private/keys/new-treasury.key

# Import a mnemonic offline (use a birthday at or before the wallet's first use):
x402_treazury wallet init --state-dir /private/state/treasury \
  --key-file /private/keys/treasury.key --birthday 2000000 \
  --mnemonic-file /private/import/mnemonic.txt
```

- **Key file:** Defaults to `wallet.key` inside `state_dir`. Owner-only file
  permissions (and suppressed inherited ACLs on macOS) are not a password or OS
  keychain; anyone who can read the state directory has both state and key. Set
  `treasury.key_file` to store the key outside `state_dir`.
- **Treasury ID:** Generated automatically and stored in state; pass
  `--treasury-id UUID` only to assert an expected existing treasury.
- **Address format:** New addresses are Unified Addresses containing only the
  Orchard-format receiver used by Ironwood (no transparent or Sapling
  receivers).

### `wallet addresses` and `wallet address`

- `wallet addresses`: reads the encrypted snapshot offline and prints known
  shielded receive addresses as JSON without deriving new addresses or changing
  sync state. Works while `serve` is running.
- `wallet address`: derives a fresh shielded receive address and commits the
  updated snapshot before returning it. Requires stopping `serve` first.

### `wallet status`

`wallet status` prints a human-readable summary by default (`--json` emits
versioned JSON) without network calls or an exclusive lock:

- Shows the primary shielded receive address, confirmed, pending, and spendable
  Ironwood ZEC balances (from the last synced snapshot), managed wallet groups
  with non-retired wallet balances, funding jobs, and `treasury_operations`.
- Displayed balances reflect the latest committed sync rather than a live RPC
  check. Human-readable `wallet status` opens the wallet key to authenticate
  the encrypted snapshot for current ZEC balances (reporting explicitly if
  unreadable rather than printing zeros). `wallet status --json` reads SQLite
  metadata only without opening `key_file` or decrypting the snapshot; use
  human output or `wallet addresses` when you want to verify encrypted snapshot
  readability.

## Syncing the treasury

```sh
# Stop serve first.
x402_treazury wallet sync --config deployment.toml
x402_treazury wallet sync --config deployment.toml --json
```

`wallet sync` runs one ZEC scan and one managed Base USDC pass (including
active funding recipients and one eligible-retirement check) without starting
background watchers or allocating funds. It requires `[treasury]` (and needs
`[funding]` when managed USDC wallets or active funding recipients are present)
and honors `--network-config FILE`.

- ZEC and Base results report independently: a ZEC failure does not prevent the
  Base pass, and invalid retirement config on one group does not block scalar
  observations on healthy groups.
- If a check fails or an item stays pending, rerun `wallet sync` or resume
  `serve`.

## Offline note inspection

```sh
x402_treazury wallet notes --config FILE
x402_treazury wallet notes --config FILE --txid TRANSACTION_ID
```

`wallet notes` always prints JSON, authenticating one committed schema-23,
format-45 snapshot without acquiring an exclusive lock or making network calls
(safe while `serve` is running).

- Reports Ironwood notes only (including historical spent notes) alongside
  spendability limitations.
- `--txid` accepts a canonical 64-hex-digit transaction ID to filter notes.
- Because `wallet notes` reads the last synced snapshot, run `wallet sync` (or
  wait for `serve`) to observe newly arrived notes.

## Bootstrap before discovery

```sh
x402_treazury wallet bootstrap --config examples/deployments/managed.local.toml
x402_treazury wallet bootstrap --config examples/deployments/managed.local.toml --json
```

`wallet bootstrap` funds enabled managed reserves without starting MCP
listeners or loading catalogs. Requires `funding.auto_fund = true` and stopping
`serve` first.

- **Scope:** When `[wallet_bootstrap]` is configured, funds only
  `bootstrap_group`'s enabled reserves without activating deferred groups.
  Without `[wallet_bootstrap]`, funds every declared managed group.
- **Behavior:** Allocates promptly without proactive early-rotation swaps.
  Prints each group's small and large reserve readiness on success (`--json`
  emits `schema_version: 3`), exiting nonzero if obstructed or canceled while
  preserving persisted work.
- **Interrupts:** One `SIGINT` or `SIGTERM` drains accepted work; a second
  signal exits immediately with unfinished obligations preserved for recovery.

## Treasury and managed pools

Zcash support is always compiled into the binary; static-wallet deployments
simply omit `[treasury]`. Build and test with the pinned Makefile entrypoints:

```sh
make build
make test CARGO_ARGS='--all-targets'
make test CARGO_ARGS='--features zcash-testutils --all-targets'
```

`zcash-testutils` adds synthetic proving tests (verifying production preparation
and proof restoration against a localhost stub without broadcasting). The
separate [regtest suite](../tests/REGTEST.md) runs under `zcash-regtest`
against Docker Zebra and Zaino containers; production constructors stay
mainnet-only even in all-feature builds.

### Indexer endpoints and sync cadence

Zcash indexer endpoints default to `https://zec.rocks:443` and can be
overridden under `[treasury]` (see
[treasury and funding keys](configuration.md#funding-capital-and-automatic-funding)).
Endpoints require HTTPS (cleartext HTTP is allowed only for loopback test
fixtures), validate TLS, and route through `src/network.rs`.

- Each sync cycle runs inside an isolated Tokio runtime so cancellation tears
  down helper tasks cleanly before saving the final snapshot.
- Treasury tip-check failures report sanitized gRPC codes with `starting_tip`
  or `final_tip`; scan failures report elapsed time and target, scanned, and
  observed heights without printing addresses or upstream prose.
- Serving runs ZEC scans at startup, on demand, and every 60 seconds after
  completion while enabled funding, pending source or refund work, or active
  direct receives require scanning (backing off at 90, 180, and 300 seconds on
  errors; idle treasuries do not poll). See
  [wallet observation policy](wallet-rotation.md#wallet-observation-policy).

### Backup and restore

```sh
# Stop serve first.
x402_treazury wallet backup --config deployment.toml --destination secrets/backup
```

- `wallet backup` copies the SQLite database, 32-byte ChaCha20-Poly1305 key
  file (saved as `key` in the backup directory), and journals into a new
  owner-only directory, fsyncs both database and key before atomically
  publishing `backup.json`, and refuses to overwrite an existing path.
- **Restore:** Stop `serve`, point `state_dir` at the backup directory and
  `key_file` at its `key` file with the same treasury ID, and never run the
  original and restored copies at the same time.
- **Mnemonic limits:** A Zcash seed recovers shielded ZEC via viewing keys, not
  EVM keys, pending payment authorizations, or swap journals. Protect the full
  backup as spending material, and never roll back to a pre-upgrade backup
  after new financial work has occurred.

### Database schema and upgrade compatibility

The application accepts **database schema 23** and **Zingolib v8 wallet format
45** exclusively for both owner opens and read-only inspection. Older and
newer versions fail before mutating state without converting files.

- **Stopped-state preflight:** Clean stopped databases are read in place.
  Stopped databases with nonempty SQLite journals are verified in a temporary
  private subdirectory copy that is removed afterward without altering original
  files.
- **Live read-only inspection:** Uses a coherent read transaction so committed
  WAL data stays visible while `serve` holds the exclusive lock (SQLite may
  create or update WAL and SHM reader files without altering financial
  journals).
- **Interrupted schema-23 initialization:** Empty source-disposition and
  payment-resolution tables can finish idempotent initialization under
  exclusive ownership without rewriting existing records. Stop serving and take
  a backup before upgrading binaries.

### Recovery commands

Stop `serve` before running recovery commands. Recovery never creates
replacement transactions without established source closure and enabled funding
authority; `wallet recover` itself never quotes, prepares, or broadcasts funds.

```sh
# Assess and run one recovery pass (prints JSON: recovered, waiting, operator_required, check_failed):
x402_treazury wallet recover --config deployment.toml

# Reconcile one source transaction by operation ID (read-only lookup and sync; prints JSON):
x402_treazury wallet reconcile --config deployment.toml --operation-id UUID

# Resubmit the EXACT saved signed bytes before expiry (never creates a new tx):
x402_treazury wallet reconcile --config deployment.toml --operation-id UUID --rebroadcast

# Low-level expiry closure for a single operation (never submits or rebroadcasts; prints JSON):
x402_treazury wallet recover-expired --config deployment.toml --operation-id UUID
```

- **`wallet recover`:** Uses the same classifier as serving and bootstrap.
  Resets proven-unprepared jobs, closes verified expired or NEAR-settled source
  work, preserves unresolved `PENDING_DEPOSIT` requests and ambiguous
  submissions, and reports damaged metadata or unrecoverable records as
  `operator_required`. Run `wallet bootstrap` or `serve` afterward to continue
  funding.
- **`wallet reconcile`:** Derives txid and signed expiry from authenticated
  original bytes (even if plaintext projections are stale). Confirms via SDK
  scan at configured depth; `--rebroadcast` resubmits only the identical saved
  bytes within expiry.
- **`wallet recover-expired`:** Requires one completed canonical tip scan
  beyond signed finite expiry plus configured confirmation depth.
- **`wallet shield-refunds`:** Unsupported under the
  [Ironwood-only policy](zingo-integration.md#ironwood-only-policy) and fails
  before calculation or network I/O.
- **`wallet pool`:** Removed. Declare managed reserves in TOML and run
  `wallet bootstrap --config FILE` or `serve`.

### Base RPC verification

Base verification defaults to **PublicNode -> dRPC -> Base**:

| Operator | HTTPS endpoint | Documentation |
| --- | --- | --- |
| PublicNode | `https://base-rpc.publicnode.com` | [Base gateway](https://base.publicnode.com/) |
| dRPC | `https://base.drpc.org` | [Base API](https://drpc.org/docs/base-api) |
| Base | `https://mainnet.base.org` | [Network details](https://docs.base.org/get-started/connect-to-base) |

Configure custom endpoints with `funding.base_rpc_url_env` (default
`BASE_RPC_URL`; falls back to PublicNode when unset, errors if explicitly named
and missing or empty) and `funding.base_rpc_fallback_url_envs` (omit for dRPC
and Base defaults; set `[]` to disable fallbacks; or name up to two custom
environment variables):

```toml
[funding]
base_rpc_url_env = "BASE_RPC_URL"
base_rpc_fallback_url_envs = ["BASE_RPC_SECONDARY", "BASE_RPC_TERTIARY"]
```

- **Failover contract:** Availability failover trusts one configured provider
  per view (not a quorum). Each view starts at the primary and visits at most
  three providers once, restarting the **entire read-only chain view** on
  failover and discarding partial evidence. Direct mode allows 2 same-client
  attempts (200 ms backoff); Tor mode uses `1 + tor_isolation_retries` scopes
  per endpoint before failing over.
- **Canonical finality:** Canonical balances, authorization nonces, and receipt
  proofs require the provider's `finalized` block tag (no depth-only
  downgrade). Scalar `balanceOf(..., latest)` reads stay advisory. RPC errors
  `-32005`, `-32016`, `-32601` (and `-32602` on `eth_getBlockByNumber`), HTTP
  `403`, `408`, `429`, `500`, `502`, `503`, and `504`, transport errors, TLS
  errors, and malformed or oversized replies permit failover; wrong chain or
  conflicting canonical history rejects the view.
- **Tor note:** `mainnet.base.org` is rate-limited and has returned HTTP `403`
  and `429` over Tor; prefer an explicit endpoint such as PublicNode
  (`export BASE_RPC_URL=https://base-rpc.publicnode.com`) when running over Tor.
  Remember that RPC providers see queried public EVM addresses (and shared
  provider API keys can link wallets).
- **Read-only diagnostic:** Exercise concurrent Base verification without keys
  or state changes (discloses your state's public EVM addresses to the chosen
  RPC; `--rpc-url` disables default fallbacks unless `--fallback-rpc-url` is
  also passed):

```sh
make run CARGO_ARGS='--example diagnose_credit -- --network-config examples/network/tor.toml --state-dir state/demo --rpc-url "$$BASE_RPC_URL" --rounds 3'
```

- **NEAR dry quote demo:** Validate a live public route (default $1; pass
  `--amount-usdc` for another amount) without credentials, wallet state, or
  funds (Cargo's `--offline` skips crate downloads while the example calls
  NEAR):

```sh
make run CARGO_ARGS='--offline --example quote_near'
```

- **Funding and payment policy:** For reserve sizing, `EXACT_OUTPUT` preview
  and `EXACT_INPUT` quote execution, `funding.confidentiality` (`basic`
  default, `public` opt-in, `advanced`), `slippage_bps`, `poll_seconds`,
  `swap_timeout_seconds`, `quote_deadline_seconds`, retirement tolerances, and
  insufficient-funds wallet traversal, see [configuration](configuration.md)
  and [wallet rotation](wallet-rotation.md).

## Versioned capacity output

Output schema versions are independent of database schema 23, SDK wallet format
45, encrypted-record and AAD version 1, and backup manifest format 1:

| Command or output | JSON `schema_version` | Key fields |
| --- | --- | --- |
| `wallet status --json` | `6` | `state_schema_version: 23`, per-pool `reserve_claims`, `payment_limit_atomic`, `retirement_tolerance_atomic`, `retirement_accounting`, `funding_jobs` (`provider_observed_at`, `selected_output`, `accepted_output`), `treasury_operations`, `sync_complete` |
| `wallet bootstrap --json` | `3` | `serving_ready_groups`, `inventory_ready_groups`, `captured_completed_groups`, `incomplete_captured_jobs` |
| Qualification lifecycle observations | `2` | Independent qualification evidence schema |

Key JSON field semantics:

- **Reserve claims:** `target_atomic` is the lower nominal sizing bound and
  `maximum_output_atomic` is the nominal ceiling including bridge-minimum
  headroom. `selected_output` is the operation's drawn nominal USDC amount;
  `accepted_output` is the executable quote's expected output.
- **Balances and caps:** `balance_estimate_atomic` is advisory (`null` when
  unknown, never `0`). A `null` payment limit means unconfigured; an unlimited
  cap serializes as the maximum U256 atomic value. `runtime_balance_evidence`
  is an optional cached historical certificate.
- **`provider_observed_at`:** Local database timestamp of the last validated
  NEAR status response (not provider `updatedAt`; `null` when unavailable).
- **`retirement_accounting`:** Sums only exact confirmed balances recorded at
  automatic retirement, separating automatic counts, manual counts, and legacy
  unknowns (`null` rather than invented zeros).
- **Bootstrap tracking:** `wallet bootstrap` tracks its initial slot
  generations (following an automatic replacement of an uncredited completed
  generation once) and stops following a generation as soon as it receives
  credit.
