# Wallet CLI and managed configuration

Start with the [README quickstart](../README.md#quickstart-z-cash-wallet-with-dynamically-managed-tools).
This reference covers treasury commands, funding controls and operator recovery.
For lifecycle invariants, see [wallet rotation](wallet-rotation.md).
Commands run from the repository root; `x402_treazury` means the built executable on PATH.

## One configuration for wallet commands

Use the same deployment file for initialization, inspection, backup, sync and
serving. `--meta-config` remains an alias for `--config`. Wallet commands do not fetch provider catalogs or require listener tokens.
`wallet bootstrap` also consumes wallet assignments and funding settings to fund
initial managed capacity; other administration commands do not start funding workers.

```sh
x402_treazury wallet init --config examples/deployments/managed.local.toml
x402_treazury wallet addresses --config examples/deployments/managed.local.toml
x402_treazury wallet status --config examples/deployments/managed.local.toml
x402_treazury wallet backup --config examples/deployments/managed.local.toml --destination secrets/backup
x402_treazury wallet sync --config examples/deployments/managed.local.toml
```

`wallet init` prints the primary ZEC receive address and a prompt to send ZEC to
it. `wallet sync` and `wallet status` show readable summaries by default, with
saved balance observations distinguished from current spending authority.
For each managed wallet group, the human summary lists every non-retired wallet's
address and saved estimated balance in USDC (or `unknown`), followed by the number
of retired wallets.
Human status also shows the primary shielded ZEC receive address from the encrypted
snapshot when the wallet key is available. If the key or snapshot cannot be read,
status still shows saved metadata and explicitly marks the address unavailable.
Use `wallet init --json`, `wallet sync --json`, or `wallet status --json` for
machine-readable output. Init returns `{state, receive_addresses}`; sync retains
those fields and adds
`observations.zec` and `observations.usdc`, each with `status` (`completed`,
`failed`, or `inapplicable`), wallet/pass counts `attempted`, `completed`, `failed`,
and an optional content-free `error` category. Setup failure attempts no remote
observation. Sync prints independent asset results even on partial failure and
returns a nonzero exit code if either applicable scope fails. Status returns the
saved state object. `wallet addresses` continues to return JSON.
Progress logs remain on stderr in both modes.

Stop `serve` before running `wallet sync`: it acquires the same exclusive treasury
ownership. Explicit sync performs one ZEC scan and one pass over existing eligible
configured managed-USDC wallets plus outstanding accepted initial-funding
recipients, including removed profiles. It neither allocates wallets nor starts
funding or a background watcher. A failed ZEC pass logs `automatic_retry=false`;
rerun `wallet sync` to try again. Serving instead reports the existing retry
backoff and retries while sync is needed. ZEC and Base run independently: a failed endpoint
or missing asset setting does not suppress the other pass. Only treasury and
relevant managed-Base settings are resolved; no provider catalogs, listener tokens,
static signing keys or NEAR credentials are required. Treasury-only deployments
remain valid without `[funding]` or Base settings. Existing scoped USDC recipients
require the Base settings in `[funding]` even when their profile was removed.
Observation scope uses managed profile modes and valid generated bindings without
validating spending limits or unused templates. Broken unrelated bindings do not
hide existing configured wallets or accepted funding recipients; serving retains
its strict configuration validation.
An authenticated store with an SDK-incompatible Zcash snapshot still permits the
Base pass: ZEC reports `zec_snapshot_unavailable` and `receive_addresses` is empty
(the human summary marks the address unavailable). Store authentication, schema,
identity and exclusive-ownership failures remain command setup errors and perform
neither asset's remote observation.

While `serve` is running, use `wallet status --config FILE` with the same deployment
file to see the shielded receive address, saved balances and sync progress. This
read-only command does not acquire treasury ownership. Status reads cached
observations without network traffic; idle incoming funds may be absent until
startup, explicit sync, or another relevant discovery trigger. `wallet addresses` also
reads the encrypted snapshot without exclusive ownership and returns all saved
receive addresses as JSON. Neither command derives an address or changes sync
readiness. `wallet init` refuses an existing wallet; `wallet address` creates a new
address and requires exclusive ownership, so it cannot run against the same
treasury while `serve` owns it. Status JSON retains its existing saved-state shape
and needs neither the key nor SDK snapshot decoding.

Only `treasury.state_dir` is required in the treasury table.
Actual spendable funds after reservations, confirmations and fees govern enabled funding;
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
location overrides with `--config`: the deployment owns its paths. You may combine
`--config` and `--network-config FILE` to replace the complete network policy for
that invocation, including initialization, synchronization, bootstrap and recovery.
Omitted network fields use defaults; they do not inherit values from the deployment.
The file may also contain deployment `[discovery_relay]` settings; wallet commands
accept them without loading relay providers or performing paid discovery.
Saved transaction bytes, wallet identities and recovery authority are unchanged.
Supervised qualification sync retains its pinned deployment policy and rejects
this override. `--birthday` and import options remain command-specific.

## Bootstrap before discovery

After depositing sufficient ZEC into the treasury, run:

```sh
x402_treazury wallet bootstrap --config examples/deployments/managed.local.toml
```

This declares configured groups, runs the production treasury/funding workers,
and waits for the first positive credit on its captured enabled
reserve generations. It closes treasury ownership before exiting, requires
`funding.auto_fund=true`, and respects source, fee, attempt and qualification limits. Static wallets are
excluded and need external funding. No catalogs, pricing, listener tokens, static
signing keys or MCP listeners are needed. Network traffic follows the deployment
policy, including Tor.

By default, successful bootstrap prints each captured wallet name, its small/large
type, and its saved advisory balance estimate in USDC. These estimates can be stale, include
partial credit, and are independent of funding targets. Errors are reported on stderr with a nonzero
exit status. Add `--json` for the versioned machine-readable result, including
`balance_estimates` with `wallet_name`, `wallet_type`, and decimal
`balance_estimate_usdc` fields. The JSON result separates current serving/inventory readiness from
completion of captured work (see [capacity output](#versioned-capacity-output)).
Repeating the command resumes current incomplete claims, including eligible recovery. Signed
transactions are replaced only after the existing canonical expiry and unspent-input
proof succeeds. Unknown preparation outcomes and conflicting evidence still require
operator review; cancellation drains
accepted financial work and preserves journals. Treasury ZEC funding and Base
confirmation can take time. The command introduces no new spending limits.
While waiting, it logs unfinished jobs and their sync/submission state once a
minute. The first positive USDC observation satisfies the captured destination
claim even when retained timeout diagnostics remain; exact pending source work
stays owned and continues reconciling during serving.

Ordinary managed `serve` binds listeners and exposes available tools before starting
initial or replacement funding. Remote catalog/pricing discovery and pending paid
relay discovery continue during serving under the retained owner; relay discovery
waits for its assigned payer without allocating wallets itself. Serving does not
wait for bootstrap completeness. `auto_fund=false` pauses new funding and makes
explicit bootstrap fail, while accepted operations remain observed; qualification
funding restrictions retain their existing lifecycle. `catalog warm` never starts
funding: use `wallet bootstrap` first when warming needs a managed paid relay wallet.

## Treasury and managed pools

Build or test the embedded wallet with the shell wrapper, which supplies a
Cargo-managed protoc compiler. First builds may download public proving
parameters. The wallet dependency is pinned upstream at zingolib v7.0.0 commit
`bfb0a2156819ea7f75a9b09b1724abfd06c8925e`; no reference checkout is needed.

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

# Configure explicit reserves and payment authority, then bootstrap with --config.
# The removed wallet pool command provides no allocation path.

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
uses the older of the information/tip observations, minus 100 blocks. Ordinary
chain advancement between reads only increases the scan overlap. Lookup uses the
network connection, dispatch-readiness and response-inactivity policies, without
a whole-operation deadline. `--birthday HEIGHT` skips lookup
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
observations; their scan completion and snapshot binding are reported separately.

After NU6.3 activation, birthday discovery and sync require the indexer's tip
TreeState to contain a nonempty Ironwood frontier at the requested height. Sync
checks before scanning and again at the final tip before accepting readiness.
Missing data, RPC failures, and timeouts fail closed. This checks server
capability; it does not independently prove that a server supplies honest or
complete chain data. No check is required before activation.

The v7 SDK also requires the sync server to advertise lightwallet protocol
version 0.5.0 or newer in `GetLightdInfo`. A missing, malformed or older version
fails the scan with `indexer_protocol_unsupported`; it does not select another
endpoint or permit a direct connection outside the configured transport. This
wire-protocol requirement is separate from the Rust `lightwallet-protocol` crate
version 0.4.0 used to encode the messages. Offline wallet inspection remains
available when the server cannot sync.

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
x402_treazury wallet sync --config examples/deployments/managed.toml
```

This command reads `[treasury]`, resolves state/key paths relative to the TOML,
and requires exclusive ownership: stop the managed server first. It does not load
API specs, require NEAR/submission credentials, or load `.env` automatically.
The indexer must serve mainnet over TLS; HTTP is permitted only on loopback for
local testing. Account 0's shielded funds are used, with `confirmations` (default
3) applied to zingolib's spendability calculation. Unconfirmed change is excluded.

Available treasury serving requests one background ZEC scan after listeners bind.
Further scans need enabled unfilled funding claims or validated pending source or
shielding transactions. Each recurring scan starts at least 60 seconds after the
previous scan completes, with independent failure backoff bounded by 300 seconds.
New reserve demand wakes discovery; idle serving does not watch for deposits.
The [wallet observation policy](wallet-rotation.md#wallet-observation-policy)
describes the Store-only exception when the SDK/source capability is unavailable.
Connection establishment and per-response read inactivity use the shared network
policy. Actual spending keeps its SDK/canonical validation; independently funded
Base payers remain usable during scan failure. Initial scans can take a long time. Sync checkpoints every 30 seconds and on completion,
failure or cancellation; each checkpoint atomically commits the encrypted wallet
and its observation under one snapshot revision. Checkpoints resume from saved
scan state. Old unreferenced snapshots are pruned, retaining the current snapshot
and every snapshot referenced by an outgoing operation.

`wallet status` reports the last persisted sync phase, checkpoint time, scan target
and scanned-block count, last successful tip height/time, confirmed and spendable
shielded balances in zatoshis, and any last sync error. `sync_complete` reports a
completed scan bound to the current snapshot revision; it is independent of host
clock age. Cached amounts are observations. SDK input selection, exact pending
input ownership and current operation permission govern new spending. Reopening
marks the owner offline until it scans. SIGINT/SIGTERM save retained scan work.
The removed `max_sync_age_seconds` and `base_max_block_age_seconds` settings fail
configuration validation explicitly.


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

The embedded Zingolib v7 SDK reads older wallet snapshots and writes SDK wallet
format 45 on the next save, including the finite-sync and transparent-scan fields.
The treasury database schema, encrypted envelope, identities and journaled
transaction bytes retain their existing formats. Stop serving and take a complete
backup before upgrading. Older v6 application binaries cannot reopen a format-45
SDK snapshot. A pre-upgrade backup is not a safe rollback after new financial work:
restoring it would discard the newer accepted operations and liabilities. Preserve
the current state and use a compatible application for recovery.

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
operator review; repeated bootstrap/recover commands do not reset it. Actual spendable
source capacity and per-operation safeguards apply to every new preparation. Qualification runs require separately authorized
recovery; this path does not grant new qualification authority. Missing bytes alone never authorize retry after
uncertain preparation. Refund handling and confirmed deposits with unresolved swap
outcomes are reported for operator review. The low-level recovery commands remain
available for advanced use, but are omitted from ordinary help.

Managed serving performs finite startup Base discovery and requested scalar
balance refreshes under the [wallet observation policy](wallet-rotation.md#wallet-observation-policy).
Idle wallets are not swept, and ordinary scalar reads do not reconcile historical
payment authorizations. Canonical qualification and explicit recovery retain their
separate evidence and authority requirements.

Each sync cycle owns a separate Tokio runtime because pinned upstream sync can
leave helper tasks alive on early exit. Teardown stops those tasks and joins
blocking work before the final snapshot; ordinary shutdown awaits that cleanup.
During wallet or bootstrap drains, a second operator signal forces a nonzero
process exit with unfinished liabilities retained for recovery.
The treasury exposes a bounded, serialized command queue in `treasury/actor.rs`
for sync, preparation, saved-byte submission and reconciliation. Accepted commands
finish even when a reply receiver is dropped. `rotation/store/funding.rs` journals
immutable operation IDs, encrypted quotes, guarded phases and persistent fair
scheduling; status includes funding progress. Preparing a journaled funding job
commits its PREPARED phase together with the wallet snapshot and signed bytes.
Managed serving defaults to `[funding].auto_fund = true`: it funds the configured
reserves under per-operation safeguards. Set it false
to pause automatic funding; reconciliation and treasury sync remain available.
This default applies only to configured managed serving, not wallet commands or
static wallets. Bounded qualification examples explicitly opt out. `near_api_key_env` names the partner credential;
`near_user_session_env` optionally names a separate user-session bearer token.
Tokens stay in environment variables and are sent only to the fixed NEAR origin.
Session issuance/refresh and authenticated access still require qualification.

The coordinator persists a unique transparent refund address and wallet derivation
range before obtaining a quote. Unsigned quote failures use per-pass backoff,
without consuming `max_attempts` for transaction recovery. A quote refresh must
prove no accepted calculation or outgoing record exists; it archives its old bindings, records
a durable refresh marker and allocates a fresh operation/refund identity.
Historical unclassified recovery records still count. Signed operations never refresh. Insufficient shielded
spendable funds leave the job unprepared; another attempt still needs current authority.

Fresh allocations and unsigned funding quotes do not start Base credit polling.
Actual preparation reads its exact destination once after the SDK provides a valid
spendable proposal. Accepted preparation and signed or previously submitted work
retain their own credit lane across restart, using the recipient acquisition's
existing due time without another credit backoff or jitter. Each recurring Base,
source or NEAR status observation starts at
least 60 seconds after the previous observation for that work completes, plus
existing jitter where applicable. With default `poll_seconds = 5`, per-job jitter
is at most five seconds. The floor is not a settlement deadline; canonical
confirmation/expiry and first-positive credit retain their separate stop rules.

`swap_timeout_seconds` is a health-warning threshold, default 1,800 seconds from
the first durable broadcast intent, not an abort deadline. When exceeded,
status shows `timed_out` and pool `funding_degraded`; reconciliation retains its
existing minute floor. This never releases funds or stops a funded active wallet.
Each observation kind retains its own runtime error backoff, bounded by 300 seconds
plus existing jitter, independently of quote retry counts. Saved health/`next_poll`
fields report aggregate progress; they do not impose one kind's failure on another
after reopen. Successful observations reset only their own error streak. Status
errors contain fixed actionable categories, never remote bodies, credentials or
deposit addresses. The coordinator resumes phases without repeating ambiguous
submissions, reconciles source confirmation, polls the same deposit, and independently
checks Base credit. Provider-reported partial deposits enter `recovery_required`;
failed/refunded swaps stay `refund_pending`. The SDK owns incoming returns and current spendable inputs. Local refund credits,
principal caps and historical refund revalidation are removed; provider status alone
never proves an on-chain return or authorizes a new deposit.

With `funding.auto_fund=true`, serving and bootstrap attempt shielding of confirmed
spendable owned transparent funds after persisted sync, one address per transaction,
including returns without job attribution. Paused funding and qualification permits
do not authorize new automatic shielding. Pending exact bytes remain observable;
uncertain submissions are never automatically replayed.

With serving stopped, explicitly prepare shielding for exactly one refund address:

```sh
target/debug/x402_treazury wallet shield-refunds \
  --config servers.toml --job-id JOB_UUID
```

This calculates and journals one transaction without broadcasting. Submit its
operation ID with `wallet reconcile --config servers.toml --operation-id UUID
--rebroadcast`. The command enforces `max_refund_shielding_fee_zec` and actual pending input ownership;
only the fee counts as new expense. Each proposal selects one address, never
combining unrelated swaps. Both calculation and submission use the existing
restart-safe outgoing journal. Refund principal becomes shielded spendable only
after the shielding transaction confirms. Expired funding deposits use automatic
recovery or `wallet recover --config servers.toml`. For an individual shielding
operation, the advanced `wallet recover-expired --config servers.toml --operation-id UUID`
remains available with serving stopped. Expiry recovery requires a fresh tip beyond expiry by the configured confirmation
count, no positive indexer inclusion, and every originally selected confirmed unspent input
matched against the immutable preparation snapshot. It closes the original input owner. A replacement funding attempt needs separately
revalidated current authority. Signed bytes and
recovery evidence remain archived; the old operation cannot be broadcast again.
Missing or conflicting evidence keeps the original ownership.

`treasury/send.rs` retains the public Zingolib proposal through accepted-input
journaling, then uses the backend's calculate-only API with explicit ordinary
NU7-aware expiry. Public proposal cleanup runs after releasing the wallet guard.
It accepts one mainnet transparent recipient, uses account 0's
shielded inputs, rejects multi-step transactions, and verifies the resulting
recipient, amount, actual fee and bounded expiry. The caller supplies the pool,
operation UUID, source/fee safeguards and quote deadline.
`funding.quote_deadline_seconds` requests a 7,200-second window by default;
the returned quote can be shorter and remains authoritative. Preparation and the
first submission require an unexpired returned deadline, without an extra local
margin. The requested window is a preference. The chain-specific 1Click watcher
arrival/confirmation and late-refund contract remains unresolved; local broadcast
acceptance does not establish timely delivery. See [transport/contract limits](network-egress.md#remaining-timers-and-budgets).
The funding worker supplies these from validated configuration/quotes,
never from MCP tool arguments.

Preparation selects SDK inputs, checks actual fee/output and permission, journals
exact selected inputs, then calculates signed bytes. Known proposal insufficiency
defers safely; uncertainty after accepted calculation retains ownership and requires
owner teardown/reopen. Completed bytes and snapshots commit idempotently.
before further preparation. Network/chain/expiry preflight precedes durable broadcast
intent, so failure there consumes no submission attempt. The preflighted send path
performs only local expiry checks before sending the saved bytes, without another
unsigned tip read. Failure or cancellation after intent still has an unknown outcome. Unprepared reservations remain conservative
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
branches. It exercises retained input ownership, exact-byte rebroadcast and canonical input
proofs. These tests do not qualify external bridge settlement. Historical accounting
revalidation no longer gates ordinary sync; the SDK owns chain corrections.

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

Managed groups persist exact enabled small/large reserve claims. Each new group
starts with one wallet per enabled slot; the first payment promotes its payer and
allows that slot to refill. Reserve targets are explicit funding preferences. An unsigned NEAR
quote can raise accepted output to a bridge minimum only within that slot's explicit
maximum (omission permits no increase); original allocation target and identity stay
immutable. Source-input and fee safeguards still apply, with actual spendable treasury
funds limiting ordinary funding. Each quote negotiation allows at most three requests;
other failures do not authorize amount changes or payment retries.

A READY payment atomically retains the prior active, activates its exact payer and
vacates only the consumed claim generation. Selection allocates nothing. The independent
funding owner fills enabled vacancies under current authority; retained wallet count
and retirement never gate refill. Source reservations preserve unresolved liabilities
across days. One pending prepared outgoing operation gates new preparation, and exact
bytes plus its wallet snapshot commit together.

A sync failure before the transaction preparer is invoked leaves the same quote
eligible for a later attempt. Interrupted calculation or durable accepted-calculation/transaction
effects still require recovery; absence of signed bytes alone never permits retry.
Recovery status retains its preceding failure instead of clearing it.

Managed profiles connect these transitions to the x402 request path using a
trusted Base RPC. [managed.toml](../examples/deployments/managed.toml) contains
a complete deployment example. Inspect it without credentials or state access:

```sh
target/debug/x402_treazury config show \
  --config examples/deployments/managed.toml
```

Serve with the application binary using the same `--config` and an
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
visible until resolved. Initial positive partial credit consumes the claim and
publishes an advisory estimate; later credit observations cannot reopen it.
Credit-persistence messages distinguish missing credit and local state failures
from RPC access failures.
An SDK proposal rejected for insufficient funds defers initial funding, while
existing Base wallets remain usable. Background funding observations retain independent
failure backoff bounded by five minutes, with existing per-job jitter for source
and provider checks. Base credit uses the shared recipient acquisition schedule;
cached errors do not count as new recurring observations. Successful observations
reset only their own error streak. Each recurring source, provider
status or Base credit check starts at least one minute after that work's previous
check completes. `poll_seconds` controls local inventory/owner scheduling and
jitter; it cannot authorize faster network observations. Successful local quote,
preparation and submission handoffs remain immediate. Failed checks retain pending
ownership. Saved estimates never grant payment authority or prohibit an otherwise
authorized signing attempt.

Unsigned `eth_chainId`, `eth_getBlockByNumber`, `eth_getTransactionReceipt` and
constrained `eth_call` reads retain exact payloads and canonical block pins.
Direct mode allows two same-client attempts for transport interruption, with
200 ms backoff. Tor instead uses the selected transport plus up to two fresh
owner-bound scopes for HTTP `400`, `403`, `429`, `503` or eligible typed transport
failures (including TLS failures) on one endpoint before provider fallback. Valid `Retry-After` delays only
a remaining attempt; final exhaustion returns without waiting. RPC rejections,
malformed results/envelopes and oversized bodies earn no circuit
repetition. Transport promotion uses method-specific observation validation;
pending receipts can establish connectivity without a transfer proof. Canonical
chain proof remains separate. These reads never use Curl relay, sign payments, allocate wallets
or accelerate observation cadence. See [network policy](network-egress.md) for
receipt validation, selection lifetime and caller-specific fallback rules. Safe diagnostics retain
numeric status codes and fixed categories, without addresses or raw causes.

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
permit fallback. Malformed JSON/envelopes/results, oversized responses and TLS
validation failures may also use configured fallback, always acquiring a complete
fresh view. Wrong chain and conflicting canonical evidence remain terminal.
Only Tor TLS failures earn fresh-scope retries; trust checks remain enforced. Paid requests and Zcash
submissions are never replayed by this mechanism. Payer SOCKS identities and the
configured Tor-only network policy remain unchanged.

Each provider uses connection and response read-inactivity settings, without a
whole-view deadline. At most three providers are visited once per view, with finite
unsigned transport retries. Caller cancellation remains supported; signed authorization expiry remains
independent of progress. Logs identify
provider indices, fixed failure categories and fallback success without URLs or
credentials. Each new view
starts with the primary; there is no global circuit cycling or cross-wallet
provider-health preference. With no URL flags, `diagnose_credit` uses the three
public defaults below. An explicit `--rpc-url` selects only that endpoint unless
up to two `--fallback-rpc-url` flags are supplied; it never silently appends defaults
to an explicit diagnostic list.

For slow-connection diagnosis, a process-local `view_sequence` correlates
overlapping fallback attempts without identifying wallets. Failed views log `verification_stage`,
`stage_elapsed_ms`, `completed_rpc_calls`, `expected_rpc_calls`, `wallet_count`
and `pending_count`. Counts are logical RPC calls, not transport retry attempts.
Few completed calls and a long current stage indicate a slow individual check;
many completed calls show accumulated independent RPC latency. These are observations, not proof of an endpoint
or local-network fault. Configured connection and per-response read-inactivity limits remain distinct. Fallback success logs
elapsed time and completed calls. No endpoint URL, wallet address or nonce is logged.

Treasury tip-check failures retain sanitized gRPC codes and identify `starting_tip`
or `final_tip`. Scan failures include elapsed observation time and scanned block
count, with target, scanned and observed heights. Ordinary age and three-block
lag gates have been removed. `Cancelled` alone does not identify who cancelled
the RPC. These diagnostics never release prepared input ownership.

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

Managed `mode = "zcash_rotation"` requires `max_api_payment_usdc`,
`max_conversion_overhead_percent` (integer 0–100), and `reserves.small` with a
positive `funding_amount_usdc`. Optional `reserves.large` must be at least as large.
Each reserve has its own optional `max_funding_amount_usdc`; omission allows no
increase above target. `max_funding_spend_zec` caps one transfer including its fee.
The group payment cap is independent of reserve size; a price above a funding
target can be attempted by an eligible wallet under that payment cap.

`max_retirement_remainder_usdc` defaults to `"0"`; `max_attempts` defaults to 3.
See [configuration](configuration.md) for exact syntax, rejected aggregate settings,
reopen-only changes, initial inventory and legacy-state rejection.
Static-wallet payment defaults remain unchanged. Payment admission has no total timeout. Legacy
`wait_seconds` is accepted but ignored with a warning; remove it from wallet
configuration. Money fields are decimal strings;
USDC permits six fractional digits and ZEC eight. Singleton treasury/funding
settings and all risk limits are validated by `config check`/`config show` without
unlocking state. Endpoint credentials remain environment references. Serving
requires those references; endpoints require HTTPS, with HTTP allowed only for
loopback fixtures. Configuration inspection does not contact Zcash or NEAR endpoints. Managed serving
and wallet operations use them for synchronization, funding and recovery as applicable.

Managed payments accept Base USDC v2 exact EIP-3009 within the configured per-payment
cap and signing domain. Balances and historical funding targets are advisory, and
unresolved exposure is not subtracted from the estimate again. The signed journal
commits payer/payee/nonce/amount/validity and ownership before disclosure, with one
saturating estimate decrement per authorization. Selection consumes a READY claim
atomically; it does not allocate or fund new wallets.

Only the pinned structured insufficient-funds encodings on HTTP 402 after a signed
attempt permit traversal to another captured existing wallet. Each identity gets
at most one attempt, with its own transport and fresh challenge and the original
cloneable arguments (including POST). Every earlier authorization may still charge
or execute. Other failures never grant paid replay, and process restart never
replays the logical call. See [Best-effort wallet rules](wallet-rotation.md#best-effort-wallet-rules).

A finite startup pass and requested refreshes read unsigned latest USDC estimates
separately from payment admission; idle wallets are not swept. A successful zero
refresh finishes its request. Failed reads and notifications arriving during reads
retain follow-up work without delaying payment fallback. See the
[wallet observation policy](wallet-rotation.md#wallet-observation-policy).
Canonical authorization/debit proofs remain a qualification observation, preserving
all attempts and the existing case permit; seller success does not prove a debit.

Pools use existing eligible Base wallets and consume initially funded reserves.
Queued replacements and automatic per-address shielding require
`funding.auto_fund = true`. Explicit refund shielding remains available with serving
stopped. Eligible source-expiry recovery runs automatically
with funding, or through `wallet recover` while serving is stopped.

The [lifecycle qualification matrix](../tests/LIFECYCLE.md) maps each recovery and
funding invariant to its offline or consensus test. The combined lifecycle test
uses real payment signing and the Base RPC adapter with a deterministic funding
backend; regtest separately verifies Zcash proofs and settlement.

Store tests cover encrypted recovery, ownership, snapshot/generation
conflicts, multi-pool targets, exact input ownership, failed-write rollback, cancelled
waiters, sync checkpoint publication, durable submission records, and recovery
after a child exits without destructors. The child helper
is marked ignored in normal enumeration and run explicitly by its parent test.
The managed tests cover offer filtering, journal failures, deadlines, independent
pools, restart/cancellation, expiration, stale/reorged evidence and promotion
rollback. Offline tests exercise encrypted zingolib restore and the
offline CLI; additional tests cover managed multi-listener serving and generated-pool
identity across template edits and scope changes. Local gRPC fixtures exercise
real zingolib sync, wrong-network rejection, periodic checkpoints, cancellation,
and encrypted restart/resume without real funds or external indexers.

A restored backup must retain its exact accepted-work obligations and declared
restoration restrictions. A rescan cannot reconstruct unpublished authorizations,
uncertain source intent or consumed initial claims. Missing off-chain history does
not authorize replacement deposits or top-ups. Undeclared filesystem rollback is
not automatically detectable; a code rollback is not a state downgrade.

## Versioned capacity output

Wallet status JSON uses `schema_version: 5`; bootstrap uses `schema_version: 3`. Status additionally
reports `state_schema_version`. Historical qualification report files retain their
original interpretation; none of these observations grants payment or retry authority.

Bootstrap reports `serving_ready_groups`, `inventory_ready_groups`, and
`captured_completed_groups` separately. The captured command waits only for its
enabled slot generations, including claims vacant when it
starts. Successful completion has an empty `incomplete_captured_jobs` list;
obstruction or cancellation remains an error with persisted work retained. Current
inventory can become incomplete again after a reserve is consumed.

Status replaces the old pool `deposit_atomic` and `bootstrapped` fields with current
payment authority, retirement tolerance, exact `reserve_claims` and readiness. Original wallet `target` and last saved
`balance_estimate_atomic` are advisory history, not available-capacity authority. Signed
exposure, financial repair blocks, destination discrepancies and retirement
observations are reported separately. Null authority means unconfigured; explicit
unlimited authority is represented by the maximum U256 atomic value.

`runtime_balance_evidence` is an optional cached historical certificate with canonical
compatibility; it has no host-age admission authority. Current saved wallet fields
include `balance_estimate_atomic`, `low_balance` and fixed refresh error categories.
Unknown balance is null, never zero. Treasury `sync_complete` denotes a completed
unchanged snapshot, not financial freshness. Status performs no RPC.

Per-pool `retirement_accounting` sums only exact confirmed amounts recorded at
automatic retirement, with automatic/manual counts and historical unknowns separate.
Latest amounts and policy bounds are never added to that sum. These are historical
recorded balances, not current retired holdings, API costs or destroyed funds.
Unsupported legacy provenance has null totals/counts rather than invented zeros.

`wallet pool` is removed. Configure managed `reserves.small`, optional
`reserves.large`, and an explicit `max_api_payment_usdc`, then run
`wallet bootstrap --config FILE`. Declaration creates no allocation; the funding
owner fills permitted vacancies under current auto-funding and qualification authority.

Historical automatic-retirement records remain readable with their original
canonical evidence; advisory balance refresh does not manufacture retirement proofs.
Low-balance traversal rotates payment selection without deleting keys or funds.

Production storage preflight rejects unsupported legacy schemas without changing
source state. `wallet recover` reconciles supported accepted work; it cannot
manufacture missing immutable destination-readiness evidence or convert an
unsupported schema. Preserve complete backups and all accepted obligations; an
old backup cannot safely undo subsequent financial work.
