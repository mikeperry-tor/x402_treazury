# Development tools

The root `Makefile` is the entrypoint for builds, tests, checks, coverage, and
operational utilities. Run Make from the repository root (or via
`make -C /path/to/checkout TARGET`).

## Prerequisites and execution rules

- **Make and shell:** GNU Make 3.81+ (including macOS Make) and a POSIX shell.
  Supported platforms are Linux and macOS (Windows runtime and native
  validation are deferred).
- **Rust and protoc:** Install Rust and Cargo. Local build and validation
  targets use the host's selected toolchain, subject to Cargo's minimum Rust
  version in [`Cargo.toml`](../Cargo.toml). The independently locked helper in
  `tools/protoc/` supplies `protoc`.
  Formatting and lint checks also need rustfmt and Clippy. For a minimal
  rustup installation, add them with `rustup component add rustfmt clippy`.
- **Python:** Not needed for the application or `make test`. Full `make check`,
  `make check-static`, `make check-clippy`, `make check-compat`,
  `make check-default-remaining`, and complexity scripts require Python 3.11+
  (override via `make check PYTHON=python3.13`).
- **Serial execution:** Targets serialize within one Make invocation even under
  `make -j` because CLI fixtures share `target/debug/x402_treazury`. Coordinate
  separate Make processes and editor Cargo checks manually, and never run
  conflicting feature builds concurrently during validation.
- **macOS test runner (`scripts/test-runner.sh`):** On macOS, copies each test
  executable without extended attributes into a private temp directory, ad-hoc
  signs it, and runs the copy (leaving Cargo's artifact intact and letting
  `current_exe()` child fixtures reuse it). Linux runs Cargo directly. Explicit
  target-triple runners take precedence, and `CARGO_BIN_EXE_*` paths are not
  redirected. See [test executable startup](../docs/testing.md#test-executable-startup).

## Common entrypoints

| Tool or target | Purpose |
| --- | --- |
| `make` or `make build` | Optimized release build (`CARGO_INCREMENTAL=0`, including `protoc`) |
| `make build-dev` | Incremental dev build (`target/debug/x402_treazury`) |
| `make build PROFILE=NAME` | Non-incremental build with another profile |
| `make toolchain` | Verify Rust and Cargo match [the CI and release pin](rust-toolchain-pin.toml) |
| `make test` | Run Cargo tests through `scripts/test-runner.sh` |
| `make cargo-check` and `make run` | Run `cargo check` or `cargo run` with locked dependencies and bundled `protoc` |
| `make fmt` and `make fmt-check` | Format Rust sources or verify `cargo fmt --check` |
| `make clippy` | All-target Clippy with `-D warnings` (no Python required) |
| `make check-clippy` | Production compiler and all-target `zcash-regtest` Clippy via `check_warnings.py` (catches dependency and cached warnings; requires Python) |
| `make check` and `make check-*` | Ordered full check or individual stages (`check-static`, `check-clippy`, `check-compat`, `check-default`, `check-default-remaining`, `check-proving`, `check-lifecycle`, `check-codec`); see [validation order](../docs/testing.md#validation-order-and-continuation) |
| `make check-compat` | Pepper verification plus `sdk_compat`, `payments`, `server`, and `crypto_vectors` |
| `make check-default` | Complete standalone `--all-targets` default suite |
| `make check-default-remaining` | Metadata-derived complement after still-valid `check-compat` (requires Python 3.11+) |
| `make coverage` | Default coverage reports (HTML, JSON, text, LCOV) under `target/coverage` |
| `make coverage-condition` | Pinned condition-only default analysis under `target/coverage-condition` |
| `make coverage-condition-financial` | Sequential proving, lifecycle, and codec condition supplement |
| `make coverage-branch` | Nightly branch coverage under `target/coverage-branch` |
| `make coverage-proving` | Synthetic proving test coverage under `target/coverage-proving` |
| `make coverage-consensus TEST=NAME` | Single consensus test coverage under `target/coverage-consensus-NAME` |
| `python3 scripts/complexity.py` | Complexity metrics and merge-base hotspot warnings under `target/complexity` |
| `python3 scripts/reproducible.py` | Two clean, network-denied release builds with hash comparison; see [reproducible builds](../docs/reproducible-builds.md) |
| `scripts/tests/coverage.sh` | Offline regression test for coverage publication and failure preservation |
| `tools/protoc/` | Independently locked build helper locating bundled `protoc` |
| `scripts/generate_crypto_vectors.py` | Regenerates independent Keccak and EIP-712 vectors via `uv` |

With rustup-managed Cargo and Rust on `PATH`, rustup selects its configured
default or explicit override. Set `RUSTUP_TOOLCHAIN` to select a different
installed version for a command, such as `RUSTUP_TOOLCHAIN=1.96.1 make build`.
With a direct Homebrew or system install, select a version by putting its
binaries first on `PATH`. Run `make toolchain` to verify the selected Rust
and Cargo against the CI and release pin. See
[release qualification](../docs/reproducible-builds.md) for byte-for-byte checks.

## Arguments and quoting

- `CARGO_ARGS` is a shell-quoted fragment; for `test` and `clippy`, it
  **replaces** the default `--all-targets` (include `--all-targets` if you want
  the full suite). Never pass Cargo profile flags in `CARGO_ARGS` for `build`
  or `build-dev`; use `PROFILE=NAME` on `build`.
- `TEST_THREADS` sets libtest concurrency for `make test`, `check-compat`,
  `check-default`, `check-default-remaining`, `check-lifecycle`, and parallel
  `coverage*` targets (default `8`). `TEST_ARGS` replaces the entire harness
  argument list (do not include the separating `--`; Make adds it).
- For `make run`, pass Cargo and application flags in `CARGO_ARGS` separated by
  `--`. Double `$` as `$$` when an environment variable should expand in the
  recipe shell, and never pass secrets in CLI arguments:

```sh
make test CARGO_ARGS='--test payments' TEST_ARGS='case_name --exact --test-threads=1'
make test CARGO_ARGS='--all-targets --offline'
make test TEST_THREADS=1
make run CARGO_ARGS='--example snapshot_spec -- "path with spaces/input.json" /tmp/output.json'
make build PROFILE=dev CARGO_ARGS='--offline'
```

## Check stages and CI diagnostics

Select checks using [validation by change](../docs/testing.md#selecting-validation-by-change).
`make check` and named `check-*` stages ignore `CARGO_ARGS` and `TEST_ARGS`
(`TEST_THREADS` still adjusts `check-compat`, `check-default`,
`check-default-remaining`, and `check-lifecycle`), stop on the first failing
stage even under `make -k`, and always execute when invoked (no cached pass
state).

- **Combined `make check` flow:** Runs static, tooling, complexity, and
  provenance checks; `check-clippy`; `check-compat`; `check-default-remaining`
  (deriving remaining targets from locked Cargo metadata so the four compat
  targets run once); and then `zcash-testutils` proving (1 thread), lifecycle
  (`TEST_THREADS`, default 8), and codec (1 thread) with
  `CARGO_PROFILE_TEST_OPT_LEVEL=2` reusing the shared `target/debug`
  dependencies.
- **CI stage wrapper (`scripts/ci-stage.sh CHECK_STAGE`):** Runs one of the
  seven stages, streams combined output, writes logs, elapsed seconds, and exit
  status to `target/ci-checks/`, and appends to `GITHUB_STEP_SUMMARY` when set.
  See [GitHub Actions and coverage](../docs/testing.md#github-actions-and-coverage).
- **Opt-in lifecycle diagnostics (`LIFECYCLE_DIAGNOSTICS=DIR`):** Compiles
  `make check-lifecycle` with JSON metadata (`build.jsonl`,
  `build-status.json`), records `status.json`, and captures periodic
  `snapshot-N/` CPU, memory, and native stacks via GDB (requiring same-user
  ptrace permission on Linux) with no lifecycle deadline.
- **Treasury hang workflow (`scripts/diagnose-treasury-sync.py`):** Compiles
  `--test treasury_sync --no-run --message-format=json`, runs 8 threads with
  `--nocapture` inside a 5-minute diagnostic window (outside compilation), and
  captures GDB stacks before exiting `124` on timeout.

## Local dependency advisory assessment

`python3 scripts/dependency_audit.py` replays an installed `cargo-audit`
against a clean RustSec database checkout at an exact full Git SHA without
installing tools, fetching advisories, or building. Freeze source edits while
collecting evidence, and run `vendor/verify.py` and `vendor/verify_pepper.py`
separately.

```sh
python3 scripts/dependency_audit.py \
  --db /path/to/advisory-db --db-revision FULL_SHA \
  --source x402_treazury=. \
  --source x402-protoc-path=tools/protoc \
  --source h2=vendor/h2 ...
```

- **Required mappings:** Repeat `--source PACKAGE=DIR` for **every** path and
  Git package in both `Cargo.lock` and `tools/protoc/Cargo.lock` (workspace,
  vendored, and pinned Git crates, including optional test crates). Missing
  mappings or dirty Git checkouts fail visibly. Optional flags:
  `--cargo-audit PATH` and `--output target/dependency-audit`.
- **Reports:** Emits three JSON reports (both original lockfiles plus a
  synthetic nonregistry inventory giving path and Git packages a crates.io
  identity solely for RustSec matching, while `summary.json` retains original
  sources and file hashes). Runs with `--no-fetch --no-yanked --stale --json`.
- **Exit codes:** `0` = no findings, `1` = findings requiring source-level
  assessment, `2` = tool or input error.
- **Tests:** `python3 -m unittest scripts.tests.test_dependency_audit`.

## Coverage, complexity, and specialized utilities

- **Coverage (`make coverage`):** Requires `cargo-llvm-cov` and LLVM tools
  matching `rustc -vV` (override with `LLVM_COV` and `LLVM_PROFDATA`). Writes
  timestamped runs under `target/coverage-runs/` (updating the mode's report
  path only on success, preserving logs on failure) with `provenance.txt`,
  `commands.txt`, `tests.txt`, `test.log`, `profiles.txt`, `full.json`,
  `summary.json`, `lcov.info`, and HTML. Subprocess CLI tests inherit only
  `LLVM_PROFILE_FILE`. Optional modes (`coverage-condition`,
  `coverage-condition-financial`, `coverage-branch`, `coverage-proving`,
  `coverage-consensus`) keep separate directories and never run the default
  suite. See [COVERAGE.md](../tests/COVERAGE.md).
- **Complexity (`scripts/complexity.py`):** Builds workspace package
  `treazury-complexity` in `tools/complexity/` into root `target/`, warns above
  CogC 20 or CycC 40 for new or worsening callables against `main` (or
  `COMPLEXITY_BASE_REF`), and joins coverage only for byte-identical files.
  Focused analyzer check:

```sh
scripts/test-runner.sh --cargo cargo test --locked -p treazury-complexity --target-dir target
cargo clippy --locked -p treazury-complexity --target-dir target --all-targets -- -D warnings
```

- **Crypto vectors:** Regenerate with
  `uv run --script scripts/generate_crypto_vectors.py` and verify via
  `make test CARGO_ARGS='--test crypto_vectors'`. Normal tests use committed
  vectors without `uv`.
- **OpenAPI fixtures:** `make run CARGO_ARGS='--example snapshot_spec -- SOURCE OUTPUT'`.
- **Tor and startup measurement:** `scripts/qualify_tor.py` (macOS live-Tor
  observation; tests via
  `python3 -m unittest discover -s scripts/tests -p test_tor_qualification.py`;
  see [tests/TOR.md](../tests/TOR.md)) and `scripts/measure_startup_tor.py`
  (unsigned startup timing and peak RSS; see
  [tests/STARTUP.md](../tests/STARTUP.md)).

## Vendor source verification

See
[source verification boundaries](../vendor/README.md#source-verification-boundaries)
for change triggers and evidence reuse.

`vendor/verify.py` and `vendor/verify_pepper.py` reconstruct byte-exact
upstream sources without requiring Rust. Patch bytes pass via binary stdin with
GNU `patch --binary` (preserving line endings on text-mode hosts and retaining
stdout, stderr, and exit status on failure). Hash mismatches report expected
and observed digests (plus a diagnostic note if CRLF-to-LF conversion would
match) and always fail without normalizing files.
