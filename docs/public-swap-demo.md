# Bounded public-swap qualification

Run the offline application and isolated consensus tests first; see the
[lifecycle matrix](../tests/LIFECYCLE.md). Mainnet execution is a separate,
operator-funded test, and neither demo deployment enables auto-funding by
default.

For the broader multi-provider qualification runner, see
[qualification status](testing.md#qualification-status) and the
[live integration runbook](../tests/live/INTEGRATION.md), which use a separate
registry and budget file.

## Choose the qualification scope

| Funds supplied | What the test verifies |
| --- | --- |
| A test EVM key holding USDC on Base | Live seller acceptance of the Rust x402 handshake (does not exercise Zcash, NEAR, or managed rotation). |
| ZEC sent to the treasury's shielded receive address | Public NEAR ZEC-to-Base-USDC funding of the small reserve slot, followed by one live x402 payment. Promotion and refill can be tested separately with additional approved paid calls. |

Pass keys, bearer tokens, and credential-bearing RPC URLs through environment
variables in the launching process. Never paste spending keys or mnemonics into
chat, TOML files, logs, or CLI arguments. Treasury funding needs only the
printed shielded receive address.

## No-spend preparation

From the repository root:

```sh
make build-dev CARGO_ARGS='--offline'
target/debug/x402_treazury config show \
  --config tests/live/deployments/public-swap-demo.toml
target/debug/x402_treazury config check \
  --config tests/live/deployments/public-swap-demo.toml
target/debug/x402_treazury catalog tools \
  --config tests/live/deployments/public-swap-demo.toml
```

This deployment uses a committed request schema exposing only
`socialfetch_twitter_profiles_handle`. These checks read no wallet keys and
make no vendor requests.

To inspect a live public NEAR route without credentials or funds, run:

```sh
make run CARGO_ARGS='--offline --example quote_near'
```

Cargo's `--offline` skips dependency downloads; the example still sends a live
dry NEAR quote request using public test addresses without allocating a deposit
or moving funds. NEAR platform and route fees appear in provider metadata, and
no application commission or local USD overhead gate is applied.

**Stop here until the operator supplies funds and approves live execution.**

## Treasury-funded test

1. **Set environment variables:** Export `PUBLIC_DEMO_MCP_TOKEN`.
   `BASE_RPC_URL` is optional (omitting it uses the default Base RPC failover
   list). Zcash endpoints default to `https://zec.rocks:443` and can be
   overridden in `[treasury]`.

2. **Copy the deployment and initialize the treasury:**

   ```sh
   cp tests/live/deployments/public-swap-demo.toml \
     tests/live/deployments/public-swap-demo.local.toml
   target/debug/x402_treazury wallet init \
     --config tests/live/deployments/public-swap-demo.local.toml
   target/debug/x402_treazury wallet addresses \
     --config tests/live/deployments/public-swap-demo.local.toml
   ```

   `wallet init` looks up the mainnet tip, rewinds 100 blocks for the wallet
   birthday, and creates `state/public-demo/wallet.key` inside the owner-only
   state directory (set `treasury.key_file` to store the key outside that
   directory). The treasury ID is stored in state and discovered automatically.
   Anyone who can read the state directory has both state and key.

   - If the treasury already exists, skip `wallet init` and run
     `wallet addresses` (`wallet init` never overwrites an existing treasury).
   - Pass `--birthday HEIGHT` to initialize offline, or
     `--mnemonic-file FILE --birthday HEIGHT` to import a seed (using a height
     at or before the wallet's first use). A failed lookup creates no state.
   - Keep `auto_fund = false` until step 6, and keep
     `max_retirement_remainder_usdc` at zero (pausing auto-funding does not
     disable automatic retirement of zero-balance wallets with cleared
     obligations).
   - To route through Tor instead of direct networking, copy the `[network]`
     table from `examples/network/tor.toml` into the local deployment before
     initializing or syncing, and start Tor first.

3. **Back up the treasury before funding:**

   ```sh
   mkdir -p secrets
   chmod 700 secrets
   target/debug/x402_treazury wallet backup \
     --config tests/live/deployments/public-swap-demo.local.toml \
     --destination secrets/public-demo-backup
   ```

   The destination directory must not exist. Protect the backup as spending
   material, and never run the original and restored copies concurrently.

4. **Fund the shielded receive address:** Give the operator the printed
   shielded receive address and agree on a ZEC amount from a fresh quote for
   one $2-3 nominal USDC deposit, plus Zcash network fees and any planned
   replacement. Spendable treasury ZEC and SDK-calculated transaction fees
   govern the transfer; there is no separate per-transfer ZEC budget or
   USD overhead gate.

5. **Sync and verify spendable ZEC:**

   ```sh
   target/debug/x402_treazury wallet sync \
     --config tests/live/deployments/public-swap-demo.local.toml
   target/debug/x402_treazury wallet status \
     --config tests/live/deployments/public-swap-demo.local.toml
   ```

   Confirm a fresh sync with enough confirmed shielded spendable balance
   (pending deposits and unconfirmed change are not spendable). Close wallet
   CLI commands before starting `serve`.

6. **Enable funding and start serving:** After reviewing the resolved config
   and spend caps, set `funding.auto_fund = true` in the local deployment copy
   and start `serve`:

   ```sh
   target/debug/x402_treazury serve \
     --config tests/live/deployments/public-swap-demo.local.toml
   ```

   Start without MCP clients connected. Watch `wallet status` for the small-slot
   funding job, source confirmations, and Base USDC credit. Wait for one `READY`
   wallet with a positive Base USDC balance (`NEAR SUCCESS` alone is not enough,
   and readiness does not require the exact nominal target).

7. **Run one approved paid call:** Connect an MCP client to
   `http://127.0.0.1:8000/mcp` with `PUBLIC_DEMO_MCP_TOKEN`, call `initialize`
   and `tools/list`, then make **one** lookup with
   `socialfetch_twitter_profiles_handle` and a `handle` argument. The
   per-payment cap is **$0.05**. Record seller acceptance and verify that the
   payment reconciles on Base afterward.

8. **Stop and back up again:** Stop with `SIGINT` or `SIGTERM`, inspect
   `wallet status`, and create a fresh backup. Do not leave automated clients
   or unattended funding running after the test.

This single-call test verifies funding and one paid call; it does not spend
down the wallet to trigger rotation (offline lifecycle tests cover promotion
and refill). To test rotation on mainnet, separately approve useful paid calls
and their total cost, then observe depletion, promotion to the `READY` wallet,
and one replacement funding job. Never edit balances, force readiness, export
managed keys, or fabricate chain state.

## Funded EVM-key alternative

Use `tests/live/deployments/public-payment-demo.toml` with
`PUBLIC_DEMO_EVM_PRIVATE_KEY` and `PUBLIC_DEMO_MCP_TOKEN` exported. The key
must hold Base USDC (this exact EIP-3009 call does not need Permit2 approval).
Run `config show` and `catalog tools`, start `serve`, make the single approved
call (capped at $0.05), and stop immediately. This test verifies only the x402
handshake, not treasury funding.

## Failure and recovery

| Observation | Action |
| --- | --- |
| Quote rejected, route unavailable, or rate limited | Inspect the sanitized job error. Bounded retries with backoff run automatically; do not relax caps or switch confidentiality modes. |
| Insufficient spendable shielded balance | Active EVM wallets stay usable. Fund and sync under operator approval; waiting creates no signed transaction. |
| Swap timeout | The health threshold marks pool status degraded while reconciliation keeps polling the same operation. Do not issue a second deposit. |
| Ambiguous source submission | Stop `serve` and run `wallet reconcile --config FILE --operation-id UUID`. Pass `--rebroadcast` only to resubmit the same signed bytes before expiry. |
| Expired source transaction | Run `wallet recover-expired`, which requires synced unspent-input evidence. If recovery refuses, keep the reservation and never delete its journal. |
| Failed or refunded swap | `FAILED` and `REFUNDED` are provider statuses, not on-chain proof; wait for the SDK to observe any Ironwood return output and determine spendability. Never shield transparent inputs or issue a replacement deposit based solely on provider status. |
| Interrupted preparation without signed bytes | Run `wallet recover-unprepared` to archive old bindings and open a fresh operation ID. It refuses any operation that already has signed bytes. |
| Reorg affecting recorded spend, refund, or expiry | Treasury readiness fails closed. Keep state intact and investigate; never reset accounting or run a restored backup alongside the original. |

Keep private diagnostics and backups local, and report confidentiality,
routing, seller acceptance, and spend results based only on what the run
observed.
