# Wallet CLI and managed configuration

Start with the [README quickstart](../README.md#quickstart-z-cash-wallet-with-dynamically-managed-tools).
This reference covers treasury commands, funding controls and operator recovery.
For lifecycle invariants, see [wallet rotation](wallet-rotation.md).
Commands run from the repository root; `x402_treazury` means the built executable on PATH.

## Preview configuration against saved groups

```sh
x402_treazury wallet preview --config deployment.toml
x402_treazury wallet preview --config deployment.toml --json
```

Preview requires a deployment file (also selected by `X402_TREAZURY_CONFIG`) and
initialized treasury state. It validates pure configuration and resolves effective
source/listener wallet bindings without loading provider files, catalogs, listener
tokens, static signing keys, Base/NEAR credentials, or the treasury encryption key.
It reads public saved metadata and the optional treasury-ID assertion in one
coherent read-only transaction, including while serving owns the store. This is
unauthenticated metadata inspection, not cryptographic verification or authority
to spend. SQLite may perform its existing read coordination; logical state and
original journals remain unchanged. `config show` remains entirely state-free.

The report identifies managed groups to reuse, reactivate or declare; requested
payment, retirement and reserve policies that differ from saved settings; omitted
groups already disabled or due to be disabled; stable UUIDs; effective bindings
and their explicit/generated origins; and saved inventory and pending work.
Every resolved managed profile is included, even if unbound. Saved balances are
advisory observations, never fresh reads or promises of capacity. Requested reserve
ranges authorize nominal USDC sizing only; accepted funding terms and actual
spendable capacity remain distinct. Historical auto-funding and fixed/range sizing
modes are not persisted and cannot be reconstructed for comparison.

Static profiles cannot reuse saved managed names, including disabled ones.
With no managed profiles, normal startup skips managed profile configuration;
preview therefore reports saved groups as retained unchanged rather than disabled.
Separately authorized direct-swap admission/recovery can still open the treasury.
Omitted holdings, identities and history remain owned: no automatic transfer,
deletion or sweeping occurs. Preview never infers rename matches or executes a
migration. Inspect name collisions before using the offline rename command below.

Valid differences return success. Invalid configuration, missing/unsupported state,
identity mismatch and startup-blocking managed/static conflicts return nonzero.
For mode conflicts the comparison report is printed before failure; other failures
produce diagnostics without a report. Deterministic JSON uses `schema_version=1`,
with `managed`, `omitted`, `startup_conflicts`, `bindings`, `generated`, `summary`,
`configure_managed_profiles`, `requested_auto_fund`, and the existing `saved` status
projection. Managed entries expose `action`, `saved_id`, `requested_policy` and
`changed_saved_policy_fields`; omitted entries expose `saved_id` and `action`.
The report is a snapshot, not an executable plan or authority token. Rename and
startup recheck current names/configuration when they execute.

## Rename a managed wallet group

Back up the treasury and stop serving before changing a group name:

```sh
x402_treazury wallet backup --config deployment.toml --destination secrets/backup
x402_treazury wallet rename research general --config deployment.toml
# Update wallet declarations and source/listener wallet references in deployment.toml.
x402_treazury wallet preview --config deployment.toml
x402_treazury serve --config deployment.toml
```

`wallet rename OLD NEW [--config FILE | --state-dir DIR] [--json]` changes
only the saved group's lookup name. Standalone commands also accept `--key-file`
and an optional expected `--treasury-id`. With configuration, only treasury
location and identity settings are used; neither name must occur in that file.
The command authenticates and exclusively owns the store without decoding the SDK
wallet, loading catalogs, contacting the network, or allocating/funding wallets.
Existing supported additive store initializers may run on opening.

The stable group UUID, payer keys, roles, claims, generations, balances, accepted
inputs, signed bytes and pending obligations remain owned by the same group.
Disabled groups and groups with pending work can be renamed. Renaming an existing
group to itself succeeds; a missing source always fails. Names follow the saved
store's lowercase letter/digit/underscore rules, beginning with a letter. Explicit
migration to or from generated `auto_v1_` names is supported; declaring those names
as explicit TOML wallet profiles remains prohibited.

An occupied destination is an error, including an empty group declared by an
earlier startup. Rename never overwrites or merges groups, edits TOML, transfers
funds, or grants funding authority. After changing configuration references,
restart reapplies the selected group's configured payment and reserve policies.
A larger configured group count can declare additional groups; a smaller count
retains omitted groups and their history under normal startup rules. Reusing a
group preserves payer identities and historical correlation, even for a new purpose.
If the command's acknowledgment is lost, inspect saved state before proceeding;
do not create a replacement identity. JSON returns `old_name`, `new_name` and the
unchanged `group_id`.

Listener principals own persisted source registries and direct payments separately.
Renaming a wallet group does not reassign those owners. Automatic wallet assignment
supplies bindings for defined listeners; it does not create listener definitions.

## One configuration for wallet commands

Use the same deployment file for initialization, inspection, backup, sync and
serving. Export `X402_TREAZURY_CONFIG` to use it by default; an explicit
`--config FILE` overrides the environment path. Unset it before using standalone
`--state-dir`, `--key-file` or `--treasury-id` options. Wallet commands do not
fetch provider catalogs or require listener tokens.
If `serve` cannot find the configured wallet directory, its error names the
resolved path. If that is the intended directory for a new wallet, run
`x402_treazury wallet init --config FILE` with the same deployment file.
`wallet bootstrap` also consumes wallet assignments and funding settings to fund
initial managed capacity; other administration commands do not start funding workers.

```sh
x402_treazury wallet init --config examples/deployments/managed.local.toml
x402_treazury wallet addresses --config examples/deployments/managed.local.toml
x402_treazury wallet status --config examples/deployments/managed.local.toml
x402_treazury wallet notes --config examples/deployments/managed.local.toml
x402_treazury wallet backup --config examples/deployments/managed.local.toml --destination secrets/backup
x402_treazury wallet sync --config examples/deployments/managed.local.toml
```

`wallet init` prints the primary ZEC receive address and a prompt to send ZEC to
it. Generated and mnemonic-imported wallets expose shielded-only receive addresses
and persist disabled transparent address discovery. Runtime reopening and SDK
reconstruction disable known-address queries and gap recovery regardless of saved
settings. Historical receive/refund identities and accepted work remain intact;
transparent/mixed refunds cannot authorize new quotes or managed deposits.
All issued receive addresses contain only the Orchard-format receiver used by
Ironwood. The [Ironwood-only policy](zingo-integration.md#ironwood-only-policy)
applies to wallet commands as well as serving. `wallet sync` and `wallet status` show readable summaries by default, with
saved balance observations distinguished from current spending authority.
The human summary computes ZEC balances from the existing saved SDK snapshot,
using the same read that supplies receive addresses. It groups them with the
snapshot revision, last completed observation time and saved scanned height:
local transaction changes can follow that observation. Confirmed shielded balance
is labeled **excluding reserved inputs**; the SDK excludes notes already linked to
a spending transaction. Confirmed and pending shielded totals include only Ironwood,
including dust. Spendable shielded balance excludes dust and follows the SDK's
confirmation and witness requirements. Pending incoming funds/change and spendable
balance appear separately. The stable transparent balance fields display `unknown`:
transparent holdings are unsupported and are not queried.
An outgoing transaction can therefore leave both confirmed and spendable shielded
balances at zero while its change is pending. If the key or SDK snapshot is
unavailable, the summary explicitly falls back to saved sync amounts for confirmed
and spendable shielded balances; the additional amounts display `unknown`.
Incoming notes are not listed as separate operations in this summary; use
`wallet notes` for retained receiving and spending history. An unchanged aggregate
balance does not establish whether a specific refund was received.
Outgoing treasury operations show the
principal and fee from existing transaction facts and saved source status
separately from cross-chain swap progress. Swap progress includes the last
validated response's local observation time, saved terminal NEAR outcome, last safe error, and `timed_out`
health threshold. The observation time includes pending/unknown responses and is
not the provider's `updatedAt`. Swap amounts show the operation's original allocation floor
and, when recorded, its selected nominal amount and accepted bridge output, not
received or available funds. These overlapping observations are never
added into a total or treated as spending authority. The human summary adds no
durable balance fields, schema changes, network requests or polling.
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
Normal sync output also prints observation failure or pending-retirement details,
even when logging is disabled. USDC counts include scalar balance reads and rejected
inventory or retirement-configuration entries;
successful reads can coexist with pending or unavailable canonical retirement.
The error category preserves an observation failure ahead of pending work from a
later wallet, while independent wallets continue through the finite pass.
Pending Base proof work includes unavailable finalized evidence and automatic
reconciliation after a canonical-history change. Rerunning sync or resuming
serving continues the original work without replaying a payment or deposit.

Stop `serve` before running `wallet sync`: it acquires the same exclusive treasury
ownership. Explicit sync performs one ZEC scan and one pass over existing eligible
configured managed-USDC wallets plus outstanding accepted initial-funding
recipients, including removed profiles, and attempts eligible retained-wallet
retirement once. It reports unresolved authorization, confirmation or proof work
without waiting for another observation interval; rerun sync or resume serving
to continue it. Retirement remains available with `auto_fund=false`. It neither allocates wallets nor starts
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
treasury while `serve` owns it. Status JSON needs neither the key nor SDK snapshot
decoding. Reserve claims include `blocked_funding_job_id` when completed, uncredited funding still occupies
the slot; the pool then reports `funding_degraded=true`. Human status explains
that automatic funding replaces this inventory after original source ownership
resolves. This is distinct from insufficient spendable ZEC, and previously
credited low-balance wallets are never topped up.

Only `treasury.state_dir` is required in the treasury table.
Actual spendable funds after reservations, confirmations and fees govern enabled funding;
the SDK calculates standard transaction fees without a separate application fee
ceiling. Transparent-input shielding is unsupported. See
[funding limits](wallet-rotation.md#funding-limits-and-fees).
The default key is `wallet.key` inside the wallet directory;
set `key_file` for separate storage. Possession of the complete default directory
is sufficient to decrypt the wallet. Owner-only permissions are not a password or
OS keychain. Initialization creates missing state parent directories, refuses an
existing state/key destination, and never silently imports or replaces a wallet.
On Linux and macOS, private files are created with owner-only access before any
wallet bytes are written. macOS creation suppresses inherited ACLs atomically;
reopening checks permissions, link count and ACLs on the opened descriptor.
Access-granting macOS ACLs on private state must be removed before reopening.
SQLite parents reject file-inheriting grants; non-inheriting parent grants remain
valid. Linux POSIX ACLs remain constrained by owner-only modes, including SQLite's
copied sidecar modes. Inspection errors fail closed.
Private storage operations on unsupported platforms return an explicit error.
The [separate namespace assessment](plans/portability_fixes.md) concerns configured
aliases and authenticated older state; these privacy checks do not detect rollback.

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
The file may also contain deployment `[relay_fallback]` settings; wallet commands
accept them without loading relay providers or performing paid discovery.
Saved transaction bytes, wallet identities and recovery authority are unchanged.
Supervised qualification sync retains its pinned deployment policy and rejects
this override. `--birthday` and import options remain command-specific.

## Offline note inspection

```sh
x402_treazury wallet notes --config FILE
x402_treazury wallet notes --config FILE --txid TRANSACTION_ID
```

`wallet notes` always returns JSON. It authenticates one committed schema-23,
mainnet, account-zero snapshot under a read-only SQL transaction and decodes it
with the SDK wallet reader. It can run while the serving owner holds the wallet.
It does not construct a client, sync, create a wallet directory, issue addresses,
prepare/reserve inputs, submit transactions or repair/save state. The configured
UUID, when supplied, must match. A missing key/snapshot, wrong identity, unsupported
schema or corrupt/incompatible SDK snapshot fails instead of falling back to
unauthenticated or repaired data.

The versioned output binds the treasury UUID, snapshot revision, saved SDK fully
scanned height, saved minimum-confirmation policy and SDK selection target/anchor.
`notes` includes retained account-zero Ironwood notes,
including spent history, sorted by pool, transaction ID and output index. Optional
`--txid` accepts a canonical 64-hex-digit transaction ID and filters receiving
transactions only. There is no silent truncation or history cap. Absence means
absent from this retained snapshot, not that no receipt happened.

Each note reports pool/output identity, value in zatoshis, key scope, actual
receiver and structured receiving/spending transaction facts. Pending transaction
classes are grouped as `pending`; they are not labeled confirmed. Confirmation
counts use the saved fully scanned height and remain unknown when it cannot
establish a count. A block hash appears only if the corresponding block remains
in the SDK snapshot; it is not a fresh canonical-chain proof. The `receiver` is
an Orchard-receiver-only UA encoding the actual Ironwood receiver, labeled by
`receiver_encoding`. Ironwood shares Orchard's receiver format but retains its
separate pool identity. This reconstructed receiver is **not** the original
multi-receiver refund UA. Seeds, keys, memos, raw transactions and unrelated
funding/payment history are excluded.

`sdk_spendable_at_snapshot` records membership in the SDK's read-only
`AllFunds(MaxSpendable)` selection restricted to Ironwood, using saved policy and
witness/nullifier coverage. Selection unavailability produces `null` membership
and a fixed reason; confirmed or unspent alone never implies spendability.
The output explicitly records that live process locks are unavailable and
migration reservations may be selected. These are SDK snapshot capabilities,
not live unreserved funds, financial authorization or permission to spend.
Correlating a receipt to a quote/refund experiment and obtaining authority to
inspect production keys remain separate steps. See the
[refund evidence recipe](../tests/live/INTEGRATION_REFERENCE.md#shielded-refund-evidence-and-qualification).

## Bootstrap before discovery

After depositing sufficient ZEC into the treasury, run:

```sh
x402_treazury wallet bootstrap --config examples/deployments/managed.local.toml
```

This declares configured groups, runs the production treasury/funding workers,
and waits for the first positive credit on its captured enabled
reserve generations. It closes treasury ownership before exiting, requires
`funding.auto_fund=true`, and respects actual source spendability, safe source closure and qualification allocation counts. Static wallets are
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
Bootstrap allocates its captured vacant claims promptly, without ordinary serving's
30–120-second opportunities or proactive rotation of captured payers. It observes
those original obligations without chasing replacement generations.
Repeating the command resumes current incomplete claims, including eligible recovery. Signed
source work closes after canonical confirmation or buried finite signed expiry
with completed SDK coverage. A confirmed deposit with unresolved settlement waits
for its bound provider outcome; verified source expiry can permit a fresh attempt
under separately revalidated current authority while the old NEAR request remains
observed. A late bound SUCCESS stops a replacement not yet disclosed.
Unpublished calculations recover automatically after exclusive offline SDK
reconstruction. Corrupt or missing immutable records needed to resolve disclosed
work still require scoped operator review; cancellation drains accepted financial
work and preserves journals. An incomplete treasury scan or existing ZEC awaiting spendability keeps bootstrap
waiting under the existing synchronization and funding retry schedules. Only a
capital shortfall assessed from a completed current-owner scan can stop bootstrap
for insufficient funds. Treasury ZEC funding and Base confirmation can take time. The command introduces no new spending limits.
While waiting, it logs unfinished jobs and their sync/submission state once a
minute. The first positive USDC observation satisfies the captured destination
claim even when retained timeout diagnostics remain; exact pending source work
stays owned and continues reconciling during serving.

Ordinary managed `serve` binds listener sockets before starting initial or replacement
funding. MCP HTTP availability follows each endpoint's `wait_for_catalogs` setting
(default true); see [catalog startup](configuration.md#multiple-mcp-ports-from-one-configuration).
Remote catalog discovery and pending paid
relay discovery continue under the retained owner; relay discovery
waits for its assigned payer without allocating wallets itself. Serving does not
wait for bootstrap completeness. Ordinary serving first schedules each group's
independent 30–120-second refill opportunity; restart redraws and downtime or funding
work can extend actual waiting. Exhaustion adds no emergency refill.
`auto_fund=false` pauses new funding and makes
explicit bootstrap fail, while accepted operations remain observed; qualification
funding restrictions retain their existing lifecycle. `catalog warm` never starts
funding: use `wallet bootstrap` first when warming needs a managed paid relay wallet.
Its relay-wallet owner temporarily denies new funding for this command only; it
uses existing funded wallets without changing the deployment's `auto_fund` setting.

## Treasury and managed pools

Build or test the embedded wallet with the shell wrapper, which supplies a
Cargo-managed protoc compiler. First builds may download public proving
parameters. The wallet dependency is pinned upstream at zingolib v8.0.0 commit
`81d799775f11e882896e8710e8af12d6d4d29a41`; no reference checkout is needed.

```sh
make build
make test CARGO_ARGS='--all-targets'
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
The executable additionally emits a safe terminal WARN for failed CLI work;
failed `serve` startup/runtime emits ERROR. Raw cause chains remain separate
CLI output, never copied into tracing fields. Routine library/input validation
does not independently repeat the terminal event.
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

The pinned SDK also requires the sync server to advertise lightwallet protocol
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
local testing. Account 0's shielded funds are used, with `confirmations` applied to zingolib's spendability calculation. Its default
comes from the pinned Zcash SDK's trusted confirmation policy (currently 3).
An explicit override replaces this default; the same threshold applies to all
supported funds. Unconfirmed change is excluded.

Available treasury serving requests one background ZEC scan after listeners bind.
Further scans need enabled unfilled funding claims or validated pending source or
shielding transactions. After success, the next scan uses the SDK consensus block
interval at the next established scanned height (75 seconds before NU7, 25 after),
without another tip query. Failures retain independent 90/180/300-second backoff.
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
and scanned-block count, last successful tip height/time, and any last sync error.
Its human summary also computes confirmed, pending and spendable shielded balances
and confirmed and pending transparent balances from the existing saved SDK
snapshot. JSON retains the existing cached state contract, including its confirmed
and spendable shielded balance fields in zatoshis; these fields do not acquire the
additional human-summary observations. `sync_complete` reports a
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

The application maintains encrypted save/reopen and address continuity coverage
for the embedded Zingolib v8 SDK's current wallet format 45, including current
finite-sync and transparent-scan state. SDK roundtrip coverage preserves a newly
inserted scan target and the next derived address; dependency-owned historical
readers remain upstream implementation.
The application accepts database schema 23 only, for owner opens and read-only
status, identity and qualification inspection. Older and future versions fail
before changing source state; opening them does not convert them. Canonical fresh
stores include current source-disposition and payment-resolution histories. A
schema-23 store interrupted before those empty histories were initialized can
finish their idempotent initialization under exclusive ownership without
rewriting financial records. Existing history and snapshot references remain
preserved, including the guard against automatic disclosure of previously closed
originals. Historical rows do not reopen settled swap workflows. Encrypted
records, identities and journaled transaction bytes retain their formats. Stop
serving and take a complete backup before a software upgrade. Older v6 application
binaries cannot reopen a format-45 SDK snapshot. A pre-upgrade backup is not a safe rollback after new financial work:
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
operation retains its owner until canonical confirmation, verified finite expiry
or bound NEAR SUCCESS; submitted or ambiguous operations are reconciled first,
never automatically rebroadcast. Recovery archives and resets an eligible job
under a new operation ID after proven nondisclosure, verified finite source expiry,
or the required bridge terminal outcome, with current funding authority. Old
accepted NEAR requests keep their exact-operation status observations after that
reset. Run bootstrap after successful standalone recovery to continue
funding.
Recoverable SDK/indexer disagreement remains pending while observations continue;
it does not require a manual reset. A fork beyond the SDK's upstream 100-block rolling
history requires a full rescan; that history window is not consensus finality.
Original transaction identity that cannot be established from surviving
authenticated evidence can still require scoped operator review. Disabled funding
is an ordinary wait after safe source closure; enabling it can resume an eligible
attempt without repairing state. Bound NEAR SUCCESS can settle its exact operation without the
original source bytes; missing bytes are not a blanket recovery veto.
An expired source's old `PENDING_DEPOSIT` request may remain unresolved without a
provider terminal declaration. Automatic funding can continue under current
authority, while cleanup of the affected signer waits; this is not an operator
repair requirement. A quote deadline alone does not prove bridge delivery ended.
Each actual retained-request status lookup emits DEBUG with its original identity
and the unresolved-outcome reason, so these additional requests remain visible.
Malformed submission counters or unknown source-status labels instead keep the
original observe-only: canonical confirmation, finite expiry or bound settlement
can close it without repairing those columns. Strict inspection can still report
the damaged metadata; it never grants disclosure authority.
Runtime recovery also ignores malformed advisory quote/polling health fields;
an unrecognized phase routes through existing recovery rather than new preparation.
Strict full-status inspection can still report these fields as unreadable. Reading
them neither changes financial ownership nor rewrites stored history.
SDK reorg recovery repairs spendability without reopening completed swap work.
A validated bound NEAR SUCCESS closes source work regardless of its last observed
Zcash status. A successful swap awaiting its first positive Base observation keeps
the credit lane; no credit or source confirmation is fabricated. Unfinished source
transactions retain their exact bytes, input owner and finite-expiry recovery.
Already accepted calculations finish journaling; already submitted I/O drains.

Recovery has no per-job retry-count cutoff. Original operation history and
nondisclosure markers survive restart for source and bridge ownership; they do not
block an otherwise permitted replacement. Repeating bootstrap/recover cannot
bypass canonical closure, an unconsumed claim, enabled funding or qualification
authority. Actual spendable
source capacity and nominal output bounds apply to every new preparation. Qualification runs enforce reviewed funding-job
counts and reserve sizing, with actual treasury spendability and operation-specific
SDK transaction validity governing funding. Recovery still requires its separate
qualification authority. Unpublished work
can use a fresh operation only after successful offline reconstruction establishes
that neither outgoing bytes nor factual transaction metadata was committed. A
committed transaction with a lost acknowledgment remains owned. Confirmed deposits with unresolved swap outcomes remain
pending in the automatic provider/credit lanes. Refunds alone grant no replacement
deposit authority. The low-level recovery commands remain
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
reserves under actual spendable funds and SDK-calculated fees. Set it false
to pause automatic funding; reconciliation and treasury sync remain available.
This default applies only to configured managed serving, not wallet commands or
static wallets. Bounded qualification examples explicitly opt out. `near_api_key_env` names the partner credential;
`near_user_session_env` optionally names a separate user-session bearer token.
Tokens stay in environment variables and are sent only to the fixed NEAR origin.
Session issuance/refresh and authenticated access still require qualification.

The coordinator persists a unique Ironwood-capable unified refund address with
only its Orchard-format receiver, along with the SDK snapshot, before obtaining a quote.
Existing transparent refund addresses remain unchanged. Shielded receipts need
ordinary wallet synchronization and confirmations, without an extra shielding
transaction. `wallet shield-refunds` is unsupported and fails without preparing or
broadcasting a transparent-input transaction.
Unsigned quote failures use per-pass backoff. A quote refresh must
prove no accepted calculation or outgoing record exists; it archives its old bindings, records
a durable refresh marker and allocates a fresh operation/refund identity.
Historical unclassified recovery records retain their original obligations. Signed operations never refresh. Insufficient shielded
spendable funds leave the job unprepared; another attempt still needs current authority.

Fresh allocations and unsigned funding quotes do not start Base credit polling.
Actual preparation reads its exact destination once after the SDK provides a valid
spendable proposal. Accepted preparation and signed or previously submitted work
retain their own credit lane across restart, using the recipient acquisition's
existing due time without another credit backoff or jitter. Recurring Base reads
retain their 60-second completion floor. Successful source and NEAR status checks
use `poll_seconds` after completion, plus existing jitter; failures retain the
independent 60-second floor and backoff. With default `poll_seconds = 5`, per-job jitter
is at most five seconds. The floor is not a settlement deadline; canonical
confirmation/expiry and first-positive credit retain their separate stop rules.

`swap_timeout_seconds` is a health-warning threshold, default 1,800 seconds from
the first durable broadcast intent, not an abort deadline. When exceeded,
status shows `timed_out` and pool `funding_degraded`; reconciliation retains its
existing cadence and minute floor. This local threshold does not establish a
provider timeout and never releases funds or stops a funded active wallet.
Each observation kind retains its own runtime error backoff, bounded by 300 seconds
plus existing jitter, independently of quote retry counts. Saved health/`next_poll`
fields report aggregate progress; they do not impose one kind's failure on another
after reopen. Successful observations reset only their own error streak. Status
errors contain fixed actionable categories, never remote bodies, credentials or
deposit addresses. The coordinator resumes phases without repeating ambiguous
submissions, reconciles source confirmation, polls the same deposit, and independently
checks Base credit. Until [funding completion is proven](wallet-rotation.md#funding-and-source-safety),
provider-reported partial deposits enter `recovery_required`; failed/refunded swaps
stay `refund_pending`, distinct from the saved provider `FAILED` or `REFUNDED`.
Current negative outcomes keep NEAR checks and shared Zcash scans until confirmed
receipt at the saved refund receiver; FAILED continues NEAR checks until a final
refund/success outcome. INFO reports safe provider refund details and local receipt
transitions. A bound REFUNDED plus confirmed receiver receipt and closed source
permits a fresh operation/quote under current enabled funding and an unused claim.
Receipt below the original amount or subsequently spent still counts; actual SDK
funds govern preparation. Restart rebuilds this observation from saved notes.
SUCCESS or first positive USDC credit prevents replacement. Later negative reports retain their audit facts without
revoking that completion proof. The SDK owns incoming returns and current spendable inputs. Local refund credits,
principal caps and historical refund revalidation are removed; provider status alone
never proves an on-chain return or authorizes a new deposit.

Wallet transparent receiving and shielding are unsupported. Serving does not
allocate automatic shielding work, and `wallet shield-refunds` fails explicitly.
Raw SDK recognition of an unsupported output does not make it supported capital.
Already accepted transaction bytes retain their canonical reconciliation owner;
this never authorizes a new shielding transaction or a replacement deposit.

Expired funding deposits use automatic
recovery or `wallet recover --config servers.toml`. For an individual shielding
operation, the advanced `wallet recover-expired --config servers.toml --operation-id UUID`
remains available with serving stopped. It never submits or rebroadcasts a
transaction. Expiry recovery requires one completed
canonical tip scan beyond original expiry by the configured confirmation count,
with the same Ready snapshot revision and coverage checked at publication. The
exact authenticated transaction supplies its signed finite expiry. Current old
inputs may be absent, relocated or differently spent; the SDK owns spendability
and automatically releases expired pending reservations. No original-input
inventory or optional indexer report adds a second expiry gate.
An already SDK-confirmed original transaction with
matching computed transaction identity/effects and sufficient depth finishes
without another indexer lookup. Original journaled signed bytes remain unchanged;
valid v5 authorization-data variants do not require an identical SDK encoding.
Recovery closes the original input owner. A replacement funding attempt needs separately
revalidated current authority. Signed bytes and
recovery evidence remain archived; reconciliation grants no automatic rebroadcast.
Missing or conflicting evidence keeps the original ownership.

`treasury/send.rs` retains the public Zingolib proposal through accepted-input
journaling, then uses the backend's calculate-only API with explicit ordinary
NU7-aware expiry. Public proposal cleanup runs after releasing the wallet guard.
It accepts one exact recipient (native Ironwood or a bound NEAR deposit), uses
account 0's Ironwood inputs, rejects multi-step transactions, and verifies the
resulting recipient, amount, actual fee and consensus expiry. Preparation binds
the operation UUID, payment amount and any provider deadline; managed funding also
binds its pool/job authority. No caller fee ceiling or derived input cap is used.
`funding.quote_deadline_seconds` requests a 1,800-second window by default;
the returned quote can differ and remains authoritative. Preparation and the
first submission require an unexpired returned deadline, without an extra local
margin. The requested window is a preference. The chain-specific 1Click watcher
arrival/confirmation and late-refund contract remains unresolved; local broadcast
acceptance does not establish timely delivery. See [transport/contract limits](network-egress.md#remaining-timers-and-budgets).
The funding worker supplies these from validated configuration/quotes,
never from MCP tool arguments.

Preparation selects SDK inputs, checks actual fee/output and permission, journals
exact selected inputs, then calculates signed bytes. Known proposal insufficiency
defers safely. The SDK owner retains live calculations and completed results;
completed bytes and snapshots commit idempotently before another mutation. After
a crash, successful offline reconstruction can close a calculation only when both
outgoing and factual transaction journals are absent. Exact accepted inputs remain
preserved; further work uses a fresh identity and current authority.
Network/chain/expiry preflight precedes durable broadcast intent, so failure there
consumes no submission attempt. The preflighted send path
performs only local expiry checks before sending the saved bytes, without another
unsigned tip read. Failure or cancellation after broadcast intent retains the
original signed transaction. Unpublished calculations recover through the existing
exclusive SDK reconstruction and guarded recovery path; no manual abandonment
is required for an ordinary pre-journal crash.

`rotation/near.rs` sizes each new operation with a zero-slippage dry `EXACT_OUTPUT`
preview for its selected nominal USDC amount, then requests an executable `EXACT_INPUT`
quote with that NEAR-priced ZEC principal fixed. `funding.slippage_bps` (default
500 basis points; range 0–10000, or 0–100%) applies once to the live quote's expected output. Its
`minAmountOut` is an execution floor, distinct from a route's minimum supported
swap size. This does not guarantee smaller swaps, faster execution or no refunds.
The demo configurations explicitly select `funding.confidentiality = "public"`:
no NEAR key, account signup or application commission is needed. This keeps the
Zcash treasury shielded but uses public NEAR settlement. `basic` and `advanced`
remain explicit confidential options requiring separately qualified authentication.
Omitting the setting retains `basic` so existing configurations never silently
lose confidentiality. No mode falls back to another after a quote error.

Run the read-only $5 route demo (public test addresses, no credentials, no wallet,
no deposit allocation or transfer):

```sh
make run CARGO_ARGS='--offline --example quote_near'
```

Cargo's `--offline` disables dependency fetching; the example itself calls NEAR.
It validates the live public quote and prints the required ZEC input. Funded
execution requires an initialized treasury, chain endpoint settings and deliberate
`auto_fund = true`. Public funded execution has bounded
[qualification scope](testing.md#qualification-status);
authenticated confidential execution remains unqualified.

Quote validation checks assets, request bindings, mainnet transparent deposits,
fixed input and output floor. The sizing preview can be retained
immutably within the encrypted quote; it adds no database fields or balance
tracking. Preview expiry does not invalidate an accepted executable quote. Old
accepted `EXACT_OUTPUT` operations keep their original request, response, slippage
and bytes across restart. Timestamp normalization is accepted only for
the same instant. An extended provider deposit deadline never extends the local
send deadline. Provider USD valuations, platform fees and rebates are advisory
metadata, retained without collector/rate/shape restrictions; requests do not add
application fees. Exact transfer and actual alternate-recipient bindings remain enforced. Public quote fixtures and
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

Confirmation uses the original computed transaction identity and the configured
depth from a coherent SDK scan. Observation and broadcast eligibility derive txid
and signed expiry from authenticated original bytes, even when their saved
plaintext projections are stale or malformed. This does not rewrite the journal
or repair missing original transaction, amount, fee or accepted authority.
Indexer disagreement retains observation work;
it does not veto sufficient SDK confirmation. Verified finite expiry or bound
NEAR SUCCESS can close source work under the
[recovery policy](wallet-rotation.md#recovery-and-reconciliation-policy).
An absent lookup or a refund alone grants no replacement authority. This
reconciliation command never creates a replacement transaction; rebroadcast
requires the explicit flag and the original saved bytes and authority.
SIGINT/SIGTERM preserve ambiguous submission state. Ordinary serving can fund
wallets under enabled automatic funding and current authority; standalone
`wallet recover` never prepares or broadcasts a transaction.

The adapters are experimental. Tests cover empty-wallet admission, synthetic gRPC
transport, journal recovery and sync. The optional proving suite exercises the
production preparation path with synthetic Orchard notes belonging to public test
keys, verifies the resulting proof, and restores identical pending bytes and wallet
state after reopening:

```sh
make test CARGO_ARGS='--features zcash-testutils --all-targets'
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
allows that slot to refill. A reserve is a fixed target or an inclusive nominal
range, with slot defaults where configured. An unsigned NEAR preview can raise
the operation-local floor to a bridge minimum and redraw within the remaining
range up to the slot maximum; fixed sizing keeps its deterministic adjustment.
Original allocation floor and identity stay immutable. Source-input and fee
safeguards still apply, with actual spendable treasury funds limiting ordinary
funding. Dry sizing negotiation continues while each bridge-minimum rejection
raises the floor within the slot maximum and the original quote deadline remains
valid. Retries do not renew that deadline.
Other failures do not authorize amount changes or payment retries. Beneficial
extra execution output is not capped by the nominal USDC sizing maximum.

A READY payment atomically retains the prior active, activates its exact payer and
vacates only the consumed claim generation. Selection allocates nothing. The independent
funding owner fills enabled vacancies under current authority; retained wallet count
and retirement never gate refill. Source reservations preserve unresolved liabilities
across days. One pending prepared outgoing operation gates new preparation, and exact
bytes plus its wallet snapshot commit together.

A sync failure before the transaction preparer is invoked leaves the same quote
eligible for a later attempt. Interrupted calculation recovers after successful
offline reconstruction proves no outgoing or factual transaction journal exists.
Durable transaction effects remain owned; an isolated missing byte record does not
authorize retry.
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
resumes its existing UUID, keys and allocation floors.

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
reset only their own error streak. Successful recurring source and provider
status checks use `poll_seconds` after completion, plus jitter; failed checks
retain their independent minute floor and backoff. Base credit keeps its shared
recipient acquisition's 60-second floor. `poll_seconds` also controls local
inventory/owner scheduling. Successful local quote,
preparation and submission handoffs remain immediate. Failed checks retain pending
ownership. Saved estimates never grant payment authority or prohibit an otherwise
authorized signing attempt.

Unsigned `eth_chainId`, `eth_getBlockByNumber`, `eth_getTransactionReceipt` and
constrained `eth_call` reads retain exact payloads and canonical block pins.
Direct mode allows two same-client attempts for transport interruption, with
200 ms backoff. Tor instead uses the selected transport plus `network.tor_isolation_retries` fresh
owner-bound scopes (default 2) for HTTP `400`, `403`, `408`, `429`, `500`, `502`, `503`, `504`
or eligible typed transport
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
final canonical-block checks. Canonical balances, nonces and receipt proofs require
the provider's finalized chain; scalar latest-balance observations remain advisory. Unsupported or unavailable finalized
evidence cannot fall back to a depth-only proof. Partial evidence is discarded. HTTP 403/408/429,
500/502/503/504, availability transport failures and RPC -32005/-32016/-32601
permit fallback. Malformed JSON/envelopes/results, oversized responses and TLS
validation failures may also use configured fallback, always acquiring a complete
fresh view. Wrong chain and conflicting canonical evidence reject the current
view; recoverable historical contradictions retain scoped reconciliation work.
RPC -32602 permits fallback only for `eth_getBlockByNumber`, including an
unsupported finalized tag; it does not grant generic invalid-parameter retries.
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
make run CARGO_ARGS='--example diagnose_credit -- --network-config examples/network/tor.toml --state-dir state/demo --rpc-url "$$BASE_RPC_URL" --rounds 3'
```

The diagnostic sends the state's public EVM addresses to the selected RPC and
prints balances indexed by pool, without printing addresses, and exits unsuccessfully
if any view fails. Use the same trusted
RPC policy as the deployment; choosing another provider discloses those addresses
to that provider.

Managed `mode = "zcash_rotation"` requires `max_api_payment_usdc`. `reserves.small` defaults to a
randomized $2-4 nominal range; an absent `reserves.large` stays disabled, while an
explicit empty `[reserves.large]` opts into $10-25. A slot may instead set
`funding_amount_usdc` for a fixed target (optionally with
`max_funding_amount_usdc` headroom), or both `min_funding_amount_usdc` and
`max_funding_amount_usdc` for an inclusive randomized range. Amounts are positive
exact micro-USDC decimals, and a maximum must be at least its minimum/target.
Actual spendable treasury funds and SDK-calculated transaction fees govern each transfer; there is no separate per-operation ZEC
spending budget. The group payment cap is independent of reserve size; a price
above a nominal funding floor can be attempted by an eligible wallet under that
payment cap.

`max_retirement_remainder_usdc` defaults to `"0.001"` (1,000 atomic USDC units)
independently per managed group; explicit `"0"` requires exact zero. Retired funds
remain owned at their addresses, and positive tolerance has no cumulative
stranded-funds cap. Finite sync applies valid
retirement tolerances only to existing configured groups. Invalid retirement
configuration reports that group's unavailable retirement work while scalar
observations continue; accepted recipients of removed groups remain scalar-only.
See [configuration](configuration.md) for exact syntax, rejected aggregate settings,
reopen-only changes, initial inventory and legacy-state rejection.
Static-wallet payment defaults remain unchanged. Payment admission has no total
timeout. Wallet, treasury and funding configuration accept only current fields;
unknown or obsolete settings fail strict parsing. Money fields are decimal strings;
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
Queued replacements require `funding.auto_fund = true`. Transparent-input
shielding is unsupported, including while serving is stopped. Eligible source-expiry recovery runs automatically
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
restoration restrictions. A rescan cannot reconstruct missing payment
authorizations, disclosed source bytes or consumed initial claims. Unpublished
calculation recovery uses the retained local journals and exclusive reconstructed
owner, not blockchain absence. Missing off-chain history does
not authorize replacement deposits or top-ups. Undeclared filesystem rollback is
not automatically detectable; a code rollback is not a state downgrade.

## Versioned capacity output

Wallet status JSON uses `schema_version: 6` and reports `state_schema_version: 23`.
Version 6 removes the obsolete `legacy_refunds` projection; current refund-address
ownership and shielding remain supported. Qualification lifecycle observations
use their own `schema_version: 2`; the status version is never copied into them.
Bootstrap summaries retain `schema_version: 3`. These are separate from SDK wallet
format 45, instance/encrypted-record/AAD version 1 and backup manifest format 1.
Saved report files remain untouched; none of these observations grants payment,
funding or replay authority.
Each funding job's additive `provider_observed_at` field exposes the existing
database observation timestamp for its last validated provider response, including
pending/unknown responses; it is null if unavailable. It is not provider
`updatedAt`. The additive `selected_output` and `accepted_output` fields expose the
current operation's selected nominal amount and the accepted executable quote's
expected USDC output; both are nominal or expected amounts, not credited balances. These
presentation changes add no persisted fields or schema version.

Unsupported owner opens preserve existing state files before writable
initialization. Stopped-state preflight recovers nonempty journals only in a
disposable copy in a private subdirectory of the state directory; normal
completion and errors remove that copy without changing original database or
journal bytes. Private creation and descriptor checks also cover encryption keys,
mnemonic input, lock files, backup files and temporary SDK/preflight directories.
SQLite sidecars use a parent without access-granting inherited ACLs; existing
sidecars are checked before SQLite is opened.
Clean stopped databases are read in place without creating a copy.
Live read-only SQLite inspection preserves database and existing
financial journal bytes; SQLite may create WAL/SHM coordination files or update
SHM reader metadata. It uses a coherent read transaction so committed WAL data
remains visible while the owner is running.

Bootstrap reports `serving_ready_groups`, `inventory_ready_groups`, and
`captured_completed_groups` separately. The captured command waits only for its
enabled slot generations, including claims vacant when it starts. A completed,
uncredited generation retained by automatic reserve replacement follows its
immediate successor; the command stops following once a generation receives
credit, even if payments consume it before bootstrap finishes. Successful
completion has an empty `incomplete_captured_jobs` list; obstruction or cancellation remains an error with persisted work retained. Current
inventory can become incomplete again after a reserve is consumed.

Status replaces the old pool `deposit_atomic` and `bootstrapped` fields with current
payment authority, retirement tolerance, exact `reserve_claims` and readiness. In
each reserve claim, `target_atomic` is the slot's lower nominal sizing bound and
`maximum_output_atomic` is its authorized nominal ceiling including bridge-minimum
headroom; neither is a credited-balance promise. Original wallet `target`, the
immutable allocation floor, and last saved `balance_estimate_atomic` are advisory
history, not available-capacity authority. Signed
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
canonical evidence. Ordinary serving and finite sync use candidate-scoped canonical
proofs; the scalar estimate alone cannot retire a wallet. Both confirmed and latest
balances must meet the group's tolerance, with their maximum recorded as the
remainder. Never-credited predecessors remain retained for possible late credit
unless each accepted current/archived swap has its bound NEAR terminal outcome
and no unresolved local source work remains. Proven never-disclosed operations
have no bridge obligation. Source confirmation or expiry, completion labels and
unattributed returned coins alone cannot establish delivery closure.
Low-balance traversal rotates payment selection without deleting keys or funds.

Production storage preflight rejects every database schema except 23, including
future versions, without changing source state. `wallet recover` reconciles
supported accepted work; it cannot manufacture missing immutable destination-readiness evidence or convert an
unsupported schema. Preserve complete backups and all accepted obligations; an
old backup cannot safely undo subsequent financial work.
