# Coverage workflow and test map

See [testing and qualification](../docs/testing.md) for commands and
qualification limits. `make coverage` writes line, region, and function reports
under `target/` with source, compiler, feature, and lockfile provenance. Failed
or empty runs never replace a prior successful report (verified by
`scripts/tests/coverage.sh`). Application coverage excludes tests, examples,
operational tools, `vendor/`, and compatibility harnesses; compare only
matching denominators and feature sets.

| Boundary | Regression location |
| --- | --- |
| TLS, DNS and rebinding, SOCKS auth, pool and runtime identities | `src/network.rs` boundary tests, `tests/network.rs`, `tests/network_audit.rs` |
| Grants, source scope, immutable invocation snapshots, concurrent mutations | `src/discovery/tests.rs`, `tests/deployment.rs`, `tests/assignment.rs` |
| EIP-3009 and Permit2 signatures, exact payload, typed insufficient-funds traversal, retained liabilities | `tests/payments.rs`, `tests/managed.rs` |
| Canonical balances and nonces, credit, quote binding, funding transitions | `tests/base.rs`, `src/rotation/` tests |
| Encryption, unsupported-schema refusal, initialization, ownership, unsafe links, atomic backups | Treasury and store integrity and filesystem tests, `tests/cli_contracts.rs` |
| MCP auth, routes, fallback, pagination, process shutdown | `tests/server.rs`, `tests/stdio.rs`, `tests/shutdown.rs`, `tests/supervision.rs` |
| Catalog complexity, concurrent startup, authored prices, passive payment hints | `src/catalog.rs`, `tests/provider_catalogs.rs`, `tests/startup.rs`, `src/pricing/runtime.rs` tests, payment selection tests |
| Visible byte and display bounds, typed media, error redaction | `tests/download_limits.rs`, `tests/media.rs`, `tests/confidential_errors.rs` |
| Registry claims, canonical settlement, no replay, frozen catalogs | `tests/integration_preparation.rs`, `tests/live/runner/` tests |

Concurrency fixtures use multithreaded Tokio runtimes with barriers at
challenge, admission, or commit boundaries (`--test-threads=1` serializes test
cases, not callers inside a test), asserting observable overlap and unchanged
state on refusal.

Optional proving, direct native send, transparent-shielding refusal, expiry,
and Docker consensus paths live in [LIFECYCLE.md](LIFECYCLE.md) and
[REGTEST.md](REGTEST.md), and live-Tor confinement checks live in
[TOR.md](TOR.md). Run optional suites separately, never remove guards to raise
percentages, and never load mainnet keys in tests. The
[mutation pilot](../docs/plans/deferred/mutation_testing.md) remains deferred.

## Condition analysis

Use `make coverage-condition` for condition-only evaluation, adding
`make coverage-condition-financial` when financial changes need focused
proving, lifecycle, and codec coverage.

- **Pinned toolchain:** `cargo-llvm-cov` 0.9.1 with compiler revision
  `92b5b8c87c61b20adc9001b41baf587634a74734` (`1.101.0-nightly`, default
  installation `nightly-2026-10-31`, override via `CONDITION_TOOLCHAIN`) and
  its LLVM 23.1.4 tools (selected by absolute path even on Homebrew hosts; the
  archive date is verified from the installed manifest).
- **Mode and known warning:** Runs `-Zcoverage-options=condition` without
  `--branch` (reporting line, region, function, and condition outcomes; MC/DC
  is unsupported). This nightly emits a future-compatibility trait-depth
  warning on the deferred catalog and paid relay `Send` proof; preserve the
  diagnostic without suppressing it or raising recursion limits.
- **Outputs and concurrency:** Default, condition, branch, and lifecycle
  coverage selections use `TEST_THREADS` (default 8; CI sets `$(nproc)`), while
  proving, codec, and consensus selections stay at 1 thread. Condition runs
  publish separately to `target/coverage-condition` and
  `target/coverage-condition-financial`. The financial supplement runs proving
  (1 thread), lifecycle (`TEST_THREADS`), and codec isolation (1 thread)
  sequentially under `zcash-testutils` (asserting nonzero tests per filter).
  Keep selections identical across baseline and final runs. Because
  `tools/protoc` uses a sibling target directory, application Cargo initializes
  the coverage target root first.
- **Exported JSON views:** `full.json` keeps default exclusions (hiding test
  folders and `*_tests.rs`). Condition modes also export `all-sources.json`
  from the same profiles with defaults disabled so operational runner and tool
  paths can be inspected without running another suite.
- **Gap triage:** Disposition missing outcomes as obsolete code to remove,
  retained behavior needing a functional assertion, or tooling-limited
  behavior, correlating high-risk gaps with CogC and CycC scores. Follow
  [testing efficiency](../docs/testing.md#testing-efficiency): run one baseline
  and one matching final pass rather than a per-test matrix or percentage-only
  tests, and never use instrumented timing as a performance baseline.
