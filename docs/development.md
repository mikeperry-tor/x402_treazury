# Development and measurement

See [testing and qualification](testing.md) for test boundaries and acceptance
limits. Run commands from the repository root.

## Portability

Target Linux and macOS. Prefer standard Rust or cross-platform dependencies
over native OS calls; isolate necessary platform behavior behind a small shared
abstraction with equivalent correctness and security contracts.

- **Fail closed on missing capabilities:** Never hide an unimplemented platform
  check behind a no-op or a weaker permission, recovery, or network check. An
  unavailable required capability must return an explicit error scoped to that
  operation.
- **Windows deferred:** Windows runtime, storage, supervision, and native
  validation remain deferred in
  [release follow-ups](plans/deferred/release_followups.md).
- **Private storage and ACLs:** Inherited macOS ACLs can defeat a mode-only
  check. The shared private-storage layer creates files and directories
  atomically without inherited grants on macOS and validates reopened
  descriptors.
  - Private files and directories reject granting macOS ACLs, and macOS SQLite
    parents reject file-inheriting grants (non-inheriting parent grants remain
    valid). Linux owner-only modes constrain POSIX ACL masks (default ACLs can
    remain), and SQLite copies the private database mode to its sidecars.
  - The HTTP cache validates unlinked SQLite sidecars with zero links for
    regular-file type, private mode, and macOS ACLs (rejecting multiple links),
    while standard private-file opens require one link. See
    [portability assessments](plans/portability_fixes.md) for remaining Unix
    namespace-authority work.
- **Platform-specific evidence:** [Release reproducibility](reproducible-builds.md)
  and [live-Tor confinement](../tests/TOR.md) currently carry macOS-specific
  evidence. Qualification connectivity probes use Rust `ErrorKind` to
  distinguish permission denial, refusal, and timeout.

## Review decisions and unfinished assessments

Maintained design decisions that are intentionally left as-is unless concrete
evidence warrants a focused change:

- **SQL archives, visibility, and loaders:** Archive `INSERT` variants preserve
  one-time vs. idempotent transitions; public Rust items serve library, test,
  and tooling consumers without exposing MCP tools; loader differences reflect
  distinct failure contracts; test-hook attributes stay where concrete
  responsibilities justify them.
- **Compiler baseline and arithmetic:** The application declares its minimum
  Rust version in [`Cargo.toml`](../Cargo.toml). For local toolchain selection,
  see the [build commands](../scripts/README.md#common-entrypoints). Release
  builds enable integer overflow checks throughout the dependency graph.
  Financial code must return checked-arithmetic errors rather than relying on
  panic recovery.
- **File locks and storage guards:** `fs2` locks protect exclusive ownership.
  Registry and treasury storage share an explicit-unlock guard released after
  the database closes; Store handles retain that guard through accepted work
  and worker failure so another owner stays excluded until the final handle
  releases it. The default adjacent `wallet.key` threat boundary (and separate
  `key_file` option) is documented in [wallet CLI](wallet-cli.md).
- **Deferred follow-ups:** Archive-build dirty provenance, provider-output
  assessment, and snapshot visibility belong to
  [release follow-ups](plans/deferred/release_followups.md).

## Dependency vendoring

Obtain informed user approval for the exact package and patch scope before
adding a maintained dependency checkout or fork (whether under `vendor/` or in
a separate Git repository). General instructions to implement a plan, upgrade
dependencies, or clear audit findings do not grant approval to vendor.

Present a concrete proposal and wait for user approval before implementing:

- **Package and benefit:** Exact package, version, proposed change, and why an
  upstream release or application-level alternative is insufficient.
- **Concrete exposure:** Call path, distinguishing reachable runtime,
  build-time, and development use from unused dependency code, along with
  mitigations and open uncertainty.
- **Alternatives:** Retaining upstream with a documented accepted advisory vs.
  what practical protection a fork adds.
- **Maintenance cost:** Ongoing rebase, provenance, regression tests, and a
  concrete path to retire the checkout.

Read-only inspection and small ignored probes may prepare the proposal; do not
implement and validate a full fork before asking. Routine rebases of an already
approved checkout within its disclosed scope need no new approval. Logging or
diagnostic changes alone rarely justify a new checkout. Track approved patches
and blockers in the [vendor ledger](../vendor/README.md).

## Embedded provider inputs

`src/providers/assets.rs` embeds every shipped provider TOML, every referenced
local schema, and the embedded directory schema into the executable (keeping
settings and descriptions in the TOML files). Build triggers, source
fingerprints, dirty checks, and runtime qualification compatibility checks all
read this manifest; private local overrides outside the manifest are excluded.
See [release inputs](reproducible-builds.md#prepare-inputs).

## Application configuration

Zcash support is always compiled into the binary and library; static-wallet and
unsigned deployments simply omit `[treasury]`. Only `zcash-testutils`
(synthetic proving) and `zcash-regtest` (optional Docker consensus) are Cargo
feature flags (`--no-default-features` does not select a different binary).
Build identity always reports the `zcash` capability marker followed by any
enabled test-feature markers.

## Build storage options

Build an optimized release executable without incremental caches:

```sh
make build
target/release/x402_treazury --help
```

- Bare `make` and `make build` default to release mode with
  `CARGO_INCREMENTAL=0` (including the protoc helper), no debug info, and
  integer overflow checks enabled. Use `PROFILE=NAME` to select another
  non-incremental profile (Cargo profile flags in `CARGO_ARGS` are rejected).
  See [entrypoint reference](../scripts/README.md) and
  [reproducible builds](reproducible-builds.md).
- Development and test builds use `debug = "line-tables-only"` for file and
  line backtraces without full variable debug info. Use `make build-dev` for an
  incremental dev build (`CARGO_INCREMENTAL=1`), `make build PROFILE=dev` for a
  non-incremental dev build, or `CARGO_INCREMENTAL=0 make test` to disable
  incremental test caches.
- Set `CARGO_PROFILE_DEV_DEBUG=2` or `CARGO_PROFILE_TEST_DEBUG=2` for full
  debugger symbols.
- `make check-lifecycle LIFECYCLE_PROFILE=test-optimized` selects an opt-in
  profile inheriting `test` with optimization level 2, debug assertions, and
  overflow checks (used in Linux CI lifecycle diagnostics).
- With builds stopped, delete only `target/debug/incremental` to reclaim
  incremental cache space. Preserve the rest of `target/`, which can hold
  qualification evidence, pinned executables, and reports; never run blanket
  `cargo clean` as routine cleanup.

### Compilation parallelism

Cargo schedules independent crates concurrently using logical CPU count by
default. Control compiler concurrency with `CARGO_BUILD_JOBS` independently of
libtest's `TEST_THREADS`:

```sh
CARGO_BUILD_JOBS=8 make cargo-check CARGO_ARGS='--all-targets --timings'
make test CARGO_ARGS='--all-targets --no-run --timings'
```

Timings write to `target/cargo-timings/` (see Cargo's
[job control](https://doc.rust-lang.org/cargo/reference/config.html#buildjobs)
and [timing reports](https://doc.rust-lang.org/cargo/reference/timings.html)).
Keep separate Cargo invocations and feature configurations sequential during
validation because CLI fixtures share `target/debug/x402_treazury`.

## Verification

Select checks using [validation by change](testing.md#selecting-validation-by-change),
and use `make check` for full application qualification. Common local checks:

```sh
make test
make clippy
make fmt-check
```

- `make check-clippy` runs production library and binary compilation followed
  by all-target `zcash-regtest` Clippy, rejecting emitted dependency and cached
  warnings as well. `make clippy` checks selected targets with `-D warnings`
  only. See [compiler diagnostics and review](testing.md#compiler-diagnostics-and-review).
- Offline suites use localhost sockets and deterministic unfunded test keys
  (never spending funds or reading `.env`). See
  [fixture maintenance](../tests/fixtures/catalogs/README.md),
  [network verification](network-egress.md), and
  [consensus tests](../tests/REGTEST.md).

### Local complexity metrics

Run `python3 scripts/complexity.py` to generate `target/complexity/report.md`
plus complete JSON and CSV scores using pinned `rust-code-analysis` (separating
production callables, tests, and examples, and joining coverage only for
byte-identical source files).

- `make check` runs the audit by default, warning on new or worsening hotspots
  above CogC 20 or CycC 40 against the merge base with `main` (scores warn
  rather than fail; incomplete analysis fails visibly).
- Pass `--base REF` or `COMPLEXITY_BASE_REF` to compare against another ref.
  See [COMPLEXITY.md](../tests/COMPLEXITY.md) and its
  [review discipline](../tests/COMPLEXITY.md#review-discipline).

### Local coverage

Install `cargo-llvm-cov` (`cargo install cargo-llvm-cov --locked`) and LLVM
tools matching `rustc -vV` (Homebrew LLVM is detected automatically; Rust
1.99.1 from Homebrew uses LLVM 23.1.3; override with `LLVM_COV` and
`LLVM_PROFDATA`, or install `llvm-tools-preview` under rustup).

```sh
make coverage
```

- Reports write to `target/coverage/` (`html/index.html`, `summary.txt`,
  `summary.json`, and `lcov.info`), covering application source and excluding
  integration harnesses, examples, operational tools, and vendored code (inline
  unit-test helpers can still count).
- Stable Rust reports line, region, and function coverage (not branch coverage).
  Optional proving, Docker consensus, and live funded tests are excluded.
- For condition-only baseline and final comparisons, use the isolated
  [condition analysis workflow](../tests/COVERAGE.md#condition-analysis).

## Logging and diagnostics

All logs go to stderr; stdout carries the MCP wire or requested CLI JSON.
`RUST_LOG` overrides the default `warn,x402_treazury=info` filter for every
command.

Manual raw-call headers use sensitive HTTP values that are never logged,
serialized into previews, cached, or persisted (validation errors are
value-free). Application sanitization does not redact upstream response bodies,
and unmodified `rmcp` can log raw MCP arguments, malformed frames, results, and
transport error prose at verbose levels. Do not enable verbose SDK logging when
handling secrets; see [manual raw headers](agent-sources.md#manual-raw-headers).
The CLI separately sanitizes stdio initialization failures.

### Log severity

Choose log level by the outcome and scope of the owning capability after its
retry and fallback paths finish:

| Level | Use for | Examples |
| --- | --- | --- |
| `ERROR` | Required service or endpoint functionality is unavailable with no recovery remaining in the current lifecycle | Program failure, required catalog unavailable, catalog warming acquisition failure, unreachable Tor SOCKS at startup, fatal Store worker or funding owner failure |
| `WARN` | A requested operation fails after exhausting recovery, or functionality is unexpectedly lost | Document retrieval fails with no fallback; potentially chargeable payment fails; one-pass CLI command fails; unexpected catalog omissions; oversized resource rejection; `tools/list` above 500 tools |
| `INFO` | Meaningful user-visible lifecycle, progress, delay, or recovery changes | Startup, catalog ready, endpoint ready, warming progress, funding progress, entering relay fallback, Tor SOCKS reachability recovery, dependency recovery, unmatched exclusions |
| `DEBUG` | Routine outcomes, expected negative results, individual attempts, and troubleshooting detail | Successful calls, speculative probe misses, search misses, routine auth rejections, background polling, cache activity, capacity waits, LRU eviction |
| `TRACE` | Fine-grained protocol or execution detail | Internal transitions and transport sequencing under the same sanitization rules |

Core severity rules:

- **No intermediate `WARN` or `ERROR`:** Failures with retry or fallback
  remaining stay at `INFO` (when signaling a meaningful delay, fallback, or
  first transition to failing) or `DEBUG` (for individual attempts and repeats).
  Only the owner that knows the final outcome emits the terminal warning or
  error.
- **CLI exit diagnostics:** Failed `serve` emits `ERROR`; other failed CLI
  commands emit `WARN`, alongside the human-readable `error:` cause chain on
  stderr. Forced second-signal shutdown emits `WARN` with unknown-outcome
  guidance.
- **Structural-budget exception:** Catalog structural-budget rejection always
  emits `WARN` with fixed reason and numeric bounds (never catalog text, names,
  or URLs).
- **Expected vs. unexpected catalog omissions:** Explicit `exclude_operations`,
  `exclude`, and `exclude_tags` suppress expected incompatibilities without
  warning. The public catalog builder warns when unsupported operations are
  omitted. During deployment serving, the source emits a `WARN` for missing
  tools or failed acquisition. A listener blocked by an acquisition failure
  emits its own `ERROR`. Catalog warming reports the result for each selected
  source using the [warming diagnostics contract](framework-integration.md#warming-diagnostics).
- **Store and funding supervision:** Fatal Store worker death emits one
  immediate `ERROR` (without formatting panic payloads into the event);
  completed unpublished transactions emit `ERROR` noting that exact bytes stay
  retained and graceful shutdown cannot finish. Retried local SDK
  reconstruction and publication stay at `DEBUG`.
- **Fix encountered violations:** Fix excessive `INFO`, unclear messages, and
  inadequate failure detail in touched code, updating owning docs and tests.

### Message clarity and diagnostic detail

- `INFO`, `WARN`, and `ERROR` messages must read clearly in plain English
  without requiring source code or decoding internal enums. Structured fields
  add IDs and counts; they never replace a clear message.
- `WARN` and `ERROR` events must state what failed, its scope and impact, the
  known cause or uncertainty, and an actionable next step when established
  (either directly or by pointing to a correlated event).
- Keep basic explanations visible at `WARN` filtering and preserve sanitization
  across every level: log safe categories, numeric codes, stages, and
  correlation IDs (never credentials, raw URLs, request arguments, signed
  bytes, or upstream prose).

### Diagnostic context

- **Catalogs, caches, and filters:** File reads, origin fetches, cache hits and
  writes, shared loads, and shadowed `send_tools` rules log at `DEBUG`.
  Per-source readiness, startup loading summaries, warming progress, and
  unmatched exclusion selectors log at `INFO`. Catalog failures report source
  ID, `failure_stage`, fixed plain-language `reason`, and numeric `http_status`
  without URLs or schemas. See [catalog startup](configuration.md#catalog-failures-and-strict-inspection)
  for listener selection warnings. Disk cache I/O failures log `WARN` with
  safe OS or SQLite codes while preserving origin responses.
- **Acquisition and relay fallback:** Unsigned document fetches carry an
  ephemeral `document_id` and `request_kind`. Entering configured relay
  fallback logs at `INFO`; individual relay exchanges, unsigned relay failures,
  and speculative URL probes stay at `DEBUG` (including oversized origin probe
  candidates); exhausted document, catalog, API, and directory owners log
  `WARN` with possible-charge facts.
- **Payments:** Spans carry `call_id`, source, and HTTP method (never URLs,
  arguments, challenges, signed bytes, or upstream prose). Managed extension
  omissions and challenge description truncation log at `DEBUG`. Terminal
  payment failures log `WARN` with fixed failure kind, stage, `transport_reason`,
  `last_http_status`, `authorization_may_charge`, allowlisted x402 error codes,
  and exact `payment_settlement_failed` body classification (internal error
  chains retain unlogged upstream bodies and string `errorReason` and
  `errorMessage` from a unique receipt header).
- **Tor SOCKS startup and `tools/list`:** Unreachable Tor SOCKS on startup logs
  `ERROR` immediately and repeats at most once every 60 seconds while waiting
  (with endpoint, safe I/O kind, and cadence), then logs `INFO` once reachable
  after a prior error; see
  [startup SOCKS prerequisite](network-egress.md#serving-startup-socks-prerequisite).
  `tools/list` above 500 definitions logs an advisory `WARN` naming the
  endpoint and count and suggesting `discover_on_demand = true`.
- **Treasury scans, funding, and Base RPC:**
  - First job phase logs report the saved local phase; later transitions log
    progress. Validated NEAR enums, confirmation polls, Base RPC attempts,
    failover ordinals, and repeat scan failures log at `DEBUG`; first scan
    failure transitions, observation recovery, terminal NEAR outcomes (after
    persistence), and actionable Ironwood shortages (printing the shielded
    receive address, or directing the operator to
    `wallet status --config FILE` when unavailable) log at `INFO`.
  - After an SDK insufficient-funds rejection, `funding_assessment` evaluates
    owned Ironwood notes only: an incomplete scan logs an `INFO` unavailable
    assessment, confirmed plus unconfirmed owned Ironwood notes covering the
    required amount log a `DEBUG` spendability wait, and an Ironwood shortfall
    logs `INFO` with the receive address.
  - Background scan failures report `automatic_retry=true` and
    `retry_after_seconds` (90, 180, and 300 seconds); one-pass `wallet sync`
    warns with `automatic_retry=false`. Zingolib errors are sanitized to fixed
    categories, numeric gRPC codes, and Pepper shard-tree `pool` and
    `operation` labels, and operator-required stops warn once per episode
    without repeating.

## Dependency compatibility

Pinned cohorts in the root lockfile:

- **MCP, hashing, and storage:** `rmcp` 3.9.0, direct `sha2` 0.11,
  `chacha20poly1305` 0.11 (stable 32-byte key, 12-byte nonce, 16-byte tag, and
  v1 AAD bindings), `rand` 0.10 (application), `toml` 1.2 (strict parsing via
  `toml::from_str`), and `rusqlite` 0.41.1 (bundling SQLite 3.54.1 via
  `libsqlite3-sys` 0.39.1).
- **Complexity analyzer:** Pins Tree-sitter 0.20.9 and its grammar cohort
  (incompatible with 0.27).
- **Protoc helper:** Resolves its own lockfile in `tools/protoc/Cargo.lock`.

### Security update boundaries

- **Tokio and `ctutils`:** Require `tokio` >= 1.54.1
  ([RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386.html);
  1.54.0 is yanked for
  [RUSTSEC-2026-0384](https://rustsec.org/advisories/RUSTSEC-2026-0384.html))
  and `ctutils` >= 0.4.8
  ([RUSTSEC-2026-0385](https://rustsec.org/advisories/RUSTSEC-2026-0385.html)).
- **EVM signing isolation:** Production EVM key generation, public-key
  derivation, and signing use native `secp256k1` 0.33.1 (`libsecp256k1`) via
  `payment::signer::EvmSigner` and `SignerLike`, erasing 32-byte keys on drop
  (best-effort). Although `x402-chain-eip155` 2.0.2 (`client` and `telemetry`
  features) and Alloy primitives 1.7.3 pull `crypto-bigint` and `k256` into the
  dependency graph, application payment keys never pass through Alloy's local
  signer or `k256`. Any new secret consumer requires a fresh exposure review.

### Reproducible local advisory evaluation

Run [`scripts/dependency_audit.py`](../scripts/README.md#local-dependency-advisory-assessment)
against a pinned RustSec checkout to scan both active lockfiles plus every
path and Git dependency (standard `cargo-audit` skips nonregistry packages).
Keep raw reports and hashes under ignored `target/`, and inspect each match
against the pinned sources and local patches before assigning a disposition.

### Locked-graph advisory disposition

The baseline assessment (`target/nu7-readiness/security-assessment/summary.json`,
RustSec `26fd2d43f7280a1fb925a3743ec463017afe7119`) reports eight vulnerability
matches and five unmaintained notices. Known accepted registry exposure:

| Package and advisory | Exposure and acceptance boundary |
| --- | --- |
| `crypto-bigint` 0.5.5 (`RUSTSEC-2026-0387`) | Pulled in by upstream Alloy features; production EVM secret arithmetic stays in native `libsecp256k1` (`EvmSigner`). |
| `rustls` 0.21.12 (`RUSTSEC-2026-0346`, `RUSTSEC-2026-0349`) | Used only by `zcash_proofs`'s build-time `minreq` Sapling parameter downloader on uncached builds (public parameters, no runtime traffic or secrets). Accepting this accepts build-time TLS exposure on uncached builds. |
| `rustls-webpki` 0.101.7 (`RUSTSEC-2026-0104`, `RUSTSEC-2026-0098`, `RUSTSEC-2026-0099`, `RUSTSEC-2026-0382`); `webpki-roots` 0.25.4 (`RUSTSEC-2026-0381`) | Same build-time parameter downloader. Fixed BLAKE2b-512 digests and file sizes protect accepted file integrity (not TLS confidentiality or verification), and release qualification runs network-denied against prefetched parameters. |

Accepted unmaintained notices (do not vendor replacements solely to silence
them): `derivative` (`RUSTSEC-2024-0388`, via `ark-ff`), `fxhash`
(`RUSTSEC-2025-0057`, analyzer only), `json` (`RUSTSEC-2022-0081`, via
Zingolib; note possible soundness warning), `paste` (`RUSTSEC-2024-0436`, via
Alloy and `ark`), and `pin-utils` (`RUSTSEC-2026-0441`, via `wasmtimer`).
Reassess whenever call paths, features, pinned sources, or upstream fixes
change.

#### Dependabot alerts

| Alert | RustSec advisory | Disposition |
| --- | --- | --- |
| [#1 URI name constraints](https://github.com/mikeperry-tor/x402_treazury/security/dependabot/1) | `RUSTSEC-2026-0098` | Accepted in the baseline above |
| [#2 Wildcard name constraints](https://github.com/mikeperry-tor/x402_treazury/security/dependabot/2) | `RUSTSEC-2026-0099` | Accepted in the baseline above |
| [#3 CRL parsing panic](https://github.com/mikeperry-tor/x402_treazury/security/dependabot/3) | `RUSTSEC-2026-0104` | Accepted in the baseline above |
| [#4 Brotli certificate decompression](https://github.com/mikeperry-tor/x402_treazury/security/dependabot/4) | `RUSTSEC-2026-0348` | Locked versions unaffected or patched |
| [#5 TLS secrets in session debug output](https://github.com/mikeperry-tor/x402_treazury/security/dependabot/5) | `RUSTSEC-2026-0346` | Accepted in the baseline above |
| [#6 Key-exchange downgrade](https://github.com/mikeperry-tor/x402_treazury/security/dependabot/6) | `RUSTSEC-2026-0349` | Accepted in the baseline above |

For alert #4, the [GitHub advisory](https://github.com/advisories/GHSA-h2v8-394m-p7vv)
flags `rustls` versions below 0.23.46. [RustSec](https://rustsec.org/advisories/RUSTSEC-2026-0348.html)
lists versions before 0.23.9 as unaffected, including the downloader's
locked 0.21.12. The application locks 0.23.46, which includes the fix.
Zingolib's parameter downloader retains `rustls` 0.21 through `zcash_proofs`
and `minreq` 2.14.1. Neither `brotli` nor `brotli-decompressor` appears in the
root lockfile. Vendored Reqwest's standalone lockfile includes Brotli for its
own tests, and the application leaves Reqwest's optional Brotli HTTP response
decompression disabled.

### TLS security updates

- **Runtime TLS:** Require `rustls-webpki` >= 0.103.16 in the 0.103 series
  and `webpki-roots` and `webpki-root-certs` >= 1.0.10 for the
  [emSign intermediate-key compromise](https://rustsec.org/advisories/RUSTSEC-2026-0382).
  Update host trust stores and rebuild deployed binaries after lockfile changes.
- **Developer Python tools:** Follow
  [Python's emSign guidance](https://discuss.python.org/t/information-for-python-users-regarding-emudhra-emsign-ca-certificate-security-incident/109576):
  use `pip` >= 26.2.2, `uv` >= 0.13.4, and `certifi` >= 2026.10.19, and keep
  system certificates updated.

### Combined workspace verification

The root workspace contains the application (default member) and
`treazury-complexity` (selected explicitly). All application, payment, MCP,
crypto-vector, and SDK compatibility targets share one lockfile, patch table,
profile, and build cache:

```text
Alloy    -> sha3 0.11.0  -> digest 0.11.3
Zingolib -> bip32 0.6.0  -> hmac 0.13.0 -> digest 0.11.3
```

- `lightwallet-protocol` 0.4.0 supplies Ironwood fields without a Git override.
- `x402-chain-eip155` requires both `client` and `telemetry` features enabled
  because its `upto` code references `tracing`.
- Run source verification and the combined compatibility suite:

```sh
python3 vendor/verify.py
make check-compat
```

- Run the three-pool `ScanMerge` regression directly (via Pepper's public
  payload constructor in `tests/support/sdk_merge.rs`):

```sh
make test CARGO_ARGS='--test sdk_compat' TEST_ARGS='sdk_merge::scan_merge_conflict_preserves_pool_context --exact'
```

- `make check-compat` runs `vendor/verify_pepper.py`, obtains Cargo-managed
  `protoc`, and executes `sdk_compat`, `payments`, `server`, and
  `crypto_vectors` using temporary offline wallets and localhost servers (no
  live sync or funds). SDK upgrade tests check format-45 roundtrips and address
  continuity. See [vendor ledger](../vendor/README.md) and
  [Zingo integration](zingo-integration.md).
