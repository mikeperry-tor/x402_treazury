# Development and measurement

See [testing and qualification](testing.md) for test boundaries and acceptance limits.
Commands run from the repository root.

## Portability

Target Linux, macOS and Windows. Prefer standard Rust or existing cross-platform
dependencies over native OS calls. When platform behavior is necessary, put it
behind a small shared abstraction with implementations for all three systems.
Define the required behavior first: filesystem privacy and durability, cancellation,
process ownership and network confinement need equivalent guarantees, not merely
matching function signatures. Avoid adding abstraction layers for behavior that
can simply be removed.

Do not hide missing implementations behind successful no-ops or silently weaker
permission, recovery or network checks. An unavailable required capability must
produce an explicit error scoped to that operation. Such an error documents an
unfinished port; it does not establish platform support. Prefer portable test
fixtures and verify native behavior on each OS before claiming support. Cross-target
compilation alone does not validate filesystem or process semantics.

These are engineering targets, not a claim that the current tree supports all
three systems. The [portability fixes plan](plans/portability_fixes.md) tracks known
gaps and removal options. Ordinary application support and optional qualification
are separate: [release reproducibility](reproducible-builds.md) and
[real-Tor confinement](../tests/TOR.md) currently have macOS-specific evidence.
Keep those claims narrow until equivalent backends are implemented and qualified.

## Embedded provider inputs

The provider bundle uses `src/providers/assets.rs` as its asset membership manifest;
settings and capability descriptions remain in provider TOML. Adding a shipped
provider requires a manifest entry and every referenced local schema. The offline
bundle test validates all entries and builds local catalogs without installation
paths or network access. API/spec lookup and local metadata search share this
bundle rather than scanning a deployment directory.

Build rebuild triggers, source fingerprints and source dirty status include those
exact TOML/schema files and the embedded directory schema. Private provider
configuration files outside the manifest remain excluded. The runtime qualification
check uses the same fingerprint inputs as the build script, so a shipped provider
edit changes binary identity and qualification compatibility. See
[release inputs](reproducible-builds.md#prepare-inputs).

## Application configuration

Zcash support is unconditional in the executable and library. Static-wallet and
unsigned deployments can omit treasury configuration; compiling support does not load
treasury keys or start its workers. Only `zcash-testutils` (synthetic proving) and
`zcash-regtest` (optional local consensus) are Cargo feature switches. Cargo's
`--no-default-features` does not select a different application.

Build identity retains its versioned schema and always reports the `zcash`
capability marker, followed by the actual enabled test-feature markers. Preparation
compares these identities and exact input hashes; historical pinned reports keep
their original metadata and remain observation-only.

## Build storage options

For end users, build an optimized executable without incremental compiler caches:

```sh
scripts/zcash.sh build
target/release/x402_treazury --help
```

The wrapper defaults `build` (and an invocation without a command) to release
mode and sets `CARGO_INCREMENTAL=0`, including for its protoc helper. Explicit
`--release` and `--profile NAME` remain supported with incremental caches disabled.
Release compilation takes longer, but produces optimized code without debug info.
This ordinary build is separate from the qualified
[reproducible release workflow](reproducible-builds.md).

Development and test builds use `debug = "line-tables-only"` in the application
and independent Zingolib compatibility workspace. This retains file/line
backtraces but omits type and variable information needed for richer debugger
inspection. Enable incremental development builds explicitly:

```sh
scripts/zcash.sh build --developer
# Development profile without incremental caches:
scripts/zcash.sh build --profile dev
```

`--developer` sets `CARGO_INCREMENTAL=1` and cannot be combined with `--release`
or `--profile`. The build wrapper selects its incremental setting even when a
value is inherited from the shell. `test`, `check`, and `clippy` retain normal Cargo
profiles and respect the inherited environment; `--developer` is build-only.
To disable incremental compilation for tests:

```sh
CARGO_INCREMENTAL=0 scripts/zcash.sh test --all-targets -- --test-threads=1
```

For full debugger information, set `CARGO_PROFILE_DEV_DEBUG=2` with `build --developer` or
`CARGO_PROFILE_TEST_DEBUG=2` for tests. Profile changes can require dependency
recompilation and leave previous artifacts on disk. Disabling incremental
compilation does not delete existing caches or other dependency/test artifacts.

With builds stopped, the regenerable incremental caches can be removed directly:

```sh
rm -rf target/debug/incremental compat/zingolib/target/debug/incremental
```

The next compilation will rebuild any needed cache. Preserve the rest of
`target/`: it also contains retained live qualification evidence, pinned
executables and reports. Do not use blanket `cargo clean` or delete `target/`
as routine disk cleanup.

## Verification

Run `scripts/check.sh` for the application, test-feature Clippy, compatibility
checks and the local synthetic-chain treasury lifecycle. Run feature configurations
sequentially because CLI tests share the executable path. See
[development tools](../scripts/README.md).
Individual commands:

```sh
scripts/zcash.sh test --all-targets -- --test-threads=1
scripts/zcash.sh clippy --all-targets -- -D warnings
cargo fmt --check
```

Tests cover local x402 handshakes and signatures, spend limits, managed admission
and rotation, HTTP authentication, actual stdio process framing, tool generation,
configuration, catalog/help caching, passive runtime price hints, Tor isolation, encrypted persistence and
Zcash sync/recovery. They use localhost and public deterministic test keys;
normal suites do not spend funds. A sandbox must permit binding localhost.

Focused provider fixtures preserve identified schema, routing and integration
quirks without freezing every tool or provider setting. All provider TOMLs validate
offline. Independent
Python-generated Keccak/EIP-712 vectors are committed as test data. See
[fixture maintenance](../tests/fixtures/catalogs/README.md),
[network verification](network-egress.md), and
[consensus tests](../tests/REGTEST.md).

### Local complexity metrics

Run `python3 scripts/complexity.py` for a local audit using pinned
`rust-code-analysis`. Open `target/complexity/report.md`; full JSON/CSV retain all
scores. Production callables, tests and examples are separated, parser failures
are explicit, and existing coverage is joined only for unchanged source files.
`scripts/check.sh` runs the audit by default. New or worsening hotspots above CogC
20 or CycC 40 receive advisory review warnings against the merge base with `main`;
scores do not fail the check. Use `--base REF` or `COMPLEXITY_BASE_REF` for another
review target. Incomplete analysis fails visibly. Apply the
[review discipline](../tests/COMPLEXITY.md#review-discipline) to act on warnings;
see [definitions and findings](../tests/COMPLEXITY.md)
for syntax qualification, macro/feature limitations and the independent tool build.

### Local coverage

Install `cargo-llvm-cov` (`cargo install cargo-llvm-cov --locked`) and LLVM tools
matching `rustc -vV` (or `rustup component add llvm-tools-preview` for rustup).
Run `scripts/coverage.sh` for the default Zcash build, all test targets and examples.
It supplies protoc and detects Homebrew LLVM; `LLVM_COV` and `LLVM_PROFDATA` can
select another matching installation. Rust 1.99.1 from Homebrew uses LLVM 23.1.3.

Open `target/coverage/html/index.html`; text and machine-readable summaries are
`target/coverage/summary.txt` and `target/coverage/summary.json`. The denominator
covers application source, excluding integration tests, examples, operational tools and vendored
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

## Logging and diagnostics

All application logs go to stderr; stdout carries the MCP wire or requested CLI
JSON. Process `RUST_LOG` overrides the default `warn,x402_treazury=info` filter for
every command. Info reports catalog completion, listener startup and
funding progress. Download, parse and protocol details belong at debug; warnings
identify failures, degraded operation or actionable configuration problems.
Normal capacity waits, retained observation deferrals and disposable cache eviction
belong at DEBUG, even when they delay work or discard advisory cached data.

A valid first-page `tools/list` with more than 250 advertised definitions emits
an advisory WARN with listener ID, count and `discover_on_demand = true` suggestion,
without URLs, descriptions or credentials. It observes inventory, not model context;
continuations and invalid cursors do not warn, and fresh first pages may repeat it.
It does not reject work, change mode or perform network/wallet work.

An unreachable configured local Tor SOCKS endpoint during serving startup emits
an immediate shared ERROR with the local address, safe I/O kind, retry cadence and
reminder interval. Failed probes repeat the ERROR after at least 60 seconds while
the shared startup wait remains pending. Recovery
emits INFO describing TCP reachability, without asserting Tor bootstrap or remote
service health. No upstream prose, credentials or destination URLs are included.

Managed extension omission is INFO (`bazaar` discovery metadata remains DEBUG).
It reports challenge preparation before offer validation or signing, not a failed
payment. Payment-call spans carry an ephemeral `call_id`, configured source and
HTTP method; catalog calls also carry tool identity. Dynamic catalog calls use
their registered source identity. Direct managed-library calls have their own
call ID without claiming a configured source. No request URL, arguments, raw
challenge, extension values, signed bytes or upstream error prose is logged.

Accepted payment work logs its terminal result inside its retained owner, even
after caller cancellation: completion is INFO, failure is WARN with a fixed
failure kind, execution stage, unsigned attempt number/maximum when available,
and `authorization_may_charge`. Counts describe dispatches in the latest payer's
unsigned sequence and reset when managed wallet traversal changes payer; they are
not a cumulative call limit. Transport failures include a fixed `transport_reason`
(TLS, connection timeout, read inactivity, connection, body, decode, request
construction or other transport), typed I/O kind and numeric OS error when available;
error prose and URLs remain excluded. The replay caution appears only when an
authorization may charge. That flag records
possible financial liability, not confirmed settlement. `last_http_status`, when
present, is the most recently observed response status; a later signing or
transport failure can follow an unsigned 402. Recoverable payer handoffs and
insufficient-funds traversal do not emit terminal failures. Direct managed-library
calls warn on unsuccessful returned HTTP status; their completion covers response
headers, while body consumption remains the caller's responsibility. Diagnostics
never authorize replay or change payment execution.

Optional cover diagnostics distinguish fresh range failures from cached refusal
and episode shortfalls. Retained range-capability loss is WARN; fallback selection
and ordinary shortfalls are INFO; cached refusals and individual attempt facts are
DEBUG. See [cover diagnostics](cover-traffic.md#cover-diagnostics) for fields,
deadline cleanup and safe resource labels.

Waiting for confirmed spendable ZEC and funding deferred by a reported treasury
sync failure are info events; their retry backoff and stored failure conditions
remain intact. Treasury sync failures stay warnings with safe connection reasons
and direct/Tor route details when available. Background warnings report
`automatic_retry=true` and `retry_after_seconds` from the existing observation
schedule (60, 120, 240, then 300 seconds after consecutive scan failures). Retry
continues while serving and sync is needed; SDK reconstruction can delay it.
One-pass `wallet sync` warnings report `automatic_retry=false` and instruct the
operator to rerun the command. Pepper Sync fetch spans are debug-only and do not
emit raw returned errors; the sanitized application warning owns failure reporting.
Base RPC attempts and successful
provider failover are debug events with provider ordinals; an exhausted complete
verification remains a warning. Selecting a configured discovery relay fallback
and actual paid relay attempts are info. Unsigned API and catalog/pricing Tor
retry events are info, with attempt and maximum counts; terminal payment failures
remain warnings. Relay pre-submission retry availability is info, but each failed
relay payment call still warns before the separate discovery workflow considers
another attempt.

Sanitize Zingolib errors before logging or persistence. Retain fixed categories
and typed reasons, numeric gRPC codes and fixed status names when available, never
upstream prose, metadata, wallet identifiers or credentials. Shard-tree diagnostics
retain the fixed pool and operation labels from the pinned Pepper Sync patch.
Keep ordinary CLI errors legible even with backtrace environment variables enabled.

## Dependency compatibility

### TLS security updates

The application and `compat/zingolib` are independent Cargo workspaces; update
both committed lockfiles for TLS security fixes. For the
[emSign intermediate-key compromise](https://rustsec.org/advisories/RUSTSEC-2026-0382),
runtime verification requires `rustls-webpki` 0.103.16 or later in the 0.103
series, together with `webpki-roots` and `webpki-root-certs` 1.0.10 or later.
The verifier rejects compromised intermediate public keys; the root-bundle
update removes the affected emSign root. Host trust-store updates remain
necessary for platform verification. Rebuild and replace deployed binaries to
consume locked dependency changes. Vendored upstream lockfiles are provenance
inputs, not the dependency resolution used by these two workspaces.

The build-only legacy TLS dependency and parameter-integrity boundary are
documented in [the Zingo integration guide](../vendor/ZINGO.md#build-time-parameter-downloads).
That graph still contains affected certificate-verification versions; these
updates do not eliminate every advisory finding from the lockfiles.

There are no repository Python requirements or uv lockfiles. The vector generator
uses inline script dependencies; those do not pin the package manager's TLS
implementation. Follow the
[Python security guidance](https://discuss.python.org/t/information-for-python-users-regarding-emudhra-emsign-ca-certificate-security-incident/109576)
for developer environments: use pip 26.2.2 or later, uv 0.13.4 or later, and
certifi 2026.10.19 or later wherever installed, and update system certificates.
Use the upstream checksum-verified bootstrap instructions when upgrading an
affected package manager. Updating certifi alone does not update pip's vendored
bundle. These tools and existing virtual environments are maintained outside
the repository.

### Combined workspace verification

The root workspace includes Pepper Sync so its private shard-merge regression can
run with the application's reviewed patches and lockfile. Ordinary root commands
select only the application; the other vendored dependencies remain nonmembers.
`scripts/check.sh` explicitly runs the real three-pool merge-conflict fixture:

```sh
scripts/zcash.sh test -p pepper-sync --lib wallet::traits::diagnostic_tests::scan_merge_conflict_preserves_pool_context -- --exact
```

Run source verification and the combined suite with the reviewed vendored patches:

```sh
python3 vendor/verify.py
python3 scripts/check_compat.py
```

The checker verifies the vendored source hashes, obtains a Cargo-managed `protoc`
compiler, and runs the combined suite with its committed lockfile. First builds
fetch Cargo dependencies and zingolib's public Sapling proving parameters;
zingolib embeds those parameters from the standard cache during its build. Tests use temporary
offline wallets and localhost payment servers. No wallet sync, funded payment, approval or broadcast
occurs.

The [vendored inventory](../vendor/README.md) records each remaining patch's
current reason, consumers, removal evidence and source-verification scope.
Both roots use unpatched Alloy core 1.7.3 with the pinned Zingolib v7 cohort:

```text
Alloy → sha3 0.11.0 → digest 0.11.3
Zingolib → bip32 0.6.0 → hmac 0.13.0 → digest 0.11.3
```

`lightwallet-protocol` 0.4.0 supplies the required Ironwood fields without a
separate root Git override. Both roots use ordinary upstream Zingolib/netutils
Git sources at the same pinned revision, with no local checkout overrides. The
[Zingo integration](../vendor/ZINGO.md) uses public RPC traits, direct Pepper sync
and backend calculation over the application transport. Pepper's retained patches
own progress/drain behavior and sanitized diagnostics. Upstream v7 handles
same-height reorg truncation and synthetic branch hashes; the application keeps
its lifecycle regressions. HTTP transport patches preserve the
[network inactivity/cancellation contract](network-egress.md#timeout-budgets-and-concurrency).
`vendor/verify.py` checks HTTP transport sources; `verify_zingo.py` checks the
remaining Pepper snapshot, patches and copied-code notice. Upstream license
texts are included. Deliberate source/extra-file mutations must still fail that
verification. SDK Git revision, tags, dirty status and actual generated descriptor
are release inputs; see [release qualification](reproducible-builds.md).

The independent hash vectors are regenerated with
`uv run --script scripts/generate_crypto_vectors.py`; normal Rust tests consume
the committed vectors without Python. The combined suite runs the shared
payment/MCP/crypto targets plus offline wallet tests. Both root manifests must
repeat the remaining patches because dependency-root patches are not inherited.
Both lockfiles retain their independently resolved graphs; neither workspace
loads real wallet state for these checks. SDK upgrade coverage includes a public
v6 snapshot, retained derived addresses and next-address continuity under v7.

This focused compatibility suite tests dependency coexistence and offline wallet
round trips. The application suites separately cover sync, proving and consensus
recovery. Neither suite qualifies mainnet NEAR settlement. Both manifests use the
reviewed vendored patches; no reference checkout is required. The
[wallet architecture](wallet-rotation.md) describes treasury ownership,
admission, funding, persistence and recovery.

`x402-chain-eip155` also requires its `telemetry` feature with `client` in this
release: the `upto` implementation otherwise references an unavailable
`tracing` dependency. The application enables both.
