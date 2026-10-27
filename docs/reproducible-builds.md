# Reproducible release builds

Dependency resolution uses committed lockfiles and `--locked`. Release
qualification additionally builds committed HEAD twice with independent source,
HOME, temporary and target directories, blocks network access at the OS level,
and compares SHA-256 hashes of the resulting `x402_treazury` binaries. A successful
run proves matching bytes for that revision, feature selection and recorded
host environment. It does not establish reproducibility across operating
systems, independent machines or different toolchain distributions.

## Toolchain and release environment

`rust-toolchain.toml` selects Rust 1.99.1 for rustup. The Makefile application targets
also run `make toolchain`, which rejects a different Rust or Cargo
version under Homebrew/system installations. Before direct Cargo commands under
Homebrew, run `make toolchain`. `rust-version` in
`Cargo.toml` remains the minimum supported language version, not a toolchain pin.
Optional nightly branch coverage deliberately uses a separate toolchain.

The current qualification profile in `scripts/release-environment.json` is
Apple Silicon macOS, Homebrew Rust/Cargo 1.99.1, Apple Clang 21.0.0
(clang-2100.1.1.101), ld-1267 and SDK 26.5, targeting macOS 14.0. The runner
rejects different compiler commit, Cargo distribution/version, Clang/linker
version, SDK version or Rust host. This is a version-checked local environment,
not a container image or a fully hermetic SDK snapshot. Keep the listed native
tools installed to reproduce this profile; replacing their version strings in
the profile requires a fresh qualification. Other platforms need an explicit
profile and equivalent OS network sandbox before support is claimed.

## Prepare inputs

Shipped provider TOML and their local schemas are compilation inputs embedded in
the executable. The asset manifest in `src/providers/assets.rs` drives both bundle
membership and source fingerprints/rebuild triggers; the directory's embedded
schema is included as well. Commit changes to these assets with the candidate.
Private provider overrides outside that manifest are excluded. An installation
does not need a separate provider data directory for bundled source discovery.

Preparation may use the network. From the repository root:

```sh
make toolchain
cargo fetch --locked --target aarch64-apple-darwin
cargo fetch --locked --manifest-path compat/protoc/Cargo.toml
make build
```

The last command populates Zingolib's public Sapling parameter cache. On the
qualified macOS host, `zcash_proofs` 0.31.0-pre.1 uses
`$XDG_DATA_HOME/ZcashParams` when `XDG_DATA_HOME` is absolute, otherwise
`$HOME/.local/share/ZcashParams`. The runner reads that cache and checks file
sizes and BLAKE2b-512 digests against the locked SDK's constants. Each build
receives independent parameter copies in its isolated HOME, with
`XDG_DATA_HOME` explicitly set to that HOME's `.local/share`; the copies are
verified again before compilation. The builds do not share a mutable parameter
cache. Missing or corrupt parameters fail before compilation. Older parameter
copies under `vendor/zingolib/zcash-params` are unused and are not deleted.
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
```

Commit tracked changes first. The script archives HEAD; untracked files,
including user scripts and secrets, are excluded. A clean checkout and its
Git archive produce the same snapshot. Each build uses `cargo build --frozen
--release --bin x402_treazury --target aarch64-apple-darwin`; Zcash is unconditional.
The locked protoc helper is also rebuilt independently.

The source archive includes `compat/zingolib/Cargo.toml`, a workspace resolution
input even when only the application is selected. Build fingerprints, dirty-input
checks and Cargo rebuild triggers include that manifest alongside the root
manifest and lockfile.

The environment is constructed from an allowlist: no inherited Rust flags,
profiling settings, proxy variables or wallet credentials. Cargo registry/Git
source caches are shared, but compiled output, Cargo configuration and credentials
are not. Incremental compilation is disabled. Rust/C source paths are remapped;
locale, timezone, deployment target and `SOURCE_DATE_EPOCH` are fixed. Git lookup
stops before reaching the surrounding application checkout. The resolved Zingolib
Git source has its own checkout: its descriptor depends on revision, matching
`zingolib_v*` tags and dirty status, not just the pinned source revision. Under each
build's isolated environment and network sandbox, the runner records the resolved
SDK package/source, manifest path, full Git revision, matching tags and
`diff-index` result, then the actual generated `git_description.txt` after building.
It does not predict the descriptor or change Cargo caches/tags to force matching
output. A repository-free SDK archive instead uses the crate-version fallback;
with Git installed, its failed dirty check can add `_dirty`.

Both attempts share downloaded Git sources. Matching binaries on one host with
that shared cache do not establish reproducibility across independently populated
caches and their Git metadata.

Each run is retained under `target/reproducibility-runs/release-*` with:

- `report.json`: success/failure, revision, source archive and lockfile hashes,
  feature selection, compiler hashes, native tool versions and artifact hashes,
  plus each build's resolved SDK/Git inputs and actual generated descriptor.
  Failures retain available per-build inputs and output evidence.
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

The earlier Homebrew Rust/Cargo 1.99.0 macOS default-Zcash profile passed a
same-host two-build comparison of original Cargo binaries, with sandbox probes,
vendor checks and CLI smoke tests. That evidence does not qualify the current
1.99.1 profile, which requires a fresh two-build comparison.
This historical result does not establish a fresh qualification of later candidates,
independent-host equality, signing or notarization. Run the procedure for the
release candidate being distributed and retain hashes and provenance with that release outside the source tree.
