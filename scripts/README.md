# Development tools

The root Makefile is the entrypoint for common operations. It requires GNU Make
3.81+ (including macOS's installed Make), a POSIX shell and standard shell tools.
The documented engineering scope is Linux and macOS; Windows runtime and native
validation work is deferred by user decision. Install Rust/Cargo
matching `rust-toolchain.toml`; Make verifies both versions before application
Cargo commands, including under Homebrew. Protoc comes from the locked Cargo
helper, not a system installation. The application and normal Rust tests do not
need Python. The full `make check` also requires Python 3.11+ for development tools.
If `python3` selects an older version, use `make check PYTHON=python3.13`
(or another installed Python 3.11+ executable).

Run Make from the repository root, or use `make -C /path/to/checkout TARGET`.
`make help` and dry runs do not invoke Cargo or build protoc. Targets run serially
even under `make -j`, since CLI fixtures share executable paths. Separate Make
processes still need coordination; do not change sources or build feature
configurations concurrently during validation.

Test commands use `scripts/test-runner.sh`. On macOS it supplies a Cargo runner
that copies each test executable without extended attributes into a private
temporary directory, ad-hoc signs the copy, and executes it. The original Cargo
artifact stays intact. Copies remain in the system temporary directory so
`current_exe()` child fixtures can reuse them. Linux entrypoints execute the usual command directly. This also applies to the analyzer
and compatibility test stages; ordinary build/run, coverage and release
qualification commands retain their existing execution paths.

Cargo's explicit target-triple runners, including environment overrides, take
precedence. If another cfg-based runner also matches macOS, Cargo reports the
ambiguity; select the intended runner with a target-triple setting instead. The
workaround does not redirect CLI fixtures using compiled `CARGO_BIN_EXE_*` paths.
See [test executable startup](../docs/testing.md#test-executable-startup) for the
platform issue and identity boundaries.

`CARGO_ARGS` is a shell-quoted argument list. For `test` and `clippy`, it **replaces**
the default `--all-targets`; include that flag explicitly when adding options for
the complete suite. `TEST_THREADS` selects libtest concurrency for `make test` and
`check-default` / `check-default-remaining` (default 8); use `TEST_THREADS=1`
for a serial run. `TEST_ARGS` replaces the complete default harness argument list,
including that thread setting.
Do not include the separating `--` in `TEST_ARGS`; Make supplies it.
`run` accepts both Cargo and application arguments in `CARGO_ARGS`, separated by
`--`. These are trusted shell fragments, not a structured argument API. Quote the
outer assignment so your shell passes it as one Make argument; retain inner quotes
for individual arguments containing spaces. Use `$$` for a dollar sign that must
reach the recipe shell (for example, `"$$BASE_RPC_URL"` inside a single-quoted
assignment). Do not place secrets in command arguments.

```sh
make test CARGO_ARGS='--test payments' TEST_ARGS='case_name --exact --test-threads=1'
make test CARGO_ARGS='--all-targets --offline'
make test TEST_THREADS=1
make run CARGO_ARGS='--example snapshot_spec -- "path with spaces/input.json" /tmp/output.json'
make build PROFILE=dev CARGO_ARGS='--offline'
```

Build targets own profile selection: use `PROFILE=NAME` with `build` or
`build-dev` for incremental dev; Cargo profile flags in `CARGO_ARGS` are rejected.
`make check` and its named `check-*` stages use fixed selections regardless of
`CARGO_ARGS` and `TEST_ARGS`. The full check runs static/tooling/complexity/provenance,
production compiler diagnostics and full feature Clippy, compatibility, the remaining
default targets, then proving/lifecycle/codec stages. It stops on the first failing
stage even with `make -k`. Individual stages run independently and always execute when requested. See
[validation order and continuation](../docs/testing.md#validation-order-and-continuation)
for the stage table, independent review and evidence needed to resume after a fix.
Coordinate editor-triggered Cargo checks as well as separate Make invocations;
`.NOTPARALLEL` only serializes one invocation.

`TEST_THREADS` changes only scheduling in either default stage, not its fixed target
selection. The lifecycle stage uses eight test threads; proving and codec stages
retain one. Build and feature stages remain sequential even when a harness runs
tests concurrently.

`make check-default` always runs the complete standalone `--all-targets` suite.
Combined `make check` uses `check-default-remaining` after `check-compat` so its
four integration targets execute once. The remainder helper obtains locked Cargo
metadata at execution time, uses its root target inventory/resolved features and
only the root test/bench `bench` flags omitted by metadata, then passes named
eligible integration/bench selectors through the existing Cargo runner. Cargo
keeps library/binary/example gating and harness behavior. Workspace default
members must still select only the root application; changed scope or inconsistent
targets fail before remainder tests. Python 3.11+ is required for this helper.

Use `make check-default-remaining` to continue only while earlier compatibility
evidence remains valid. Explicitly requesting `check-compat` and then standalone
`check-default` intentionally requests the full suite again. Repeated stages always
execute; there is no saved-success cache. Help and dry runs do not resolve metadata.

`make check-clippy` runs the production library/binary compiler check followed by
all-target `zcash-regtest` Clippy. Both use `check_warnings.py` to display Cargo
compiler diagnostics and reject emitted warnings, including dependency warnings
and cached diagnostics. This fixed stage requires Python; ordinary `make clippy`
keeps Cargo's selected-target `-D warnings` behavior and requires no Python.
See [compiler diagnostics and review](../docs/testing.md#compiler-diagnostics-and-review)
for enforcement boundaries and evidence requirements.

Coverage keeps an internal shell helper for provenance, failure preservation and
atomic report publication; invoke its public Make targets below. Specialized Python tools remain
available directly.

| Tool | Purpose |
| --- | --- |
| `python3 scripts/reproducible.py` | Two clean, network-denied release builds with binary hash comparison; see [release qualification](../docs/reproducible-builds.md) |
| `make toolchain` | Verify Rust/Cargo match `rust-toolchain.toml`, including Homebrew installations |
| `make check` / `make check-*` | Ordered full check / individual stages; selections and continuation rules in the [testing guide](../docs/testing.md#validation-order-and-continuation) |
| `python3 scripts/complexity.py` | Pinned complexity metrics and advisory regressions against merge-base with `main`; Markdown and complete JSON/CSV under `target/complexity` |
| `make coverage` | Default Zcash build coverage; HTML, JSON and text under `target/coverage` |
| `make coverage-condition` | Pinned condition-only default analysis; `target/coverage-condition` |
| `make coverage-condition-financial` | Sequential proving/lifecycle/codec condition supplement; `target/coverage-condition-financial` |
| `make coverage-branch` | Nightly branch coverage; separate reports under `target/coverage-branch` |
| `make coverage-proving` | Only the named synthetic proving test; `target/coverage-proving` |
| `make coverage-consensus TEST=NAME` | One explicitly selected consensus test; `target/coverage-consensus-NAME` |
| `scripts/tests/coverage.sh` | Offline regression check for report publication and failure preservation |
| `make build` / `make test` | Cargo-managed protoc; the application always includes Zcash |
| `scripts/test-runner.sh --cargo cargo test …` | Automatic macOS copied-and-signed test execution; other hosts execute Cargo directly |
| `make check-compat` | SDK wallet, payment, MCP and cryptographic integration targets with the root lockfile and build cache |
| `make check-default` / `make check-default-remaining` | Complete standalone default suite / metadata-derived complement after still-valid compatibility evidence |
| `tools/protoc/` | Independently locked build utility locating the bundled protoc compiler |
| `generate_crypto_vectors.py` | Independent Python Keccak/EIP-712 reference vectors using pinned script dependencies |

Zcash support is unconditional; static-wallet deployments omit treasury
configuration. Only synthetic proving and local consensus are opt-in test features.
Run feature configurations sequentially: CLI tests share
`target/debug/x402_treazury`. `make check` leaves the application executable there.
Its complexity audit warns above CogC 20 or CycC 40 for new/unmatched or worsening
callables; scores never fail tests. Missing baseline history or incomplete analysis
fails the audit. Set `COMPLEXITY_BASE_REF` for another review target; CI must fetch
that ref and its merge-base history. See [audit details](../tests/COMPLEXITY.md).
It also runs the feature-gated `treasury::send::proving_tests` with `zcash-testutils`
after the default suite, exercising actual SDK input selection, proving, typed
insufficient funds and retained identical publication after local storage failure.
The synthetic-chain lifecycle separately covers real actor scans and submission.
All default checks use local fixtures and public test keys; none reads `.env` or
performs funded transactions. Local socket tests require permission to bind localhost.
Cargo and Zingolib may download dependencies/public proving parameters on first build.
Set `CARGO_NET_OFFLINE=true` to require cached Cargo inputs; it does not block
network requests from build scripts. Release qualification adds an OS network
sandbox and validates prefetched proving parameters. The release runner discovers
the resolved upstream SDK through frozen metadata in each isolated environment,
records its observed Git inputs and actual generated descriptor, and retains that
evidence on failure. Shared-cache comparison does not qualify independent caches.
`vendor/verify_pepper.py` verifies the retained Pepper snapshot/patches and
copied-code notice, including an exact upstream-tree comparison after patch
reversal; both graphs use ordinary pinned upstream SDK/netutils sources.

`make build` (also bare `make`) produces `target/release/x402_treazury` with
incremental caches disabled, including for the protoc helper. `make build-dev`
selects an incremental development build at `target/debug/x402_treazury`.
`make build PROFILE=NAME` selects a non-incremental build with another profile.
Tests, checks and Clippy retain normal Cargo profiles and environment behavior.
See [build storage options](../docs/development.md#build-storage-options) for
debug-information settings and selective cache cleanup, and
[compilation parallelism](../docs/development.md#compilation-parallelism) for
Cargo's existing job controls and timing reports. The focused SDK/payment tests share the
application target directory and remain a separately selected, sequential stage.

## Local dependency advisory assessment

`python3 scripts/dependency_audit.py` replays an operator-installed `cargo-audit`
against a clean RustSec database checkout at an explicitly supplied full Git SHA.
It does not install tools, fetch advisories, resolve Cargo dependencies or build.
Freeze source changes while collecting evidence. Run the existing
`python3 vendor/verify.py` and `python3 vendor/verify_pepper.py` provenance checks
separately; advisory matching does not verify that local patches preserve fixes.

Supply `--db /path/to/advisory-db --db-revision FULL_SHA` and repeat
`--source PACKAGE=/actual/package/directory` for **every** nonregistry package in
both `Cargo.lock` and `tools/protoc/Cargo.lock`. This includes workspace packages,
all vendored packages and the pinned Git packages, including optional test
packages. For example, the root package mapping is `--source x402_treazury=.`,
the helper mapping is `--source x402-protoc-path=tools/protoc`, and a patched
transport mapping is `--source h2=vendor/h2`. Use the actual locked Git checkout,
not an unrelated reference revision. Missing mappings fail visibly. Use
`--cargo-audit /path/to/cargo-audit` to select the installed executable and
`--output target/dependency-audit` to select the artifact directory.

The tool snapshots and hashes both active lockfiles, records the auditor version
and executable hash, checks the database revision and clean tree, and hashes the
actual Git-inventoried package files (tracked plus unignored files). Ignored local
state and build artifacts are excluded; symlinks are recorded without following
them. Git dependencies must have the exact locked HEAD and a clean checkout.
Workspace-inherited versions are resolved from their workspace manifests.

Three JSON reports cover the two original lockfiles and a synthetic nonregistry
name/version inventory. RustSec normally skips path and Git sources. The synthetic
inventory gives those exact package versions a crates.io identity **only for
advisory matching**; `summary.json` retains their original source, package path
and file hashes. It is not a dependency resolution or evidence that a Git package
is byte-identical to the same registry version. Inspect advisory applicability
against those actual sources and patches before assigning a disposition.

All audit calls use `--no-fetch --no-yanked --stale --json`: the pinned historical
database is allowed, network-dependent yank checks are omitted, and no advisory
IDs, architectures or operating systems may be filtered by ambient audit settings.
Exit 0 means no reported findings; 1 means findings requiring assessment; 2 means
tool/input failure and incomplete evidence. Raw stdout/stderr and the final status
are retained under the output directory. Findings do not prove exploitability;
a clean report does not establish a complete security audit. Runtime, build-only,
optional-test and helper exposure need separate dispositions. Vendored upstream
lockfiles are provenance inputs, not additional active resolutions.

Run the focused offline tests with
`python3 -m unittest scripts.tests.test_dependency_audit`.

Regenerate the independent cryptographic vectors with:

```sh
uv run --script scripts/generate_crypto_vectors.py
make test CARGO_ARGS='--test crypto_vectors'
```

`uv` is needed only for this optional regeneration command; it manages an isolated
script environment without a Python application package or project virtualenv.
Normal tests consume committed vectors. Review changes rather than automatically
accepting new expected values.

Build OpenAPI request fixtures with the Rust utility:

```sh
make run CARGO_ARGS='--example snapshot_spec -- SOURCE_JSON_OR_URL OUTPUT_JSON'
```

See `tests/fixtures/catalogs/README.md` for provider contract review and
`tests/REGTEST.md` for explicitly enabled Docker consensus tests. The runtime Tor
inventory is documented in `docs/network-egress.md`. The optional macOS
`scripts/qualify_tor.py` runner observes real circuits and validates SOCKS-only
process confinement using an installed Tor binary; see `tests/TOR.md`. Its
standard-library Python observer tests run with
`python3 -m unittest discover -s scripts/tests -p test_tor_qualification.py`.

Coverage requires `cargo-llvm-cov` and LLVM tools matching `rustc -vV`. Homebrew
LLVM is detected; explicit `LLVM_COV` / `LLVM_PROFDATA` overrides take precedence.
With rustup, install `llvm-tools-preview`. The report excludes integration test
harnesses, examples, operational tools and vendor code, but can include inline unit-test helpers.
CLI tests clear inherited environment except `LLVM_PROFILE_FILE` for subprocess
coverage. Never replace this with unrestricted environment inheritance.

Each coverage run lives in `target/coverage-runs/`. The mode's report path points
to the latest successful run; failed runs keep their logs and do not replace it.
Existing report directories are retained inside the first new run. `provenance.txt`
records the revision, dirty/untracked paths, lockfile hash, compiler/tool versions,
feature mode and exclusion rule. `commands.txt`, `tests.txt` and `test.log` capture
exact commands, selected tests and outcomes. `full.json` and
`unfiltered-summary.json` retain evidence excluded from `summary.json`/HTML.
`profiles.txt` inventories collected raw profiles, including CLI children that
preserve `LLVM_PROFILE_FILE`. Abruptly exited children may not flush coverage;
parent assertions establish their test outcomes independently. Run from a stable
working tree and do not run two coverage commands concurrently for the same mode.

Optional modes do not run the default suite. Consensus accepts only the
named cases in `tests/REGTEST.md`, one per process; it never enables a broad ignored
test filter. Docker/images must already be available. No live Tor or mainnet tests
are selected. Keep optional results distinct from default-build percentages.

The complexity reporter builds the `treazury-complexity` workspace package in
`tools/complexity/`, sharing the application's root lockfile and development
profile. It selects only the analyzer and writes to the shared root `target/`
directory, regardless of inherited `CARGO_TARGET_DIR`. Python 3.11+ assembles
reports and joins existing LLVM coverage only for byte-identical source files.
See [COMPLEXITY.md](../tests/COMPLEXITY.md) for parser qualification, score
definitions, baseline findings and limitations. `make check-static` runs both
report-integrity tests and the analyzer's syntax probes. For focused checks:

```sh
make toolchain
scripts/test-runner.sh --cargo cargo test --locked -p treazury-complexity --target-dir target
cargo clippy --locked -p treazury-complexity --target-dir target --all-targets -- -D warnings
```

The opt-in `measure_startup_tor.py` runner compares unsigned startup time and
per-process peak RSS through an existing, warmed Tor SOCKS endpoint. It retains
the daemon and its learning state; see [startup qualification](../tests/STARTUP.md).

Condition-only comparisons use the pinned
[condition analysis workflow](../tests/COVERAGE.md#condition-analysis), with one
test profile and isolated target directories. The helper selects absolute nightly
compiler and matching LLVM paths and verifies the compiler revision and wrapper
version. It records tracked source hashes, per-filter listings and child profiles;
failures preserve earlier reports. Do not multiply the normal check stages across
analysis compilers, features, profiles or coverage modes. Follow
[testing efficiency](../docs/testing.md#testing-efficiency) for evidence reuse,
setup consolidation and deterministic synchronization. Existing optional coverage
commands are separate capabilities, not additional evaluation requirements.

## Vendor source verification

Vendor verification reconstructs byte-exact sources and patches without requiring
Rust. Patch bytes are supplied through binary stdin with the restored tree as the
working directory, avoiding platform path arguments. GNU patch uses `--binary` to
preserve source bytes on hosts with text-mode I/O; failures retain its stdout,
stderr and exit status. Original-source hash mismatches report expected/actual
digests and whether CRLF-to-LF conversion would match for diagnostics only. Every
mismatch remains fatal and no source is normalized.
