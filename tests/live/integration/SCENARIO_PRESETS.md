# Scenario presets

These template manifests list explicit case IDs, arguments, windows, and
reservations. Fill in treasury, run, and authorization IDs, absolute paths, Tor
binary, and future UTC windows before running `plan`.

| Manifest | Deployment | Calls | Total test budget (USDC) | New funding jobs |
| --- | --- | ---: | ---: | --- |
| `short.example.toml` | `single-deployment.example.toml` | 1 | $0.20 | 0 |
| `standard.example.toml` | `managed-deployment.example.toml` | 4 | $0.80 | 0 |
| `rotation.example.toml` | `managed-deployment.example.toml` | 14 | $2.80 | 1 |
| `extended.example.toml` | `managed-deployment.example.toml` | 28 | $5.60 | 2 |
| `treasury-only.example.toml` | `managed-deployment.example.toml` | 28 | $5.60 | 4 |
| `catalog.example.toml` | `unsigned-deployment.example.toml` | 3 unsigned | $0 | 0 |
| `reliability.example.toml` | `single-deployment.example.toml` | 2 | $0.40 | 0 |

For the 23-provider sweep, see [PROVIDER_PRESETS.md](PROVIDER_PRESETS.md).
`Total test budget (USDC)` caps the running sum of potentially chargeable API
authorizations across the run (every authorization consumes the budget even if
its response is lost or another wallet is tried). New wallet funding is bounded
separately by reviewed job counts, reserve sizes, spendable ZEC, and SDK fees.

## Wallet and listener scope

- `single-deployment.example.toml` declares only `test_pool`, shared by
  SocialFetch across listeners A and B.
- `managed-deployment.example.toml` adds an independent SocialFetch source on
  listener C (`independent_pool`) and an Arkham depletion source on listener A
  (`test_pool`), each with distinct prefixes and exact allowlists.
- `standard.example.toml` runs one shared-wallet A and B pair followed by one
  separate-wallet A and C pair, reporting driver, payment-work, and
  signed-future overlap separately.

## Rotation, service, and restart

- Each rotation round lists twelve $0.20 Arkham depletion calls followed by two
  SocialFetch service calls (one per wallet). The runner checks fresh balances
  and exposure before depletion, stops on natural `READY` consumption (marking
  unused suffix cases `SKIPPED_TARGET_REACHED`), dispatches service calls while
  refill is pending, and verifies independent credit and source confirmation.
- Twelve depletion calls are a finite candidate list for a $2-$3 reserve range.
  If a wallet's starting balance or live price needs more calls, expand the
  explicit case list and reservations **before preparation** rather than
  editing balances or stalling funding.
- `extended.example.toml` rotates `test_pool` twice with one graceful
  application restart at the first `queued_refill` checkpoint while keeping Tor
  running. `treasury-only.example.toml` runs the same calls starting from empty
  pools (2 bootstrap jobs plus 2 replacement jobs).
- Positive-funding presets require setting `funding.auto_fund = true` in the
  copied deployment, authorizing sufficient cumulative job limits, and passing
  `--allow-funding`. Zero-funding runs install `--qualification-no-new-funding`
  regardless of `auto_fund`.

## Unsigned and reliability scenarios

- `catalog.example.toml` runs in keyless `--qualification-unsigned` mode for a
  directory help fetch, cache hit, and directory API call (refusing any `402`
  before signing; still requires a treasury baseline identity without funded
  EVM wallets).
- `reliability.example.toml` preserves the two-window schema for offline
  validation. Automatic cross-window continuation is unsupported; for
  multi-day reliability sampling, run separate smoke or provider manifests with
  fresh case IDs under the same cumulative registry.

## Configure and validate

Copy the deployment and manifest into an owner-only directory, convert relative
paths to absolute paths, fill in IDs, Tor path, and free listener ports, then
run `plan`, `authorize-registry`, `prepare`, and `run` using binaries built
together (see [INTEGRATION.md](../INTEGRATION.md)). Offline tests validate
every template's graph, bindings, allowlists, tools, and arguments against
committed catalogs.
