# Development tools

The application and normal Rust tests do not need Python. These tools run from
any working directory unless a command explicitly supplies relative data paths.

| Tool | Purpose |
| --- | --- |
| `check.sh` | Format, vendor provenance, both feature configurations, all-feature Clippy and compatibility suite |
| `check.sh --no-default-features` | Only the tests and Clippy without the embedded wallet, plus format/provenance |
| `coverage.sh` | Default Zcash build coverage; HTML, JSON and text under `target/coverage` |
| `zcash.sh build` / `zcash.sh test --all-targets` | Cargo-managed protoc; default features include the wallet, `--no-default-features` disables it |
| `check_compat.py` | Standard-library Python orchestration of the separate payment/Zingolib compatibility workspace |
| `generate_crypto_vectors.py` | Independent Python Keccak/EIP-712 reference vectors using pinned script dependencies |

Run feature suites sequentially: CLI tests share `target/debug/treazure`.
`check.sh` leaves the Zcash-enabled executable available at that path.
All default checks use local fixtures and public test keys; none reads `.env` or
performs funded transactions. Local socket tests require permission to bind localhost.
Cargo and Zingolib may download dependencies/public proving parameters on first build.
Set `CARGO_NET_OFFLINE=true` to require cached build inputs.

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
inventory and opt-in live smoke test are documented in `docs/network-egress.md`.

Coverage requires `cargo-llvm-cov` and LLVM tools matching `rustc -vV`. Homebrew
LLVM is detected; explicit `LLVM_COV` / `LLVM_PROFDATA` overrides take precedence.
With rustup, install `llvm-tools-preview`. The report excludes integration test
harnesses, examples and vendor code, but can include inline unit-test helpers.
CLI tests clear inherited environment except `LLVM_PROFILE_FILE` for subprocess
coverage. Never replace this with unrestricted environment inheritance.
