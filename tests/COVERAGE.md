# Coverage workflow and test map

Use [the testing guide](../docs/testing.md) for commands and qualification status.
`make coverage` writes line, region and function evidence under `target/`.
Each report records source/compiler/features/lockfile provenance. Failed or empty
collections must not replace a successful report; `scripts/tests/coverage.sh`
checks that publication boundary. Compare identical denominators and features.
Stable regions do not establish branch or mutation coverage. The application
report excludes tests, examples, operational tools, vendor and compatibility sources.
Moving maintenance utilities to `tools/` preserves that denominator policy and does
not establish a coverage improvement.

| Boundary | Regression location |
| --- | --- |
| TLS, DNS/rebinding, SOCKS authentication, pool/runtime identities | `src/network.rs` boundary tests, `tests/network.rs`, `tests/network_audit.rs` |
| Grants, source scope, immutable invocation snapshots, concurrent mutations | `src/discovery/tests.rs`, `tests/deployment.rs`, `tests/assignment.rs` |
| EIP-3009/Permit2 signatures, exact payload, typed insufficient-funds traversal and retained liabilities | `tests/payments.rs`, `tests/managed.rs` |
| Canonical balances/nonces, credit, quote binding and funding transitions | `tests/base.rs`, `src/rotation/` tests |
| Encryption, migrations, ownership, unsafe links and atomic backup failures | Treasury/store integrity and filesystem tests, `tests/cli_contracts.rs` |
| MCP auth, routes, fallback, pagination, process shutdown | `tests/server.rs`, `tests/stdio.rs`, `tests/shutdown.rs`, `tests/supervision.rs` |
| Catalog complexity, concurrent startup, authored prices and passive payment hints | `src/catalog.rs`, `tests/provider_catalogs.rs`, `tests/startup.rs`, `src/pricing/runtime.rs` tests and payment selection tests |
| Visible byte/display bounds, typed media and error redaction | `tests/download_limits.rs`, `tests/media.rs`, `tests/confidential_errors.rs` |
| Registry claims, canonical settlement, no replay and frozen catalogs | `tests/integration_preparation.rs`, `tests/live/runner/` tests |

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

## Condition analysis

For the pruning evaluation, use `make coverage-condition` and then
`make coverage-condition-financial`. These select one condition-only analysis
workflow with cargo-llvm-cov 0.9.1, compiler revision
`92b5b8c87c61b20adc9001b41baf587634a74734` (1.101.0-nightly), and its LLVM 23.1.4
tools. The default installation is `nightly-2026-10-31`;
`CONDITION_TOOLCHAIN` can name another installation of that exact revision.
The script verifies it and selects absolute compiler/tool paths, including on
Homebrew hosts. The installed manifest verifies the archive date; the compiler
commit date alone does not identify an archive.

The condition wrapper probe retained four locations/eight true/false outcomes,
including the last operand of an assigned conjunction. Its recursive-error
async-drop reproducer also compiled. This analysis pin replaces the original
probe compiler, which failed application compilation with
[Rust regression #165263](https://github.com/rust-lang/rust/issues/165263).
`-Zcoverage-options=condition` is used without `--branch`; it includes line,
region, function and branch evidence. It does not prove MC/DC independence or
exhaustive paths. The completed support probe found MC/DC unsupported; none is
planned. Do not substitute another mode or run a compiler/profile/feature matrix
if collection fails; resolve the specific blocker.

Default condition and financial reports publish separately to
`target/coverage-condition` and `target/coverage-condition-financial`.
The financial supplement selects the existing proving module, lifecycle module
with submodules, and codec isolation test sequentially under `zcash-testutils`,
retaining profiles across the filters. Each filter must list nonzero tests.
This is broader than the existing single-test `coverage-proving` selection;
it does not rerun the default suite or authorize consensus, Tor or funded work.
Both use the ordinary test profile. Fixed concurrency matches the maintained
ordinary stages: default and lifecycle use eight threads; proving and codec use
one. Keep these selections identical for baseline and final collection; existing
other coverage modes retain their serial selection. Record exact selections/counts, raw child
profiles, source hashes and tool provenance; keep baseline and final artifacts.
The protoc helper uses a separate sibling target directory, so application Cargo
initializes the coverage target root before later workspace cleanup. Preserve
failed analysis artifacts; do not bypass Cargo's cache-directory protection.

Inspect full JSON locations and missing outcomes with complete call chains.
Disposition gaps as obsolete behavior to remove, retained behavior needing a
focused functional assertion, or tooling-limited/unobserved behavior with a reason.
Compare unchanged sources and map changed functions explicitly; distinguish deleted
denominators from lost outcomes in retained code. Correlate high-risk gaps with
CogC/CycC and the maintained test map. Application filters exclude operational
runner/tools and integration harnesses; inline helpers can still contribute.
`full.json` retains the wrapper's default exclusions, which also hide test
folders and `*_tests.rs` files. Condition modes additionally export
`all-sources.json` with those defaults disabled, from the same collected profiles.
Select operational runner/tool paths explicitly from that raw export for their
separate view; it includes dependencies and helpers outside the application
denominator. Mark absent paths unobserved. Do not collect another suite to fill
that view.

Follow [testing efficiency](../docs/testing.md#testing-efficiency): one default baseline
and matching final condition run, with only the necessary focused supplements.
Equivalent hit locations do not establish redundant assertions. Avoid a suite-wide
per-test coverage matrix or trivial tests added to raise percentages. Instrumented
timing is not the ordinary performance baseline. Measure ordinary unchanged test
executables with identical selections and record source/profile/cache provenance;
compare focused profiles only when aggregate evidence cannot resolve a candidate.
