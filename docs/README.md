# Documentation

Start with the [project README](../README.md) for initial setup,
[advanced deployments](../README_ADVANCED.md) for multi-listener setups, and the
[usage guides](usage/configuration.md) for operator-focused configuration and
CLI walkthroughs.

## Operator guides

| Guide | Subject |
| --- | --- |
| [Configuration guide](usage/configuration.md) | Operator choices for listeners, sources, wallets, Tor, and relay fallback |
| [Command-line guide](usage/cli.md) | Setup, treasury management, inspection, and recovery workflows |
| [Advanced deployments](../README_ADVANCED.md) | Research desk and comprehensive walkthroughs, Tor, and wallet bootstrap |
| [Deployment examples](../examples/README.md) | Example choices, wallet groups, ports, and source selections |
| [Public swap demo](public-swap-demo.md) | Bounded single-call qualification for treasury or static-wallet demos |

## Technical references

| Guide | Subject |
| --- | --- |
| [Configuration](configuration.md) | TOML composition, [source distribution](configuration.md#source-tool-distribution), tool filters, [direct swap control](configuration.md#direct-swap-control), provider compatibility, and [pricing](configuration.md#pricing-metadata-and-runtime-hints) |
| [Architecture](architecture.md) | Catalog generation, [MCP output](architecture.md#mcp-and-output-boundaries), execution, ownership, and [wallet selection and network isolation](architecture.md#wallet-selection-and-network-isolation) |
| [Agent sources](agent-sources.md) | Endpoint-local APIs, [manual raw headers](agent-sources.md#manual-raw-headers), wallet selection, dynamic tools, and persistence |
| [Framework integration](framework-integration.md) | Catalog warming JSON, degraded coverage, and endpoint readiness |
| [Wallet rotation](wallet-rotation.md) | [Best-effort rules](wallet-rotation.md#best-effort-wallet-rules), [observation](wallet-rotation.md#wallet-observation-policy), [funding safety](wallet-rotation.md#funding-and-source-safety), and [reconciliation](wallet-rotation.md#recovery-and-reconciliation-policy) |
| [Wallet CLI](wallet-cli.md) | Treasury commands, funding configuration, and recovery |
| [Zingo integration](zingo-integration.md) | [Ironwood-only policy](zingo-integration.md#ironwood-only-policy), [SDK boundaries](zingo-integration.md#application-sdk-boundaries), [refund receivers](zingo-integration.md#refund-receivers-and-seed-recovery), Pepper patches, and attribution |
| [Network egress](network-egress.md) | Direct and Tor client factories, isolation identities, HTTP caching, timeouts, and SDK boundaries |
| [Local resource limits](local-resource-limits.md) | Failure thresholds, cache lifetimes, and maintenance cadence |
| [Cover traffic](cover-traffic.md) | Optional bounded HTTP/2 ranges, padding, sampling, and compatibility |

## Maintenance, testing, and builds

| Guide | Subject |
| --- | --- |
| [Provider updates](provider-updates.md) | Evidence and pricing capture, comparison, and provider maintenance |
| [Provider policy and caveats](../providers/README.md) | Bundled provider policy and [support boundaries](../providers/CAVEATS.md) |
| [Development](development.md) | Local checks, [portability](development.md#portability), logging, coverage, and complexity tooling |
| [Testing](testing.md) | Offline checks, GitHub CI, coverage, consensus, Tor, and [qualification status](testing.md#qualification-status) |
| [Live integration runbook](../tests/live/INTEGRATION.md) | Funded provider walkthrough and [technical reference](../tests/live/INTEGRATION_REFERENCE.md#ownership-and-authority) |
| [Reproducible builds](reproducible-builds.md) | Toolchain, dependency pins, and vendored patch ownership |
| [Vendor checkout tracking](../vendor/README.md#checkout-retirement-tracking) | Vendored patch provenance and checkout retirement blockers |
| [Planning workflow](planning-workflow.md) | Active plans, independent review evidence, handoffs, and completion |

## Plans and documentation policy

[Active plans](plans/) track unfinished work that spans multiple stages or
requires explicit design decisions, including
[artifact storage](plans/artifact_storage.md),
[custom pricing adapters](plans/custom-pricing-adapters.md),
[discovery improvements](plans/discovery_improvements.md),
[OpenAPI authentication](plans/openapi_authentication.md),
[portability assessments](plans/portability_fixes.md),
[seed-based wallet recovery](plans/seed_based_wallet_recovery.md),
[vendor retirement](plans/vendor_retirement.md), and
[wallet merge sweeping](plans/wallet_merge_sweeping.md).

[Deferred plans](plans/deferred/) hold postponed work:
[polling tools](plans/deferred/polling_tool_support.md),
[mutation testing](plans/deferred/mutation_testing.md),
[HTTP/2 cover traffic shaping](plans/deferred/http2_cover_traffic.md),
[broader live acceptance](plans/deferred/live_integration_acceptance.md),
[managed wallet follow-ups](plans/deferred/managed_wallet_followups.md),
[swap timing tightening](plans/deferred/near_swap_timing_tightening.md),
[optional Zcash Tor read recovery](plans/deferred/zcash_tor_read_recovery.md),
[Coinbase bundle access](plans/deferred/coinbase_bundles.md),
[stateless MCP subscriptions](plans/deferred/stateless-mcp-subscriptions.md), and
[release follow-ups](plans/deferred/release_followups.md) (including deferred
Windows runtime work).

Follow these rules when updating documentation:

- **Move completed work into maintained guides:** When a plan finishes, move
  lasting explanations into the owning guide, keep any remaining concrete work
  in a focused plan, update inbound links, and delete the completed plan file.
  Git history preserves completed plans; do not keep an archive directory. See
  [planning workflow](planning-workflow.md).
- **Keep generated reports out of Git:** Store generated reports, audit
  inventories, metrics, and private financial ledgers in ignored local paths or
  private external storage. Commit durable findings as tests, configuration, or
  maintained prose rather than dated logs or report copies. Documentation
  cleanup never alters or deletes private financial ledgers.
- **Keep guides current, not backlogs:** Track accepted unfinished findings in
  an active or deferred plan with a concrete next step and acceptance criteria.
  Maintained guides state current behavior and limitations, linking to a
  plan only when work is genuinely scheduled. Document why a rejected proposal
  was declined instead of adding perpetual revisit notes.
