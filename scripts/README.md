# Development tools

The root Makefile is the entrypoint for common operations. It requires GNU Make
3.81+ (including macOS's installed Make), a POSIX shell and standard shell tools.
On Windows, use a GNU Make/POSIX shell environment such as MSYS2; this does not
establish native Windows application or qualification support. Install Rust/Cargo
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
`current_exe()` child fixtures can reuse them. Linux and Windows POSIX-shell
entrypoints execute the usual command directly. This also applies to the analyzer
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
`check-default` (default 8); use `TEST_THREADS=1` for a serial run. `TEST_ARGS`
replaces the complete default harness argument list, including that thread setting.
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
production compiler diagnostics and full feature Clippy, compatibility, default tests, then proving/lifecycle/codec
stages. It stops on the first failing stage even with `make -k`. Individual stages
run independently and always execute when requested. See
[validation order and continuation](../docs/testing.md#validation-order-and-continuation)
for the stage table, independent review and evidence needed to resume after a fix.
Coordinate editor-triggered Cargo checks as well as separate Make invocations;
`.NOTPARALLEL` only serializes one invocation.

`TEST_THREADS` changes only scheduling in `check-default`, not its fixed target
selection. The proving, lifecycle and codec stages retain one test thread. Build
and feature stages remain sequential even when a harness runs tests concurrently.

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
| `make coverage-branch` | Nightly branch coverage; separate reports under `target/coverage-branch` |
| `make coverage-proving` | Only the named synthetic proving test; `target/coverage-proving` |
| `make coverage-consensus TEST=NAME` | One explicitly selected consensus test; `target/coverage-consensus-NAME` |
| `scripts/tests/coverage.sh` | Offline regression check for report publication and failure preservation |
| `make build` / `make test` | Cargo-managed protoc; the application always includes Zcash |
| `scripts/test-runner.sh --cargo cargo test …` | Automatic macOS copied-and-signed test execution; other hosts execute Cargo directly |
| `check_compat.py` | Standard-library Python orchestration of the separate payment/Zingolib compatibility workspace |
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
`vendor/verify_zingo.py` verifies only the retained Pepper snapshot/patches and
copied-code notice; both graphs use ordinary pinned upstream SDK/netutils sources.

`make build` (also bare `make`) produces `target/release/x402_treazury` with
incremental caches disabled, including for the protoc helper. `make build-dev`
selects an incremental development build at `target/debug/x402_treazury`.
`make build PROFILE=NAME` selects a non-incremental build with another profile.
Tests, checks and Clippy retain normal Cargo profiles and environment behavior.
See [build storage options](../docs/development.md#build-storage-options) for
debug-information settings and selective cache cleanup.

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

The optional complexity reporter builds a small independent Rust helper in
`tools/complexity/` using its own lockfile; it does not build the wallet or change
application dependencies. Python 3.11+ assembles reports and joins existing LLVM
coverage only for byte-identical source files. See [COMPLEXITY.md](../tests/COMPLEXITY.md)
for parser qualification, score definitions, baseline findings and limitations.
`make check` runs the offline report-integrity tests without installing the analyzer.
The helper's syntax probes are an explicit development-tool check:

```sh
cargo test --locked --manifest-path tools/complexity/Cargo.toml --target-dir target/tools/complexity
cargo clippy --locked --manifest-path tools/complexity/Cargo.toml --target-dir target/tools/complexity --all-targets -- -D warnings
```

The opt-in `measure_startup_tor.py` runner compares unsigned startup time and
per-process peak RSS through an existing, warmed Tor SOCKS endpoint. It retains
the daemon and its learning state; see [startup qualification](../tests/STARTUP.md).
