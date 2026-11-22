# Testing and qualification

Use offline regressions to verify application behavior and reserve optional
consensus and live runs for the integration boundaries they exercise. Coverage
percentages and HTTP `200` responses do not replace financial, permission, and
network-isolation assertions.

When updating tests, keep complete schema, description, and assertion contracts
intact; never trim documentation or loosen checks to pass. Discovery tests
inject public-URL documents and localhost fixtures only under `cfg(test)`,
never through a production loopback bypass.

## Platform scope and portable controls

Engineering and runtime scope covers Linux and macOS (Windows runtime and
native acceptance remain deferred in
[release follow-ups](plans/deferred/release_followups.md)).

- **Connectivity controls (`network::qualification_probe_tests`):** Classify
  permission denials via Rust `ErrorKind` and native OS mappings, exercising
  loopback TCP and UDP positive controls, a held non-listening TCP port,
  reply timeouts, and destination preflight without firewall changes. Used by
  the runner's `EgressProbe` command (separate from `scripts/qualify_tor.py`).
- **Private storage (`src/rotation/store/` and `src/http_cache.rs`):** Native
  Linux and macOS controls cover atomic creation under inherited macOS ACLs,
  descriptor reopen rejection, unlinked SQLite sidecar validation (symlink,
  hardlink, mode, and macOS ACL checks), denied-read controls, and private
  preflight cleanup. See [portability assessments](plans/portability_fixes.md).
- **Crash and SQLite I/O fault fixtures (Unix-only):** `SIGKILL` tests kill an
  owned signing child at signature, journal-acknowledgement, and signed-dispatch
  boundaries and assert atomic reopen with zero replay; a custom SQLite test
  VFS fails WAL and checkpoint writes, fsyncs, and `SQLITE_FULL` page limits.

## Provider regressions

Shared capture, comparison, and candidate regressions live in
`provider_updates` using synthetic inputs (`make test CARGO_ARGS='--example
provider_updates'`; see [provider maintenance](provider-updates.md)). Sweep
outputs stay ignored under `target/provider-updates/`.

- Store provider-specific fixtures only for a named quirk, failure, or custom
  transformation, using the smallest faithful input and asserting the target
  schema, routing, filter, or output behavior. Put generic parsing and
  selection in shared synthetic tests; validate all bundled TOMLs offline
  without freezing whole settings maps or inventories.
- Catalog structural-budget tests cover media-type precedence, wildcard
  fallback, long flat `$ref` chains, repeated pointer lookup charging,
  duplicated path, pricing, and metadata copies, depth, node, and byte limits,
  overflow errors, sanitized `WARN` diagnostics, dynamic and recursive `$ref`s,
  scalar and union parameters, and `Idempotency-Key` rules.
- Qualification budget tests verify atomic `total_test_budget_usdc`, per-case,
  and `cumulative_test_budget_usdc` enforcement across concurrent runs before
  signed dispatch (preserving `u64` budget parses and `U256` per-case totals
  and keeping refused admissions free of journals or balance changes).

## Framework degradation

The [framework integration contract](framework-integration.md) separates
completed warming from catalog coverage and MCP listener availability. Focused
warming, selection, and readiness fixtures are mapped in
[best-effort framework startup](../tests/STARTUP.md#best-effort-framework-startup).

## Dynamic registry publication

Registry fixtures run in isolated child processes to observe import builds,
confirm reuse across unrelated additions and static refreshes, and rebuild only
invalidated inputs. Publication fixtures hold a commit at a deterministic
test-only boundary so concurrent readers keep serving the prior complete
snapshot while duplicate registrations wait and finish without a second fetch
(switching metadata, tools, instructions, omissions, and search cursors
together).

## Testing efficiency

- **Pinned single toolchain:** Run normal validation on the pinned production
  compiler and named Make stages without building compiler, feature, profile,
  or coverage matrices. Nightly condition analysis (`make coverage-condition`)
  runs only for explicit coverage comparisons, outside `make test` and
  `make check`.
- **Reuse valid evidence:** Keep passed stage results while source, features,
  profiles, and inputs are unchanged, and resume unfinished stages after
  localized fixes. Never add automatic skip caches to Make stages.
- **Consolidate setup, isolate state:** Share immutable synthetic inputs and
  helpers while keeping fresh mutable Stores, keys, and authorities per test.
- **Assert functionality and forbidden side effects:** Check results, exact
  requests, persisted transitions, counts, and absent side effects (no
  signature on refusal, no publication on denial, no send before persistence,
  no bypass of `src/network.rs`). Put pure parser and policy checks in fast
  unit tests and keep representative integration boundaries.
- **Deterministic synchronization:** Use paused Tokio time for owned timers and
  barriers or notifications at the exact boundary under test (inventory
  capture, client publication, or final owner drop). Avoid wall-clock sleeps
  unless a cross-process or OS I/O invariant requires one, and never alter
  production cadences or add arbitrary deadlines to speed up tests.
- **Optimized arithmetic:** Validate arithmetic changes under the release
  profile (`make test CARGO_ARGS='--release --test payments'` plus focused
  release library tests), which enables integer overflow checks throughout the
  dependency graph.

## Selecting validation by change

Pick the smallest check set that covers the changed behavior and its consumers,
plus any user-requested checks:

| Change | Validation |
| --- | --- |
| Prose or comments only | Review accuracy, local links, heading anchors, and `git diff --check`; skip Cargo builds and tests (executable examples and doc-embedded assets still verify their consumers). |
| Configuration, provider TOML, or schema data | Parse edited inputs and run matching catalog, routing, or request tests. Recompile when testing embedded bundled providers (`src/providers/assets.rs`). |
| Tests or fixtures only | Compile, lint, and run matching test targets and consumers; broaden when shared helpers change. |
| Runtime Rust | Run `make check-clippy` and focused behavior and consumer tests; add compatibility and runtime stages for touched boundaries. |
| Dependencies, toolchain, build rules, or broad refactors | Check resolution, compilation, compatibility, and complete call chains; run vendor verifiers under [source verification boundaries](../vendor/README.md#source-verification-boundaries). |
| Financial authority, payment or recovery, network isolation, protocol, or concurrency | Always include boundary, failure, recovery, and forbidden-side-effect tests regardless of file type. |

Verify filtered test runs execute nonzero tests. Run
[reproducible release builds](reproducible-builds.md) only for release
candidates, reproducibility changes, or explicit requests (neither `make build`
nor `make check` runs it). Prefer `make build-dev` for iterative binary builds
unless release optimization is required.

## Default development checks

The application always compiles Zcash support. Run checks through the root
[Makefile](../Makefile) so pinned Cargo and Cargo-managed `protoc` are used
(do not pass a second `--locked`). See [scripts/README.md](../scripts/README.md).

`make check` runs these stages in order (stopping on the first failure even
under `make -k`), using `check-default-remaining` after `check-compat`:

| Stage | Selection |
| --- | --- |
| `check-static` | Formatting, tooling regressions, analyzer tests and CogC and CycC report, vendor provenance |
| `check-clippy` | Production library and binary compiler check, then all-target Clippy with `zcash-regtest` (fails on emitted application, dependency, and cached warnings) |
| `check-compat` | Pepper verification plus `sdk_compat`, `payments`, `server`, and `crypto_vectors` |
| `check-default-remaining` | All remaining default root targets derived from Cargo metadata after valid `check-compat` |
| `check-default` (standalone) | Complete `--all-targets` default suite (not repeated inside `make check`) |
| `check-proving` | `zcash-testutils` treasury proving tests (1 thread) |
| `check-lifecycle` | `zcash-testutils` synthetic-chain treasury lifecycle (8 threads, `--nocapture`) |
| `check-codec` | `zcash-testutils` address-codec isolation regression (1 thread) |

- Default harnesses use 8 threads (`TEST_THREADS=1` runs serially for
  `make test`, `check-compat`, `check-default`, and `check-default-remaining`).
  `CARGO_ARGS` customizes `make build`, `make build-dev`, `make test`,
  `make cargo-check`, `make run`, and `make clippy`, and `TEST_ARGS` customizes
  `make test` only; neither affects `check-*` stages.
- `tests/integration_preparation.rs` runs shared `live_integration` unit tests
  once alongside compiled-executable integration tests (`live_integration`'s
  own test harness is empty under `--all-targets`).
- Diagnostic tests whose callsites are also hit by subscriberless siblings use
  [log_capture.rs](../tests/support/log_capture.rs) in an isolated child
  process so cached disabled tracing interest never drops events.
- Cleared child environments forward only `LLVM_PROFILE_FILE` when set (never
  ambient credentials or proxy variables).

### Compiler diagnostics and review

`make check-clippy` runs a production-only check (library and binaries without
test features) followed by all-target `zcash-regtest` Clippy, piping Cargo JSON
through `scripts/check_warnings.py` so emitted dependency warnings and cached
warnings fail the stage (plain `make clippy` uses `-D warnings` on selected
application targets only).

- Inspect compiler diagnostics alongside exit status; resolve warnings in
  touched application and vendored code and disposition any remaining warnings.
- Large macOS debug test binaries can trigger the linker's
  `__eh_frame section too large` diagnostic (Apple `libunwind` falls back to
  full DWARF search). Keep unwind data, panic strategy, and toolchain pins
  unchanged and verify panic and restart coverage.

### Validation order and continuation

1. Apply [selecting validation by change](#selecting-validation-by-change),
   complete independent design review, and run cheap static checks and
   [complexity review](../tests/COMPLEXITY.md#review-discipline) before broad
   runtime suites.
2. When selected, run `make check-clippy check-compat` and focused contract
   tests before long suites.
3. For a full check once sources stabilize, finish remaining stages:

   ```sh
   make check-default-remaining check-proving check-lifecycle check-codec
   ```

4. After a localized failure, fix it, revalidate dependent consumers and
   invalidated stages, and resume unfinished stages, recording commands,
   revision, features, profiles, and nonzero test counts (describe assembled
   stage runs as continuations rather than one `make check` run).
5. Keep sources and executables frozen during validation and run feature builds
   sequentially because CLI fixtures share `target/debug/x402_treazury`.
   Coordinate external Cargo and editor checks manually.

### Test executable startup

On macOS, Make test targets run through
[scripts/test-runner.sh](../scripts/test-runner.sh), which copies each test
binary without extended attributes into a private temp directory, ad-hoc signs
it, and runs the copy to work around pre-main `_dyld_start` hangs
([OpenAI Codex #23802](https://github.com/openai/codex/issues/23802)). Linux
runs binaries directly. Copy or signing errors fail closed rather than falling
back; `CARGO_BIN_EXE_*` child paths still invoke the original binary, and
ad-hoc-signed copies are never used for hash-pinned live or release evidence.

### Verified TLS fixtures

Local provider fixtures use verified TLS with the public test CA in
`tests/fixtures/tls/ca.pem` (see [tls/README.md](../tests/fixtures/tls/README.md)):

- Independent MCP and payment clients must opt in to that CA; subprocess
  fixtures pass it via `[network].trusted_ca_files`. Negative controls use an
  independent context and `untrusted.der`.
- Tests needing distinct immutable network policies run in isolated child
  processes asserting nonzero child test counts.
- Serving fixtures verify origin handler receipt as well as SOCKS selection,
  drain HTTP servers before reopening registries, and surface early client
  errors on deferred barriers.

### Sandbox permissions

Request socket-capable execution up front when the current sandbox blocks
localhost sockets rather than provoking a failure first:

| Target selection | Runtime requirements |
| --- | --- |
| `crypto_vectors`, `headers`, `provider_catalogs`, `managed_config`, `network_audit`, `cli_layout` | Socket-free targets (local computation, files, and CLI child processes); run in the normal sandbox. |
| `network`, `startup`, `cli_contracts`, `config` | Include localhost socket fixtures; run with localhost socket access. |
| `make test`, `make check` | Full suites requiring localhost socket access. |

`config` and `cli_contracts` mix socket-free and socket-backed tests; inspect
filtered cases before choosing permissions. Never skip wire, SOCKS, gRPC, or
lifecycle coverage to avoid a permission prompt.

## Test boundaries

| Area | Principal coverage |
| --- | --- |
| Catalog, config, and providers | Schema projection, structural budgets, `prefer_post`, `send_tools`, `response_mappings`, pricing descriptions, cache acceptance, alias refresh, pricing retirement, and pinned local catalogs (`src/catalog.rs`, `tests/provider_catalogs.rs`, `tests/config.rs`, `tests/catalog_selection.rs`). |
| MCP, deployment, and discovery | Listener auth and Host validation, local, remote, and raw authority, manual raw headers (`discovery::raw::tests`, `TRACE` redaction, header-capacity overflow), directory subsets, immutable invocation snapshots, concurrent registration, deferred startup, foreground drain (`tests/foreground_capacity.rs`, `tests/shutdown.rs`, `tests/supervision.rs`). |
| Payments and managed rotation | Local EIP-3009 and Permit2 signing, cap and asset checks, journal-before-disclosure, narrow structured insufficient-funds traversal, advisory selection, passive hints, atomic `READY` promotion, early rotation, and automatic retirement (`tests/payments.rs`, `tests/managed.rs`, `tests/base.rs`). |
| Treasury, Store, and funding | Encrypted persistence, schema-23 preflight, Ironwood-only proposals, offline calculation ownership, publication retries, synthetic proving, deep reorgs, managed refund replacement, and backups (`tests/funding.rs`, `tests/treasury_sync.rs`, `tests/rotation.rs`, [LIFECYCLE.md](../tests/LIFECYCLE.md)). |
| Network and Tor isolation | Authenticated fake SOCKS, wire-credential correlation, pre-I/O cache race isolation, TLS retry and post-header corruption (`network::tls_retry_tests`), `Retry-After` waits, HTTP and gRPC inactivity, zero-stream-capacity cancellation, relay fallback, and `tests/network_audit.rs` (`tests/network.rs`, `rotation::base::tor_*`). |
| Output and resource limits | Exact byte bounds, visible rejections and logs, no partial cache writes, no paid retry on oversized bodies, inline image extraction (`tests/download_limits.rs`, `tests/media.rs`, `tests/confidential_errors.rs`). |
| Live runner offline tests | Registry reservations, budget atomicity, lifecycle schema-2 projection, content assertions, report composition, Tor audit interpretation, and preset parity (`tests/integration_preparation.rs`, `tests/live/runner/`). |

See [COVERAGE.md](../tests/COVERAGE.md) for the full boundary map and
condition-analysis workflow. Key testing invariants:

- Fixed-chain Base RPC fixtures capture block timestamps at snapshot creation
  so headers never contradict across a wall-clock second boundary.
- Deployment-profile tests keep literal expected values rather than deriving
  expectations from the TOML under test.
- Automatic retirement tests run through serving or finite sync and canonical
  publication, crossing 64-record authorization pages, both directions of
  finalized vs. latest disagreement, malformed optional history, and never-credited
  source closure.
- Progress-policy tests cover fragmented 20-second gRPC streams, slow SOCKS
  connects, stalled bodies, sibling-traffic isolation, first-signal drain, and
  second-signal forced exit.

## GitHub Actions and coverage

CI runs on Ubuntu 24.04 in [`.github/workflows/ci.yml`](../.github/workflows/ci.yml)
on `push` to `main` and `ci_coverage` and on `pull_request` to `main`:

- **Jobs:** `check` runs all seven stages sequentially via
  `scripts/ci-stage.sh` (`check-static`, `check-clippy`, `check-compat`,
  `check-default-remaining`, `check-proving`, `check-lifecycle`, and
  `check-codec`). `coverage` runs in parallel with `check` on every trigger,
  collecting default Linux line coverage via `make coverage` and reporting with
  `octocov`. `publish-badge` runs after `check` and `coverage` succeed on
  `push` to `main`.
- **Coverage badge:** `publish-badge` writes `coverage.svg` and
  `source-revision.txt` to the root of the orphan `badges` branch (serializing
  writes via concurrency group `main-coverage-badge` and skipping superseded
  commits). It reflects default Linux line coverage only (excluding optional
  proving, Docker consensus, and live tests).

### Temporary CI diagnostics

`scripts/ci-stage.sh` records per-stage logs, durations, and exit status under
`target/ci-checks/`, uploaded on every `check` run (`if: always()`, default
GitHub retention). In `check`, `check-lifecycle` currently runs with
`LIFECYCLE_PROFILE=test-optimized` and automatic `LIFECYCLE_DIAGNOSTICS` GDB
capture after installing `gdb` and relaxing `kernel.yama.ptrace_scope`.

Retire the automatic GDB capture in `ci.yml` and the temporary `push` trigger
on `ci_coverage` in `treasury-sync-diagnostics.yml` once the `main` coverage
badge is accepted, keeping `scripts/diagnose-treasury-sync.py` and manual
`workflow_dispatch` available for future investigations.

### Potential SDK mempool shutdown hang

[`.github/workflows/treasury-sync-diagnostics.yml`](../.github/workflows/treasury-sync-diagnostics.yml)
supports manual `workflow_dispatch` (plus a temporary `push` trigger on
`ci_coverage` for diagnostic script and test edits). It compiles
`tests/treasury_sync.rs` and runs `scripts/diagnose-treasury-sync.py` (8
threads, 5-minute diagnostic window outside compilation) to capture native GDB
stacks before exiting `124` if a Pepper mempool shutdown hang occurs.

## Qualification status

Offline suites never qualify live settlement, upstream uptime, or live Tor
circuits. Current status across verification layers:

| Scope | Status | Basis and limits |
| --- | --- | --- |
| Offline application suite (`make check`) | Qualified locally and in Linux CI | Unfunded fixtures, localhost sockets, synthetic-chain proving and lifecycle (`tests/LIFECYCLE.md`). |
| Docker Zcash consensus (`zcash-regtest`) | Opt-in; NU6.3 images predate v8 SDK | Temporary Zebra and Zaino regtest containers (`tests/REGTEST.md`); requires requalification on v8 SDK. |
| Live Tor circuit isolation and confinement | Qualified on macOS (`scripts/qualify_tor.py`) | Verified on macOS 26.6.2 with Tor 0.4.9.13 using dummy identities and `zec.rocks` reads (`tests/TOR.md`); Linux namespace confinement remains separate. |
| Reproducible release builds | Qualified on prior 1.99.0 profile; 1.99.1 pending | Same-host network-denied two-build comparison on Apple Silicon macOS (`docs/reproducible-builds.md`). |
| Live mainnet public swap and single paid call | Opt-in manual runbook | Bounded single-call demo (`docs/public-swap-demo.md`). |
| Live 23-provider sweep over owned Tor | Opt-in runner preset (`PROVIDER_PRESETS.md`) | Historical 2026-10-04 sweep; new providers, GenuineGood `POST`, and fresh availability require new reviewed runs. |
| Automated two-round rotation, restart, and combined cover and payment | Deferred | Tracked in [live integration acceptance](plans/deferred/live_integration_acceptance.md); offline fixtures cover local invariants only. |
| Confidential Zcash swaps and managed Permit2 `upto` | Outside current scope | Managed pools support public Base USDC v1 and v2 `exact` EIP-3009 only. |
| Live shielded-refund end-to-end delivery | Unqualified | Dry quotes verify format acceptance; live refund observation requires explicit operator review (`tests/live/INTEGRATION_REFERENCE.md#shielded-refund-evidence-and-qualification`). |

## Live runner regression map

Shared runner unit tests run once in `tests/integration_preparation.rs`
alongside compiled-executable supervision tests:

| Area | Coverage in `tests/live/runner/` and `tests/integration_preparation.rs` |
| --- | --- |
| Planning and case selection | Offline `plan` and `select-cases`, placeholder blocking, tag and dependency checks, absolute path pinning, and schema-2 plan JSON. |
| Registry and budgets | Atomic per-case, `total_test_budget_usdc`, and `cumulative_test_budget_usdc` reservation across concurrent runs, reopen retention, refusal rollback, and legacy read compatibility. |
| Preparation and frozen reload | Build-identity pinning, full 128 MiB catalog capture, format checks (`date`, `date-time`, `uri`), frozen reload parity, and failure retention without partial publication. |
| Execution and supervision | Batch reservation, per-case dispatch intent, independent phase continuation, graceful drain on cancellation, `--qualification-no-new-funding` and `--qualification-unsigned`, and `funded_pools` adoption. |
| Assertions and reports | Content checks (`json_exists`, `json_equals`, `text_contains`, nested `isError: true`, scan bounds), fee and challenge capture bounds, help-cache states (`fetch`, `hit`, `shared`), canonical Base debit proofs, lifecycle schema-2 projections, JSON and sanitized Markdown reports, and offline `audit-tor`. |
