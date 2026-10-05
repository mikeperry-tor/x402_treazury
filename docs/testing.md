# Testing and qualification

Use offline regressions to establish behavior; use optional consensus and live
runs for the integration boundaries they actually exercise. A coverage percentage
or successful HTTP response does not substitute for financial, permission or
network-isolation assertions.

## Default development checks

The default build includes Zcash. `scripts/zcash.sh` supplies the pinned Cargo
invocation and Cargo-managed protoc; do not append a second `--locked` flag.

```sh
scripts/zcash.sh test --all-targets -- --test-threads=1
cargo test --locked --no-default-features --all-targets
cargo fmt --check
scripts/zcash.sh clippy --all-targets -- -D warnings
```

Run feature configurations sequentially: subprocess tests use the built executable
path. Prefer focused tests for a change, then the relevant broader suite.
`scripts/check.sh` is the full qualification wrapper, including compatibility and
vendor verification. Building dependencies/proving parameters can require network
access; normal runtime fixtures use localhost and deterministic unfunded keys,
never a developer's `.env` or mainnet wallet. Ignored child-process helpers invoked
by their parent tests differ from ignored optional live tests.

## Test boundaries

| Area | Principal tests / contract |
| --- | --- |
| Catalog/config/providers | Generated schemas, selected operations, routing, pricing descriptions, composition and pinned local catalogs. |
| MCP/deployment/discovery | Auth, scope and grants, immutable invocation snapshots, update/remove races, persistent idempotency, cancellation, pagination, shutdown and ownership release. |
| Payments/managed rotation | Real local signing, cap/asset validation, reserve-before-send, no paid replay, concurrent admission, single promotion/outbox and canonical liability release. |
| Treasury/store/funding | Encrypted persistence, migrations, snapshot ordering, budget gates, uncertain source sends, refund/expiry rules, backups and filesystem protections. |
| Network | Production HTTP/gRPC through authenticated fixture SOCKS, remote hostnames, identity/pool boundaries, proxy failure, TLS, redirects and public-destination enforcement. |
| Output/resource limits | Exact/over-bound behavior, visible errors and logs, no partial cache success, no retry of paid oversized responses, typed inline image/mapping behavior. |

Tests should establish the absence of forbidden side effects as well as the
expected result: no signature for a rejected payment, no publication for a denied
registration, no source send before persistence, no connection outside the network
factory. Use barriers and controlled clocks to expose concurrency boundaries;
sleeps alone do not prove ordering. Retain positive fixture controls so a broken
fixture cannot make every negative test pass.

The [coverage map](../tests/COVERAGE.md) identifies regression boundaries and
optional qualification methods.

## Coverage and complexity

[Catalog startup qualification](../tests/STARTUP.md) documents rolling-concurrency
and isolation tests, an opt-in local provider replay, and measurement methodology.

`scripts/coverage.sh` produces default line, region and function reports with
compiler/source/lockfile provenance. Generated reports stay under ignored `target/`;
failed collection must not replace the latest successful report. Application
filters exclude external/vendor/test/example paths, although inline test helpers
can still contribute to the application denominator. Compare identical feature
sets and filters, and explain denominator changes before attributing a percentage
change to better tests.

Branch collection is an optional separate invocation using a compatible nightly
compiler and matching LLVM tooling: `RUSTUP_TOOLCHAIN=nightly scripts/coverage.sh
--branch`. Stable region coverage is not branch coverage. Optional proving,
consensus and Tor results remain distinct from default report percentages.
The development guide's [coverage section](development.md#local-coverage) documents setup and
artifact locations; its [complexity section](development.md#local-complexity-metrics)
documents the local rust-code-analysis workflow.

Review uncovered regions by risk: financial release boundaries, grants, transport
fallback, durable publication, error handling and concurrency matter more than a
repository-wide score target. Do not remove guards or mirror implementation details
in tests solely to raise a number. The small
[mutation pilot](plans/deferred/mutation_testing.md) remains deferred; no mutation
score is implied by coverage results.

## Optional proving, consensus and live work

- [LIFECYCLE.md](../tests/LIFECYCLE.md) covers synthetic-note proof and exact-byte
  restore assertions without mainnet funds.
- [REGTEST.md](../tests/REGTEST.md) covers mined local deposits, reorgs, refund
  shielding, expiry proof and recovery CLI behavior.
- [TOR.md](../tests/TOR.md) covers controlled real-Tor circuits and per-process OS
  confinement using an independently installed Tor binary. A local SOCKS fixture
  cannot prove actual Tor circuit assignment or host firewall behavior.
- [Live integration walkthrough](../tests/live/INTEGRATION.md) is the single funded
  provider workflow; [its reference](../tests/live/INTEGRATION_REFERENCE.md) defines
  assertions, registry accounting, concurrency, lifecycle and Tor evidence.

## Qualification status

| Boundary | Evidence and limitation |
| --- | --- |
| Catalogs, MCP, payments, permissions, durable funding and cancellation | Offline fixtures cover named invariants and interleavings, not exhaustive race freedom or real settlement. |
| Synthetic proving and local consensus | Optional suites exercise saved-byte recovery, mined deposits, reorg refusal, refunds and expiry. See LIFECYCLE and REGTEST; these results are separate from default coverage. |
| Real Tor and confinement | Exercised on macOS with an installed Tor process, authenticated control events and OS egress controls. Local SOCKS tests do not prove this; other operating systems and all-network anonymity remain outside the claim. |
| Funded provider workflow | The reusable runner has exercised treasury bootstrap, a selected provider sweep, help caching, canonical debit reporting and a separate startup-pricing check through Tor. Failed or omitted providers remain failed or unobserved; this is not universal provider compatibility or current health. |
| Shared/separate-wallet concurrency and refill service | Manual live tests exercised overlapping paid calls and continued service during pending refill. Runner assertions have offline coverage; manual evidence does not qualify every automated lifecycle preset. |
| Automated lifecycle/restart | Full confirmed-refill/two-round restart acceptance remains deferred. Promotions or service during treasury insufficiency alone are insufficient. |
| Cover with paid traffic | Unsigned cover checks are separate. A fully successful combined cover/payment qualification remains deferred; range refusal or missing samples cannot be hidden by payment success. |
| Mutation and branch coverage | No mutation score is claimed. Branch collection requires a compatible nightly/LLVM setup; stable region coverage is not branch coverage. |

The [scenario presets](../tests/live/integration/SCENARIO_PRESETS.md) retain explicit
wallet scope, single-use cases, finite authority and observation windows. The
[provider preset](../tests/live/integration/PROVIDER_PRESETS.md) retains 23 providers,
22 independently reviewed paid requests, one unsigned directory call and 40 help
calls. Presets are inputs, not proof of live feasibility or permission to spend.
[Deferred acceptance](plans/deferred/live_integration_acceptance.md) specifies the
remaining lifecycle and cover gates.

## Runner regression map

| Contract | Maintained tests |
| --- | --- |
| Every provider is selected or explicitly excluded | `tests/provider_catalogs.rs::live_provider_scope_accounts_for_every_bundled_definition` |
| Exact reviewed requests, caps, schemas and wallet arrangements | `examples/live_integration/presets.rs` and independent request fixtures |
| Authenticated MCP signs exactly once | `tests/server.rs::authenticated_mcp_paid_call_signs_once_and_delivers_the_provider_result`, `tests/payments.rs` |
| Crash/failure cannot replay or refund a charged attempt | Runner registry tests and `tests/integration_preparation.rs` real-process tests |
| Prior permits and liabilities survive new runs | `src/qualification/funding/registry_state.rs` and immutable-baseline tests |
| Complete frozen inventories and typed loading failures | `tests/startup.rs`, `tests/integration_preparation.rs` |
| Identity/origin scope and confined egress | Managed/unsigned identity-map tests, `tests/network_audit.rs`, optional `tests/TOR.md` procedure |
| Parent death, drain and unsigned refusal before signing | `tests/supervision.rs` |

MCP success, useful provider content, seller receipts, consumed nonces and verified
canonical debits are different facts. Missing proof is not a zero fee. Financial
ledgers, attempted case IDs and failure evidence remain private and immutable;
removing a Git report never resets authority or permits a retry.

## Generated artifacts

Write live reports, metrics and audit inventories under ignored `target/` or an
operator-selected private external directory. Preserve raw evidence locally with
its exact build/configuration pins. Commit durable findings as tests, provider
configuration, maintained documentation or focused unfinished plans. Do not append
dated run logs, handoffs, balances or report copies to these documents.
