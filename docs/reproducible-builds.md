# Reproducible release builds

Dependency resolution uses committed lockfiles and `--locked`. Release
qualification additionally builds committed HEAD twice with independent source,
HOME, temporary and target directories, blocks network access at the OS level,
and compares SHA-256 hashes of the resulting `treazury` binaries. A successful
run proves matching bytes for that revision, feature selection and recorded
host environment. It does not establish reproducibility across operating
systems, independent machines or different toolchain distributions.

## Toolchain and release environment

`rust-toolchain.toml` selects Rust 1.99.0 for rustup. The Zcash and check wrappers
also run `scripts/check_toolchain.sh`, which rejects a different Rust or Cargo
version under Homebrew/system installations. Direct Cargo commands under
Homebrew bypass that wrapper; run the checker first. `rust-version` in
`Cargo.toml` remains the minimum supported language version, not a toolchain pin.
Optional nightly branch coverage deliberately uses a separate toolchain.

The initial qualification profile in `scripts/release-environment.json` is
Apple Silicon macOS, Homebrew Rust/Cargo 1.99.0, Apple Clang 21.0.0
(clang-2100.1.1.101), ld-1267 and SDK 26.5, targeting macOS 14.0. The runner
rejects different compiler commit, Cargo distribution/version, Clang/linker
version, SDK version or Rust host. This is a version-checked local environment,
not a container image or a fully hermetic SDK snapshot. Keep the listed native
tools installed to reproduce this profile; replacing their version strings in
the profile requires a fresh qualification. Other platforms need an explicit
profile and equivalent OS network sandbox before support is claimed.

## Prepare inputs

Preparation may use the network. From the repository root:

```sh
sh scripts/check_toolchain.sh
cargo fetch --locked --target aarch64-apple-darwin
cargo fetch --locked --manifest-path compat/protoc/Cargo.toml
scripts/zcash.sh build
```

The last command populates the public Sapling parameter copies used by Zingolib.
The runner checks their sizes and BLAKE2b-512 digests against the constants in
locked `zcash_proofs` 0.30.0 before copying them into each source snapshot, and
checks the copies again. Missing or corrupt parameters fail before compilation.
No wallet, seed, `.env` or funded transaction is involved.

`--locked` fixes dependency resolution but permits downloads. `--frozen` combines
locked resolution with Cargo offline mode. Neither controls arbitrary network
requests made by build scripts: the qualification runner separately uses macOS
`sandbox-exec` with `deny network*`, and first verifies that a loopback connection
fails with a permission error. It fails closed if this sandbox is unavailable.
An agent's outer execution sandbox may require approval to launch this nested
OS sandbox; running it directly in a local terminal does not require a service.

## Qualify a committed candidate

```sh
python3 scripts/reproducible.py
# Separate optional qualification without the embedded wallet:
python3 scripts/reproducible.py --no-default-features
```

Commit tracked changes first. The script archives HEAD; untracked files,
including user scripts and secrets, are excluded. A clean checkout and its
Git archive produce the same snapshot. Each build uses `cargo build --frozen
--release --bin treazury --target aarch64-apple-darwin`, with default Zcash unless
explicitly disabled. The locked protoc helper is also rebuilt independently.

The environment is constructed from an allowlist: no inherited Rust flags,
profiling settings, proxy variables or wallet credentials. Cargo registry/Git
source caches are shared, but compiled output, Cargo configuration and credentials
are not. Incremental compilation is disabled. Rust/C source paths are remapped;
locale, timezone, deployment target and `SOURCE_DATE_EPOCH` are fixed. Git lookup
stops before reaching the surrounding checkout. Thus Zingolib's existing
no-checkout fallback embeds its crate version instead of host tags or a dirty
working-tree description; no additional vendor patch is required.

Each run is retained under `target/reproducibility-runs/release-*` with:

- `report.json`: success/failure, revision, source archive and lockfile hashes,
  feature selection, compiler hashes, native tool versions and artifact hashes.
- Complete build logs for both attempts, including the exact commands.
- Both source snapshots, separate build directories and resulting binaries.

The binaries must match exactly and each must pass `--help` under the network
sandbox. Errors preserve available evidence and return a nonzero status; there
is no fallback to a network-enabled build and no stripping or rewriting of
binaries after the comparison. Two fresh release builds can take substantial
CPU time and disk space. Remove individual retained run directories manually
when no longer needed; the runner never deletes prior evidence.

This check supplements the ordinary test/Clippy/vendor-verification suite. It
does not prove dependency safety, runtime correctness or supply-chain integrity.
An eventual distribution pipeline should repeat qualification in independently
provisioned environments and publish the revision, environment and artifact
hashes alongside the release. Code signing/notarization, if added, needs a
separate comparison policy; this check compares Cargo's original output.

## Qualification scope

The macOS default-Zcash profile has passed a same-host two-build comparison of
original Cargo binaries, with sandbox probes, vendor checks and CLI smoke tests.
This does not establish independent-host equality, no-Zcash releases, signing or
notarization. Run the procedure for the release candidate being distributed and
retain hashes and provenance with that release outside the source tree.
