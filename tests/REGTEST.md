# Zcash consensus qualification

The opt-in `treasury::regtest::deposit_settlement_and_reorg_recovery` test launches
Zebra validators and a Zaino indexer in disposable Docker containers. It uses the
public BIP-39 abandon/about mnemonic, locally mined coins, no peer seeds, and
random loopback-only RPC ports. It does not use deployment profiles, `.env`,
operator wallets, external indexers, or real funds.

Docker must be running. Pull these images before running the test; the test uses
`--pull never`. The digests and activation schedule match zingolib v6.0.0's
pinned test infrastructure. Zaino requires amd64 emulation on ARM hosts.

```sh
docker pull docker.io/zfnd/zebra:6.0.0@sha256:78a10b7f24b83a86e6223d97e857094a353454e3268f84a87bd987e7140a33bb
docker pull --platform linux/amd64 docker.io/zingodevops/zaino:0.6.0-rc.1-no-tls@sha256:e48b133dbf53dbed77b872de74d65aa8a45357a3844b658ea472a340949feb7f
scripts/zcash.sh test --features zcash-regtest --lib treasury::regtest::deposit_settlement_and_reorg_recovery -- --ignored --exact --nocapture
```

The network selector's regtest variant is compiled only into this crate's unit
test binary, even when the executable is built with all features. Public treasury
constructors and the sender remain mainnet-only. The test's database identity and
encrypted record authentication bind to `regtest`; the production constructor
must reject it. Existing mainnet databases retain their schema and ciphertext
associated-data format.

The scenario exercises the shared treasury, journal and submission code:

1. Mine a transparent first block, then shielded coinbase funds through NU6.3.
2. Sync the treasury, prepare one transparent deposit and reopen its encrypted
   state. No transaction is submitted by preparation.
3. Submit saved bytes, mine one confirmation, and retain the reservation because
   two confirmations are required.
4. Build a longer competing branch on the second validator and submit its blocks
   to the first. Sync and reconcile the orphaned deposit without freeing its
   reservation or recalculating it.
5. Explicitly rebroadcast the same saved bytes and mine sufficient confirmations.
   Resolve the operation and account its original amount plus fee.
6. Replace that confirmed history with another longer branch. Sync must fail
   closed with `treasury_confirmed_spend_reorg`, including after reopening. The
   consumed budget must not become available just because the spend was orphaned.

The harness has a fifteen-minute outer deadline. It removes its UUID-named
containers and dedicated bridge on normal return, error, panic or timeout. A hard
process kill can prevent cleanup: remove only the `x402-regtest-<UUID>-*`
containers and `x402-regtest-<UUID>` network belonging to that interrupted run.
No host directories are mounted writable into the containers; their chain state
is disposable. Cached images remain available for subsequent tests.

The deposit scenario does not qualify NEAR swaps, refunds, deep finalized-chain rollback,
or automatic repair of source spends invalidated after accounting confirmation.
Such spends retain their consumed budget and block new treasury preparation until
the original transaction regains the configured confirmation depth. No replacement
transaction or automatic rebroadcast is created as a recovery shortcut.

## Additional isolated scenarios

Run each scenario in its own test process, using its complete name and `--exact`:

| Test in `treasury::regtest` | Evidence |
| --- | --- |
| `high_index_refunds_and_separate_shielding` | High-index transparent discovery after restore and independent shielding of two addresses |
| `expired_ambiguous_deposit_releases_only_after_chain_proof` | Expiry requires chain proof and owned unspent inputs; archived bytes cannot submit again |
| `indexer_non_inclusion_response` | Missing transaction response remains ambiguous |
| `tor_consensus_lifecycle` | Deposit, refund and expiry scenarios through authenticated fake SOCKS; run alone because it installs process policy |

For an instrumented run of exactly one scenario:

```sh
scripts/coverage.sh --consensus high_index_refunds_and_separate_shielding
```

Use `scripts/coverage.sh --proving` for the synthetic proving test without Docker.
These commands are optional qualification, not tests run by the default suite.
A fake SOCKS endpoint proves client routing/credentials, not real Tor circuit
selection. Neither the optional suite nor its coverage qualifies real NEAR swaps.
