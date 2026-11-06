# Documentation

The [project README](../README.md) is the entry point for setup, TOML configuration
and CLI usage. Maintained implementation and operational explanations live here:

| Guide | Subject |
| --- | --- |
| [Provider updates](provider-updates.md) | Public evidence capture, comparison and reviewed provider maintenance |
| [Configuration](configuration.md) | TOML composition, [source distribution](configuration.md#source-tool-distribution), tool filters, provider compatibility and [pricing](configuration.md#pricing-metadata-and-runtime-hints) |
| [Wallet CLI](wallet-cli.md) | Treasury commands, funding configuration and recovery |
| [Development](development.md) | Local checks, coverage and complexity tooling |
| [Planning workflow](planning-workflow.md) | Active plans, independent review evidence, handoffs and completion with preserved Git history |
| [Zingo integration](zingo-integration.md) | SDK boundaries, Pepper patches and adapted-code attribution |
| [Architecture](architecture.md) | Catalog generation, [MCP output](architecture.md#mcp-and-output-boundaries), execution, ownership and the [wallet selection/isolation decision](architecture.md#wallet-selection-and-network-isolation) |
| [Wallet rotation](wallet-rotation.md) | Treasury ownership, payment admission, reserve claims, retained capacity, [automatic retirement](wallet-rotation.md#automatic-retirement), funding and recovery |
| [Network egress](network-egress.md) | Direct/Tor factory, isolation identities, timeout policy and SDK boundaries |
| [Local resource limits](local-resource-limits.md) | Existing failure thresholds, cache lifetimes and maintenance cadence for troubleshooting |
| [Cover traffic](cover-traffic.md) | Optional bounded HTTP/2 ranges, padding, sampling and evidence |
| [Agent sources](agent-sources.md) | Endpoint-local APIs, [manual raw headers](agent-sources.md#manual-raw-headers), wallet selection, dynamic tools and persistence |
| [Testing](testing.md) | Offline checks, coverage, consensus, Tor and funded qualification boundaries |
| [Public swap demo](public-swap-demo.md) | Bounded operator workflows for treasury or static-wallet demos |
| [Reproducible builds](reproducible-builds.md) | Toolchain, dependency pins and vendored patch ownership |

[Plans](plans/) describe unfinished work, including artifact storage,
[stale cache expiry](plans/stale_cache_expiry.md),
[discovery keywords and recommended tools](plans/discovery_improvements.md),
[OpenAPI authentication](plans/openapi_authentication.md) and the
[remaining vendor retirement assessments](plans/vendor_retirement.md).
[Deferred swap timing assessment](plans/deferred/near_swap_timing_tightening.md)
owns optional service-contract and recovery-duration questions. The achieved
refund-address query-suppression contract and its regression coverage live in the
[SDK guide](zingo-integration.md#refund-receivers-and-seed-recovery).
[deferred release follow-ups](plans/deferred/release_followups.md) own Windows,
Linux CI and the remaining focused review assessments.
The [seed recovery plan](plans/seed_based_wallet_recovery.md) covers derivation,
discovery, retained financial obligations and the choice between root recovery and
discarding retired keys.
Funding/source authority and indexer recovery are maintained in the
[funding/source contract](wallet-rotation.md#funding-and-source-safety) and
[SDK recovery guide](zingo-integration.md). Catalog construction and faithful
request contracts live in [architecture](architecture.md#catalog-configuration-and-execution),
[OpenAPI inputs](configuration.md#openapi-inputs-and-request-bodies) and
[provider maintenance](provider-updates.md). Calculation ownership and lazy public
proving parameters live in the [SDK boundary](zingo-integration.md#application-sdk-boundaries)
and [wallet guide](wallet-rotation.md#funding-limits-and-fees).
Dynamic registry build reuse, publication and read contracts live in
[registration guidance](agent-sources.md#registration-and-tool-invocation) and
[runtime architecture](architecture.md#catalog-configuration-and-execution).
HTTP cache ownership is maintained in [network policy](network-egress.md), and
qualification budgets and complete lifecycle snapshots in the
[technical reference](../tests/live/INTEGRATION_REFERENCE.md#ownership-and-authority).
The [storage assessment](plans/portability_fixes.md) retains the separate Unix
namespace-authority question; implemented ACL privacy contracts belong in
[development](development.md#portability) and [wallet administration](wallet-cli.md). Windows runtime work remains deferred by user decision.
The maintained
[checkout ledger](../vendor/README.md#checkout-retirement-tracking) tracks complete
removal blockers and upstream/candidate follow-up.
The maintained [Best-effort wallet rules](wallet-rotation.md#best-effort-wallet-rules)
describe advisory estimates, per-wallet attempts, no top-ups, SDK-owned Zcash
funds, owner recovery and automatic owned-transparent shielding. The
[reconciliation policy](wallet-rotation.md#recovery-and-reconciliation-policy)
requires repair from established facts and forbids redundant consistency gates
that obstruct otherwise authorized progress. The
[wallet observation policy](wallet-rotation.md#wallet-observation-policy) owns
USDC/ZEC discovery triggers, recurring cadence, stop rules and accepted-work
exceptions.
The [live runbook](../tests/live/INTEGRATION.md) is the provider execution walkthrough;
[testing](testing.md#qualification-status) states qualification limits.
[Deferred plans](plans/deferred/) cover polling,
mutation testing, cover shaping, broader lifecycle/cover acceptance, managed
wallet extensions, [optional Zcash Tor read recovery](plans/deferred/zcash_tor_read_recovery.md)
and [Coinbase bundle access](plans/deferred/coinbase_bundles.md).
Plans grant no execution or spending authority.

When implemented, move useful explanations into maintained guides, retain concrete
unfinished work in a focused plan, fix links and remove the completed plan. Do not
keep an implemented-plan archive. Preserve plans in Git history through integration;
the [planning workflow](planning-workflow.md) describes their lifecycle and review.
Generated reports, audit inventories and metrics
belong in ignored local or private external storage. Commit durable findings rather
than dated logs, balances, handoffs or report copies. Financial ledgers remain private
and unchanged regardless of documentation cleanup.

Accepted-valid unfinished review findings belong in focused active or deferred
plans with a concrete next action and acceptance evidence, not lingering backlog
paragraphs in maintained guides. Guides retain truthful current behavior and
limitations with concise links until those findings are resolved. A rejected
proposal needs durable rationale rather than a perpetual future-fix note. Moving
work to a plan does not fix it, waive its release conditions or erase evidence.
Plans describe work we intend to perform, including bounded assessments with a
required decision. A hypothetical promise to revisit behavior only if a future bug
appears is ordinary maintenance, not planned work.
