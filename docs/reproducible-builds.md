# Reproducible release builds

Dependency resolution uses committed lockfiles and `--locked`. Release
qualification builds committed `HEAD` twice in independent source, `HOME`,
temporary, and target directories with OS-level network denial, then compares
SHA-256 digests of the resulting `x402_treazury` binaries.

This check is for release binaries, reproducible-build changes, or explicit
requests; `make build` and `make check` do not run it. See
[validation selection](testing.md#selecting-validation-by-change).

## Toolchain and release environment

- `rust-toolchain.toml` pins Rust 1.99.1 for rustup, and `make toolchain`
  enforces matching Rust and Cargo versions under Homebrew and system installs
  (run `make toolchain` before direct Cargo commands under Homebrew).
  `rust-version` in `Cargo.toml` is the minimum supported language version, not
  the toolchain pin.
- The current profile in `scripts/release-environment.json` pins Apple Silicon
  macOS, Homebrew Rust and Cargo 1.99.1, Apple Clang 21.0.0
  (`clang-2100.1.1.101`), `ld-1267`, and SDK 26.5 targeting macOS 14.0. The
  runner rejects mismatched tool versions or host architectures; updating
  profile versions requires a fresh qualification, and other operating systems
  require their own profile and OS network sandbox.
- The root release profile enables integer overflow checks across the
  application and dependencies; preserve that setting in release builds.

## Prepare inputs

Shipped provider TOMLs, referenced local schemas, and the embedded directory
schema are compilation inputs tracked by `src/providers/assets.rs`; commit
changes to those assets with the release candidate. Private local overrides
outside the manifest are excluded.

Preparation may use the network. From the repository root:

```sh
make toolchain
cargo fetch --locked --target aarch64-apple-darwin
cargo fetch --locked --manifest-path tools/protoc/Cargo.toml
make build
```

- `make build` populates Zingolib's public Sapling parameter cache
  (`$XDG_DATA_HOME/ZcashParams` when `XDG_DATA_HOME` is absolute, otherwise
  `$HOME/.local/share/ZcashParams`).
- The release runner verifies parameter file sizes and BLAKE2b-512 digests
  against the locked SDK constants, copies them into each build's isolated
  `HOME/.local/share/ZcashParams`, and re-verifies before compiling. Missing or
  corrupt parameters fail before compilation. Legacy copies under
  `vendor/zingolib/zcash-params` are unused.
- `--frozen` enforces locked offline Cargo resolution, while macOS
  `sandbox-exec` with `deny network*` blocks arbitrary build-script network
  calls (verified up front with a failing loopback probe; fails closed if the
  sandbox is unavailable).

## Qualify a committed candidate

```sh
python3 scripts/reproducible.py
```

Commit tracked changes first. The runner archives `HEAD` (excluding untracked
files and secrets), rebuilds `tools/protoc` independently (tracking its
manifest, lockfile, and sources in fingerprints), and runs two clean builds of
`cargo build --frozen --release --bin x402_treazury --target aarch64-apple-darwin`.

- **Allowlisted environment:** Clears inherited Rust flags, profiling vars,
  proxy vars, and wallet credentials; disables incremental compilation; remaps
  Rust and C source paths; fixes locale, timezone, deployment target, and
  `SOURCE_DATE_EPOCH`; and stops Git lookup outside the build directory.
- **SDK Git descriptor evidence:** Under each isolated build, the runner
  records the resolved Zingolib source, manifest path, revision, `zingolib_v*`
  tags, `diff-index`, and generated `git_description.txt` without mutating
  Cargo caches. Because both builds share downloaded Cargo source caches, a
  same-host match proves identical bytes for that revision, cache, and host
  profile, not cross-OS or independent-cache reproducibility.
- **Retained artifacts (`target/reproducibility-runs/release-*`):**
  - `report.json`: outcome, revision, archive and lockfile hashes, compiler and
    native tool versions, binary hashes, and per-build SDK descriptor inputs.
  - Complete build logs with exact commands.
  - Both source snapshots, build directories, and output binaries.

Both binaries must match byte-for-byte and pass `--help` inside the network
sandbox (no stripping, rewriting, or network fallback). Delete old run
directories manually when no longer needed.

## Qualification scope

An earlier Homebrew Rust 1.99.0 macOS profile passed a same-host two-build
comparison; the current 1.99.1 profile requires a fresh run. Re-run this
procedure for each distributed release candidate, keep hashes and provenance
with the release outside the source tree, and handle code signing or
notarization under a separate comparison policy.
