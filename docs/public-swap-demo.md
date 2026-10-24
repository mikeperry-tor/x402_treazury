# Bounded public-swap qualification

The offline application and isolated consensus tests are the prerequisite; see
[the lifecycle matrix](../tests/LIFECYCLE.md). Mainnet execution is a separate,
explicitly funded test. Neither example starts funding by default.

For the broader two-pool provider qualification, use the
[qualification scope](testing.md#qualification-status) and
[funded integration runner](../tests/live/INTEGRATION.md). Its durable registry and budgets
are separate from this single-call demonstration.

## Choose the qualification scope

| Funds supplied | What the test establishes |
| --- | --- |
| A test EVM key holding USDC on Base | Real seller acceptance of the Rust x402 handshake. It does not exercise Zcash, NEAR or managed rotation. |
| ZEC sent to the treasury's shielded receive address | Public NEAR ZEC → Base USDC funding of both managed wallet slots, then a real x402 payment. Promotion/refill can be qualified separately after approving additional paid calls. |

Put keys, bearer tokens and credential-bearing RPC URLs in the launching process's
environment. Do not paste a spending key or mnemonic into chat, TOML, logs or CLI
arguments. Treasury funding needs only its shielded receive address; no seed or
EVM private key needs to be shared.

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

The example uses a committed request schema and exposes
only `socialfetch_twitter_profiles_handle`. Inspection loads no wallet secrets and
makes no vendor requests. These are configuration checks, not a promise that the
live seller's current schema or price still matches the fixture.

Optionally inspect a current public route without credentials or funds:

```sh
make run CARGO_ARGS='--offline --example quote_near'
```

Cargo's `--offline` only disables dependency fetching. This example makes a real
**dry** NEAR quote request using public test addresses; it does not allocate a
deposit or send funds. No application commission is configured. NEAR platform and
route fees are subject to the validated quote limits.

**Stop here until the operator supplies funds and approves live execution.**
No funded mainnet swap or paid seller call is covered by the offline qualification.

## Treasury-funded test

1. Supply `PUBLIC_DEMO_MCP_TOKEN` through the environment. `BASE_RPC_URL` is
   optional; omitted values use the default RPC failover list. Zcash indexer and
   submission endpoints default to `https://zec.rocks:443`, with optional overrides
   in `[treasury]`. No Zcash environment variables are required.
2. Copy the deployment before creating the treasury, so initialization and serving
   use the same paths and network policy:

   ```sh
   cp tests/live/deployments/public-swap-demo.toml \
     tests/live/deployments/public-swap-demo.local.toml
   target/debug/x402_treazury wallet init \
     --config tests/live/deployments/public-swap-demo.local.toml
   target/debug/x402_treazury wallet addresses \
     --config tests/live/deployments/public-swap-demo.local.toml
   ```

   Initialization discovers the mainnet tip and rewinds 100 blocks for the birthday.
   It creates `state/public-demo/wallet.key` inside the owner-only wallet directory.
   The treasury ID is discovered from state; there is no UUID to copy into TOML.
   Anyone obtaining the complete directory obtains both wallet state and its key.
   An external `treasury.key_file` remains available when separate storage is needed.

   If this treasury already exists, skip `init` and use `addresses`; initialization
   never overwrites it. An explicit `--birthday HEIGHT` keeps new initialization
   offline. Seed imports require `--mnemonic-file FILE --birthday HEIGHT`, using a
   height at or before the original wallet's first use. Lookup failure creates no
   wallet state. Local copies are gitignored. Keep `auto_fund = false` and the
   reviewed retirement tolerance at zero. Pausing new funding does not disable
   independently eligible automatic retirement; zero tolerance requires exact
   confirmed and latest zero balances plus cleared financial obligations.

   This demo defaults to direct networking. For Tor, add the `[network]` table from
   `examples/network/tor.toml` to the local deployment before initialization or sync,
   and start Tor on its configured SOCKS port.
3. Back up the newly initialized state and key before funding:

   ```sh
   mkdir -p secrets
   chmod 700 secrets
   target/debug/x402_treazury wallet backup \
     --config tests/live/deployments/public-swap-demo.local.toml \
     --destination secrets/public-demo-backup
   ```

   The backup directory must not exist. Store it securely; it contains spending
   material. Never run the original and restored copy concurrently.
4. Provide the printed **shielded** receive address to the operator. Agree on the
   ZEC amount using a fresh quote for two $5 USDC deposits, including source fees.
   The example permits at most **0.006 ZEC per transfer including its fee**,
   **5% quoted conversion overhead**, and **0.0003 ZEC per shielding fee**.
   These are per-operation ceilings, not a price estimate or a promise that a
   route fits. Actual spendable treasury funds govern production funding. The
   separate reviewed qualification manifest/registry controls this run's cumulative
   authority; retain `auto_fund=false` unless that explicit run authorization permits
   funding. Do not increase per-operation safeguards automatically after quote failure.
5. After the operator transfers ZEC, sync and inspect the treasury:

   ```sh
   target/debug/x402_treazury wallet sync \
     --config tests/live/deployments/public-swap-demo.local.toml
   target/debug/x402_treazury wallet status \
     --config tests/live/deployments/public-swap-demo.local.toml
   ```

   Require a fresh ready sync and enough confirmed shielded spendable balance for
   bootstrap. Pending deposits/change do not qualify. Close administration commands
   before serving; the treasury has one exclusive owner.
6. Only after reviewing the resolved configuration and spend caps, set
   `funding.auto_fund = true` in the local copy and serve:

   ```sh
   target/debug/x402_treazury serve \
     --config tests/live/deployments/public-swap-demo.local.toml
   ```

   Start with no MCP clients issuing tool calls. Observe the two funding jobs,
   source confirmations and independent Base credit through `wallet status`.
   For this demo, wait for complete configured inventory: one ACTIVE and one READY
   wallet, each satisfying its 5,000,000 atomic USDC target. Ordinary serving can
   begin with one eligible funded payer; that does not satisfy this stronger demo
   inventory gate. NEAR SUCCESS alone is insufficient.
7. Connect an MCP client to `http://127.0.0.1:8000/mcp` using the bearer token.
   Initialize and list tools first. Make **one** approved profile lookup with
   `socialfetch_twitter_profiles_handle` and a `handle` argument. The per-payment
   cap is **$0.05**. Record seller acceptance and verify the payment authorization
   later reconciles against Base. A tool response alone is not a settlement proof.
8. Stop with SIGINT/SIGTERM, inspect status, and make a new backup. Keep the test
   within its separately reviewed run authority; UTC rollover grants no new authority.
   Do not leave automated clients or unattended funding running after the test.

The first test funds two slots and proves one paid call. It does not deliberately
burn the active wallet's $5 to demonstrate rotation. The offline lifecycle test
covers promotion and refill. To qualify that on mainnet, separately agree on useful
paid calls and their aggregate cost; observe confirmed depletion, promotion to the
existing READY address, and exactly one new replacement address/funding job. Never
edit balances, force readiness, export managed keys or fabricate chain evidence to
trigger a demonstration.

## Funded EVM-key alternative

Use `tests/live/deployments/public-payment-demo.toml` with `PUBLIC_DEMO_EVM_PRIVATE_KEY` and
`PUBLIC_DEMO_MCP_TOKEN` in the environment. The key must hold USDC on Base; this
exact EIP-3009 test does not require Permit2 approval. Inspect with `config show`
and `catalog tools`, then start serving and perform only the single approved call
as described above. The $0.05 cap applies per payment. Stop immediately afterward.
This qualification cannot substitute for the treasury-funded test.

## Failure and recovery

| Observation | Action |
| --- | --- |
| Quote rejected, route unavailable or rate limited | Inspect the sanitized job error. Bounded retries/backoff are automatic; do not silently relax caps or select a confidential mode. |
| Insufficient spendable shielded balance | Keep the active EVM wallet usable; fund/sync under the reviewed run authority. No signed operation is created by this wait. |
| Swap timeout | Pool status becomes degraded; slower reconciliation continues with the same operation. Do not issue another deposit. |
| Ambiguous source submission | Stop serving before `wallet reconcile --config FILE --operation-id UUID`. `--rebroadcast` explicitly permits only the same bytes while still valid. |
| Expired source transaction | Use `wallet recover-expired`; it requires canonical synced unspent-input evidence. Refused recovery keeps the reservation. Never delete its journal. |
| Failed/refunded swap | NEAR status does not credit funds. Sync discovers confirmed refund outputs and credits returned principal once. With serving stopped, use `wallet shield-refunds --config FILE --job-id UUID`, then explicitly submit the returned operation via `wallet reconcile --rebroadcast`. Refund jobs remain operator-visible; shielding does not automatically retry the failed swap. |
| Interrupted preparation with no signed bytes | `wallet recover-unprepared` archives old bindings and creates a fresh operation identity. It refuses every operation with signed bytes. |
| Reorg of accounted source spend/refund/expiry evidence | Treasury readiness fails closed; retain state and investigate. Do not reset consumed accounting or run a restored copy alongside the original. |

Keep private diagnostics and backups local. Public-swap confidentiality, funded
routing, seller acceptance and spend limits must be described as actually observed;
do not claim confidential unlinkability or successful mainnet settlement from
fixtures or a dry quote.
