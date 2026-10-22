# Documentation

The [project README](../README.md) is the entry point for setup, TOML configuration
and CLI usage. Maintained implementation and operational explanations live here:

| Guide | Subject |
| --- | --- |
| [Configuration](configuration.md) | TOML composition, tool filters, provider compatibility and pricing |
| [Wallet CLI](wallet-cli.md) | Treasury commands, funding configuration and recovery |
| [Development](development.md) | Local checks, coverage and complexity tooling |
| [Architecture](architecture.md) | Catalog generation, MCP execution, ownership and the [wallet selection/isolation decision](architecture.md#wallet-selection-and-network-isolation) |
| [Wallet rotation](wallet-rotation.md) | Treasury ownership, payment admission, reserve claims, retained capacity, funding and recovery |
| [Network egress](network-egress.md) | Direct/Tor factory, isolation identities, timeout policy and SDK boundaries |
| [Local resource limits](local-resource-limits.md) | Existing failure thresholds, cache lifetimes and maintenance cadence for troubleshooting |
| [Cover traffic](cover-traffic.md) | Optional bounded HTTP/2 ranges, padding, sampling and evidence |
| [Agent sources](agent-sources.md) | Endpoint-local APIs, wallet selection, dynamic tools and persistence |
| [Testing](testing.md) | Offline checks, coverage, consensus, Tor and funded qualification boundaries |
| [Public swap demo](public-swap-demo.md) | Bounded operator workflows for treasury or static-wallet demos |
| [Reproducible builds](reproducible-builds.md) | Toolchain, dependency pins and vendored patch ownership |

[Plans](plans/) describe unfinished work, including artifact storage and the
[remaining vendor retirement assessments](plans/vendor_retirement.md). The maintained
[checkout ledger](../vendor/README.md#checkout-retirement-tracking) tracks complete
removal blockers and upstream/candidate follow-up.
The [curated source management plan](plans/curated_source_management.md) covers
bundled provider matching, registration and accepted configuration persistence.
The maintained [Best-effort wallet rules](wallet-rotation.md#best-effort-wallet-rules)
describe advisory estimates, per-wallet attempts, no top-ups, SDK-owned Zcash
funds, owner recovery and automatic owned-transparent shielding. The
[wallet observation policy](wallet-rotation.md#wallet-observation-policy) owns
USDC/ZEC discovery triggers, recurring cadence, stop rules and accepted-work
exceptions.
The [live runbook](../tests/live/INTEGRATION.md) is the provider execution walkthrough;
[testing](testing.md#qualification-status) states qualification limits.
[Deferred plans](plans/deferred/) cover independent provider startup, polling,
mutation testing, cover shaping, broader lifecycle/cover acceptance, managed
wallet extensions and [post-merge Zcash isolation recovery](plans/deferred/zcash_isolation_retries.md).
Plans grant no execution or spending authority.

When implemented, move useful explanations into maintained guides, retain concrete
unfinished work in a focused plan, fix links and remove the completed plan. Do not
keep an implemented-plan archive. Generated reports, audit inventories and metrics
belong in ignored local or private external storage. Commit durable findings rather
than dated logs, balances, handoffs or report copies. Financial ledgers remain private
and unchanged regardless of documentation cleanup.
