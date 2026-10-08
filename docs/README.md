# Documentation

The [project README](../README.md) is the entry point for setup, TOML configuration
and CLI usage. Maintained implementation and operational explanations live here:

| Guide | Subject |
| --- | --- |
| [Configuration](configuration.md) | TOML composition, tool filters, provider compatibility and pricing |
| [Wallet CLI](wallet-cli.md) | Treasury commands, funding configuration and recovery |
| [Development](development.md) | Local checks, coverage and complexity tooling |
| [Architecture](architecture.md) | Catalog generation, MCP execution, caching and module boundaries |
| [Wallet rotation](wallet-rotation.md) | Treasury ownership, payment admission, double buffering, funding and recovery |
| [Network egress](network-egress.md) | Direct/Tor factory, isolation identities, timeout policy and SDK boundaries |
| [Cover traffic](cover-traffic.md) | Optional bounded HTTP/2 ranges, padding, sampling and evidence |
| [Agent sources](agent-sources.md) | Endpoint-local APIs, wallet selection, dynamic tools and persistence |
| [Testing](testing.md) | Offline checks, coverage, consensus, Tor and funded qualification boundaries |
| [Public swap demo](public-swap-demo.md) | Bounded operator workflows for treasury or static-wallet demos |
| [Reproducible builds](reproducible-builds.md) | Toolchain, dependency pins and vendored patch ownership |

[Plans](plans/) describe unfinished work, including artifact storage.
The [live runbook](../tests/live/INTEGRATION.md) is the provider execution walkthrough;
[testing](testing.md#qualification-status) states qualification limits.
[Deferred plans](plans/deferred/) cover independent provider startup, polling,
mutation testing, cover shaping, broader lifecycle/cover acceptance and managed
wallet extensions. Plans grant no execution or spending authority.

When implemented, move useful explanations into maintained guides, retain concrete
unfinished work in a focused plan, fix links and remove the completed plan. Do not
keep an implemented-plan archive. Generated reports, audit inventories and metrics
belong in ignored local or private external storage. Commit durable findings rather
than dated logs, balances, handoffs or report copies. Financial ledgers remain private
and unchanged regardless of documentation cleanup.
