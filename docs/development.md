# Development and measurement

See [testing and qualification](testing.md) for test boundaries and acceptance limits.
Commands run from the repository root.

## Verification

Run `scripts/check.sh` for both build configurations, all-feature Clippy and
compatibility checks. Use `scripts/check.sh --no-default-features` to check only
the build without the embedded wallet. See [development tools](../scripts/README.md).
Individual commands:

```sh
scripts/zcash.sh test --all-targets -- --test-threads=1
scripts/zcash.sh clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --locked --no-default-features --all-targets
```

Tests cover local x402 handshakes and signatures, spend limits, managed admission
and rotation, HTTP authentication, actual stdio process framing, tool generation,
configuration, pricing/help caches, Tor isolation, encrypted persistence and
Zcash sync/recovery. They use localhost and public deterministic test keys;
normal suites do not spend funds. A sandbox must permit binding localhost.

Golden fixtures pin **860 tool contracts across 22 providers** and settings for
all 26 bundled providers. Names, full descriptions, schemas, methods, paths and
parameter routes are checked without Python or live vendor access. Independent
Python-generated Keccak/EIP-712 vectors are committed as test data. See
[fixture maintenance](../tests/fixtures/catalogs/README.md),
[network verification](network-egress.md), and
[consensus tests](../tests/REGTEST.md).

### Local complexity metrics

Run `python3 scripts/complexity.py` for an optional local audit using pinned
`rust-code-analysis`. Open `target/complexity/report.md`; full JSON/CSV retain all
scores. Production callables, tests and examples are separated, parser failures
are explicit, and existing coverage is joined only for unchanged source files.
No complexity thresholds are enforced. See [definitions and findings](../tests/COMPLEXITY.md)
for syntax qualification, macro/feature limitations and the independent tool build.

### Local coverage

Install `cargo-llvm-cov` (`cargo install cargo-llvm-cov --locked`) and LLVM tools
matching `rustc -vV` (or `rustup component add llvm-tools-preview` for rustup).
Run `scripts/coverage.sh` for the default Zcash build, all test targets and examples.
It supplies protoc and detects Homebrew LLVM; `LLVM_COV` and `LLVM_PROFDATA` can
select another matching installation. Rust 1.99.0 from Homebrew uses LLVM 23.1.2.

Open `target/coverage/html/index.html`; text and machine-readable summaries are
`target/coverage/summary.txt` and `target/coverage/summary.json`. The denominator
covers application source, excluding integration tests, examples and vendored
code; inline unit-test helpers can still count. Stable Rust reports line, region
and function coverage, not branch coverage. Optional proving/Docker and live
funded tests are outside this run. Reports contain local paths and are gitignored.

Region coverage is already included in these reports. Branch coverage needs a
fresh instrumented run with nightly Rust. With rustup installed and its binaries
on PATH:

```sh
rustup toolchain install nightly --profile minimal --component llvm-tools-preview
RUSTUP_TOOLCHAIN=nightly scripts/coverage.sh --branch
```

This writes HTML/JSON/text to `target/coverage-branch` and keeps its build/profiles
separate from the stable run. It uses nightly's matching LLVM tools; unset any
`LLVM_COV`/`LLVM_PROFDATA` overrides pointing at a different LLVM version. Production
builds continue to use the existing stable compiler. Branch results measure the
compiler's supported branch instrumentation, not every possible logical path.

## Dependency compatibility

Run the reproducible combined suite with the reviewed vendored connector patch:

```sh
python3 vendor/verify.py
python3 scripts/check_compat.py
```

The checker verifies the vendored source hashes, obtains a Cargo-managed `protoc`
compiler, and runs the combined suite with its committed lockfile. First builds
fetch Cargo dependencies and zingolib's public Sapling proving parameters;
zingolib caches/copies those parameters during its build. Tests use temporary
offline wallets and localhost payment servers. No wallet sync, funded payment, approval or broadcast
occurs.

The [vendored patch](../vendor/README.md) changes only the `sha3` dependency in
`alloy-primitives` and `alloy-sol-macro-expander` 1.7.3 from `0.11.0` to
`=0.10.9`. Both workspace roots apply it. No Rust source in those Alloy crates is modified. The separate
[Zingolib connector patch](../vendor/ZINGO.md) adds injected-channel support for
the shared direct/Tor transport. The graphs can then retain:

```text
Alloy → sha3 0.10.9 → digest 0.10.7
zingolib → bip32 → hmac prerelease → digest 0.11.0-pre.9
```

`vendor/provenance.json` records upstream archive/file hashes; `vendor/verify.py`
checks that these are the only modifications. Upstream license texts are included.
The independent hash vectors are regenerated with
`uv run --script scripts/generate_crypto_vectors.py`; normal Rust
tests consume the committed vectors without Python.

The combined workspace must also repeat zingolib's **existing upstream**
`lightwallet-protocol` patch, pinned to
`9bdfdc77eb283f2a3d26c27100cc2ac90148cd93`. Cargo does not inherit a dependency
workspace's root patches. Without it, compilation fails on missing Ironwood
protobuf fields. This is separate from the Alloy compatibility patch.

The combined suite runs the 12 shared payment/MCP/crypto tests plus an offline
wallet test: restore a public mnemonic, derive shielded addresses, serialize and
restore wallet bytes, check the next derived address, and create an EVM payer
in the same process. Its lockfile resolves Alloy's upper stack to 2.5.0 and
Zcash primitives to 0.30.1; the application lock retains Alloy 2.1.1.
Both graphs are tested, with Alloy core 1.7.3 and the same two-line patch.

This focused compatibility suite tests dependency coexistence and offline wallet
round trips. The application suites separately cover sync, proving and consensus
recovery. Neither suite qualifies mainnet NEAR settlement. Both manifests use the
reviewed vendored patches; no reference checkout is required. The
[wallet architecture](wallet-rotation.md) describes treasury ownership,
admission, funding, persistence and recovery.

`x402-chain-eip155` also requires its `telemetry` feature with `client` in this
release: the `upto` implementation otherwise references an unavailable
`tracing` dependency. The application enables both.
