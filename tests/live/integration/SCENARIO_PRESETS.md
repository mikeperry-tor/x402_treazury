# Scenario presets

These manifests use explicit case IDs, arguments, windows and reservations. They
are templates, not executable authorizations: fill the treasury/run/authority IDs,
paths, Tor binary and future UTC windows before planning. No command expands a
hidden repeat count. All mainnet work remains an explicit invocation.

| Manifest | Deployment | Calls | API reservation ceiling | New funding jobs / source ceiling |
| --- | --- | ---: | ---: | --- |
| `short.example.toml` | `single-deployment.example.toml` | 1 | $0.20 | 0 / 0 ZEC |
| `standard.example.toml` | `managed-deployment.example.toml` | 4 | $0.80 | 0 / 0 ZEC |
| `rotation.example.toml` | `managed-deployment.example.toml` | 14 | $2.80 | 1 / 0.002 ZEC |
| `extended.example.toml` | `managed-deployment.example.toml` | 28 | $5.60 | 2 / 0.004 ZEC |
| `treasury-only.example.toml` | `managed-deployment.example.toml` | 28 | $5.60 | 6 / 0.012 ZEC |
| `catalog.example.toml` | `unsigned-deployment.example.toml` | 3 unsigned | $0 | 0 / 0 ZEC |
| `reliability.example.toml` | `single-deployment.example.toml` | 2 | $0.40 | 0 / 0 ZEC |

The broader [23-provider sweep](PROVIDER_PRESETS.md) is a separate breadth preset.
The table's API amounts reserve the effective $0.20 per-payment cap, not the
historical SocialFetch fee of $0.014. Source ceilings are upper bounds in ZEC, not
fee forecasts. Swaps allocate USDC to active/standby wallets; those allocations
are not API consumption. Use the planner and actual starting-state observations
to review both kinds of exposure.

## Wallet and listener scope

The single-pool deployment declares only `test_pool`; a quick test does not need
an unused second managed pool initialized. The shared SocialFetch source assigns
that wallet explicitly across listeners A and B. The two-pool deployment adds
an independent SocialFetch source on listener C using `independent_pool`, and an
Arkham depletion source on A using `test_pool`. Each source has a distinct tool
prefix, an exact listener allowlist and startup pricing disabled.

The standard preset first runs a shared-wallet A/B pair, then a separate-wallet
A/C pair. Both batches have two calls. Existing concurrency qualification reports
driver/application overlap, payment-work overlap and signed-future overlap
separately; the preset does not promise every level of overlap will be observed.

## Rotation, service and restart

Each rotation round declares twelve $0.20 Arkham depletion attempts, followed by
two SocialFetch service cases, one per wallet. The runner checks current balance
and unresolved exposure before depletion, stops at the natural promotion boundary,
and marks unused depletion suffixes not executed. Service attempts must actually
pay while the replacement remains pending, and credit/source confirmation must
qualify independently. A provider failure remains a failure; no automatic paid
retry is purchased.

Twelve attempts are a finite candidate list, not a promise to rotate every wallet.
The configured deposit target is $2; the accepted bridge minimum can raise it.
A larger starting balance, different live fee, incomplete chain evidence, fast
refill or missed checkpoint can leave qualification incomplete. Review and expand
the explicit cases and reservations **before preparation** if the observed balance
requires more. Do not lower balances, force promotion or delay real funding to
make an assertion pass.

The extended preset rotates `test_pool` twice and requests one graceful restart
at the first queued-refill checkpoint while retaining Tor. Both rounds include
service from `independent_pool`. The treasury-only variant has the same calls but
starts with no EVM allocations: four bootstrap jobs (active plus standby for each
of two pools) and two replacement jobs. Existing-wallet presets assume funded
managed pools; they do not substitute an unrelated EVM private key.

For positive-funding presets, explicitly enable `funding.auto_fund` in the copied
deployment, authorize sufficient cumulative registry limits and pass
`--allow-funding` to the supervised command. The committed deployment keeps
auto-funding off. Zero-funding runs install the production deny-all restriction
even when the copied deployment enables funding. Existing liabilities and prior
reservations persist; starting a new run never resets them.

## Unsigned and reliability scenarios

The catalog preset uses the production keyless mode for a directory help fetch,
cache hit and directory API call. It selects no managed pools, never reads the
static key variable and refuses any 402 before signing. Registry authorization
still requires an initialized treasury identity/baseline, but no funded EVM wallet
is required. Successful cached help is not evidence of a second external fetch.

The reliability template preserves the historical two-window schema for offline
validation and archived reports. Automatic continuation is no longer supported:
do not expect one invocation to schedule tomorrow's window or restart it with
`report --eligibility`. For current live reliability sampling, prepare separate smoke/provider
runs with new case IDs and identical reviewed requests/assertions, each in its own
window, using the same cumulative registry. Compare their dated observations;
failed/uncertain cases remain recorded and are never replayed.

## Configure and validate

Copy the matching deployment and manifest into an owner-only directory. Their
committed relative paths resolve from `tests/live/integration`; make provider,
binary, state, key and evidence paths absolute when relocating them. Set the
existing treasury UUID consistently, choose fresh run IDs, point to the installed
Tor binary and configure the dedicated SOCKS endpoint before pinning. Listener
ports must be free; do not share the serving process with unrelated callers.

Run `plan`, review the expanded case/wallet map and ceilings, then follow the
[integration runbook](../INTEGRATION.md) for registry authorization and
`prepare` followed by `run`. Build the runner and executable together before pinning. None of
these templates is evidence that a funded lifecycle or a new Tor run passed.
Offline tests validate every template's graph, allocations, wallet bindings,
listener allowlists, tool existence and arguments against committed catalogs.
