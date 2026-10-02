# Plan: Rust MCP server with Zcash-funded wallet rotation

## Intended behavior and scope

Implement the service as one Rust process containing the MCP server, x402 paid
HTTP client, named virtual EVM wallet pools, NEAR Intents funding worker, and
one embedded zingolib Zcash treasury. Evolve `rust-prototype/` into this implementation.
The executable owns its wallet state directly; there is no separately running
wallet daemon, Python payment coordinator, or wallet RPC protocol.

A single Zcash treasury funds one or more named virtual EVM wallets. Each virtual
wallet is a durable rotation pool, exposed as an entry in TOML `wallets`, with
its own `deposit_size` in native Base USDC (decimal string, default **5.00**),
per-payment cap, active address and fully funded standby. Managed funding uses
confidential ZEC-to-USDC swaps. Each deployment source may select a virtual
wallet by name using `wallet`, overriding the server's optional default `wallet`.
A template-backed `wallet_assignment` supplies any remaining binding, with
`deployment`, `server`, `source` or `binding` scope. Every server/source binding
must resolve to a named profile; bindings selecting
the same name share that pool. Different virtual wallets never share EVM keys, balances or payment
reservations, even though they draw from the same Zcash treasury.

For each pool, maintain one active address and one fully funded standby. When the active
address cannot cover the next permitted payment, atomically promote the standby,
retire the active address, and allocate/fund a new standby in the background.
The MCP caller continues to see ordinary API tools; funding and signer selection
never become model-controlled tool arguments.

```mermaid
sequenceDiagram
    participant Call as MCP / paid HTTP client
    participant Pool as Rust pool manager
    participant Store as Durable journal
    participant Funding as Embedded Zcash + NEAR worker
    Call->>Call: Receive and validate x402 challenge
    Call->>Pool: Admit payment amount
    Pool->>Pool: Confirm A depleted and B ready
    Pool->>Store: Commit A retired, B active, C key + funding job
    Pool-->>Call: Lease B signer and reserve payment
    Call->>Store: Persist signed authorization identity before submission
    Call->>Call: Submit one paid retry using B
    Funding->>Store: Claim C funding job
    Funding->>Funding: Prepare durable Zcash deposit; submit; reconcile swap
    Funding->>Store: Confirm Base credit; mark C ready
```

Bootstrap funds two distinct addresses before declaring the pool ready: roughly
10 USDC worth of ZEC plus fees per pool at the default deposit size. Across
configured pools, initial funding is twice the sum of their deposit sizes plus
fees; treasury balance and aggregate limits may delay some pools. Each pool
becomes ready independently. Restart restores the same pools and reconciles
their operations, rather than allocating new pairs.

“Empty” means insufficient available funds for the next eligible payment, not
necessarily zero. If A has 0.003 USDC and a call costs 0.014 USDC, B must sign
that call. Leave A's dust recorded; do not transfer it to B or C.

Double buffering avoids waiting for a swap during a normal promotion. It cannot
guarantee continuous service if the promoted wallet empties before replenishment
finishes. Require bounded waiting and explicit degraded status. Choose the
production target using measured peak spend and replenishment latency; never
silently increase the configured target to satisfy a swap minimum.

The first managed release supports **x402 v2 exact EIP-3009 payments in canonical
Base USDC only**. Static Rust mode retains its tested Base EVM v1/v2 exact and
v2 upto paths. SVM rotation, shared wallets across processes, gas/approval
funding, automatic dust sweeping, and arbitrary treasury accounts are outside
this release. The Python launchers remain runnable as compatibility references;
implement this feature in Rust without introducing a parallel Python manager.
Full replacement of Python launchers additionally requires the catalog/static
payment parity checks below; do not claim SVM or custom-handler parity that has
not been implemented.

## Implementation base and build contract

Read the repository's root `AGENTS.md` before implementation. Use these concrete
starting points:

| Existing source | Reuse and required extension |
| --- | --- |
| [catalog.rs](../../rust-prototype/src/catalog.rs) | JSON OpenAPI/digest loading, schema normalization, names, filters, descriptions and argument routing; retain explicit-only text limits |
| [payment.rs](../../rust-prototype/src/payment.rs) | Static/managed dispatch, SDK signing, Base USDC cap, description sanitization and one paid retry; extend structured receipt capture |
| [server.rs](../../rust-prototype/src/server.rs), [main.rs](../../rust-prototype/src/main.rs) | `rmcp` stdio and bearer-gated stateless Streamable HTTP, lazy help, CLI/config loading; extend shutdown to persist wallet outcomes |
| [deployment.rs](../../rust-prototype/src/deployment.rs) | TOML sources/wallets/servers, per-listener tool selection and authentication, shared static/managed profiles, singleton treasury ownership, atomic binding and bounded shutdown |
| [tests](../../rust-prototype/tests/) | Real SDK signatures against local fake sellers; independent Python-generated Keccak/EIP-712 vectors; catalog comparison with Python |
| [store.rs](../../rust-prototype/src/rotation/store.rs) | Encrypted snapshot/key storage, ownership, atomic pool roles and funding jobs, budget/send-gate primitives, atomic admission/promotion and durable authorizations; add full funding phases and funding-policy hashes |
| [treasury adapter](../../rust-prototype/src/treasury/mod.rs), [wallet CLI](../../rust-prototype/src/wallet_cli.rs) | Runnable upstream-pinned offline init/restore, address derivation and pool allocation; managed serving restores this owner; add sync, transaction preparation and backup |
| [managed admission](../../rust-prototype/src/rotation/manager.rs), [Base adapter](../../rust-prototype/src/rotation/base.rs), [settings](../../rust-prototype/src/rotation/config.rs) | Strict TOML profiles, immutable leases, per-pool deadlines, on-demand canonical balance/nonce reconciliation; add background reconciliation and reorg recovery |
| [combined test crate](../../rust-prototype/compat/zingolib/Cargo.toml) | Working embedded-wallet dependency graph and offline wallet restore/address test |
| [vendor directory](../../rust-prototype/vendor/README.md) | Two precisely bounded Alloy manifest patches, upstream source hashes and licenses |

The standalone Rust suite has 52 tests, with two offline treasury tests and two
managed deployment tests under `--features zcash`, plus two fixture utility tests. The combined suite runs 12 shared
payment/MCP/crypto tests and
an offline Zcash wallet creation/address derivation/save/restore test in the
Zcash dependency graph. The catalog comparison covers 726 tool definitions
across 14 committed configs/fixtures. These establish a buildable starting
point. Prepared-byte/snapshot recovery, managed TOML serving, journal-before-send
admission and payment-triggered promotion are implemented and exercised against
local fake Base RPC and seller services. Funding jobs are queued but not executed;
live funding and Zcash transaction construction are not implemented. Payment
reconciliation is on demand and a changed confirmed anchor blocks admission
until explicit recovery; add background reconciliation and reorg recovery.
Zcash transaction construction with spendable notes, proving, broadcast, chain
sync, and the live NEAR route require the tests specified later.

### Dependency pins and patch ownership

Use Rust 1.98.1, the toolchain used to validate the pinned graph. Pin zingolib to
`zingolib_v6.0.0`, commit `c6381534f802b1022041beda4b01c106ad132329`, with
`default-features = false`. Use upstream zingolib directly, not extracted Zimppy
wallet code. The release's recovery/send APIs are the integration target.

The runnable package's root manifest must contain the equivalent of:

```toml
[dependencies]
zingolib = { git = "https://github.com/zingolabs/zingolib", rev = "c6381534f802b1022041beda4b01c106ad132329", default-features = false }
rmcp = { version = "=3.5.0", default-features = false, features = ["server", "transport-io", "transport-streamable-http-server"] }
x402-chain-eip155 = { version = "=2.0.2", features = ["client", "telemetry"] }
x402-reqwest = "=2.0.2"
x402-types = "=2.0.2"

[patch.crates-io]
alloy-primitives = { path = "vendor/alloy-primitives" }
alloy-sol-macro-expander = { path = "vendor/alloy-sol-macro-expander" }
lightwallet-protocol = { git = "https://github.com/zingolabs/lightwallet-protocol-rust", rev = "9bdfdc77eb283f2a3d26c27100cc2ac90148cd93" }
```

Both vendored Alloy crates are version 1.7.3. Their sole changes are the
normalized manifests' `sha3` requirements, from `0.11.0` to **`=0.10.9`**.
Keep upstream Rust source, provenance and license texts intact, and run
`python3 rust-prototype/vendor/verify.py`. This permits SHA-3's `digest 0.10`
to coexist with zingolib's `digest 0.11.0-pre.9`; it does not force a digest API
upgrade on wallet cryptography. Repeat root patches in any separate test
workspace, because Cargo does not inherit dependency-workspace patches.
`lightwallet-protocol` is zingolib's own required patch for Ironwood protobuf
fields. `telemetry` is required to compile this EVM client's upto implementation.

Commit the final integrated `Cargo.lock`. Start from the combined experiment's
working versions: Alloy upper stack 2.5.0, Alloy core 1.7.3 and Zcash primitives
0.30.1. The standalone prototype lock has Alloy upper stack 2.1.1; do not mix
lockfile entries by hand. Resolve and test the final graph as one unit. Review
any additional cryptographic dependency changes explicitly.

The normal delivered build must fetch pinned upstream revisions and build from
committed vendored sources without `reference_repos/`. Verify the git revisions
are fetchable as part of integrating the manifest. If unavailable, report that
build blocker; do not silently substitute a tag or an unpinned local checkout.
The reference-path combined test remains a diagnostic, not the production
wallet dependency.

Provide a build script under `rust-prototype/scripts/` that resolves the existing
Cargo-managed [protoc package](../../rust-prototype/compat/protoc/Cargo.toml),
sets `PROTOC` for the entire Cargo invocation, and builds/tests with `--locked`.
Run Cargo from the package directory so its pinned toolchain is selected.
A dependent crate's build script cannot configure sibling dependencies' protoc.
Zingolib's build downloads/caches public Sapling proving parameters; document the
cache, source, and offline-build prerequisites. Runtime startup must not build
crates or download proving parameters. No Python interpreter is required by the
installed executable; Python is acceptable for development/build verification.

## Runtime architecture and process ownership

One process owns one treasury, all configured active/standby pools, and one state directory.
Acquire an exclusive OS file lock for that directory before unlocking wallet
state or making funding-network requests. A second process using it fails with
`state_in_use`. Distinct processes require separately initialized treasuries and
state directories; do not restore the same treasury seed into multiple active
owners. There is no cross-process treasury sharing in this design.

Use the existing `--meta-config` TOML model in `deployment.rs`: named sources,
wallet profiles, and MCP listeners. Extend wallet profiles with managed rotation
settings. Every `mode = "zcash_rotation"` profile is a separate virtual EVM
wallet with its own pool manager and `deposit_size`; all managed profiles refer
to the singleton process treasury implicitly. Source wallet assignments override
listener defaults; assignments belong in the deployment, never provider files.
Bindings referencing the same profile share that pool, payment admission gate
and spend policy. `--show-config`, `--check` and inventory expose the effective
wallet and source/server field origin for every declared binding.

Automatic assignment is a pure expansion into named managed profiles, implemented
in `rotation/assignment.rs`. Templates use managed wallet settings; explicit
source and server references win over `[wallet_assignment]`. Scope determines
one pool per deployment, server, source, or server/source pair. Generate only
fallbacks for declared bindings, independently of catalog filters. Templates and
unreferenced sources never create pools by themselves. All resolved managed
profiles participate in ownership, budget accounting and startup reconciliation.

Keep generated identity stable within the treasury: `auto_v1_deployment`,
`auto_v1_server_<server>`, `auto_v1_source_<source>`, or
`auto_v1_binding_<server-name-length>_<server>_<source>`. Explicit profiles cannot
use the reserved `auto_v1_` prefix. Do not include template name/settings, file
paths or ordering in identity. Preserve existing allocations on template edits;
scope/name changes may add pools while retaining old state for recovery.
Inspection displays generated names, template/scope provenance, effective profile
settings and distinct managed-pool counts with exact active-plus-standby USDC
targets. These targets describe configuration, not current balances or swap quotes.
Static profiles may coexist and do not use the treasury. Do not replicate the
Zcash wallet or state directory per virtual wallet.

Persist the profile-name-to-pool-UUID mapping before allocation. A profile name
is a durable identifier, not a display label. Adding a name explicitly requests
a new pool; renaming must be an offline operator migration that preserves the
UUID and verifies no collision. Removing a profile disables new admission and
allocations for that pool, but retains its encrypted keys, reservations and
funding/refund reconciliation. Re-adding the same name resumes its saved pool.
Changing a managed profile to static while durable managed state exists must
fail until an explicit operator retirement resolves its liabilities. Never
reassign an existing pool's addresses to another profile.

Each source uses inline TOML settings or a one-level provider `extends` and
retains its base URL, timeout, routing, filters and overrides. Paths resolve
relative to the file declaring them. Providers live in `providers/`; locally
curated catalogs remain OpenAPI JSON. Each listener has its own bearer-token environment reference and
include/exclude tool-name patterns, enforced both at listing and invocation.
Keep source prefixes and reject duplicate tool names. Assemble labeled source
instructions without silent truncation. Preserve secret-free validation and
inventory, all-or-nothing listener binding, and coordinated shutdown. Extend
the shutdown path to persist pending authorizations and funding outcomes before
closing the encrypted store. Preserve the existing single-source invocation.

| Proposed source under `rust-prototype/src/` | Responsibility |
| --- | --- |
| `runtime.rs` | Own task lifetimes, readiness, cancellation, and ordered shutdown |
| `rotation/config.rs` | Strict managed-mode settings, integer/decimal validation |
| `rotation/manager.rs` | Registry of named pools; per-pool admission gates, immutable signer leases, promotion, bootstrap and readiness |
| `rotation/store.rs` | SQLite migrations, ownership lock, encrypted records, budget ledger, atomic transitions |
| `rotation/types.rs` | IDs, states, amounts, structured errors and outcomes |
| `funding/near.rs` | Typed token/quote/status client and validation |
| `funding/base.rs` | Chain identity, block-tagged balances, authorization use, receipt/log evidence |
| `funding/worker.rs` | Fair scheduling across pools, shared treasury budget reservations, persisted funding jobs, refunds and reconciliation |
| `treasury/zingolib.rs` | Embedded `LightClient` owner, serialized wallet mutations and sync control |
| `treasury/persistence.rs` | Encrypted snapshots, durable prepared transaction extraction and recovery |
| `wallet_cli.rs` | Local init/status/reconcile/shield/backup operations |
| `payment.rs` | Static/managed payment mode dispatch, challenge selection, journal-before-send boundary |
| `server.rs`, `main.rs`, `catalog.rs` | Per-catalog routing, lifecycle/config integration and MCP error rendering |

Use Tokio tasks and bounded channels. A treasury actor exclusively owns the
`LightClient` and handles mutation commands; a store worker owns a `rusqlite`
connection on a blocking thread. Choose `rusqlite` with bundled SQLite,
`chacha20poly1305` 0.10, `zeroize`, `uuid`, `fs2` and `rust_decimal` for the new
support code, then lock their compatible versions. Keep NEAR and Base clients
behind injectable Rust traits; fakes must require no wallet files or network.

Use typed in-process commands with a stable `OperationId`, such as
`TreasuryStatus`, `DeriveRefundAddress`, `PrepareDeposit`, `BroadcastPrepared`,
`ReconcileOperation` and `ShieldRefunds`. Mutation commands carry a canonical
parameter hash covering operation kind, network/account, recipient, amount,
limits, pool UUID (or treasury-maintenance purpose) and deadline. Repeating an ID/phase with identical parameters returns
its durable result. A conflicting reuse returns `operation_conflict`.
A dropped oneshot receiver or cancelled MCP request never cancels a committed
funding operation. There is no JSON-RPC wallet socket or exported raw-key API.

The funding coordinator maintains a durable queue keyed by pool and candidate.
Use round-robin scheduling among eligible pools, with at most one current
replenishment candidate per pool and the two bounded bootstrap candidates.
Choose the next pool before acquiring the treasury send gate; do not let one
pool hold it while waiting for a quote, retry delay or Base credit. Obtain or
refresh a quote only when its job is eligible for preparation, so time spent
queued does not silently exhaust quote validity. A broadcast Zcash transaction
holds the shared outgoing gate until safely reconciled; a NEAR swap may continue
polling after its source transaction resolves, allowing another pool's deposit.
Expired or failed jobs remain bound to their original pool. Aggregate budget
reservations are atomic across all pools. Shared-treasury shortages degrade
funding, while already funded pools can continue independently.

Keep payment admission separate from treasury work: a background proof, sync or
swap cannot hold the payment admission lock. Never hold a SQL transaction across
a network request. Treasury snapshot callers may hold a wallet guard while
awaiting the store worker; the store worker must never call back into the
wallet/manager. Do not block Tokio executor threads with SQLite or proof work;
use bounded blocking execution where the upstream API permits moving ownership.
Retain and join task handles; unexpected worker exits set readiness/degraded
state and surface a sanitized error rather than silently restarting sends.

## Durable state, encryption and invariants

Use **one SQLite database** for all pools, treasury snapshots, Zcash operations,
payment authorizations and funding outbox. Enable foreign keys, WAL and
`synchronous=FULL`; version migrations and run them only under exclusive
ownership. Network calls occur outside transactions. One database makes a
post-calculation wallet snapshot, prepared transaction and funding state one
atomic local commit; it does not make a blockchain submission atomic.

| Table | Required fields and constraints |
| --- | --- |
| `instance` | Schema version, stable instance/treasury UUIDs, networks, Base token and Zcash account; reject changed identity/network/account |
| `pools` | UUID, UNIQUE profile name, enabled/retired state, configured deposit size, bootstrap flag, current generation and readiness; retain rows when configuration removes a profile |
| `wallets` | UUID, pool foreign key, per-pool allocation sequence, globally UNIQUE address, encrypted EVM key, immutable allocation target, role, balance block hash/height; UNIQUE `(pool_id, allocation_sequence)` and partial UNIQUE indexes on `pool_id` for ACTIVE and READY roles |
| `treasury_snapshots` | Monotonic revision, encrypted wallet bytes, network/birthday/account metadata, committed sync anchor and derived refund-address range |
| `treasury_operations` | UNIQUE operation ID, phase parameter hashes, pool foreign key (nullable only for treasury maintenance), purpose, encrypted raw transaction(s), txid, expiry, fee, state, last evidence; one unresolved outgoing operation at most |
| `funding_jobs` | Pool and candidate foreign keys with enforced matching ownership, attempt number, UNIQUE treasury operation ID, quote/deposit/refund binding, immutable target and funding-policy snapshot, status, deadline, next poll and bounded retry count; one current job per candidate |
| `payment_attempts` | Logical call ID, pool/wallet foreign keys, catalog/route identity, generation, chosen requirements hash, reserved amount, nonce, payee, validity, phase, receipt/chain evidence; UNIQUE `(wallet_id, nonce)` |
| `budget_entries` | Operation, owning pool or treasury-maintenance purpose, UTC day, reserved/consumed/refunded zatoshis, fee evidence; no duplicate reservation, debit or release |

Store USDC amounts as checked U256 decimal strings, and ZEC amounts as checked
zatoshis compatible with zingolib's `Zatoshis`; never convert through floating
point. Use exact decimal arithmetic for quote USD comparisons. SQLite storage
must not truncate U256 values into signed integers.

Create the state directory mode 0700 and database/key/backup files owner-only.
Use a random 32-byte key in a separate mode-0600 key file. Encrypt EVM keys,
wallet snapshots, signed Zcash transaction bytes and sensitive quote/receipt
payloads with authenticated encryption and fresh nonces. Associated data binds
schema version, record ID/type, network, treasury identity and pool identity for
pool-owned records. Encrypt before
SQLite insertion, so WAL and temporary database pages contain ciphertext for
those fields. Metadata/addresses still reveal relationships; protect the entire
directory and backups. Zeroize temporary secret buffers where practical and
never write plaintext wallet snapshots, seeds or signed authorizations to logs.

Persist allocation/key material before requesting a live quote. Preserve retired
keys and outstanding operations. Encrypted snapshot and journal backups must
come from one consistent SQLite backup, with the key stored separately. A
seed-only restore does not recover EVM keys, outstanding swap bindings, address
ranges or signed-payment liabilities; refuse to treat it as a complete live-pool
recovery. Wrong key, corrupt state or identity mismatch is a hard error, never a
reason to initialize a new wallet.

The following role and promotion invariants apply independently to each pool.
Wallet roles are `ALLOCATED -> FUNDING -> READY -> ACTIVE -> RETIRED`.
Failed/unknown funding retains the candidate and its keys. Healthy steady state
has one ACTIVE and one READY; replenishment has one ACTIVE and one
ALLOCATED/FUNDING candidate. Bootstrap allocates at most two candidates, funds
them, then commits role assignment and readiness.

Promotion uses `BEGIN IMMEDIATE` and compares `(pool_id, expected_generation)`. In one
commit, retire A, activate B, insert C's encrypted key and fixed target, and
insert C's funding outbox job. Generate C's key before that transaction and
discard it if the generation check loses a race. The worker claims the existing
job for that pool; polling timeout never allocates a fourth wallet in that pool. A crash after promotion
must leave enough durable state to launch C's worker exactly once logically.

## Embedded Zcash treasury

Use account 0 on Zcash mainnet in managed production mode. Initialization is an
explicit operator command; serving requires an existing treasury identity.
Secrets come from a protected input file or terminal prompt, never command-line
seed arguments or tool inputs. Persist network, mnemonic birthday and account.
Use operator-configured TLS indexer and submission endpoints; confidential NEAR
execution does not imply a Zcash mixnet connection.

### v6.0.0 adapter API map

These are APIs in the pinned tagged checkout, not Zimppy's dependency version:

| Need | Concrete integration |
| --- | --- |
| Create/restore | `ClientConfig::builder()`, `WalletConfig::MnemonicPhrase` / `NewSeed`, `LightClient::new(config, false)`; `LightClient::from_bytes(bytes, config)` for encrypted snapshot restore |
| Sync | `sync()`, `await_sync()` / `sync_and_await()`, `latest_sync_status()`, `stop_sync()`, `pause_sync_scoped()` in [lightclient/sync.rs](../../reference_repos/zingolib/zingolib/src/lightclient/sync.rs) |
| Spendable balance | Under `client.wallet()` read guard, `shielded_spendable_balance(account_id, false)`; total balance is not sufficient for admission |
| Refund address | `generate_transparent_address(account_id, enforce_no_gap)`; commit derivation/snapshot before returning it |
| Proposal | `propose_send(zip321::TransactionRequest, account_id)` and `data::proposal::total_fee(&proposal)` |
| Calculate | `calculate_stored_proposal()` returns `NonEmpty<TxId>`; records are stored in public `wallet_transactions` with `Calculated` status |
| Exact bytes | `WalletTransaction::transaction()`, `status()` and the returned txid; use `zcash_primitives::transaction::Transaction::write` |
| Persist | `LightWallet::mark_dirty()` / `save()` or `write()` under a wallet guard; restore with `from_bytes` |
| Submit/lookup | `LightClient::transmit_calculated` for reviewed first-submit semantics; configured `zingo_netutils::Indexer::send_transaction` / `get_transaction` for explicit raw submission and reconciliation |
| Shield refunds | `propose_shield(account_id)`, then the same calculation/journal/submission path |

Check the corresponding [send](../../reference_repos/zingolib/zingolib/src/lightclient/send.rs),
[proposal](../../reference_repos/zingolib/zingolib/src/lightclient/propose.rs),
[wallet](../../reference_repos/zingolib/zingolib/src/wallet.rs) and
[wallet serialization](../../reference_repos/zingolib/zingolib/src/wallet/disk.rs)
source when implementing the adapter. Never call `quick_send` or `quick_shield`:
they combine operations across the persistence boundary. Do not launch
`save_task()` or use `flush()` paths that write upstream plaintext wallet files.
Our persistence task owns snapshots during sync, operations and shutdown.
Use a monotonic snapshot revision and the actor's mutation order so an older
background snapshot cannot overwrite a post-calculation snapshot.

Require synced spendable confirmed funds and a recent successful tip check.
Pending change is not available for the next deposit. Start with **one
unconfirmed outgoing treasury operation**, including refund shielding. This
serializes bootstrap deposits and replenishment across all pools but avoids reusing uncertain
inputs. No new treasury operation may proceed while an outgoing transaction's
outcome is unknown. Payment calls using already funded EVM wallets remain usable.

### Prepare, persist, broadcast and recover

1. Reserve an operation ID and funding budget. Check sync freshness and
   spendable funds; acquire the treasury actor's send gate. Any earlier
   unresolved operation blocks preparation.
2. Hold an outer `pause_sync_scoped()` guard. Create the ZIP-321 proposal and
   check recipient, amount, actual source fee and maximum total input. Require
   exactly one deposit transaction to the validated mainnet transparent address.
   Reject multi-step/TEX/OP_RETURN/memo requirements for this route. Clear a
   rejected proposal and restore the prior sync mode.
3. Calculate without transmitting. Keep the indexer configured: indexerless
   calculation deliberately uses a longer epoch-based expiry. The calculation
   method releases its own proposal pause; the outer pause must remain alive
   through extraction and snapshot commit.
4. Extract the exact transaction bytes, ID, expiry and output facts. Under the
   wallet guard, serialize the post-calculation snapshot. Atomically commit its
   encrypted bytes, encrypted raw transaction, `PREPARED` operation, actual fee,
   budget adjustment and funding job phase. Persistence failure prohibits send
   and poisons the in-memory session: restore the last committed snapshot before
   accepting another operation. Release the sync pause after successful commit.
5. Recheck quote deadline and budget, then commit `BROADCAST_REQUESTED` before
   any submission call. Use only the saved bytes. Verify returned transaction
   identity. Record successful submission as `BROADCAST`; timeout, cancellation,
   process death or conflicting response remains `UNKNOWN`. A persisted
   `BROADCAST_REQUESTED` row is possibly broadcast even if no result was saved.
6. Reconcile with sync/indexer evidence by that exact txid. Rebroadcast only
   byte-identical transactions within valid expiry; never regenerate the
   proposal because a response was lost. Review the upstream transmission
   endpoint/retry behavior and use a directly configured raw indexer sender if
   necessary to enforce our endpoint policy. Do not reset upstream transaction
   status or roll back a wallet snapshot merely to invoke `transmit_calculated`
   again. Upstream local failure status cannot release our input reservation.
7. Retain the send gate until confirmed at the configured depth. A prepared
   operation proven never submitted may be cancelled with a durable wallet
   reconciliation. For any possibly submitted operation, absence from one lookup
   is insufficient: expiry/non-inclusion requires a current canonical-chain
   check and no unresolved spend. Quarantine unresolved cases for operator
   recovery. Cancellation of a caller alone cannot release funds.

A process crash before PREPARED restores the previous snapshot; no submission
was possible. A crash after PREPARED uses its saved transaction. A crash during
submission requires lookup/rebroadcast, not another deposit. Apply these same
rules to refund shielding. Output/proposal locks inside zingolib are not a
replacement for the persisted operation journal.

For fresh transparent refunds, persist both derivation and the scanning range.
Prove in regtest that high-index outstanding refund addresses are discovered
after restart. Respect the library's discovery-gap rules; fail explicitly if a
fresh address cannot be tracked instead of silently reusing an address. Once a
refund is confirmed, shield it through an operator-requested, journaled operation
before it contributes to the shielded spendable treasury balance.

## NEAR confidential funding

Use NEAR 1Click's foreign-chain flow with `depositType=ORIGIN_CHAIN`,
`recipientType=DESTINATION_CHAIN` and confidentiality `basic` or `advanced`.
Do not silently switch to public execution or the embedded confidential-balance
flow. Pin fixtures and validate the availability/guarantees of both configured
modes for this route. [Confidential swap documentation](https://docs.near-intents.org/integration/distribution-channels/1click-api/quickstart/confidential-swaps).

The bridge documents transparent Zcash addresses. Send from the shielded
treasury to a validated transparent deposit and supply a supported wallet-owned
transparent refund address. [Chain support](https://docs.near-intents.org/resources/chain-support).
Use `EXACT_OUTPUT`: the requested amount is destination USDC atomic units,
so 5 USDC is `"5000000"`. Deposit the quoted ZEC `amountIn`, including its input
slippage buffer; the source transaction fee is additional. Account for excess
input refunds. Do not fall back to exact input. [Swap types](https://docs.near-intents.org/integration/distribution-channels/1click-api/swap-types).

Keep an optional partner credential in `X-API-Key`, confined to the NEAR client.
Verify access requirements and fees during route qualification.
[Authentication](https://docs.near-intents.org/integration/distribution-channels/1click-api/authentication).
The production API origin is `https://1click.chaindefuser.com`; arbitrary endpoints
are injected in tests rather than exposed as production token/network overrides.

### Funding a candidate

1. Use the persisted candidate key/address and immutable target. Discover token
   IDs with `/v0/tokens`, verifying ZEC mainnet and native Base USDC by chain,
   contract and decimals, not symbol alone. Persist the validated IDs.
2. Derive and durably save a fresh refund address for this funding operation.
   Obtain a quote bound to that address and the candidate recipient.
3. Validate cost, confidentiality, target, deposit destination and expiration;
   reserve the quoted input plus a bounded source-fee allowance before preparing.
4. Prepare, persist and submit the Zcash deposit through the embedded treasury
   protocol above. An optional deposit-hash notification does not transfer funds
   and cannot establish completion.
5. Poll persisted status with bounded exponential backoff and jitter. On swap
   success, independently verify confirmed Base credit and canonical block
   identity. READY requires at least the candidate's target balance, no payment
   reservation, and the correct chain/token; the API's success alone is not enough.

Use this request shape, with locally validated bindings:

```json
{
  "dry": false,
  "swapType": "EXACT_OUTPUT",
  "originAsset": "<validated ZEC asset ID>",
  "destinationAsset": "<validated native Base USDC asset ID>",
  "depositType": "ORIGIN_CHAIN",
  "recipientType": "DESTINATION_CHAIN",
  "refundType": "ORIGIN_CHAIN",
  "amount": "5000000",
  "recipient": "<persisted candidate EVM address>",
  "refundTo": "<persisted treasury transparent address>",
  "confidentiality": "basic",
  "slippageTolerance": 100,
  "deadline": "<UTC quote deadline>"
}
```

Check the response's request echo/bindings, input/output amounts, deadline and
network-valid deposit address. Reject any memo requirement, output below target,
unknown semantics or missing required fields. Use `dry=true` for read-only route
qualification; live quotes supply deposit details. Record public sanitized
fixtures for the schema actually supported.
[Quote API](https://docs.near-intents.org/api-reference/oneclick/request-a-swap-quote).

Persist funding phases `ALLOCATED`, `QUOTED`, `PREPARING`, `PREPARED`,
`BROADCAST_REQUESTED`, `DEPOSIT_PENDING`, `SWAPPING`, `VERIFYING_CREDIT`,
`COMPLETE`, `REFUND_PENDING`, `REFUNDED`, `RECOVERY_REQUIRED`. The treasury
operation's durable result and funding phase advance in one transaction where
both are local. Each new reconciled attempt has a new operation ID and budget
reservation; status retries reuse the current ID.

Map NEAR statuses `PENDING_DEPOSIT`, `KNOWN_DEPOSIT_TX`, `INCOMPLETE_DEPOSIT`,
`PROCESSING`, `SUCCESS`, `REFUNDED`, `FAILED` explicitly. Unknown statuses remain
unresolved. Track by deposit address and memo if applicable to status lookup;
this release rejects deposit routes requiring a memo. Partial deposits do not
trigger another automatic send. A provider timeout or `FAILED` is not proof of
a refund. Reconcile returned funds with the treasury before releasing exposure.
[Status API](https://docs.near-intents.org/api-reference/oneclick/check-swap-execution-status).

Pre-deposit quote failures may retry within the attempt limit. After any possible
broadcast, only reconcile that transaction/swap until conclusively resolved.
A swap timeout sets degraded status and slower polling, not a fresh deposit.
No direct EVM funding fallback, target increase, or retired-address reuse is
allowed. An adequately funded active wallet remains usable during funding errors.

## Challenge-aware payment admission

### Selection and signer lease

Refactor `PaidClient` into static and managed modes. Keep SDK payload construction
and cryptography; implement managed policy/admission in our request pipeline.
Perform async admission outside `PaymentSelector`, which is synchronous, and
pass an immutable leased signer to payload creation.

For a managed request:

1. Build a replayable request with the correct per-catalog routing and timeout.
   Send the ordinary unpaid request. Return a non-402 response without funding
   side effects. An initial 402 is a challenge, never proof of depletion.
2. Decode v2 `PAYMENT-REQUIRED` once into typed/raw-preserving requirements.
   Retain the selected offer's complete JSON. Sanitize only the challenge's
   resource description to the existing 500-character facilitator limit.
3. Filter offers before any rotation. Permit only `exact`, `eip155:8453`, native
   USDC `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` and EIP-3009 transfer
   semantics. Reject Permit2 even when its scheme is `exact`; check
   `assetTransferMethod` and flow metadata. Permit absent/default documented
   EIP-3009 and authorization flow, reject unknown explicit methods, `upfront`,
   escrow and unsupported extensions that change signing/settlement semantics.
   Reject v1, upto, other chains/assets and SVM in managed mode. Mixed challenges
   must select a supported offer rather than reject because the first is unsupported.
4. Apply our integer per-payment cap (default 1 USDC) and configured funding
   target. Select the first acceptable offer in response order. Amounts must be
   positive valid atomic integers. If none qualifies, fail without signature,
   promotion or replenishment. Disabling the amount cap never disables the
   managed network/asset/scheme allowlist or funding budgets.
5. Acquire the pool's bounded async payment gate. Reconcile balances and existing
   authorizations; obtain readiness within the configured admission deadline.
   Check latest usable funds as well as a confirmed snapshot. Stale/unavailable
   RPC data is not evidence of readiness or depletion.
6. If the active wallet can cover the amount after reservations, persist an
   admission reservation and lease its immutable signer/generation. If its
   actual balance suffices but reservations prevent admission, wait/reconcile
   or return `payment_pending`; do not retire a busy wallet. Otherwise refresh
   standby readiness and ensure its available balance covers this specific
   amount, then promote transactionally and reserve against the new active.
   If it cannot cover the call, return a bounded `funding_unavailable` error;
   a target increase must not churn through slots funded to smaller old targets.
7. Construct a per-lease `X402Client` registering only `V2Eip155ExactClient` with
   the leased `Arc<PrivateKeySigner>`. Narrow the challenge passed to
   `make_payment_headers` to the one already validated offer while preserving
   that offer and required resource/extensions fields. Its selector rechecks
   version, asset, network, payee and exact amount. No shared client signer is
   mutated. Sign with the SDK; do not manually recreate EIP-712 payloads.
8. Decode and validate the returned payment header against the lease/offer.
   Persist nonce, payer, payee, amount, validity interval, requirements hash,
   attempt ID and reservation as `POSSIBLY_SUBMITTED` **before** executing the
   signed request. Reject a mismatch or journal failure; no signed bytes may
   leave on either path. Keep signature bytes in memory only.
9. Execute one signed retry. Capture structured receipt/challenge/body/status
   data; reconcile reservations and return the normal MCP tool result. The lease
   remains bound to that generation until the request completes, even if the
   pool later promotes. The payment gate serializes managed handshakes initially;
   cancellation leaves durable uncertainty after the submission boundary.

The prototype captures a payer before the unpaid request in static mode. Managed
mode must choose its lease at admission, after seeing the price and current
pool generation. A bare `replace_payer` call is not a rotation transaction and
cannot replace persistence/reservation logic.

Admission occurs before signing; a crash with only an `ADMITTED` row is safe to
release because the send path requires the later durable submission boundary.
A signature created but not durably recorded is never transmitted. After
`POSSIBLY_SUBMITTED`, process death or cancellation always requires reconciliation.
Persisting only a balance or in-memory mutex is insufficient.

### Settlement evidence and retry policy

Return an internal `PaidResponse` containing HTTP status/body, selected
requirements, attempt/generation IDs and parsed settlement evidence. Use typed
errors such as `unsupported_payment`, `price_limit`, `payment_pending`,
`payment_outcome_unknown`, `wallet_not_ready`, and `funding_unavailable`, mapped
to MCP `isError` with sanitized messages. Preserve final challenge error details
for diagnosis, but never classify arbitrary text containing “funds” as proof.

There is **no automatic post-submission replay** in this release. A final 402,
SDK exception, timeout, malformed receipt or application error after settlement
cannot authorize another signed request using B. A documented insufficient-funds
code corroborated by current Base evidence may retire A/promote B for subsequent
calls; the failed logical call still returns its structured error. Keep at most
one unpaid attempt and one paid attempt per logical call. Avoid stacking SDK
middleware retries around the explicit request loop. Disable redirects on paid
requests, including the retry, to avoid forwarding authorizations elsewhere.

Use Base RPC `eth_chainId`, block numbers/hashes, block-tagged USDC `balanceOf`,
`authorizationState(payer, nonce)`, receipts and relevant logs. Verify chain 8453.
Reconcile balance and authorization use at a common confirmed block. A confirmed
balance may overstate current funds; account for more recent known debits and
reservations before signing. Keep a settled debit reserved until the confirmed
snapshot includes it, then remove the reservation atomically to avoid either
overspending or double-subtracting it. Detect invalidated anchors/reorgs.

An unused authorization is still a liability until its validity ends and a fresh
confirmed chain view proves it unused. Vendor rejection alone does not revoke
it. A receipt alone cannot release exposure without validated chain evidence.
Retired wallets remain under reconciliation but accept no new payments. A
standby's credit must be revalidated at promotion; revoke readiness if reorged.

Managed upto support is deferred: each new wallet would need a confirmed Permit2
allowance, a gas source and metered-settlement accounting, all included in
readiness. Static upto signing remains available with the operator's existing
allowance. Approval failures must never trigger managed wallet churn.

## Budgets, configuration and operator workflow

Managed configuration uses the existing deployment TOML composition. Add optional
singleton `[treasury]` and `[funding]` tables and a tagged `WalletConfig` enum:
`Static { private_key_env, max_price_usd }` or
`ZcashRotation { deposit_size, max_price_usd, max_input_zec, max_fee_bps,
wait_seconds, max_attempts }`. Do not put treasury settings in provider files or
reintroduce global mode/target environment switches. Numeric money values are
strings parsed directly to atomic integers. Secret/endpoint credentials use
explicit environment-variable references, populated from the process environment
or `--env-file`; meta-config fields retain their existing deterministic precedence.

The following schema is accepted by the managed TOML runtime. Funding-worker
settings are validated but no Zcash/NEAR transfer is performed yet. The example's
ZEC limits are illustrative operator limits,
not an estimate of the ZEC needed for either deposit size:

```toml
version = 1

[treasury]
id = "11111111-1111-4111-8111-111111111111" # Replace with wallet init's UUID.
state_dir = "./state/treasury"
key_file = "./secrets/treasury.key"
indexer_url_env = "ZCASH_INDEXER_URL"
submission_url_env = "ZCASH_SUBMISSION_URL"
daily_input_zec = "0.10"
shield_max_fee_zec = "0.001"

[funding]
base_rpc_url_env = "BASE_RPC_URL"
confidentiality = "basic"
# near_api_key_env = "NEAR_API_KEY"

[wallets.research]
mode = "zcash_rotation"
deposit_size = "5.00"
max_price_usd = "1.00"
max_input_zec = "0.02"
max_fee_bps = 500

[wallets.social]
mode = "zcash_rotation"
deposit_size = "10.00"
max_price_usd = "1.00"
max_input_zec = "0.04"
max_fee_bps = 500

[sources.socialfetch]
extends = "../../providers/socialfetch.toml"
timeout = 90

[servers.research]
listen = "127.0.0.1:8000"
bearer_token_env = "RESEARCH_MCP_TOKEN"
wallet = "research"
sources = ["socialfetch"]
tags = ["LinkedIn"]

[servers.social]
listen = "127.0.0.1:8001"
bearer_token_env = "SOCIAL_MCP_TOKEN"
wallet = "social"
sources = ["socialfetch"]
tags = ["Twitter", "YouTube"]
```

Each `wallets.<name>` managed profile accepts:

| Field | Behavior |
| --- | --- |
| `mode` | Required `zcash_rotation`; static profiles retain `mode = "static"` |
| `deposit_size` | Positive USDC decimal string; default `"5.00"`, at most 6 decimal places; immutable target captured at each new address allocation |
| `max_price_usd` | Independent per-payment cap; default `"1.00"`; retain static cap syntax but never bypass managed chain/asset/target checks |
| `max_input_zec` | Required positive per-deposit input-plus-source-fee hard cap |
| `max_fee_bps` | Required integer 0..10000 for quote-implied USD overhead |
| `wait_seconds` | Default 30, range 1–3600; total admission/readiness deadline including this pool's payment-gate wait |
| `max_attempts` | Default 3 reconciled funding attempts per candidate, not status polls |

The singleton treasury and shared funding fields are:

| Field | Behavior |
| --- | --- |
| `treasury.id` | Required initialized UUID; serving rejects a different identity |
| `treasury.state_dir` | Required persistent directory for this treasury and all pools; one process owner |
| `treasury.key_file` | Required protected 32-byte encryption key file; distinct from the database |
| `treasury.indexer_url_env` | Required environment reference to a trusted TLS sync/lookup endpoint |
| `treasury.submission_url_env` | Required environment reference to a trusted TLS raw submission endpoint; may name the indexer variable |
| `treasury.confirmations` | Default 3, positive integer |
| `treasury.max_sync_age_seconds` | Default 300 since successful tip reconciliation |
| `treasury.shield_max_fee_zec` | Required positive fee cap for operator-requested refund shielding |
| `treasury.daily_input_zec` | Required positive aggregate input/fee budget across every pool and treasury-maintenance operation |
| `funding.base_rpc_url_env` | Required environment reference to trusted Base RPC; check chain ID 8453 |
| `funding.base_confirmations` | Default 12, positive depth; not a guarantee of L1 finality |
| `funding.base_max_block_age_seconds` | Default 120; reject an older latest RPC block before admission/promotion |
| `funding.confidentiality` | Default `basic` or explicit `advanced`; public forbidden |
| `funding.near_api_key_env` | Optional environment reference to a server-side partner credential |
| `funding.slippage_bps` | Default 100; integer 0..1000 |
| `funding.poll_seconds` | Default 5; backoff with jitter capped at 60 seconds |
| `funding.swap_timeout_seconds` | Default 1800; mark the affected pool degraded and continue slower reconciliation on expiry |
| `funding.quote_deadline_seconds` | Default 1800; require at least 300 seconds remaining before prepare and before first broadcast |

Require `[treasury]` and `[funding]` whenever any managed pool is configured.
Reject unknown fields, nonfinite amounts, negatives, zero where positive is
required, fractional atomic units, overprecision and overflow. Require all risk
limits for managed serving. Reject `private_key_env` on a managed profile and
managed-only fields on a static profile; unrelated static profiles and their
key environment variables remain valid. Only effective static profiles used by selected tools load signers. All declared managed profiles explicitly request durable pools and
bootstrap, even if no listener currently selects them. Inspection/discovery
commands never create pools or initialize a treasury.

Resolve state and key paths relative to the deployment file declaring them.
`--show-config` displays profile settings, default/origin information and only
environment reference names, never endpoint credentials or key contents.
`--check` validates shape, references and amount syntax without unlocking wallet
state or making funding requests. Static-only serving does not initialize,
unlock, fund or sync a treasury. Fix production Zcash account 0/mainnet, Base
8453 and canonical USDC. Regtest requires explicit separate fixture configuration
and state; do not expose production network overrides.

Compute quote overhead with exact decimal arithmetic:
`max(0, (amountInUsd - target_usdc) / target_usdc) * 10000`.
This includes the quote's slippage buffer but excludes Zcash fees; vendor USD
values are not an independent oracle. Missing required USD fields fail the check.
Hard zatoshi limits include actual source fees and remain authoritative even
when the x402 amount cap is disabled. Reserve input plus maximum permitted fee
before preparation, tighten to actual input/fee at PREPARED, and preserve unknown
exposure across UTC-day rollover. For a new reservation require today's consumed
costs plus all outstanding reservations plus the new reservation to fit the
budget. Do not reset unresolved liabilities at midnight. Only confirmed refunds
reduce consumed exposure, once; never count expected refunds as available funds.
Changing a profile's `deposit_size` affects only that pool's future allocations,
never top-ups of existing addresses. Persist the target with the candidate and
job so queued work retains its original amount. Other profiles are unaffected.
A shared treasury budget is an aggregate bound, not a guarantee that every pool
can fill: fair scheduling prevents queue starvation but cannot create funds.

Bound network operations (initial connect timeout 15 seconds, request timeout
30 seconds), honor `Retry-After`, and bound queues and worker concurrency. Route
calls retain their configured longer vendor timeouts. Treat deadline expiry or
operator limits as recoverable degraded funding, not permission to spend more.

Add `wallet` subcommands to the existing Rust executable, retaining root serve
flags for ordinary use. Commands accept `--meta-config` to locate the singleton
treasury; pool-specific diagnostics use `--wallet <profile-name>`. State identity
and existing journals, not listener selections, determine recovery scope:

- `wallet init`: explicitly create/import a treasury into new state, recording
  birthday/network and generating an encrypted snapshot; refuse overwrite.
  Return the treasury UUID and deposit address, never the seed. Take an import
  seed from a protected file or prompt. No swap is initiated by initialization.
- `wallet status`: read committed SQLite state without acquiring write ownership,
  unlocking keys or making network requests. Report its snapshot age, active/
  standby readiness, balances, pending operations and budget for each pool, plus
  the shared treasury balance, aggregate reservations and funding queue. It may run while
  serving and must label its data as the last persisted observation.
- `wallet rename --wallet OLD --new-name NEW`: requires exclusive ownership and
  collision checks; updates the durable profile mapping while preserving pool
  UUID, keys and operations. Require matching deployment configuration before serving.
- `wallet retire --wallet NAME`: requires exclusive ownership, resolves all
  liabilities/funding operations before marking a pool retired; retains keys and
  audit history. Never sweeps dust or reallocates addresses implicitly.
- `wallet reconcile --operation-id ...`: requires the serving process stopped
  and exclusive ownership; unlock and reconcile/rebroadcast only the saved
  operation. No replacement transfer is implicit.
- `wallet shield-refunds`: also requires exclusive ownership and uses the same
  durable send path and configured fee/budget limits.
- `wallet backup` / `wallet restore`: operate under exclusive ownership on a
  consistent encrypted database backup, verifying key/network/identity; refuse
  destination overwrite. Explain separate key backup and address-range metadata.

Use the same runtime components for serving and operator commands; do not build
a second wallet implementation. No treasury administration is exposed as an MCP
tool or public HTTP endpoint. Default logs use opaque IDs and stderr; reveal
addresses only in explicit local diagnostics. Never log seeds, keys, partner
credentials, raw signed transactions or authorization headers.

Validate settings per command: offline init/backup/restore need state, encryption
and identity inputs, not NEAR credentials or live endpoints. Read-only status
needs only the state directory. Serving and transaction-capable reconciliation
require the full managed configuration and funding limits.

## Startup, shutdown and catalog compatibility

Parse configuration and validate HTTP bearer auth before starting wallet tasks.
For managed serving: acquire state ownership, validate schema/identity, unlock
state, restore the single treasury, reconcile the named profile/pool mappings,
create one manager per enabled pool, and launch shared funding reconciliation.
Initialize/list-tools may respond while funding is not ready; paid admission
waits only within its deadline. Local help tools do not wait for treasury funds.
Discovery flags must skip all wallet/key/funding initialization and probing;
a remote spec fetch is still needed if the operator explicitly selects one.

Restart reconciles every pool, including disabled pools with pending work, and
shared treasury operations before
allowing new sends, refreshes Base anchors, and resumes existing jobs. Recovery
must find an already credited candidate and mark it ready, not fund it again.
An unresolved treasury send blocks new treasury work but need not block calls
using independently verified active USDC. A pool-local shortfall or ambiguous payment blocks only that pool's affected
admissions; shared state corruption blocks all managed payments until explicit
recovery. A bad treasury sync stops funding but need not stop independently
verified Base spending. Static profiles remain independent of managed readiness.

Both MCP transports own the same `Runtime` lifecycle. On shutdown, stop new
admission and new funding preparation, drain bounded in-flight work, stop/await
sync, commit the latest encrypted snapshot and pending outcomes, then close the
store and release ownership last. Persist cancellation ambiguity before aborting
workers on the shutdown deadline. Never release a reservation merely because a
task was cancelled. Keep bearer authentication before MCP dispatch, stateless
HTTP JSON responses and stderr-only logging. Configure HTTP allowed hosts
explicitly when supporting non-loopback deployment; do not silently disable
host checks while adding transport integration.

Keep existing TOML provider composition and JSON API catalogs usable. Extend differential fixtures
to cover all intended production catalogs, free/local help calls, absolute URLs,
per-catalog bases/timeouts, duplicate names, body/query collisions, schema bounds,
description overrides and tag selection. Preserve the Rust correction that
renamed body arguments map back to original body keys and required fields.
No implicit description or response truncation is permitted. Preserve the
implemented pricing cache behavior and qualify any remaining custom handlers
before claiming full launcher parity;
unimplemented config behavior must be explicit rather than silently ignored.
Python compatibility tests remain useful even though runtime coordination is Rust.

## Privacy and availability constraints

The intended benefit is reducing public linkage between successive Base payer
addresses. All payments within one address remain linkable. Fixed funding
amounts, deposit/withdrawal timing, refund addresses, RPC queries, provider logs
and application/network identifiers can reveal relationships. The local state
knows the full mapping. Confidentiality selection is not a universal anonymity
guarantee.

Never directly transfer retired funds to another pool address. Persist a fresh
supported refund address per attempt and keep operation records private.
Randomized amounts/timing, cross-profile EVM address sharing, mixnet transport and embedded Intents
balances need separate designs; do not introduce them by changing the target or
route silently. Publish measured replenishment latency and the capital required
for two addresses per pool plus all outstanding deposits/refunds.

## Implementation sequence and acceptance criteria

Each step must leave runnable offline tests. Do not gate offline implementation
on a live deposit. Mainnet transfers require explicit authorization and a bounded
spend; none is authorized by this plan alone.

1. **Integrate the build and runtime interfaces.** Fetch the pinned zingolib and
   protocol revisions, carry the Alloy patch into the executable root, lock the
   unified graph, provide protoc/build tooling, and run crypto/wallet regressions.
   Add config/types and static/managed dispatch without funding side effects.
2. **Implement encrypted state and recovery.** Add migrations, exclusive
   ownership, budget ledger, snapshot ordering, role transitions and outbox.
   Use fake treasury/Base adapters to prove per-pool crash boundaries, independent
   admission, fair shared-treasury scheduling and aggregate budget concurrency.
3. **Implement the embedded treasury.** Add offline init/restore and status,
   controlled sync, durable preparation/raw extraction, endpoint-constrained
   submission and reconciliation. Exercise shielded-to-transparent deposits,
   refunds and exact-byte recovery in isolated regtest.
4. **Implement NEAR funding and qualification.** Capture public token/quote/status
   fixtures and dry confidential exact-output quotes around the configured target.
   Verify route minimums, fees, refund compatibility, authentication and deadline
   semantics. Wire the durable worker and Base confirmation gate using fakes.
   Failure to quote 5 USDC is a reported route constraint, not a target change.
5. **Wire challenge-aware admission and transports.** Implement leased signers,
   journal-before-send, confirmed authorization reconciliation, promotion and
   error classification. Exercise stdio/HTTP and multi-listener/multi-source routing with
   the same runtime. Keep one signed retry and preserve static behavior.
6. **Finish operation and release qualification.** Provide backup/recovery and
   status commands, configuration examples, multi-catalog setup, dependency-patch
   maintenance and measured replenishment behavior. Run an explicitly authorized
   small live swap only after regtest recovery passes. Document unverified live
   conditions; do not call a compile/test result a production funding validation.

Use fake time, deterministic throwaway EVM keys, temporary state directories and
injected transports. Tests must never load a developer's `.env`, wallet key
files, real seed or mainnet RPC by default. Build-time proving-parameter downloads
are separate from runtime tests. Regtest-only tests are separately gated and use
local node/indexer infrastructure, with exact setup/invocation documented in
`rust-prototype/README.md`.

| Scenario | Required assertion |
| --- | --- |
| Bootstrap target 5 USDC | Exactly two distinct encrypted keys/candidates; paid readiness only after both verified credits; restart resumes them |
| A=0.003, B=5, eligible call=0.014 USDC | B signs, A retires, exactly one C job exists; block C's swap and prove B's call still completes |
| Concurrent callers see depleted A | One promotion/key/job; calls lease the committed generation without duplicate reservations |
| Target 5, challenge 6, or cap rejected | No signature, promotion or funding job; raising spend cap alone cannot cause refill loops |
| Mixed EIP-3009/Permit2 offers | EIP-3009 selected; unsupported-only/v1/upto/SVM/foreign-asset challenge has no managed side effects |
| Funds exist but are reserved | Bounded wait or `payment_pending`, no rotation churn |
| Journal write fails after SDK signature creation | No signed HTTP request is sent |
| Seller timeout/final rejection/malformed receipt | No second paid attempt; persisted authorization remains reserved until verified release |
| Settlement debit becomes confirmed | Reservation released once, without double-subtracting balance; invalidated block triggers reconciliation |
| Candidate success reported without Base credit | Remains VERIFYING_CREDIT; a reorg before promotion revokes readiness |
| Crash around promotion/outbox claim | Existing C job resumes; no missing key and no duplicate deposit |
| Crash before/after PREPARED commit | No send before commit; committed snapshot and raw tx restored together |
| Crash during broadcast or submission timeout | Recover identical txid/bytes; no recalculation; treasury gate stays closed until resolved |
| Caller cancellation during preparation/submission | Actor/journal retains operation; dropped waiter cannot erase exposure |
| Upstream marks an outgoing transaction failed | Journal still requires chain reconciliation; no conflicting new send/shield |
| Partial deposit, refund, expired quote, exhausted budget | Bounded degraded status; no top-up/fallback; active wallet remains usable when funded |
| UTC rollover with unresolved input | Liability carried forward; new funding cannot exceed aggregate limit |
| High-index refund address and restart | Regtest sync discovers it; shielding uses durable operation path |
| Wallet round-trip and wrong-key restore | Addresses/next derivation preserved; no plaintext snapshot; wrong key never creates replacement state |
| Two owners / multiple catalogs | Second owner rejected before networking; servers sharing a profile use one pool; different profiles use distinct keys and admission gates |
| Two pools with deposit sizes 5 and 10 | Four distinct bootstrap addresses; each gets its own immutable target; repeat restart creates no new pair |
| Simultaneous depletion across pools | One promotion/job per pool; funded calls progress while shared treasury sends serialize; no duplicate Zcash input use |
| Aggregate treasury budget race | Sum of both pools' reservations and maintenance fees stays within one limit, including across UTC rollover |
| One pool degraded or repeatedly retrying | Other funded pool serves; eligible refill jobs get fair turns; shared outgoing ambiguity blocks new sends everywhere |
| Profile removal, re-addition and rename | No key loss or fresh pool on re-add; pending work reconciles while disabled; explicit rename preserves UUID |
| Change one pool's deposit_size | Only future allocations in that pool change; active/standby balances and queued jobs are never topped up or retargeted |
| Listing/help/HTTP authentication | No funding during discovery; lazy help independent of funded readiness; unauthorized HTTP rejected before tool work |
| Graceful shutdown and forced termination | Encrypted snapshot revision consistent; unknown operations survive restart |

Run the existing baseline commands while integrating:

```bash
python3 rust-prototype/vendor/verify.py
cargo test --locked --manifest-path rust-prototype/Cargo.toml
cargo clippy --locked --manifest-path rust-prototype/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path rust-prototype/Cargo.toml --check
python3 rust-prototype/compat/check.py
uv sync
.venv/bin/python rust-prototype/tests/compatibility.py
uv run pytest tests/ -q
```

Once zingolib is in the executable graph, route Cargo build/test/clippy commands
through the documented protoc-providing script, retaining `--locked`. Add new
rotation/store/treasury/funding Rust tests to the default offline suite and keep
gated regtest/mainnet suites distinct. Run stdio and HTTP smokes against local
fake payment/funding services through initialization, tool listing/calling and
shutdown. Keep README and AGENTS.md descriptions aligned with implemented
behavior, and gitignore database/WAL/key/backup artifacts as well as build output.
