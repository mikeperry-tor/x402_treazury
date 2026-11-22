# Zcash consensus qualification

`treasury::regtest::deposit_settlement_and_reorg_recovery` launches Zebra
validators and a Zaino indexer in temporary Docker containers using the public
BIP-39 `abandon ... about` mnemonic, locally mined coins, no peer seeds, and
random loopback RPC ports. It never reads deployment profiles, `.env`, operator
wallets, external indexers, or live funds.

Docker must be running. Pull these pinned images first (the test uses
`--pull never`; Zaino requires amd64 emulation on ARM hosts). Note that these
NU6.3 digests were qualified with Zingolib v6.0.0 and have not yet been
requalified against the current v8 SDK:

```sh
docker pull docker.io/zfnd/zebra:6.0.0@sha256:78a10b7f24b83a86e6223d97e857094a353454e3268f84a87bd987e7140a33bb
docker pull --platform linux/amd64 docker.io/zingodevops/zaino:0.6.0-rc.1-no-tls@sha256:e48b133dbf53dbed77b872de74d65aa8a45357a3844b658ea472a340949feb7f
make test CARGO_ARGS='--features zcash-regtest --lib treasury::regtest::deposit_settlement_and_reorg_recovery' TEST_ARGS='--ignored --exact --nocapture'
```

The regtest network variant compiles only into unit tests; public treasury
constructors and the shipped executable remain mainnet-only even in
all-feature builds. Regtest state binds its database identity and AEAD
associated data to `regtest`, which production constructors reject.

## Primary deposit and reorg scenario

1. Mine through NU6.3 activation so validator coinbase outputs fund Ironwood
   (never using transparent-input shielding).
2. Sync the treasury, prepare one bound native Ironwood payment, and reopen
   encrypted state (confirming preparation submits nothing).
3. Submit the saved bytes and mine one confirmation (retaining the reservation
   because two confirmations are required).
4. Mine a longer competing branch on the second validator, deliver it to the
   first, and sync without releasing the reservation or recalculating bytes.
5. Rebroadcast the identical saved bytes, mine required confirmations, and
   confirm original amount and fee facts persist.
6. Replace confirmed history with another longer branch; SDK sync updates
   spendability through reopen while keeping exact original bytes and facts
   without automatic replacement or rebroadcast.

The harness has a 15-minute outer deadline and removes its UUID-named
containers and bridge on normal exit, error, panic, or timeout (if killed with
`SIGKILL`, remove only `x402-regtest-<UUID>-*` containers and the
`x402-regtest-<UUID>` network). No host directories mount writable into
containers. This scenario does not qualify NEAR swaps, refunds, or deep
finalized-chain rollbacks.

## Additional isolated scenarios

Run each ignored scenario individually with its exact name and
`TEST_ARGS='--ignored --exact --nocapture'` (add `--test-threads=1` for
`tor_consensus_lifecycle`):

| Test in `treasury::regtest` | What it verifies |
| --- | --- |
| `transparent_refund_shielding_is_unsupported` | Wallet API and CLI reject transparent-input shielding without state, mempool, or network side effects |
| `expired_ambiguous_deposit_releases_only_after_chain_proof` | Authenticated finite expiry requires completed canonical SDK coverage at configured depth; archived bytes cannot submit again |
| `indexer_non_inclusion_response` | Missing-transaction responses from Zaino stay ambiguous (`Unknown`) |
| `recovery_cli_lifecycle` | Production Clap parser and recovery handlers in isolated subprocesses: native Ironwood preparation, read-only reconciliation, same-byte rebroadcast, confirmed-rebroadcast refusal, and expiry recovery |
| `tor_consensus_lifecycle` | Deposit, refund, and expiry flows through authenticated fake SOCKS (must run alone because it installs process network policy) |

For coverage on one scenario (or synthetic proving without Docker):

```sh
make coverage-consensus TEST=transparent_refund_shielding_is_unsupported
make coverage-proving
```

Fake SOCKS verifies client routing and credentials, not live Tor circuit
selection or live NEAR swaps.

## Recovery command subprocess adapter

`recovery_cli_lifecycle` drives the production wallet CLI dispatcher against
isolated regtest state by injecting only the test network constructor under
`cfg(test)` and `zcash-regtest` (never exposed in the shipped binary).

- Each child loads relative paths from a TOML profile containing a missing API
  spec, proving recovery never loads API sources. Read-only children receive no
  submission environment variable.
- The parent verifies zero shielding or read-only broadcast, identical prepared
  bytes, immutable facts, refusal to rebroadcast a confirmed operation, release
  only after expiry proof, legible early-expiry errors, and ownership release
  after both success and failure.
- Runs with a 30-minute outer deadline and a 3-minute per-child deadline:

```sh
make test CARGO_ARGS='--features zcash-regtest --lib treasury::regtest::recovery_cli_lifecycle' TEST_ARGS='--ignored --exact --nocapture'
make coverage-consensus TEST=recovery_cli_lifecycle
```
