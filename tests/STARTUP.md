# Catalog startup qualification

Run the focused startup and deployment regression suites sequentially:

```sh
make test CARGO_ARGS='--test startup --test deployment --test provider_catalogs --test network_audit'
make build
```

All tests below use unfunded local fixtures and loopback servers rather than
live providers, live settlement, or live Tor.

## Startup, deferred loading, and routing coverage

- **Store loss and direct-swap isolation:** `deployment::store_loss_tests`
  verifies Store worker failure fails dependent calls and `close` while
  unrelated Stores and listeners keep serving. `deployment::direct_swap_tests`
  keeps catalog inspection independent of incompatible state, restores only
  authenticated owned status, preserves removed-listener background work, and
  checks safe Base, NEAR, and treasury diagnostics without opening operator
  wallets.
- **Catalog concurrency and CLI boundaries (`tests/startup.rs`):** Covers
  direct bypass of Tor capacity, the default two-scope Tor gate, cancellation
  and failure cleanup, completion-order-independent inventories, compatible
  alias sharing (with strict limit and timeout separation), per-alias filters
  and wallets, JSON stdout with sanitized stderr progress, malformed-catalog
  failure, and origin-based SOCKS discovery credentials in an isolated child.
- **Deferred serving and readiness (`deployment::deferred::tests`,
  `readiness_tests`):** Covers incremental serving (`wait_for_catalogs = false`),
  default per-endpoint waiting (`wait_for_catalogs = true`), publication before
  MCP `initialize`, `tools/list`, and readiness logs. Covers independent local
  peers, optional source failures (`allow_catalog_failure = true`), `ERROR`
  plus socket closure for required failures, runtime refresh resolving readiness immediately,
  cached-generation diagnostics (in an isolated child process), and
  discovery-help teardown waiting for final `Manager` drop before registry
  unlock.
- **Funding independence and startup announcement:**
  `deployment::lifecycle_tests::funding_failure_keeps_listeners_and_base_reconciliation_available`
  verifies listeners and Base reads survive funding failures.
  `deployment::readiness::tests` verifies the once-only `All MCP endpoints
  listening, funded, and ready` `INFO` line waits for catalog settlement, every
  listener, and sufficient estimates across bound pools (exact cap equality,
  split balances, low or unknown estimates, disabled pools, and clean Store
  release on shutdown).
- **Selectors, collisions, and `send_tools` (`deployment::selection_diagnostics`,
  `tests/deployment.rs`):** Covers missing selectors, empty inventories,
  unmatched exclusions, collision omission in opposite completion orders,
  warnings for selected collisions, calls to healthy siblings, `send_tools`
  fanout, exclusions, shadowed rules, reserved-namespace source
  isolation, shared-payer cover conflict checks, and Store cleanup on early
  bind failure. See
  [source tool distribution](../docs/configuration.md#source-tool-distribution).
- **Caching and refill scheduling:** `deployment::startup::price_alias_tests`
  covers shared alias acquisition, revision capture, HTTP `304` joint price
  renewal, direct-warm and relay provenance, and invalidation.
  `rotation::funding::scheduler::opportunity_tests` covers paused-time deferred
  allocation, accepted-job recovery, and explicit bootstrap.
  `deployment::bootstrap::actor_tests` covers `wallet bootstrap` monitoring
  with credited and pending claims through shutdown and reopen.
- **Startup SOCKS prerequisite and wallet observation:**
  `network::startup_socks::tests`, `deployment::deferred::startup_socks_tests`,
  `rotation::manager::maintenance::tests::startup_socks`, and
  `treasury::actor::tests::startup_socks_preserves_commands_and_failed_scan_cooldown`
  cover delayed local SOCKS startup, shared and cancelled waiters, 60-second
  `ERROR` reminders, fresh cache hits while offline, preserved failure
  cooldowns across proxy recovery, finite Base startup passes (consumed on
  exhaustion or transient inventory error without retrying that frontier), and
  Store-only fallback when the Zcash SDK is unavailable. See
  [wallet observation policy](../docs/wallet-rotation.md#wallet-observation-policy).
- **Opt-in sync diagnostics:** Pass `X402_TEST_SYNC_DIAGNOSTICS=1` with
  `--nocapture` on treasury command-sync and actor fixtures to print narrow
  phase breadcrumbs (not enabled in normal tests). Linux hang capture uses
  [treasury-sync-diagnostics.yml](../.github/workflows/treasury-sync-diagnostics.yml);
  see [GitHub Actions and coverage](../docs/testing.md#github-actions-and-coverage).

## Reproducible measurement

```sh
make test CARGO_ARGS='--test startup_benchmark' TEST_ARGS='--ignored --nocapture'
```

This ignored benchmark runs three synthetic OpenAPI specs across five sources
(including two aliases sharing three URLs) over localhost with 100 ms per
request, comparing inventories across three samples per Tor capacity. Because
direct requests bypass the Tor gate, it measures direct acquisition stability
rather than Tor concurrency. Record queue, headers, body, parse, build, peak
RSS, request counts, and failures under ignored `target/`.

## Default capacity and Conflux rationale

`[startup].catalog_concurrency` defaults to `2` (range `1..=64`). Requests
sharing effective SOCKS credentials share one slot and run in parallel;
distinct scopes and retry scopes count separately. Slots release after the last
response body finishes (before retry delays or wallet waits), and direct
egress, cache hits, and local files bypass the gate.

Tor normally maintains three prebuilt multipath Conflux sets, so admitting two
active isolation scopes leaves replenishment headroom and matched the best
observed live Tor performance during operator testing. Live comparisons should
alternate limits against fixed catalogs on persistent warm Tor state (using
controlled `NEWNYM` between runs) while recording circuit assignments,
failures, and latency.

## Best-effort framework startup

`deployment::remote_selection_tests` covers empty local and remote selections,
unknown names, empty recommendation opt-ins, contradictory filters, namespace
independence, retained routing, and runtime refresh recovery, while keeping
strict inspection inventory-dependent.
`catalog_warm_all_remote_sources_unavailable_returns_degraded_json` verifies
exit `0` with one degraded JSON summary without opening wallets or listeners,
and warming regressions cover sibling completion, partial acceptance, invalid
replacement rejection, and accepted relay draining on cancellation. See
[framework integration](../docs/framework-integration.md).
