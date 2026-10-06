# Development tools

The application and normal Rust tests do not need Python. These tools run from
any working directory unless a command explicitly supplies relative data paths.

| Tool | Purpose |
| --- | --- |
| `python3 scripts/reproducible.py` | Two clean, network-denied release builds with binary hash comparison; see [release qualification](../docs/reproducible-builds.md) |
| `check_toolchain.sh` | Verify Rust/Cargo match `rust-toolchain.toml`, including Homebrew installations |
| `check.sh` | Format, vendor provenance, default Zcash tests, all-feature Clippy and compatibility suite |
| `check.sh --no-default-features` | Explicit opt-in only: tests and Clippy without the embedded wallet, plus format/provenance |
| `python3 scripts/complexity.py` | Pinned source complexity metrics; ranked Markdown and complete JSON/CSV under `target/complexity` |
| `coverage.sh` | Default Zcash build coverage; HTML, JSON and text under `target/coverage` |
| `coverage.sh --branch` | Nightly branch coverage; separate reports under `target/coverage-branch` |
| `coverage.sh --proving` | Only the named synthetic proving test; `target/coverage-proving` |
| `coverage.sh --consensus TEST` | One explicitly selected consensus test; `target/coverage-consensus-TEST` |
| `tests/coverage.sh` | Offline regression check for report publication and failure preservation |
| `zcash.sh build` / `zcash.sh test --all-targets` | Cargo-managed protoc; default features include the wallet, `--no-default-features` disables it |
| `check_compat.py` | Standard-library Python orchestration of the separate payment/Zingolib compatibility workspace |
| `generate_crypto_vectors.py` | Independent Python Keccak/EIP-712 reference vectors using pinned script dependencies |

Do not run the no-default-feature checks during ordinary development unless explicitly
requested. The standard build includes Zcash; static-wallet deployments need no treasury
configuration. If both variants are requested, run them sequentially: CLI tests share
`target/debug/x402_treazury`.
`check.sh` leaves the Zcash-enabled executable available at that path.
All default checks use local fixtures and public test keys; none reads `.env` or
performs funded transactions. Local socket tests require permission to bind localhost.
Cargo and Zingolib may download dependencies/public proving parameters on first build.
Set `CARGO_NET_OFFLINE=true` to require cached Cargo inputs; it does not block
network requests from build scripts. Release qualification adds an OS network
sandbox and validates prefetched proving parameters.

Regenerate the independent cryptographic vectors with:

```sh
uv run --script scripts/generate_crypto_vectors.py
cargo test --locked --test crypto_vectors
```

`uv` is needed only for this optional regeneration command; it manages an isolated
script environment without a Python application package or project virtualenv.
Normal tests consume committed vectors. Review changes rather than automatically
accepting new expected values.

Build OpenAPI request fixtures with the Rust utility:

```sh
cargo run --locked --example snapshot_spec -- SOURCE_JSON_OR_URL OUTPUT_JSON
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
harnesses, examples and vendor code, but can include inline unit-test helpers.
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

Optional modes do not run the default suite. Consensus accepts only the five
named cases in `tests/REGTEST.md`, one per process; it never enables a broad ignored
test filter. Docker/images must already be available. No live Tor or mainnet tests
are selected. Keep optional results distinct from default-build percentages.

The optional complexity reporter builds a small independent Rust helper in
`tools/complexity/` using its own lockfile; it does not build the wallet or change
application dependencies. Python 3.11+ assembles reports and joins existing LLVM
coverage only for byte-identical source files. See [COMPLEXITY.md](../tests/COMPLEXITY.md)
for parser qualification, score definitions, baseline findings and limitations.
`check.sh` runs the offline report-integrity tests without installing the analyzer.
The helper's syntax probes are an explicit development-tool check:

```sh
cargo test --locked --manifest-path tools/complexity/Cargo.toml --target-dir target/tools/complexity
cargo clippy --locked --manifest-path tools/complexity/Cargo.toml --target-dir target/tools/complexity --all-targets -- -D warnings
```

The opt-in `measure_startup_tor.py` runner compares unsigned startup time and
per-process peak RSS through an existing, warmed Tor SOCKS endpoint. It retains
the daemon and its learning state; see [startup qualification](../tests/STARTUP.md).
