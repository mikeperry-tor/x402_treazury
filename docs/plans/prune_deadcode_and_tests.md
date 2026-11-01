# Prune obsolete compatibility and tests

## Intent, scope and starting evidence

Remove application responsibilities for obsolete database, configuration, cache
and wallet formats, together with tests maintained only for those responsibilities.
Prefer assertions about results, persisted state, requests and forbidden side
effects over assertions about human log wording. Reduce redundant execution and
fixture cost without weakening current correctness or moving useful tests out of
the default check merely to make it faster.

The audited baseline is `59dbc8db55245bcc93a5d54855b08d6776ddcdb9`.
Three independent read-only audits covered storage/wallet formats, config/catalog
compatibility, and diagnostic tests/runtime. The coordinator also inspected
qualification producers and consumers, historical reporting, CLI contracts and
test orchestration. A subsequent standalone compiler/LLVM probe verified coverage
support as recorded below. No application builds, runtime suites or fresh suite
timing runs were performed for this plan. Findings are source evidence, not a measured
speedup or a qualification claim. Unrelated local files remain outside scope.

Implementation is authorized on `prune_deadcode_and_tests`. Checkout reassessment
starts at `3ceb88f2e41038c4f6ef6959494c2f501373f1cd`: only this plan and its index
link differ from the audited application baseline. Unrelated local files remain
outside scope. Private-state conversion and live-funded qualification are not
authorized. Commit identifiers below describe implementation boundaries.

Read these owning guides before the corresponding implementation:

- [Development](../development.md), [testing](../testing.md),
  [test entrypoints](../../scripts/README.md) and
  [complexity review](../../tests/COMPLEXITY.md#review-discipline).
- [Wallet ownership](../wallet-rotation.md), especially best-effort rules,
  observation policy and recovery/reconciliation; [wallet CLI](../wallet-cli.md),
  [Zingo integration](../zingo-integration.md) and
  [lifecycle coverage](../../tests/LIFECYCLE.md).
- [Configuration](../configuration.md), [runtime architecture](../architecture.md),
  [agent sources](../agent-sources.md), [network egress](../network-egress.md),
  [local resource limits](../local-resource-limits.md),
  [provider policy](../../providers/README.md) and
  [provider caveats](../../providers/CAVEATS.md).
- [Live runbook](../../tests/live/INTEGRATION.md) and
  [reference](../../tests/live/INTEGRATION_REFERENCE.md) for qualification readers.

## Preservation and format policy

Proposed behavior is current-only application input and storage support. Current
code does not consistently enforce that policy: owner opens accept database
schemas 21–23, status projects much older schemas, configuration accepts aliases
and obsolete ignored fields, and qualification interprets historical shapes.

Keep distinct format domains distinct. At baseline the database is schema 23,
`instance.version` and encrypted record/AAD are version 1, backup manifest format
is 1, status JSON is version 5, bootstrap JSON is version 3, deployment TOML is
version 1, and the current SDK wallet format is 45. These numbers are not evidence
of obsolete support and need not match. Removing `Status.legacy_refunds` requires
status JSON version 6; retain bootstrap
JSON version 3 with its nested current status. Lifecycle evidence has its own
version 2 contract, independent of status JSON and database versions. Document
these output boundaries rather than silently reusing an incompatible contract.

Unsupported financial stores must fail before schema/data mutation. Keep complete
private backups, exact accepted inputs, encrypted records, signed bytes, original
identities, destinations and liabilities. No deletion of local state, automatic
conversion, backup rollback, new funding deposit or replay authority follows from
this cleanup. Current-format corruption/reorg repair and reconciliation remain
supported. Never interpret absence of a result as proof that disclosure failed.

Disposable caches can miss and refetch; they do not need converters. Persisted
dynamic-source registrations are not disposable catalog cache entries: reject an
unsupported snapshot without overwriting it, retaining current owner/registration
semantics. Reject obsolete configuration through ordinary strict parsing instead
of maintaining custom migration messages or accepting ignored settings.

Do not equate “legacy” names with dead behavior. Preserve current x402 v1/v2
wire support, challenge/receipt normalization, Tor identity/token domains,
HTTP compatibility flags, optional tool routing defaults, listener `sources`
attachments and absent `send_tools` semantics. Preserve existing `X402_*`
environment contracts and generated pool names. New arbitrary deadlines, caps,
queues, financial gates and dependency forks are outside the proposed approach.

## Findings and proposed removals

### Application database and reporting

`src/rotation/store.rs` owner preflight and identity readers accept 21–23, while
`read_status` still branches for schemas below 2, 4 and 21 and reads refunds for
7–20. `src/rotation/store/reporting.rs` has additional branches below 3, 8, 16
and 18. Align status, owner opens, identity, inspection and qualification on the
chosen current storage contract, then delete historical projections and invented
empty/default output for missing old tables.

Concrete candidates:

- Delete `funding::legacy_readiness_recorded_valid` after status no longer reaches
  unsupported schemas; supported owner schemas already use `initial_credit_recorded`.
- Delete `src/rotation/store/refunds.rs`, its historical type and
  `Status.legacy_refunds` with their CLI/JSON/regtest consumers and output docs.
  Current refund-address ownership and shielding are unrelated and remain.
- Delete test-only `funding::preflight_funding_upgrade` and
  `evidence::initialize_in_transaction` plus their sole historical conversion
  tests. The latter builds 2,048 rows to exercise retired conversion code.
- Remove 21/22 upgrade acceptance and the upgrade-specific test
  `owned_legacy_upgrade_excludes_old_readers_without_rewriting_financial_records`.
  Remove obsolete index-drop/version-write work when its last supported consumer
  disappears, rather than retaining a migration framework with no supported inputs.

An important prerequisite: fresh `schema.sql` says schema 23 but relies on runtime
initializers to create `source_disposition_history`, `payment_resolution_history`
and current indexes. Move required persistent structures into the canonical fresh
schema before deleting initializer responsibilities. Keep ephemeral
`evidence_generations` initialization. Determine which existing schema-23 shapes
the current program actually produces before choosing the exact deletion. Prefer
retaining a small necessary idempotent current initializer over adding a schema
fingerprint framework or rejecting valid current stores. If an initializer is
solely an old-shape upgrade, remove it with a coherent format boundary and
read-only rejection path. Resolve this in P2's design review and verify it at G1; do not simply remove
DDL because the version check was tightened.

Keep source disposition and payment resolution histories, `archive_expiry`,
`source_previously_closed`, original transaction references, quote-refresh recovery
classification and current derived-evidence repair. They protect current accepted
work. Tests mentioning old records can still expose current bugs; rewrite their
inputs around current reachable facts where that preserves the contract.

Replace the 13-version historical projection/populated-schema matrices in
`store/reporting_tests.rs`, `tests/rotation.rs` and
`tests/support/store_integrity.rs` with compact boundary tests: unsupported old and
future versions reject without mutation; current read-only queries preserve
journals; current malformed observation/facts fail without repair. Remove
`tests/fixtures/state/historical.sql` only after all consumers are replaced.
Prune historical halves of `obsolete_capacity_metadata_is_optional_and_never_rewritten`;
review `legacy_pool_generation_is_report_metadata_not_admission_authority` as a
current authority test because generation remains in the fresh schema.

### Configuration and CLI

Remove the provider `timeout` alias on `Config.read_timeout_seconds`,
`config::normalize_timeout`, network
`request_timeout_seconds`, CLI `--timeout` and `--meta-config` aliases, and ignored
`WalletConfig.wait_seconds` plus its warning. Preserve canonical timeout
composition/provenance and current inactivity behavior. Replace the custom
`impl_config_serde!`/remote-Self/replaced-setting machinery in
`src/rotation/config.rs` with ordinary derived strict Serde where possible;
keep validation of current monetary settings and required reserves.

Delete historical setting-name/migration-wording matrices in `src/config.rs`,
`tests/pricing.rs`, `tests/managed_config.rs` and rotation config tests. Keep one
shared meaningful strict unknown-field contract per parser boundary. Retain exact
monetary parsing, required fields, canonical defaults, roundtrips and CLI
precedence/conflicts. In `tests/config.rs`, preserve canonical override/origin
assertions; remove alias cases. In `tests/network.rs`, preserve timeout
defaults/validation/inactivity while deleting old-spelling assertions.

Remove the obsolete `wallet pool` command shim and tailored removal-message tests
in `tests/cli_contracts.rs` and `tests/treasury.rs` if the implementation exists
only to explain the removed command. Keep current wallet initialization, overwrite
protection and restore contracts. Remove duplicate `config show --meta-config`
invocations from `tests/cli_layout.rs`; convert alias usage in supervision,
wallet-workflow and CLI fixtures instead of deleting their functional coverage.

Update actual consumers atomically: `examples/deployments/research_desk.toml`,
the agent-facing `providers/regimeshift.toml` description, `providers/CAVEATS.md`,
configuration, network-egress and wallet CLI guides. Remove stale promises for
`fetch_timeout_seconds`, which has no implementation in `src`. Review tool
instructions and discovery keywords when touching provider guidance.

### Registry and catalog caches

Remove `State._legacy_receipts` and `ignore_registration_receipts` in
`src/discovery/mod.rs`; strict deserialization then rejects obsolete snapshots.
Replace `legacy_registry_bookkeeping_does_not_block_restore_or_registration`,
which builds 10,001 tombstones and 10,001 obsolete receipts, with a small
current-format restore/register/tombstone test and unsupported-snapshot
preservation coverage. No current count limit justifies the old numerical fixture.
Keep registry version-zero initialization: it creates an empty database, not an
old-format upgrade.

No multi-format catalog-cache migration chain was found. `src/http_cache.rs`
uses one `discovery-v1.sqlite` database and versioned profile keys; older keys
already miss. Consider removing the residual `Metadata.local_override` Serde
default: missing older metadata can follow the existing best-effort miss path.
Review its diagnostic severity so an expected disposable miss is not a warning.
Do not build replacement cache migration/version machinery for this small removal.

Delete the historical missing-`request_url` assertion in
`src/catalog/operation_url_tests.rs` if no longer an input promise, but retain
`request_url: None` and ordinary base/path routing. Current serializers omit this
field when it is `None`, so missing-field deserialization and current roundtrips
must remain supported. Optional mappings, image limits
and headers are current defaults, not automatically obsolete serialized formats.
Retain cache conditional revalidation, age, identity separation, relay provenance
and warming tests. Consolidate the six test callers of the test-only
`discovery_relay::decode` wrapper into existing adapter/sequence coverage, then
delete the wrapper if it adds no distinct contract.

### Wallet formats and qualification readers

Remove `tests/treasury.rs::v6_sdk_snapshot_retains_addresses_and_scan_targets_through_v7_save`
and `sdk_v6_wallet.bin`. Preserve current-format encrypted save/reopen, address
continuity and scan-target persistence; add only missing assertions to existing
roundtrip tests. Two active `src/rotation/near.rs` tests consume
`sdk_v6_addresses.json`; replace them with a small public current-address fixture
before deleting that JSON. Keep refund receiver/quote tests intact.

Also audit `store/funding.rs::quote_amounts`, which accepts absent `swapType`
for pre-mode journals, and `near::Quote::{output_atomic,sizing_output_atomic}`,
which treat non-`EXACT_INPUT` quotes as output mode. Current request builders emit
an explicit mode, but accepted journals with missing mode can exist within the
same database version. Trace acceptance/history consumers before removing this
fallback. Preserve exact accepted terms unless the coherent storage boundary
actually excludes those inputs; a version check alone does not establish that.

The pinned Zingolib Git dependency and vendored Pepper serialization also contain
historical readers. Dropping an application support obligation does not require
forking Zingolib or rewriting dependency-owned codecs. Default scope leaves those
readers alone; any later dependency surgery needs its own concrete benefit,
retirement/provenance assessment and current serialized-state qualification.

Qualification has another compatibility layer: `src/qualification/lifecycle.rs`
contains the old promotion model beside current schema-2 reserve consumption;
`lifecycle_restart.rs` branches on old role/phase rules; `lifecycle_estimate.rs`
accepts unversioned balances and old inventory; `balance.rs` accepts historical
unversioned admission evidence beside current evidence. The current producer
incorrectly copies status JSON version 5 into lifecycle
output, although reserve-model consumers recognize lifecycle version 2. Emit
explicit lifecycle version 2 and test producer-to-consumer handoff in P2 alongside
its status version 6 change, before deleting old branches in P4. At that same
boundary, bootstrap readiness must consume current enabled inventory and
`inventory_ready`/`serving_ready`, including eligible READY inventory; it must not
retain an ACTIVE-only prerequisite from the old status model. Trace producers in
`src/rotation/store/qualification.rs` and consumers in
`tests/live/runner/execution_rotation.rs`, registry and integration-preparation
tests, then remove obsolete reader branches and old synthetic inputs together.
Do not assume the newest number is always the sole current format: exact chain
observations use balance version 2 while advisory admission estimates use version
3. Preserve both where currently produced, including their different authority.
The producer also exports unversioned
evidence for accepted journals with `valid_until` and missing `support`. Reject
that unsupported qualification export without rewriting journals or impeding
ordinary reconciliation; it must not be relabeled as current advisory evidence.

Trace the complete runner handoff, including historical admission/ownership
markers in `registry_application.rs` and the old budget/refund/anchored-wallet
model in `accounting.rs`, `accounting_baseline.rs` and `accounting_attribution.rs`.
Current payment intent marker 2 and ownership marker 1 are distinct from balance,
lifecycle and status formats. Update execution/bootstrap, lifecycle registry and
rotation-report consumers with their producers. Unsupported immutable evidence
must not qualify new work or release execution reservations.

Review the `legacy_single` prior-run settlement shortcut in
`qualification/funding/registry_state.rs` with original-case/per-attempt evidence
consumers. Remove only historical evidence acceptance, retaining canonical
correlation and refusal of duplicate submission. Old immutable report files remain
untouched; unsupported reports must not qualify new work. A missing historical
reader grants neither rerun nor spending authority. Keep current multi-attempt,
restart, drained-progress and separately authorized qualification coverage.

## Functional tests and diagnostic contracts

Use outcomes as the primary oracle: request counts and exact bytes, tool/schema
results, durable roles/claims, exit status, cancellation ownership and absence of
signing, funding, replay or credential disclosure. Remove complete human-message
inventories and formatter details when those outcomes already establish behavior.
Do not replace message strings with equally brittle snapshots of every event field.

Retain narrow diagnostics tests where diagnostics are the contract: stdout MCP/JSON
purity, stderr placement, secret redaction, correct terminal/intermediate severity,
failure scope, visible truncation and ambiguous-payment guidance. Use existing
structured fields or captured events for semantic assertions where practical;
avoid introducing production event infrastructure solely for testing. Small
representative semantic phrase checks remain appropriate for user guidance that
has no other observable representation. Do not delete all log tests indiscriminately.

| Candidate | Proposed treatment |
| --- | --- |
| `http_cache::tests::catalog_local_ttl_overrides_origin_freshness_and_logs_policy` | Retain TTL, disabled cache, origin override and relay-age assertions; remove logger/tempfile/child and routine DEBUG prose/field assertions. |
| `tests/startup.rs::cli_progress_stays_on_stderr_and_stdout_is_inventory_json` | Keep valid complete JSON, stderr progress and privacy boundary; drop the 14-plus-message/timing-field inventory and redundant CLI launch where possible. Consolidate parse-failure setup with structured failed-inspection coverage while keeping one executable failure boundary. |
| First three `tests/provider_status.rs` tests | Test advisory tags and quiet inspection primarily through functionality; retain inheritance/clear/replace validation, one narrow warning-deduplication assertion and one executable smoke test instead of approximately 11 children freezing warning prose. |
| `tests/funding.rs::ambiguous_submission_never_repeats_and_api_success_cannot_fund_wallet` | Retain journal, submission count and chain-credit assertions; remove three exact progress sentences. Keep representative privacy/severity separately or in that scenario. |
| `confirmation_wait_and_retrying_reconciliation_errors_do_not_warn` | Retain recovery/progress and severity contract; remove exact message wording. |
| `tests/rotation.rs::canonical_depletion_is_debug_without_an_sdk_balance_admission_gate` | Assert role/balance/claim changes and rollback on incomplete evidence first; keep only the distinct severity/privacy contract. |
| `src/rotation/base/logging_tests.rs` | Fold representative severity/redaction checks into functional fallback tests; remove duplicated INFO/DEBUG endpoint scenarios and routine success prose inventories. |
| `tests/credit_pricing.rs::unsupported_metadata_has_visible_fallback_and_warning` | Retain fallback/filtering and representative visible warning/privacy; drop formatter quoting/module-path and repeated diagnostic inventories. |
| `src/cover/rejection_tests.rs` and runtime diagnostic tests | Keep independent capability refusal, cleanup, no replay and scope/severity; remove full sentence matches and duplicate scenarios. |

There are 56 textual `run_in_child(` occurrences across 34 files, including helper
definitions/tests; that is not a count of removable subprocesses. The tracing
first-use interest race is real. Remove isolation only after its capture requirement
disappears; retain `tests/support/log_capture.rs` while callers need it. Combine
tests only when setup and contract ownership coincide, not merely to lower test
counts. Keep cases identifiable and failure-local; retain independent concurrency,
negative-side-effect and cancellation/restart coverage.

Provider warning deduplication currently lives inside `provider_status::warn()`;
retain its narrow diagnostic observation rather than adding artificial production
state to test it. `SlowPricing` remains a supported advisory tag despite historical
wording; pruning its prose test does not remove the metadata contract.

## Runtime opportunities and measurement

1. **Eliminate known duplicate full-check execution.** `check-compat` runs
   `sdk_compat`, `payments`, `server`, `crypto_vectors`; `check-default` immediately
   executes them again via `--all-targets` under the same configuration. Partition
   the combined `make check` selections while preserving early compatibility
   detection and the complete standalone `check-default` selection. Prefer a small
   metadata-derived harness partition or equivalent explicit composition with an
   exhaustiveness check over a hand-maintained omission-prone target list.
   `scripts/tests/test_makefile.py` should prove all default targets, including
   library, binaries, examples and future integrations, execute exactly once in
   the combined selection. Keep standalone stage behavior documented.
2. **Factor duplicate provenance work only if worthwhile.** Pepper verification
   runs in both static and compatibility stages. Preserve standalone verification
   and fail-fast ordering with simple composition; do not add persisted pass caches
   or a generic stage scheduler to save a small unmeasured cost.
3. **Remove obsolete fixture volume and needless processes first.** Historical SQL,
   2,048-row migration and 20,002-item registry tests, alias CLI runs and unnecessary
   log children are direct candidates above. Avoid shared mutable wallets, caches,
   registries or process-global subscribers as an optimization.
4. **Coordinate instead of sleeping for synchronization.** Inspect shutdown's
   11/6-second waits, treasury sync's 31-second checkpoint-lock case, Base's
   former-deadline progress case and gRPC's roughly 21-second streaming case.
   Some establish current progress-versus-stall contracts and are not dead tests.
   Shutdown's 11-second wait crosses former 10-second abandonment; its 6-second
   wait crosses the actual external rmcp EOF-drain allowance. The 31-second lock
   test crosses a current checkpoint cadence. Notifications alone do not prove
   survival beyond an external timer; retain that representative evidence unless
   equivalent coverage replaces it.
   Replace readiness-only waits with barriers/notifications; use paused time only
   for code whose actual clock/IO behavior supports it. Keep representative real
   HTTP/gRPC/SOCKS/process cases. Move boundary matrices to deterministic tests
   only when equivalent progress, sibling inactivity and no-replay assertions
   remain. Never reduce production timers, loosen test deadlines or skip slow
   tests to claim a speedup.
5. **Profile linking and expensive setup before restructuring.** There are 50
   top-level integration Rust targets. Related small targets may share a harness
   to reduce linking and macOS copy/sign startup, but measure first; avoid one
   enormous executable, altered `current_exe()` children or global serialization.
   Existing example/runner duplicate-harness prevention already works. Profile
   proving/lifecycle setup; share immutable public chain inputs or independently
   cloned templates only when wallet, journal and authority isolation is preserved.

Historical ignored aggregate evidence from `target/full-suite-20261029` concerns
older revisions `d914647…` through `b2dc9ef…`, not this baseline. It suggests where
to measure: default library about 145 seconds, integration preparation 57,
shutdown 44, treasury sync 35, network 22, Base failover 20 and proving 63.
The initial lifecycle run took about 356 seconds and failed one case before
focused continuations; it is not a clean timing baseline. Compatibility repeated
28 test executions in that run. These are prioritization hints, not current
counts or promised savings.

Before runtime changes, record current revision/source hashes, lockfile/toolchain,
features/profile, thread counts, host, warm/cold build and proving-parameter state,
selected test names/counts, child launches and stage/harness elapsed time under
ignored `target/`. Separate compile/link/setup costs from execution. Compare
scheduling on unchanged executables and compare before/after under matching
conditions; repeat enough to identify noise. Measure affected slow cases and the
final composed check, with identical retained contracts. Do not run coverage as a
separate prerequisite before every deletion; establish and reuse the baseline
below, then collect focused evidence for unresolved gaps and the final comparison.

## Fine-grained coverage analysis

Establish coverage before implementation pruning begins, alongside the runtime
baseline. Reuse a complete source-matched report only when its toolchain, features,
test selection and instrumentation match the proposed comparison. Coverage analysis
is required work in this plan, rather than an optional follow-up. It does not
replace functional assertions, independent review or ordinary compiler checks.

Use **condition instrumentation only** for this evaluation:
`-C instrument-coverage -Z coverage-options=condition`. Its exports already
include line, region, function and branch information. Do not collect separate
block-only, stable, branch-only or MC/DC application reports. Existing coverage
entrypoints are not an instruction to execute every mode.

The completed socket-free probe used `rustc 1.101.0-nightly`, compiler revision
`f9ceb4a4d45a471eed7bd8c40dddb7c38e749e21` (commit date 2026-10-31), on
`aarch64-apple-darwin`, with its matching LLVM 23.1.4 `llvm-cov` and
`llvm-profdata`. It compiled a standalone program and executed all four input
combinations for two booleans, including `let conjunction = a && b;` and an
`if a || b` decision, then merged profiles and exported JSON and annotated text.

| Probe mode | Verified result; not additional application evaluation modes |
| --- | --- |
| Standard instrumentation | Compilation, execution and export succeeded; line, region and function data present. |
| Branch | Compilation, execution and export succeeded; 3 locations / 6 outcomes. |
| Condition | Compilation, execution and export succeeded; 4 locations / 8 outcomes, all exercised. The extra location records the final operand of the assigned conjunction. |
| MC/DC | Compiler rejected `coverage-options=mcdc`; accepted levels are block, branch and condition. No MC/DC collection is planned. |

Probe artifacts remain ignored under `target/coverage-support-probe/`; the source,
per-mode compiler logs, profiles, exports and `summary.json` are local evidence,
not application coverage results. Homebrew Rust 1.99.1 remains the ordinary compiler.
The original probe used rustc/LLVM directly. P0's Cargo wrapper probe confirmed
four condition locations/eight outcomes, but the original compiler then failed
application compilation before tests with
[Rust regression #165263](https://github.com/rust-lang/rust/issues/165263).
The user explicitly authorized replacing only the analysis pin with
`nightly-2026-10-31`, revision
`92b5b8c87c61b20adc9001b41baf587634a74734` (compiler date 2026-10-29).
Its installed manifest verifies archive date 2026-10-31 and matching LLVM 23.1.4.
The replacement condition-only wrapper probe retained the same four locations /
eight observed outcomes and the recursive async-drop reproducer compiled.
Cargo-llvm-cov remains 0.9.1. These are narrow qualification probes, not
application baseline results or a new validation dimension. Production pins stay
unchanged; do not repeat the completed coverage-mode comparison.

Pin and verify that analysis compiler revision and its matching LLVM tools.
Keep the existing provenance, nonempty selections, child-profile inventory and
failure-safe publication. Keep analysis artifacts separate from ordinary build
artifacts. If condition collection breaks, resolve or report that specific blocker;
do not silently substitute another mode or start testing a toolchain matrix.

Condition instrumentation includes branches and additional boolean expressions,
as described in [Rust's coverage options](https://doc.rust-lang.org/unstable-book/compiler-flags/coverage-options.html).
It does not establish MC/DC independence, exhaustive paths or every possible
boolean construct. Inspect locations and missing true/false outcomes, alongside
line/region/function data from the same run; use focused functional truth-table
assertions where a compound authority decision needs stronger evidence.

Collect one complete default-suite condition baseline and one matching final
condition run. Use one ordinary test profile for both; no debug/release comparison.
For financial paths absent from default features, add only the focused existing
synthetic proving/lifecycle/codec selections needed to assess those paths, in their
existing `zcash-testutils` configuration and the same condition mode/profile.
Keep that supplement's scope explicit and compare it once before/after as needed;
do not rerun all default tests under every feature combination. The existing
proving coverage command selects only one test and must not be presented as
coverage of all feature suites. No coverage command authorizes Docker, real Tor or
funded transactions. Follow usual validation ordering before broad collection and
reuse applicable completed checks instead of rerunning them for each report.

Review file/function and branch/condition locations with complete call chains,
not just repository percentages. Produce a concise gap disposition: obsolete code
to remove, retained reachable behavior needing a focused functional test, or
unobservable/tooling-limited coverage with its reason. Correlate high-risk gaps
with CogC/CycC and the existing test map. Keep the analysis denominator explicit:
application code, operational runner/tools, inline helpers, dependencies and
excluded tests have different scopes. Where the selected harnesses exercise
runner/tools, export a separate view from the same collected profiles for P4/P7.
The wrapper's default filters hide runner paths in test folders; use the
condition-only `all-sources.json` export with those defaults disabled, selecting
operational paths explicitly while retaining `full.json` and the application
exclusions unchanged;
otherwise mark them unobserved or out of scope. Do not add another suite run merely
to populate that view or silently broaden the application denominator. Do not add
unreachable input cases or trivial tests merely to raise percentages.

For candidate heavy-test consolidation, compare focused profiles only when the
existing aggregate evidence cannot establish the retained cases' contribution.
Equivalent hit locations do not prove redundant assertions: two tests may cover
the same branches while proving different bytes, ordering or forbidden side
effects. Avoid a suite-wide per-test coverage matrix or repeatedly executing every
test merely to find a mathematically minimal suite.

At G3 compare the final retained suite with the baseline using the same condition
toolchain, instrumentation, scoped feature selections, test profile and exclusion rules. Match unchanged
source and explicitly map changed functions; separate deleted code/denominator
changes from newly lost coverage of retained behavior. Disposition lost outcomes
and new gaps individually, restoring necessary assertions before completion.
Keep JSON/HTML, raw profiles, exact source/tool hashes, selections, counts and
gap/denominator comparisons under ignored `target/`; keep durable conclusions in
the owning guides/review. No blanket percentage gate, MC/DC claim or safety claim
follows from passing coverage collection. Instrumented runtime is not the ordinary
test-performance benchmark.

## Permanent rules against heavyweight testing patterns

Add a maintained testing-efficiency policy to `docs/testing.md` in P0, linked from
AGENTS.md's commands/test guidance, with supporting coverage/time techniques in
`docs/development.md`, `tests/COVERAGE.md` and `scripts/README.md`. These rules must
survive plan removal and apply to new tests and subsequent agent workflows:

- **Do not build a validation cross product.** Keep ordinary validation on the
  existing pinned stable compiler and maintained stage selections. This evaluation
  adds one isolated condition-coverage workflow, not stable/beta/nightly runs of
  every test, alternate Rust compatibility cohorts, arbitrary feature combinations,
  debug/release suites or every coverage mode. Existing named feature stages prove
  distinct contracts; do not multiply each stage across those dimensions. Any
  future matrix expansion needs an independently demonstrated contract and explicit
  user authorization; this plan does not grant it. Nightly remains analysis-only,
  outside normal `make test` / `make check` dependencies and production pins.
- **Do not rerun passed tests without a reason.** Reuse valid source/feature/profile
  evidence and resume unfinished stages. Reruns need changed relevant inputs, an
  observed failure/flakiness investigation, or a stated measurement/coverage goal.
  The scoped condition evaluation has a distinct purpose; merely repeating ordinary
  execution in multiple stages does not. Do not create success caches
  that conceal stale evidence or skip explicitly requested standalone commands.
- **Consolidate expensive setup.** Reuse small shared fixture builders and immutable
  synthetic inputs; combine scenarios that genuinely share setup and ownership.
  Avoid duplicating heavyweight wallets, chains, subprocesses or servers to assert
  another property of the same scenario. Retain fresh mutable stores, keys and
  authority per isolated case; never introduce shared mutable financial state or
  global serialization as the shortcut. Require a named distinct boundary for a
  separate expensive fixture, not a new test for every historical bug name.
- **Assert functionality first.** Log substrings must not substitute for observable
  results, state transitions, exact requests, counts or absence of side effects.
  Preserve narrow privacy, severity, wire-output and user-guidance contracts as
  described above; avoid prose snapshots and repeated end-to-end log scenarios.
- **Use deterministic time and synchronization by default.** Use paused/mock time
  for owned timer logic and barriers/notifications for readiness and ordering.
  Do not wait real seconds merely to let a background task probably run. A real
  wait needs a named external-clock, process or transport invariant that mocked
  time cannot establish, plus the smallest representative integration case.
  Mocked Tokio time does not control other processes, OS IO or independent clocks;
  preserve the required external-timer crossing instead of claiming a mock proves it.
  Do not add production timeouts or change cadence to accelerate tests.
- **Choose the cheapest layer that proves the contract.** Put input matrices and
  pure decisions in unit/contract tests; retain representative real CLI/network/SDK
  tests for actual integration boundaries. Do not prove every parser variant by
  launching a process or repeating proving/chain setup. Keep current recovery,
  concurrency and no-replay cases that require their integration layer.

Reviewers must ask what each heavyweight scenario uniquely proves, whether existing
setup can carry the assertion, and why real time or another execution is necessary.
Document concrete exceptions next to the owning test/helper and in review evidence.
Use focused orchestration tests for duplicate selection and ordinary review for
semantic duplication; avoid brittle source-text bans, arbitrary duration caps or a
new test-policy framework.

## Ordered commits, review gates and acceptance

Before each substantive change, an independent read-only design reviewer must
challenge necessity, scope, current reachability and simpler deletions. Record the
baseline and expected retained contracts; resolve material findings before building
on the decision. Implement each coherent removal with its replacement safety
coverage and owning documentation, rather than deleting behavior first and restoring
recovery later.

| ID | Coherent implementation and documentation | Acceptance evidence |
| --- | --- | --- |
| P0 | Establish runtime and condition-only coverage baselines; reuse the verified compiler/LLVM probe and qualify Cargo wrapper integration with exact analysis tools. Make minimal workflow extensions and add permanent efficiency/no-matrix rules with AGENTS links in testing/development/coverage/tooling guides. | One default condition baseline plus only necessary focused financial supplements, one test profile, verified condition exports, explicit unsupported MC/DC, scoped gaps and preserved publication. No additional coverage modes, compiler/feature/profile cross product or nightly dependency in ordinary checks. Independent design/tooling review before broad collection; no application pruning before baseline establishment. |
| P1 | Canonical config/CLI only; simplify strict parsing; update examples, provider descriptions/caveats, configuration/network/wallet guides together. | Focused config/pricing/managed config/CLI layout and precedence tests, canonical authored examples, unchanged timeout functionality and no unexpected IO; reject obsolete input generically. |
| P2 | Current database contract, canonical initialization and historical reporting/readiness/refund removal, with test-only converter/SQL fixture retirement and compact replacement safety tests. Update wallet CLI/rotation/Zingo/testing, qualification output/readiness and state-fixture docs. Keep status6, lifecycle2 and database23 handoffs explicit, including READY inventory bootstrap. | Current fresh/reopen/status/backup/inspection paths; producer-to-current-consumer reserve/readiness assertions; unsupported old/future refusal before mutation; current corruption behavior; accepted bytes/identity/liability and no-replay preservation. Resolve schema-23 shape issue before deleting startup DDL. |
| P3 | Remove obsolete wallet upgrade fixture/test, replace active address-fixture consumers and preserve current roundtrip coverage. Update Zingo/testing/state-fixture docs. | Current encrypted SDK/address/scan-target save/reopen and Near refund receiver/quote cases; no removed fixture references. |
| P4 | Remove historical qualification report branches and prior-run shortcut where obsolete, updating producers, live runner fixtures/reference and qualification claims together. | Current schema-2 lifecycle and current balance-2/3 inputs, reserve consumption/vacant replacement, exact checkpoint/drained continuation, per-attempt prior-run correlation; unsupported report cannot grant authority. |
| P5 | Remove registry receipt compatibility and redundant cache/default/decoder regressions; update agent-source/config/testing guides as affected. | Current registration/tombstone persistence, unsupported snapshot preservation, cache miss/refetch, request routing, conditional/identity/relay-age and adapter decision contracts. |
| P6 | Consolidate diagnostic coverage around functionality and distinct output/severity/privacy boundaries; remove unnecessary captures/children. Update testing guidance and affected startup/cover/network diagnostic docs. | Named functional tests and representative diagnostic contracts execute with nonzero counts; stdout purity, secrecy, severity/scope and ambiguity guidance remain. No tracing race or global serialization introduced. |
| P7 | Partition combined full-check execution without omissions; optionally factor measured duplicate provenance work. Update Makefile tooling tests, scripts README and testing stage table. | Tooling tests for exact selection/order/standalone stages, newly added targets, failures and continuation; unchanged feature/profile coverage; measured duplicate execution removed. |
| P8 | Apply only justified synchronization, fixture-setup or harness-layout improvements from measurements. Update affected testing/lifecycle/network guides. | Comparable timings plus current progress/stall, cancellation/restart, isolation and forbidden-side-effect cases; feature builds remain sequential. Omit speculative restructuring if measurements do not justify it. |
| P9 | Finish permanent guidance/link handoff and remove this completed plan. | Exact cleanup SHA review and final combined branch review after removal; no broken references, completed-plan archive or lost financial evidence. |

**G1 — Storage/format architecture gate, after P2.** Review P0–P2 and every fixup,
not just changed lines. Follow create/open/status/identity/backup and treasury
consumers end to end. Resolve schema-23 initializer shape, read-only rejection,
output versioning and status6/lifecycle2/bootstrap handoffs, corruption/reorg
recovery, current history ownership and exact accepted-byte preservation. Exit requires no unresolved material finding and
current storage safety tests actually executed.

**G2 — Financial and catalog handoff gate, after P5.** Review P0–P5 and all fixups
with wallet serialization/refund, qualification prior-run, source ownership,
payment, cache and network call chains. Verify no wire/dependency compatibility
or active current defaults were deleted by name, and unsupported evidence cannot
silently qualify work. Confirm fixture consumers and permanent documentation.
Material late findings reopen G1 where they affect its conclusions.

**G3 — Integration gate, after P8 and again after P9.** Review every plan commit
and fixup with all retained consumers, test selection, startup/shutdown and
financial/network boundaries. Examine why fixups were needed and whether repeated
assumptions require a wider audit, regression coverage or simpler design. Verify
test consolidation preserves distinct failure boundaries and measured improvements
do not depend on missing tests. Review final condition-only coverage against P0,
disposition lost outcomes of retained behavior, and verify permanent efficiency
rules, absence of a new validation matrix and AGENTS handoff. Per-commit approvals
do not replace this gate.

Every exact committed SHA must receive independent review by someone other than
its author. Address findings with `git commit --fixup=<introducing-SHA>`, independently
review each exact fixup, and preserve unsquashed history through final integration
review. Supply reviewers intent, scope, baseline, retained contracts, actual
validation evidence and known limits; ask them to challenge unnecessary enforcement
and verify claims. Review agents remain read-only unless explicitly assigned edits.

For substantive Rust changes and each architecture gate, compare CogC/CycC with the
agreed baseline using the pinned analyzer. Inspect complete report/source hashes,
changed call chains and unchanged financial/permission/network hotspots, including
warnings caused by moved code. Evaluate deletion of responsibilities and simpler
representations before retaining a hotspot. Do not extract helpers merely to lower
a score. Use fixup-local comparisons where needed and disposition relevant
warnings by named function in review evidence. Generated reports stay in `target/`.

## Validation and documentation handoff

Run affected cheap contracts first. Once sources stabilize, follow the maintained
ordering: `make check-static`, independent complexity disposition, production
compiler/all-target warning gate (`make check-clippy`) and `make check-compat`, then
the complete default, proving, lifecycle and codec stages. For P7, validate both
standalone stages and the composed partition; a manually filtered run does not
establish full coverage. Use pinned Make entrypoints, inspect diagnostics as well
as exit status, and verify filtered selections execute nonzero tests.

Those are the existing ordinary validation stages, not dimensions of a matrix.
Do not repeat them on nightly or in both debug and release. The separate
condition-only baseline/final evaluation uses its one analysis toolchain and test
profile, with only the scoped financial supplements described above. It does not
become a prerequisite for normal testing or repeat at every commit/review gate.

Coordinate all builds and freeze sources/executables during validation. Choose
socket-capable execution up front for socket suites; retain ordinary sandbox use
for socket-free selections. Use temporary state and public unfunded keys only.
Keep feature configurations sequential and preserve real SDK proving/lifecycle
coverage. Docker consensus, live Tor and funded settlement are separate optional
qualification, not prerequisites granted by this plan.

After localized corrections, revalidate affected consumers and resume unfinished
named stages; carry forward only evidence whose source/feature/profile inputs
remain applicable. Record test counts, commands, diagnostics and timing provenance
under ignored output. Do not claim a successful full check if required stages are
unaccounted for.

Maintain enduring current-format rules in configuration, wallet CLI/rotation,
agent-source, Zingo and testing guides, not this plan. Update `docs/development.md`
for any testing/diagnostic tooling policy and `scripts/README.md` for changed
entrypoints. Existing AGENTS links already reach those guides; update the relevant
financial, provider, commands/test and read-first references if their destinations
or contracts change. Verify these links and remove stale historical coverage
promises as each implementation commit lands. At P9 remove the plan and its
documentation-index link only after the handoff is complete; preserve concrete
unfinished work if any remains instead of declaring it implemented.
