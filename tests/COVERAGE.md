# Coverage workflow and test map

Use [the testing guide](../docs/testing.md) for commands and qualification status.
`scripts/coverage.sh` writes line, region and function evidence under `target/`.
Each report records source/compiler/features/lockfile provenance. Failed or empty
collections must not replace a successful report; `scripts/tests/coverage.sh`
checks that publication boundary. Compare identical denominators and features.
Stable regions do not establish branch or mutation coverage.

| Boundary | Regression location |
| --- | --- |
| TLS, DNS/rebinding, SOCKS authentication, pool/runtime identities | `src/network.rs` boundary tests, `tests/network.rs`, `tests/network_audit.rs` |
| Grants, source scope, immutable invocation snapshots, concurrent mutations | `src/discovery/tests.rs`, `tests/deployment.rs`, `tests/assignment.rs` |
| EIP-3009/Permit2 signatures, exact payload and no paid replay | `tests/payments.rs`, `tests/managed.rs` |
| Canonical balances/nonces, credit, quote binding and funding transitions | `tests/base.rs`, `src/rotation/` tests |
| Encryption, migrations, ownership, unsafe links and atomic backup failures | Treasury/store integrity and filesystem tests, `tests/cli_contracts.rs` |
| MCP auth, routes, fallback, pagination, process shutdown | `tests/server.rs`, `tests/stdio.rs`, `tests/shutdown.rs`, `tests/supervision.rs` |
| Catalog complexity, concurrent startup and one-shot pricing | `src/catalog.rs`, `tests/provider_catalogs.rs`, `tests/startup.rs`, `tests/pricing.rs` |
| Visible byte/display bounds, typed media and error redaction | `tests/download_limits.rs`, `tests/media.rs`, `tests/confidential_errors.rs` |
| Registry claims, canonical settlement, no replay and frozen catalogs | `tests/integration_preparation.rs`, `examples/live_integration/` tests |

Concurrency fixtures use multithreaded Tokio runtimes and barriers at challenge,
admission or commit boundaries. `--test-threads=1` serializes tests, not callers
inside a concurrency case. Require observable overlap and unchanged state on
refusal; simultaneous dispatch alone is insufficient. These establish selected
interleavings, not exhaustive freedom from races or maximum throughput.

Optional funded calculation, proof generation, shielding and expiry paths use
[LIFECYCLE.md](LIFECYCLE.md) and [REGTEST.md](REGTEST.md). Authenticated fixture
SOCKS verifies application identity routing; actual circuit/confinement checks use
[TOR.md](TOR.md). Run optional suites separately and retain their scope. Never
remove defensive guards to improve percentages or use mainnet keys in default tests.
The [mutation pilot](../docs/plans/deferred/mutation_testing.md) remains deferred.
