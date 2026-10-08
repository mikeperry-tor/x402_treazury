# Wallet CLI and managed configuration

Start with the [README quickstart](../README.md#quickstart-zcash-and-tor).
This reference covers treasury commands, funding controls and operator recovery.
For lifecycle invariants, see [wallet rotation](wallet-rotation.md).
Commands run from the repository root; `x402_treazury` means the built executable on PATH.

## One configuration for wallet commands

Use the same deployment file for initialization, inspection, backup, sync and
serving. `--meta-config` remains an alias for `--config`. Wallet commands do not fetch provider catalogs or require listener tokens.
`wallet bootstrap` also consumes wallet assignments and funding settings to fund
initial managed pairs; other administration commands do not start funding workers.

```sh
x402_treazury wallet init --config examples/deployments/privacy.local.toml
x402_treazury wallet addresses --config examples/deployments/privacy.local.toml
x402_treazury wallet status --config examples/deployments/privacy.local.toml
x402_treazury wallet backup --config examples/deployments/privacy.local.toml --destination secrets/backup
x402_treazury wallet sync --config examples/deployments/privacy.local.toml
```

Only `treasury.state_dir` is required in the treasury table. The optional
`daily_treasury_spend_limit_zec` bounds native treasury withdrawals;
`max_funding_transaction_fee_zec` and `max_refund_shielding_fee_zec` each default to
`"0.0003"` and reject proposals above that fee. See
[funding limits](wallet-rotation.md#funding-limits-and-fees).
The default key is `wallet.key` inside the wallet directory;
set `key_file` for separate storage. Possession of the complete default directory
is sufficient to decrypt the wallet. Owner-only permissions are not a password or
OS keychain. Initialization creates missing state parent directories, refuses an
existing state/key destination, and never silently imports or replaces a wallet.

Omit `treasury.id` to read the persisted UUID at runtime. An explicit ID asserts an
expected wallet and must match; omit it for new-wallet initialization. Inspection
reports `treasury_identity = "from_wallet_state_at_runtime"` without reading state.
The durable identity, exclusive ownership and authenticated store checks remain.
Live qualification manifests retain their explicit treasury binding.

Endpoints default to `https://zec.rocks:443`; submission follows the selected
indexer unless overridden. Set `indexer_url` / `submission_url` for explicit URLs,
or `indexer_url_env` / `submission_url_env` for required environment references.
A URL and environment reference for the same role are mutually exclusive. Missing
or empty explicitly referenced variables are errors, never a fallback. Prefer
environment references for credential-bearing URLs. Config-driven initialization
uses this same endpoint policy. Standalone initialization retains its optional
`ZCASH_INDEXER_URL` fallback for existing scripts.

Standalone `--state-dir` remains available; `--key-file` defaults inside that
directory and `--treasury-id` is an optional assertion. Do not combine those
location overrides or `--network-config` with `--config`: the deployment owns its
paths and network policy. `--birthday` and import options remain command-specific.

## Bootstrap before discovery

After depositing sufficient ZEC into the treasury, run:

```sh
x402_treazury wallet bootstrap --config examples/deployments/privacy.local.toml
```

This allocates all declared managed pools, runs the production treasury/funding
workers, waits until each initial active/standby pair has confirmed USDC credit,
and closes treasury ownership before exiting. It requires `funding.auto_fund=true`
and respects existing source, fee, daily and attempt limits. Static wallets are
excluded and need external funding. No catalogs, pricing, listener tokens, static
signing keys or MCP listeners are needed. Network traffic follows the deployment
policy, including Tor.

The JSON result lists `bootstrapped_wallets`: durable completion of initial pairs,
not a fresh balance or payment-admission guarantee. Repeating the command skips
completed pairs and resumes incomplete jobs, including eligible recovery. Signed
transactions are replaced only after the existing canonical expiry and unspent-input
proof succeeds. Unknown preparation outcomes and conflicting evidence still require
operator review; cancellation drains
accepted financial work and preserves journals. Treasury ZEC funding and Base
confirmation can take time. The command introduces no new spending limits.
While waiting, it logs unfinished jobs and their sync/submission state once a
minute. A completed swap's retained timeout does not block bootstrap: confirmed
USDC credit satisfies that job, while pending source accounting retains its
reservations and continues reconciling during serving.

Ordinary managed `serve` performs the same bootstrap before catalog/pricing
discovery when `auto_fund=true`, then starts normal serving and replacement funding.
This allows a new deployment to pay for discovery fallback. It waits for all initial
pairs, even if discovery would succeed without payment. `auto_fund=false` skips this
startup step and makes explicit bootstrap fail; qualification funding restrictions
retain their existing lifecycle. `catalog warm` never starts funding: use this
command first when warming needs a managed paid relay wallet.

## Treasury and managed pools

Build or test the embedded wallet with the shell wrapper, which supplies a
Cargo-managed protoc compiler. First builds may download public proving
parameters. The wallet dependency is pinned upstream at zingolib v6.0.0 commit
`c6381534f802b1022041beda4b01c106ad132329`; no reference checkout is needed.

```sh
scripts/zcash.sh build
scripts/zcash.sh test --all-targets
```

The binary supports these treasury commands:

```sh
# STATE_DIR must not exist; KEY_FILE must be new, with an existing parent directory.
# Generate a new seed; query the mainnet tip automatically.
x402_treazury wallet init --state-dir /private/state/new-treasury \
  --key-file /private/keys/new-treasury.key

# Supply the correct birthday for your mnemonic when importing.
x402_treazury wallet init --state-dir /private/state/treasury \
  --key-file /private/keys/treasury.key --birthday 2000000 \
  --mnemonic-file /private/import/mnemonic.txt

# Without --mnemonic-file, init generates a new seed inside zingolib.
# Status reads persisted metadata and needs neither key nor network access.
x402_treazury wallet status --state-dir /private/state/treasury

# This allocates two keys and queued jobs;
# it does not contact NEAR, transfer ZEC, or fund either EVM address.
x402_treazury wallet pool --state-dir /private/state/treasury \
  --key-file /private/keys/treasury.key \
  --name research --funding-amount-usdc 5.00

# Display existing receive addresses without derivation or network access.
# The treasury UUID is read from state; --treasury-id UUID is an optional check.
x402_treazury wallet addresses --state-dir /private/state/treasury \
  --key-file /private/keys/treasury.key

# Derive another shielded receive address and save its snapshot before returning it.
x402_treazury wallet address --state-dir /private/state/treasury \
  --key-file /private/keys/treasury.key
```

Use `target/debug/x402_treazury` or put the built binary on
PATH. Mnemonic files must be owner-only and are never passed as seed arguments.
`wallet addresses` reads the encrypted snapshot without changing address indices,
snapshot revision, or sync readiness. `wallet address` creates a new address.
For either command, an optional `--treasury-id` must be the UUID shown by init/status,
not an account number such as `0`.
Expected CLI failures print a concise error and cause chain to stderr and return
a nonzero exit code, even when `RUST_BACKTRACE` is enabled.
Initialization refuses existing state/key paths; a partial initialization failure
is not automatically retried over those files. Wallet data stays in memory until
written as an authenticated encrypted snapshot. The wallet's internal path uses a
private temporary directory, separate from the durable database. Upstream plaintext
save tasks are never started. These commands never broadcast or fund anything.

Standalone new-wallet initialization queries `ZCASH_INDEXER_URL` when set, otherwise
`https://zec.rocks:443`, and records the mainnet tip minus 100 blocks as its
birthday. `--indexer-url-env NAME` selects another environment variable. Lookup
must succeed before any seed, key or state is created; it checks the network and
tip consistency and has a 30-second deadline. `--birthday HEIGHT` skips lookup
and keeps initialization offline. Importing with `--mnemonic-file` requires an
explicit birthday at or before the wallet's first use, so older funds are scanned.

Address JSON separates `receiver_capabilities` (`orchard_protocol`, `sapling`,
`transparent`) from balances. `orchard_protocol_pools` lists `orchard` and
`ironwood` when that receiver is present: both pools use the same receiver and
keys. It does not report where funds reside. After sync,
`state.sync.confirmed_pool_balances_zatoshis` reports `ironwood`, `orchard`, and
`sapling` individually, alongside the aggregate shielded balance. A null pool
report means no pool breakdown has been recorded (including older snapshots);
a null individual balance means the SDK did not provide it. These are cached
observations whose freshness is reported separately.

After NU6.3 activation, birthday discovery and sync require the indexer's tip
TreeState to contain a nonempty Ironwood frontier at the requested height. Sync
checks before scanning and again at the final tip before accepting readiness.
Missing data, RPC failures, and timeouts fail closed. This checks server
capability; it does not independently prove that a server supplies honest or
complete chain data. No check is required before activation.

Both reference Zodl apps default to `zec.rocks:443`. They also list regional
`na`, `sa`, `eu`, and `ap.zec.rocks` endpoints, and `us.zec.stardust.rest` and
`eu.zec.stardust.rest`. These public lightwalletd services need no partner key.
Environment overrides are optional. To use the following variables, explicitly
reference them with `indexer_url_env` and `submission_url_env` in TOML:

```sh
export ZCASH_INDEXER_URL=https://zec.rocks:443
export ZCASH_SUBMISSION_URL=https://zec.rocks:443
```

The same service can handle birthday lookup, sync and transaction submission.
Using it for both roles lets that provider observe both sync queries and
submission traffic. The roles can point to separate providers or your own
servers. Selecting a Zodl endpoint does not implement its failover or privacy
transport. Full treasury sync and funded swaps against these public endpoints
remain part of live qualification.

To sync an existing treasury using the selected endpoint policy, run:

```sh
x402_treazury wallet sync --config examples/deployments/servers-managed.toml
```

This command reads `[treasury]`, resolves state/key paths relative to the TOML,
and requires exclusive ownership: stop the managed server first. It does not load
API specs, require NEAR/submission credentials, or load `.env` automatically.
The indexer must serve mainnet over TLS; HTTP is permitted only on loopback for
local testing. Account 0's shielded funds are used, with `confirmations` (default
3) applied to zingolib's spendability calculation. Unconfirmed change is excluded.

Managed serving starts one shared background sync worker after all listeners bind.
It retries or refreshes after half `max_sync_age_seconds`, bounded to 1–60 seconds;
connection and tip requests have 15-second deadlines. Failed sync disables new
treasury spending while independently funded EVM pools keep serving. Initial
scans can take a long time. Sync checkpoints every 30 seconds and on completion,
failure or cancellation; each checkpoint atomically commits the encrypted wallet
and its observation under one snapshot revision. Checkpoints resume from saved
scan state. Old unreferenced snapshots are pruned, retaining the current snapshot
and every snapshot referenced by an outgoing operation.

`wallet status` reports the last persisted sync phase, checkpoint time, scan target
and scanned-block count, last successful tip height/time, confirmed and spendable
shielded balances in zatoshis, and any last sync error. `sync_fresh` also checks the
snapshot revision and `max_sync_age_seconds` (default 300). Cached balances remain
observations during sync/failure; they are not authorization to spend. New sends
must also pass the unresolved-outgoing gate and have sufficient spendable input
including fees. Reopening and clean shutdown mark readiness offline; a fresh sync
is required after restart. SIGINT/SIGTERM cancel the CLI sync and save its state.

Stop serving before running an offline backup:

```sh
target/debug/x402_treazury wallet backup \
  --state-dir /path/to/state --key-file /path/to/key \
  --treasury-id <UUID> --destination /path/to/new-backup
```

The command requires exclusive ownership and never overwrites a destination.
It copies a consistent SQLite snapshot (including pending signed transactions,
EVM keys and all journals) and its encryption key into an owner-only directory.
`backup.json` is published atomically after the database, key and manifest are
fully written and synced. A failed destination cannot be reused. If a directory
sync fails after publication, the command returns an error even though a complete
backup may be present; durability is not assured until sync succeeds. Protect the
entire backup as spending material. To restore, stop the original process and use the backup directory as
`state_dir`, its `key` as `key_file`, and the same treasury ID. Do not run the
original and restored copies simultaneously. A Zcash mnemonic alone cannot restore
random EVM keys, pending payment authorizations or the rotation journal.

To assess existing funding problems and perform one recovery pass, stop serving and run:

```sh
target/release/x402_treazury wallet recover --config servers.toml
```

The JSON report distinguishes `recovered`, `waiting`, `operator_required` and
`check_failed`. This command does not create pools, request quotes, prepare deposits
or broadcast transactions. It uses the same recovery logic as bootstrap and serving.
A known pre-preparation failure can reset an eligible unprepared job. A signed
operation retains its reservation until verified expiry; submitted or ambiguous
operations are reconciled first, never automatically rebroadcast. Recovery archives
the original operation and resets the job under a new operation ID. Run bootstrap
after successful standalone recovery to continue funding.

The wallet profile's `max_attempts` also bounds recovery using archived job history,
including quote refreshes, across process restarts. Reaching that bound requires
operator review; repeated bootstrap/recover commands do not reset it. Funding budgets
still apply to every replacement. Qualification runs require separately authorized
recovery; this path does not grant new qualification authority. Missing bytes alone never authorize retry after
uncertain preparation. Refund handling and confirmed deposits with unresolved swap
outcomes are reported for operator review. The low-level recovery commands remain
available for advanced use, but are omitted from ordinary help.

Managed serving reconciles each pool's Base balances and authorizations every five
seconds, sharing the admission gate with paid calls. This can resolve confirmed
payments and finish bootstrap; it never signs a payment or promotes the active
wallet. Changed canonical anchors remain blocked for explicit recovery.

Each sync cycle owns a separate Tokio runtime because pinned upstream sync can
leave helper tasks alive on early exit. Teardown stops those tasks and joins
blocking work before the final snapshot; shutdown must await that cleanup.
The treasury exposes a bounded, serialized command queue in `treasury/actor.rs`
for sync, preparation, saved-byte submission and reconciliation. Accepted commands
finish even when a reply receiver is dropped. `rotation/store/funding.rs` journals
immutable operation IDs, encrypted quotes, guarded phases and persistent fair
scheduling; status includes funding progress. Preparing a journaled funding job
commits its PREPARED phase together with the wallet snapshot and signed bytes.
Managed serving defaults to `[funding].auto_fund = true`: it funds the initial
active/standby pair and later replacements within configured limits. Set it false
to pause automatic funding; reconciliation and treasury sync remain available.
This default applies only to configured managed serving, not wallet commands or
static wallets. Bounded qualification examples explicitly opt out. `near_api_key_env` names the partner credential;
`near_user_session_env` optionally names a separate user-session bearer token.
Tokens stay in environment variables and are sent only to the fixed NEAR origin.
Session issuance/refresh and authenticated access still require qualification.

The coordinator persists a unique transparent refund address and wallet derivation
range before obtaining a quote. Quote retries and expired unprepared-quote refreshes
share `max_attempts`; every refresh archives its old bindings and allocates a fresh
operation/refund identity. Signed operations never refresh. Insufficient shielded
funds or budget leave the job unprepared and retryable.

`swap_timeout_seconds` starts at the first durable broadcast intent. On timeout,
status shows `timed_out` and pool `funding_degraded`; reconciliation continues at
least 60 seconds apart. This never releases funds or stops a funded active wallet.
Status failures have their own persisted exponential backoff (up to 300 seconds,
plus per-job jitter), independent of quote retry counts. Successful observations
reset the error streak. Status errors contain fixed actionable categories, never
remote bodies, credentials or deposit addresses. It resumes phases without repeating ambiguous
submissions, reconciles source confirmation, polls the same deposit, and independently
checks Base credit. Partial deposits enter `recovery_required`; failed/refunded swaps
stay `refund_pending`. Confirmed refund outputs reduce the originating operation's
consumed exposure once, capped at its principal; source and shielding fees remain
consumed. Reorgs of credited refunds fail treasury readiness closed. API refund
status alone never credits funds.

With serving stopped, prepare shielding for exactly one refund address:

```sh
target/debug/x402_treazury wallet shield-refunds \
  --config servers.toml --job-id JOB_UUID
```

This calculates and journals one transaction without broadcasting. Submit its
operation ID with `wallet reconcile --config servers.toml --operation-id UUID
--rebroadcast`. The command enforces `max_refund_shielding_fee_zec` and the daily budget;
only the fee counts as new expense. Each proposal selects one address, never
combining unrelated swaps. Both calculation and submission use the existing
restart-safe outgoing journal. Refund principal becomes shielded spendable only
after the shielding transaction confirms. Expired funding deposits use automatic
recovery or `wallet recover --config servers.toml`. For an individual shielding
operation, the advanced `wallet recover-expired --config servers.toml --operation-id UUID`
remains available with serving stopped. Expiry recovery requires a fresh tip beyond expiry by the configured confirmation
count, no positive indexer inclusion, synced invalidation, and confirmed unspent inputs
matched against the immutable preparation snapshot. It then releases the reservation
and resets any unfinished funding job with a new operation ID. Signed bytes and
recovery evidence remain archived; the old operation cannot be broadcast again.
Missing or conflicting evidence keeps the reservation. A later contradictory reorg
fails treasury readiness closed.

`treasury/send.rs` implements explicit deposit preparation through zingolib's
calculate-only API. It accepts one mainnet transparent recipient, uses account 0's
shielded inputs, rejects multi-step transactions, and verifies the resulting
recipient, amount, actual fee and bounded expiry. The caller supplies the pool,
operation UUID, source/fee limits, aggregate daily limit and quote deadline.
Preparation and the first submission require at least 300 seconds of quote validity.
The funding worker supplies these from validated configuration/quotes,
never from MCP tool arguments.

Preparation reserves the maximum source cost, marks readiness `preparing`, then
atomically commits the encrypted post-calculation wallet and signed bytes with
transaction ID, expiry, amount and fee. The reservation shrinks to actual cost.
An error/cancellation after reservation poisons the in-memory owner; close and
reopen it before further preparation. Unprepared reservations remain conservative
until explicitly abandoned through the store's guarded recovery API. The automatic funding coordinator calls this API through the serialized owner.

`rotation/near.rs` supports public and confidential foreign-chain EXACT_OUTPUT swaps.
The demo configurations explicitly select `funding.confidentiality = "public"`:
no NEAR key, account signup or application commission is needed. This keeps the
Zcash treasury shielded but uses public NEAR settlement. `basic` and `advanced`
remain explicit confidential options requiring separately qualified authentication.
Omitting the setting retains `basic` so existing configurations never silently
lose confidentiality. No mode falls back to another after a quote error.

Run the read-only $5 route demo (public test addresses, no credentials, no wallet,
no deposit allocation or transfer):

```sh
scripts/zcash.sh run --offline --example quote_near
```

Cargo's `--offline` disables dependency fetching; the example itself calls NEAR.
It validates the live public quote and prints the required ZEC input. Funded
execution requires an initialized treasury, chain endpoint settings and deliberate
`auto_fund = true`. Public funded execution has bounded
[qualification scope](testing.md#qualification-status);
authenticated confidential execution remains unqualified.

Quote validation checks assets, request bindings, mainnet transparent deposits,
exact output and integer cost caps. Timestamp normalization is accepted only for
the same instant. An extended provider deposit deadline never extends the local
send deadline. The echoed platform fee is allowed only for the captured 1Click
collector, with its rate and USD overhead bounded by `max_conversion_overhead_percent`; requests never
add application fees. Unknown collectors are rejected. Public quote fixtures and
confidential-auth rejection fixtures live in `tests/fixtures/near/`. Redirects are
disabled; optional credentials remain confined to the NEAR client.

`rotation/transaction.rs` makes submission consume a `BroadcastTransaction` handle
created only after `BROADCAST_REQUESTED` is durable. Each retry requires another
journaled intent. `treasury/submission.rs` uses the configured submission endpoint
for raw sends and the indexer endpoint for lookups, with mainnet/TLS validation,
15-second bounds, no fallback endpoints and no implicit retries. It verifies returned
transaction identity. Timeouts, cancellation, rejection and conflicting responses
retain unresolved exposure; successful acceptance alone does not release inputs.
Status includes `treasury_operations` with public transaction facts and attempt state.

Recovery requires exclusive ownership. Without `--rebroadcast`, this command only
syncs and looks up the saved transaction and needs no submission credential:

```sh
x402_treazury wallet reconcile --config servers.toml --operation-id UUID
# Explicitly allow resubmission of the SAME saved bytes, within deadline/expiry:
x402_treazury wallet reconcile --config servers.toml --operation-id UUID --rebroadcast
```

Confirmation releases the send gate only when fresh wallet sync and exact-byte
lookup agree at the configured depth; lookup also checks transaction inclusion in
a stable canonical compact block. An absent or expired transaction is quarantined,
not treated as unspent. Recovery never generates a replacement transaction.
SIGINT/SIGTERM preserve ambiguous submission state. Managed startup still only
syncs and queues funding jobs; it never sends ZEC automatically.

The adapters are experimental. Tests cover empty-wallet admission, synthetic gRPC
transport, journal recovery and sync. The optional proving suite exercises the
production preparation path with synthetic Orchard notes belonging to public test
keys, verifies the resulting proof, and restores identical pending bytes and wallet
state after reopening:

```sh
scripts/zcash.sh test --features zcash-testutils --all-targets
```

`zcash-testutils` opts into upstream test helpers and an Orchard verifier; normal
serving needs only `zcash`. The proof test uses a localhost info stub and never
broadcasts. Lookup tests reject a changing confirmation block, missing inclusion,
wrong heights, malformed hashes and failed rechecks. These synthetic tests do not
establish consensus acceptance or recovery after a mined transaction is reorged.
The separate [regtest suite](../tests/REGTEST.md) uses pinned Zebra/Zaino containers
and mined shielded funds to exercise submission, confirmation, restart and competing
branches. It verifies reservation retention before confirmation depth, explicit
rebroadcast of identical bytes, and quarantine after a previously accounted spend
is orphaned. Sync rechecks accounted source spends; loss of confirmation depth
sets `treasury_confirmed_spend_reorg`, retains consumed budget and blocks new
preparation until the original transaction regains sufficient depth. There is no
automatic replacement transaction or repair command for this condition.

The regtest network variant exists only in the unit-test binary under
`zcash-regtest`; production constructors remain mainnet-only, including all-feature
builds. Refund shielding and authenticated route qualification remain required before
unattended funding.

`rotation/store.rs` provides one exclusively owned, owner-only SQLite database
with WAL and full synchronization. A separate random 32-byte key encrypts wallet
snapshots, EVM keys and prepared transaction bytes using ChaCha20-Poly1305 and
fresh nonces. Authenticated record context includes the treasury and pool where
applicable. Keep both state and encryption key: a Zcash seed alone cannot restore
EVM keys or pending funding work. `wallet backup` preserves both in one consistent backup directory; the database
and key are not interchangeable backups.

Named pools persist two distinct bootstrap candidates and their USDC funding
floors. New profiles default to a $2 floor. An unsigned NEAR quote can raise a
candidate's target to the bridge's current minimum only within an explicit
`max_funding_amount_usdc` (omission allows no increase). The validated quote and new
wallet/job target commit atomically before any preparation or transfer. Successful
quotes freeze that target. Source-input, fee and treasury budget caps still apply;
an unaffordable minimum leaves funding unavailable rather than raising those caps.
Minimum hints are accepted only from the specific bridge-minimum error with a
positive atomic Base USDC amount. Each quote negotiation allows at most three
requests; other failures do not trigger amount changes or payment retries. Repeating `wallet pool` resumes the existing pool; changing its
`funding_amount_usdc` affects future allocations only. Transactional promotion retires
one address, promotes the standby, and creates one new key/funding job. Pools
have independent generation checks and roles. Shared ZEC budget reservations
carry unresolved exposure across days; confirmed costs are charged conservatively
on their confirmation day. A single pending prepared outgoing operation gates
new preparation, and exact bytes plus its wallet snapshot commit together.

A sync failure before the transaction preparer is invoked leaves the same quote
eligible for a later attempt. Interrupted calculation or durable transaction/budget
effects still require recovery; absence of signed bytes alone never permits retry.
Recovery status retains its preceding failure instead of clearing it.

Managed profiles connect these transitions to the x402 request path using a
trusted Base RPC. [servers-managed.toml](../examples/deployments/servers-managed.toml) contains
a complete deployment example. Inspect it without credentials or state access:

```sh
target/debug/x402_treazury config show \
  --config examples/deployments/servers-managed.toml
```

Serve with the Zcash-enabled binary using the same `--config` and an
`--env-file` containing the referenced endpoints and listener tokens. Paths are
relative to the deployment file. `[treasury]` identifies an already initialized
wallet; serving never creates/imports a Zcash seed. All declared managed profiles
allocate or resume their own pool, including profiles without a listener.
Source/server bindings sharing a profile share its gate and reservations. Static profiles can
coexist; their `private_key_env` is forbidden in managed profiles. Removing a
profile while serving managed pools disables it without deleting its history.
A name with durable managed state cannot become static; retain the treasury
reference so this identity conflict can be detected. Re-adding a managed name
resumes its existing UUID, keys and allocation targets.

Base verification defaults to **PublicNode → dRPC → Base**. The primary can be
overridden through `funding.base_rpc_url_env` (default `BASE_RPC_URL`); when that
default variable is absent, PublicNode is used. An empty value or a missing
explicitly named custom variable is an error. RPC access is separate from
API-provider access: a successful NEAR swap can still wait for independent Base
verification if the RPC rejects an isolated Tor exit. Logs identify the verification
step, HTTP status or JSON-RPC error code, and elapsed time without printing wallet
addresses, endpoint credentials or upstream response bodies. Funding status retains
RPC rejection codes. NEAR quote failures identify the asset-catalog or quote stage
and a sanitized HTTP, transport or validation category. Confirmed destination
credit clears obsolete quote/credit errors; source-reconciliation failures remain
visible until resolved. Late credit checks leave an already-completed job's newer
balances and roles unchanged. Credit-persistence messages distinguish insufficient
confirmed credit and local state failures from RPC access failures.
An empty treasury pauses refill before preparation and
reports how to fund/sync it, while existing funded wallets remain usable. Background reconciliation backs off after failures from 30
seconds up to five minutes; successful polls return to the normal five-second
interval. Failed checks never release reservations or authorize payments.

Unsigned `eth_chainId`, `eth_getBlockByNumber` and `eth_call` requests may retry
once after a transport interruption, with a 200 ms delay inside the original
request timeout. Both attempts reuse the exact payload, endpoint and
identity-bound HTTP client, including canonical block pins. HTTP/RPC rejections,
malformed JSON/envelopes and identified TLS errors are not retried. The read-only
retry never resends a paid API request or changes provider/Tor identity. Warnings
record each failed attempt and retry decision, request phase, safe HTTP/IO/TLS
categories and numeric status codes; raw error chains and addresses stay private.
The diagnostic example emits the same warnings to stderr.

When `funding.base_rpc_fallback_url_envs` is omitted, dRPC and Base are fallbacks
(a default equal to the primary is skipped). Set it to `[]` to disable fallbacks,
or name up to two environment variables to replace the default fallback list.
Missing custom variables fail startup rather than reverting to public services.
`config show` includes a secret-free `base_rpc_policy` describing these defaults
and environment references, and startup logs whether default fallbacks are active.
All selected services may receive wallet-address queries; shared provider accounts
or API keys can additionally link wallets. Custom-list example:

```toml
[funding]
base_rpc_url_env = "BASE_RPC_URL"
base_rpc_fallback_url_envs = ["BASE_RPC_SECONDARY", "BASE_RPC_TERTIARY"]
```

Failover restarts the **entire read-only chain view** on one endpoint, including
chain ID, persisted anchor, confirmed/latest balances, authorization nonces and
final canonical-block checks. Partial evidence is discarded. HTTP 403/408/429,
500/502/503/504, availability transport failures and RPC -32005/-32016/-32601
permit fallback. Malformed data, wrong chain, conflicting/stale chain evidence,
TLS validation failures and other rejections fail closed. Paid requests and Zcash
submissions are never replayed by this mechanism. Payer SOCKS identities and the
configured Tor-only network policy remain unchanged.

Each provider gets a complete-view deadline of 15 seconds in direct mode or the
Tor request floor (240 seconds by default), including its bounded transport retry.
At most three providers are visited once per view; earlier caller/admission
deadlines still apply. Logs identify provider indices, failure categories,
deadlines and fallback success without logging URLs or credentials. Each new view
starts with the primary; there is no global circuit cycling or cross-wallet
provider-health preference. With no URL flags, `diagnose_credit` uses the three
public defaults below. An explicit `--rpc-url` selects only that endpoint unless
up to two `--fallback-rpc-url` flags are supplied; it never silently appends defaults
to an explicit diagnostic list.

Default public endpoints, in order (availability and Tor acceptance vary):

| Operator | HTTPS endpoint | Documentation |
| --- | --- | --- |
| PublicNode | `https://base-rpc.publicnode.com` | [Base gateway](https://base.publicnode.com/) |
| dRPC | `https://base.drpc.org` | [Base API](https://drpc.org/docs/base-api) |
| Base | `https://mainnet.base.org` | [Network details](https://docs.base.org/get-started/connect-to-base) |

This list provides availability failover, not a quorum. A successful view
still trusts one configured provider; availability failover does not protect
against that provider returning fabricated but internally consistent data.

The public `mainnet.base.org` endpoint returned HTTP 403 and 429 during Tor
qualification; Base documents its public RPC as [rate limited and unsuitable for
production](https://blog.base.org/base-mainnet-is-open-for-builders). Use an
explicitly selected endpoint that works with your Tor identities. The application
does not use providers outside the configured failover policy, bypass Tor, or
weaken confirmation checks.
For the Tor qualification deployment, explicitly select PublicNode:

```sh
export BASE_RPC_URL=https://base-rpc.publicnode.com
```

PublicNode passed concurrent canonical balance checks and application reconciliation
through the test's wallet isolation identities, but also returned two intermittent
HTTP 403 block-read responses. This is a qualified test choice,
not a guarantee of production availability or Tor acceptance. The RPC sees the
queried public addresses; a shared provider account/API key can additionally link
wallets. Credit verification remains mandatory: NEAR success alone does not mark
a wallet ready. Balance and authorization reconciliation also protects concurrent
payment reservations. RPC failures defer affected operations, never imply a zero
balance, and never justify replacement funding. These checks trust the configured
RPC; they are not an independent light-client proof.

A read-only diagnostic can exercise concurrent verification without keys or changes
to wallet state:

```sh
scripts/zcash.sh run --example diagnose_credit -- \
  --network-config examples/network/tor.toml --state-dir state/demo \
  --rpc-url "$BASE_RPC_URL" --rounds 3
```

The diagnostic sends the state's public EVM addresses to the selected RPC and
prints balances indexed by pool, without printing addresses, and exits unsuccessfully
if any view fails. Use the same trusted
RPC policy as the deployment; choosing another provider discloses those addresses
to that provider.

Managed `mode = "zcash_rotation"` requires `max_conversion_overhead_percent`
(an integer percentage from 0 to 100). `max_funding_spend_zec` is an optional
ceiling on one funding transfer including its network fee. See
[funding limits and fees](wallet-rotation.md#funding-limits-and-fees) for USDC
budgets, bridge-minimum headroom, and optional native-ZEC safeguards.
Small deposits increase refill frequency; validate swap minimums and total fee overhead
before choosing sub-dollar targets. A payment larger than the configured deposit
target is refused rather than repeatedly rotating wallets. Explicit sizes in
existing configurations are unchanged.

`funding_amount_usdc` defaults to `"2.00"`, `max_api_payment_usdc` to `"1.00"`, `wait_seconds`
to 30 (range 1–3600), and `max_attempts` to 3. Money fields are decimal strings;
USDC permits six fractional digits and ZEC eight. Singleton treasury/funding
settings and all risk limits are validated by `config check`/`config show` without
unlocking state. Endpoint credentials remain environment references. Serving
requires those references; endpoints require HTTPS, with HTTP allowed only for
loopback fixtures. Zcash/NEAR endpoint settings are reserved for the funding
worker and are not contacted by this implementation.

Managed payments accept only v2 `exact` EIP-3009 on Base with canonical USDC,
the `USD Coin`/`2` signing domain, and an amount within both the cap and current
deposit target. Selection preserves the first compatible offer, even if earlier
offers are unsupported. Permit2, `upto`, v1, non-authorization flows, nested
extensions and unknown extra/offer fields are rejected. Top-level extension maps
are removed before signing, with warnings naming the omitted extensions (never
logging their values). Managed payments attempt one ordinary payment without
extension behavior or attribution echoing. Providers requiring extension echoes
may reject it; failed submitted requests report this omission and retain their
reservation until chain reconciliation. Never automatically retry a signed
payment. Malformed extension maps are rejected before admission. A seller or
facilitator may still add its own on-chain attribution. Static payment support
is unchanged. An ordinary non-402 response never enters admission.

The per-pool deadline includes waiting for its gate and chain verification;
Tor applies the network request-timeout floor to this deadline. The gate is
released after the signed authorization is durably journaled, before sending it
to the provider. A slow signed response therefore does not block another call
with sufficient unreserved balance in the same pool, or background reconciliation.
Chain verification still holds the pool gate; unrelated pools remain independent.
Readiness failures return immediately; funding runs asynchronously when enabled.
Both bootstrap addresses must hold their original targets before the first paid
call. RPC verification checks chain 8453, block freshness, and EIP-1898 canonical
block-hash queries for USDC balances and `authorizationState`. The RPC must support
these queries. Admission uses the lower of confirmed and latest balances minus
unresolved authorizations. The default confirmation depth is 12 and maximum
latest-block age is 120 seconds. A changed confirmed anchor blocks the pool with
`chain_recovery_required`; automated reorg recovery is not implemented.

Admission, promotion and the replacement key/job commit transactionally. A busy
wallet returns `payment_pending` instead of rotating. Actual depletion promotes
only a freshly verified standby able to cover that call. A signed authorization's
payer, payee, nonce, amount, validity, requirements hash, attempt and generation
are journaled before its single signed HTTP attempt. Signatures are not persisted.
A final 402, timeout, cancellation or HTTP success does not release its reservation.
A subsequent admission reconciles it against a common confirmed block: used
nonces or expired authorizations verified unused release exposure. This conservative
accounting can temporarily understate spendable funds. Reconciliation runs on demand
and every five seconds in the background.

Pools can spend independently verified Base balances and promote funded standbys.
Queued replacements are funded only with `funding.auto_fund = true`. Refund shielding
requires explicit operator handling. Eligible source-expiry recovery runs automatically
with funding, or through `wallet recover` while serving is stopped.

The [lifecycle qualification matrix](../tests/LIFECYCLE.md) maps each recovery and
funding invariant to its offline or consensus test. The combined lifecycle test
uses real payment signing and the Base RPC adapter with a deterministic funding
backend; regtest separately verifies Zcash proofs and settlement.

Store tests cover encrypted recovery, ownership, snapshot/generation
conflicts, multi-pool targets, budget contention, failed-write rollback, cancelled
waiters, sync checkpoint atomicity/freshness, durable submission records, and recovery
after a child exits without destructors. The child helper
is marked ignored in normal enumeration and run explicitly by its parent test.
The managed tests cover offer filtering, journal failures, deadlines, independent
pools, restart/cancellation, expiration, stale/reorged evidence and promotion
rollback. Two `zcash` feature tests exercise encrypted zingolib restore and the
offline CLI; two more test managed multi-listener serving and generated-pool
identity across template edits and scope changes. Local gRPC fixtures exercise
real zingolib sync, wrong-network rejection, periodic checkpoints, cancellation,
and encrypted restart/resume without real funds or external indexers.
