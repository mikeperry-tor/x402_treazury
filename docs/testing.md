# Testing and qualification

Use offline regressions to establish behavior; use optional consensus and live
runs for the integration boundaries they actually exercise. A coverage percentage
or successful HTTP response does not substitute for financial, permission or
network-isolation assertions.

When changing tests, preserve full schema/description contracts and meaningful
assertions; do not trim documentation or loosen expectations to obtain a pass.
Discovery tests inject public-URL documents and localhost fixtures only under
`cfg(test)`. Never add a production loopback bypass for fixture convenience.

## Portable connectivity controls and platform scope

Focused `network::qualification_probe_tests` classify permission denials using
production reporting and native OS-code mappings, including exact refusal
classification, then exercise real loopback TCP/UDP positive controls, a held
non-listening TCP endpoint (native refusal or timeout), received-request reply
timeout and destination preflight. These unfunded controls require no firewall
changes and do not establish actual OS confinement. The runner EgressProbe command
consumes this classifier; legacy standalone live-Tor socket controls are a separate
implementation and do not validate it. Unix-only builder mutation remains scoped
to Unix without changing other platform behavior.

The engineering scope is Linux and macOS. Windows implementation/native acceptance
is owned by the [deferred release plan](plans/deferred/release_followups.md), with
no Windows support claim or rerun requirement for the selected cleanup. Preserve
historical Git/private evidence; partial native builds or API results do not
qualify privacy, recovery or power-loss behavior.

The shared Linux/macOS private-storage boundary has native fixtures for atomic
creation under inherited ACLs, descriptor reopen rejection, SQLite sidecars and
private preflight cleanup. A held journal descriptor is unlinked before sidecar
inspection validation to reproduce concurrent SQLite removal deterministically;
ordinary file validation still rejects it, and sidecar inspection retains
symlink, hardlink, mode and macOS ACL rejection. Source review alone does not
qualify these native contracts: execute the host-specific controls, including a
denied-read control.
The [storage assessment](plans/portability_fixes.md) retains the separate Unix
namespace-authority question. Normal Unix financial, lifecycle and compiler checks
remain required. Record exact source/executable/features, commands, diagnostics and
nonzero test counts; retain valid evidence and revalidate affected consumers.
No live-funded, consensus or Tor qualification follows from these controls.

HTTP cache race fixtures run in exact isolated children because the optional
private scope observer is installed once per process. Two synchronous constructors
enter the same Tokio runtime and are held by that actual pre-I/O observer; the
coordinator checks the mutex before starting a competitor, releases callbacks and
joins workers before assertions. Constructor errors/panics also notify completion.
Tests preserve one canonical winner, truthful unused-scope observations, failure
propagation despite a concurrent winner, zero pre-dispatch SOCKS requests and the
winning scope's actual fixture disclosure. Existing promotion/key/eviction tests
retain their separate ownership contracts; these fixtures add no production
coalescing, timeout or scope-history framework.

Catalog acceptance fixtures exercise ordinary inspection, serving and deferred
startup with fresh acquisition owners: semantically rejected JSON is not cached,
restart can reach a recovered origin, and rejected downloads or HTTP 304 content
cannot replace or renew the previous snapshot. Optional pricing format rejection
also retains the accepted cache. Concurrent alias refresh fixtures hold actual
acquisitions and publish each source separately, preserving admission while the
last accepted shared cache generation wins and obsolete hints remain retired.
Deferred-constraint commit failures cover family replacement, unchanged metadata
renewal, revision deletion and oversized-entry deletion: rollback retains durable
rows, live observation identity and the original expiry.
Pricing retirement fixtures hold an actual blocking writer before its final commit
guard, retire the source, and verify both cache bytes and current observations
survive. A delayed retired acquisition also cannot restore old metadata across
restart. Prepared enrichment retains tool ownership before publication and does
not poll its loader if publication is discarded.
Dynamic refresh fixtures retain proposed tools across removal, revision replacement,
snapshot rejection and persistence refusal; none starts optional pricing I/O.
Successful replacement publishes before optional pricing completes.
Registration snapshot/save refusal and restore-conflict fixtures retain proposed
tools while verifying zero optional pricing requests. Successful restore publishes
before enrichment and retains the saved acquisition expiry.

Catalog refresh publication regressions observe the final visibility boundary for
static publishers with and without a dynamic registry, and for registered-source
refresh. Failed validation preserves old admission; successful replacement retires
old handles before exposure while an already submitted signed response can finish.
The two-endpoint managed fixture holds a bootstrap-signed response during coalesced
catalog refresh, injects credit into preallocated destination reserves, and exercises
destination handoff and READY-member promotion. Exact requests, payer/nonce journals
and unchanged funding-job/address inventories establish no replay or refresh-induced
allocation. Synthetic credit does not qualify treasury funding or live settlement.
Fixture cleanup releases catalog snapshots before joining the Store worker.

## Provider regressions

Shared provider-update capture/comparison/candidate regressions live in the
`provider_updates` development example and use synthetic inputs. Run
`make test CARGO_ARGS='--example provider_updates'`; see the
[maintenance runbook](provider-updates.md). They establish offline evidence mechanics, not live vendor
completeness, billing interpretation or paid compatibility. Sweep outputs remain
ignored under `target/provider-updates/`, separate from request fixtures and full
pinned runtime catalogs.


Provider-specific regression storage should reproduce a named quirk, historical
failure or authored catalog transformation. Use the smallest faithful input and
assert the affected request schema, routing, filtering or output behavior. Generic
parsing, composition and selection belong in shared synthetic tests. Cheap offline
TOML validation should cover every bundled definition without freezing all settings
or the current vendor inventory. Ordinary vendor prose does not need a snapshot.

Generated tool/settings snapshots are not required for provider additions.
The ignored startup benchmark uses synthetic workloads independently of provider
fixtures. Inspect every consumer before pruning inputs,
including production local catalogs, discovery, media, payments and qualification.
Preserve financial evidence. Provider existence neither requires paid qualification
nor establishes support or current upstream health. Qualification must explicitly
identify its selected requests, caps, authority and unobserved outcomes.
`total_test_budget_usdc` bounds the running sum of potentially chargeable API
authorizations, not prefunded case reservations or wallet funding. Concurrent
admissions check and record that total atomically before disclosure, including
concurrent admissions from distinct runs sharing a registry. Reopening retains
accepted charges and rolls back refused admissions; uncertain and failed attempts
remain charged across restart. Per-case limits bound each case,
and cumulative registry authority spans runs. Admission derives all three totals
from one complete strict JSON history pass inside its existing write transaction;
each original payment amount retains its u64 budget parse, and same-case history
also retains its original U256 parse. Converting or normalizing numeric text must
not widen accepted evidence. Scoped usage reads retain their own SQL scope rather
than acquiring unrelated-run validation requirements. Budget refusal emits WARN and a
clear test error. A PaidClient regression covers run and registry budget refusals
before signed dispatch: payment journals and admission evidence remain absent,
balance estimates and reserve ownership remain unchanged, and sufficient authority
allows the same case to proceed. Funding has no ZEC spending budget; reviewed
allocation counts, reserve sizes, actual treasury spendability and SDK transaction
validity apply.

Catalog media regressions preserve precedence, parameter case and first-duplicate
lookup semantics, plus usable wildcard fallback and exact selected Content-Type.
Repeated shared-body names, including invalid/unsupported media, wildcard
synthesis, candidate/range and nested parameter amplification must exhaust the existing
conversion budget deterministically while the physical document still fits;
wall-clock thresholds and a separate media quota are unnecessary. A long flat
reference chain must reach the depth bound before growing the projection stack;
small exact-budget tests charge repeated pointer lookups once each and reject
excess work before copying or allocating it.
Seeded-budget tests also cover repeated OpenAPI path/metadata copies, duplicated
pricing fields, parameter issue metadata and fallback description copies without
large fixtures or charging moved schema contents twice. An isolated diagnostic
regression catches depth/node/byte rejections and arithmetic overflow errors and
requires a sanitized WARN for each, even when the caller handles the error.
Used dynamic/recursive schema references are covered in body, nested property,
parameter/header and referenced-target positions. Partial builders retain healthy
siblings; strict builders reject omissions. Opaque examples/defaults/unused
definitions and OpenAPI3.0 ignored reference siblings retain their own semantics.
Scalar parameter compatibility tests cover direct non-scalar array-item/null-only
omissions, representable scalar/tuple alternatives, ignored protocol declarations
and managed Idempotency-Key diagnostics. Static checks stay limited to explicit
type domains; runtime codecs retain their actual-value guards.

## Dynamic registry publication

Registry efficiency fixtures observe actual import builds under isolated child
execution, verify reuse across unrelated additions/static replacement, and rebuild
only invalidated candidate/document/name inputs. Small synthetic schemas retain
POST preference, listener tag/name filtering, C2 omission reasons, compatible help
ownership and instruction replacement; no provider inventory snapshots are needed.

Publication fixtures hold an actual accepted commit at a deterministic test-only
boundary, remove its completed optional fetch-cache entry, and start a duplicate
add at the actual missing-registration recheck. Known registration and source-filtered
search readers must continue on the prior complete snapshot while the duplicate
waits before acquisition. Releasing the commit must produce identical results
without a second document request. Tests release held callbacks and join owned
workers before reporting failure. Existing cancellation, persistence refusal and
crash/restart controls remain necessary; a fixture watchdog establishes no runtime
deadline, freshness or spending authority. Snapshot source metadata, tools,
instructions, omission diagnostics and cursor generation must switch together.

## Testing efficiency

Keep ordinary validation on the pinned production compiler and maintained named
stages. Do not build a cross product of compilers, feature combinations, profiles
or coverage modes. A future matrix expansion needs an independently demonstrated
contract and explicit user authorization. A condition comparison uses one isolated
analysis toolchain and test profile, with a default baseline/final pair and only
focused financial supplements. Run it for a stated coverage comparison, separately
from ordinary validation and per-commit reviews. Nightly analysis is outside
`make test` and `make check`; it never changes production pins.

Reuse passed evidence while its source, feature, profile and relevant inputs remain
valid; resume unfinished stages after focused fixes. Reruns need changed inputs,
an observed failure/flakiness investigation, or an explicit measurement/coverage
goal. Do not add success caches or skip explicitly requested standalone commands.

Consolidate expensive setup when scenarios share contract ownership. Prefer small
immutable synthetic inputs and shared fixture builders. Keep fresh mutable stores,
keys and authority for isolated cases; shared mutable financial state and global
serialization are not shortcuts. Each separate heavyweight wallet, chain, server
or subprocess fixture needs a named distinct boundary. Review what each scenario
uniquely proves and whether existing setup can carry its assertions.

Assert functionality first: results, exact requests, persisted transitions, counts
and forbidden side effects. Preserve narrow privacy, severity, wire-output and
user-guidance contracts, without full prose inventories or duplicate end-to-end
logging scenarios. Put pure decisions and parser variants in cheap unit/contract
tests; retain representative CLI/network/SDK boundaries and the recovery,
concurrency and no-replay cases that need integration.

Use paused/mock time for owned timer logic and barriers/notifications for ordering.
Do not wait real seconds merely to let background work probably run. A real wait
needs a named process, transport or external-clock invariant that mocked time cannot
prove, with the smallest representative integration case. Tokio time does not
control other processes, OS IO or independent clocks. Never change production
cadence or add timeouts to accelerate tests. Document exceptions beside the owning
test/helper and in review evidence. Use focused orchestration tests for selection
and ordinary review for semantic duplication, rather than source-text bans,
arbitrary duration caps or a new policy framework.

Optimized arithmetic changes require relevant payment/treasury tests with
`make test CARGO_ARGS='--release --test payments'` and focused release library
coverage. Normal debug tests do not exercise the release profile. Release enables
overflow checks for application and dependencies without changing explicit
checked/wrapping/saturating arithmetic or panic ownership.

## Selecting validation by change

Choose checks from the affected behavior and its consumers before starting Cargo.
Use the smallest selection that establishes the changed contract, plus any checks
explicitly requested by the user or required by an applicable qualification.
Independent review still applies; it does not require a full build for every edit.

| Change | Applicable validation |
| --- | --- |
| Prose or ordinary comments only | Review accuracy, links and diff formatting; no Cargo build, Clippy or runtime suites. Executable examples, doctests and compiler-consumed directives follow their affected contracts. |
| Configuration, provider TOML or schema data | Parse the edited inputs and check affected catalog, routing or request contracts. Use an existing executable only when the command reads the edited files. Bundled providers are embedded: tests or inspection of embedded content require a matching compilation, even without a new deliverable. No blanket production compiler/all-target Clippy gate for data-only changes. |
| Tests or fixtures only | Compile and run the affected test targets and consumers; lint changed Rust test targets. Broaden only when shared helpers or contracts affect other suites. |
| Runtime Rust | Run the production compiler/all-target Clippy warning gate (`make check-clippy`) and focused behavior/consumer tests. Select compatibility and broader runtime stages for affected boundaries. |
| Dependencies, toolchain, build rules or broad refactors | Check affected source resolution, production/test compilation and compatibility; select broader stages according to the affected call chains. Run relevant source verifiers under the [vendor verification policy](../vendor/README.md#source-verification-boundaries). |
| Financial authority, payment/recovery, network isolation, protocol or concurrency behavior | Include the relevant boundary, failure, recovery and forbidden-side-effect tests regardless of the changed file extension. |

Rows can overlap. A configuration change that alters payment authority needs its
financial contract checks; a spelling correction does not need a runtime suite.
Use existing focused coverage where sufficient, rather than adding tests that
merely restate a flag or document. Filtered tests must execute the intended cases,
not pass with zero tests. Name the scope and limits of the resulting evidence.

Full `make check` and explicitly requested named stages retain their fixed
selections. Do not narrow their recipes or add automatic skip caches to implement
this policy. Select focused commands instead of a broad stage when only a subset
is needed. In particular, `check-static` includes both vendor verifiers and
`check-compat` includes the Pepper verifier; requesting either still runs them.

Dependency source verification is not a per-build requirement. Carry forward
unchanged-input evidence under the vendor policy; ordinary application, data or
documentation edits do not by themselves require reconstructing vendored sources.
[Reproducibility qualification](reproducible-builds.md) is separate: run its two
clean builds for release qualification, changes to the reproducible-build contract,
or an explicit request. Ordinary development does not require it, including when
`make build` uses the release optimization profile. Neither `make build` nor
`make check` invokes that qualification runner. Keep ordinary toolchain/lockfile
checks and embedded build identity intact.

Batch compatible test targets in one invocation and keep feature/profile choices
stable. Freeze the implementation snapshot, and commit it before expensive
validation when practical. Git metadata is a build-identity input, so commits
between checks can trigger compilation even when behavior is unchanged. Reuse
valid evidence across prose-only commits with the original source and executable
revision recorded; never describe an older binary as embedding the newer commit.
Do not rebuild solely to relabel evidence. Different compiler, Clippy, test and
feature/profile selections can still require distinct Cargo artifacts.

Build an updated executable when needed for use or to validate embedded inputs;
do not append a release build to every validation run. Prefer the incremental
`make build-dev` for iterative executable development unless the release profile
is required by the affected contract or the requested artifact.

## Default development checks

The application always includes Zcash. The [Makefile](../Makefile) verifies pinned Cargo
and supplies Cargo-managed protoc; do not append a second `--locked` flag.
See the [entrypoint reference](../scripts/README.md) for focused test arguments.

`make check` runs the fixed stages below in order, using `check-default-remaining`
after compatibility, and stops at the first failing stage, including with `make -k`.
Every stage can also run independently; it does not implicitly rerun earlier
stages. `CARGO_ARGS` and `TEST_ARGS` customize
`make test` / `make clippy`, but never narrow a `check-*` stage.

| Stage | Selection |
| --- | --- |
| `check-static` | Formatting, tooling regressions, analyzer tests and CogC/CycC report, vendor provenance |
| `check-clippy` | Production library/binary compiler check, then all-target Clippy with `zcash-regtest`; emitted compiler warnings from both application and dependencies fail the stage |
| `check-compat` | SDK wallet, payment, MCP and cryptographic integration tests using the application lockfile |
| `check-default-remaining` | All other eligible default root targets, derived from Cargo metadata; the four compatibility integrations already ran |
| `check-default` (standalone) | Complete `--all-targets` selection, including compatibility integrations and examples; not repeated inside `make check` |
| `check-proving` | `zcash-testutils` treasury proving tests |
| `check-lifecycle` | `zcash-testutils` synthetic-chain treasury lifecycle |
| `check-codec` | `zcash-testutils` address-codec isolation regression |

Default application harnesses use eight libtest threads. `TEST_THREADS=1` selects
serial execution for `make test`, `make check-default` and
`make check-default-remaining`; other positive counts can be measured the same
way. `TEST_ARGS` still replaces the whole harness argument
list for focused `make test` runs, while `check-default` retains its complete target
selection. The lifecycle stage uses eight threads; proving and codec stages keep
one thread. Feature builds remain sequential.

The combined compatibility/remainder partition uses the same root lockfile,
default features, test profile and runner. Cargo metadata supplies current targets
and resolved features; root test/bench declarations supply the bench flag omitted
by metadata. Library, binary and example selection remains with Cargo; eligible
integrations and benches are selected by name. A changed workspace default-member
selection or inconsistent compatibility targets fails explicitly. Standalone
`check-default` always requests the complete suite; separately requesting it after
`check-compat` intentionally repeats those integrations. No stage caches success.

`tests/integration_preparation.rs` owns the shared live-runner unit tests and its
real-executable integration cases. The `live_integration` example has an empty
test harness under `--all-targets`; its ordinary implementation remains covered
by all-target Clippy and explicit runner builds. Shared tests are not repeated in
the example harness.

Compare concurrency on an unchanged executable with the same test counts and
assertions, then validate the complete default suite. Keep timing contracts intact;
increasing their deadlines would hide contention or isolation failures rather than
establish a faster test configuration. A measured default is not an optimum for
every host, so use `TEST_THREADS` when local measurements justify another count.

Diagnostic fixtures whose callsites are also reached by subscriberless siblings
use the [log-capture guard](../tests/support/log_capture.rs) to run in an exact child
process. A sibling's first use can otherwise cache disabled tracing interest
without the fixture's scoped subscriber. This also applies to synchronous
`tracing::subscriber::with_default` captures. All severity, count, privacy and financial
assertions stay in that child; the parent verifies successful execution of one
test. Other fixtures remain parallel; this is not a reason to serialize an entire
harness.

Production subprocess fixtures cover CLI JSON rendering, remote HTTPS catalog-size
limits and command-line overrides, using explicit fixture CA configuration and real
TLS requests. Local specification files are not downloads and do not acquire the
remote `max_spec_bytes` contract. These fixtures add no production trust environment
variable and modify no OS trust store.

Exercise ordinary CLI rendering once per distinct output contract. Use direct
configuration/catalog APIs for advisory metadata and parser combinations; retain
actual serving-source selection for warning scope. Functional cache lifetime and
cover capability checks need no prose capture. Narrow diagnostic fixtures still
verify severity, privacy, failure scope and user guidance alongside real results,
requests and durable state. Do not replace removed message inventories with full
field inventories.

Test children that clear their environment preserve only the inherited
`LLVM_PROFILE_FILE` when present, so isolated condition analysis can observe their
work. This does not broaden operator-controlled live child environments or permit
copying ambient credentials and proxies. Keep exact child-test execution checks.

### Compiler diagnostics and review

`check-clippy` first checks the production library and binaries without test
features or development targets, then runs Clippy over all targets with
`zcash-regtest`, including binary test targets. These selections are separate:
test/example feature unification can hide unused production code. Both commands
stream Cargo JSON through `scripts/check_warnings.py`, which displays compiler
diagnostics and fails on emitted warnings, including local dependency warnings
and diagnostics replayed from cached builds. Cargo failures retain their nonzero
status. Unexpected non-JSON stdout is displayed and fails the diagnostic check.

Clippy's `-- -D warnings` remains enabled for selected application targets; it
does not enforce dependency warning cleanliness. Registry dependencies can have
their lints suppressed by Cargo, so this check covers emitted diagnostics rather
than every possible upstream lint. Vendoring a dependency can expose warnings
without changing its version. Ordinary `make build`, `make test`, `make clippy`
and compatibility tests retain their normal warning/exit behavior; use the fixed
`check-clippy` stage for this stronger warning check.

Reviewers must inspect diagnostics as well as exit status before accepting
validation evidence. Resolve compiler warnings in affected application or vendored
code, and record the disposition of any remaining diagnostics from other checks;
a successful command alone does not establish a warning-free run. For dependency
or build-rule changes, verify the actual production and test feature selections,
warning enforcement scope and source provenance. Keep compiler warnings separate
from advisory complexity scores and runtime fixture logs: those need review under
their owning policies, rather than blanket warning-text rejection. Do not silence
lints or enable unused features merely to obtain a clean check.

Large macOS debug library-test binaries can emit the system linker's
`__eh_frame section too large` diagnostic. Compact-unwind DWARF hints have a
24-bit offset field; [Apple libunwind](https://github.com/apple-oss-distributions/libunwind/blob/main/libunwind/src/UnwindCursor.hpp)
can fall back to searching the retained full DWARF section. Record the actual
target, warning and retained unwind information, then complete affected panic,
cancellation/restart and optimized checks. The diagnostic reports a possible
unwind-search performance cost, without itself demonstrating a correctness failure.
Keep it visible; do not remove unwind data or change panic strategy, profile or
toolchain pins to silence it. This does not waive the production/Clippy warning gate.

### Validation order and continuation

Apply [validation selection](#selecting-validation-by-change) first; the sequence
below orders selected stages and full application checks, not every edit. Start
with applicable design review and cheap static/contract checks. Complete applicable
independent [complexity review](../tests/COMPLEXITY.md#review-discipline), including disposition
of findings that could require refactoring, before broad runtime suites. Generating
a report does not complete that review; `make check` runs automatically without a
human review pause. Reuse a verified source-matched report when its inputs have not
changed.

When selected, run `make check-clippy check-compat` before long runtime suites to
expose lint, shared-test compilation and dependency compatibility failures early. Run focused
tests for the changed behavior and affected inexpensive configuration, authored
example, schema or diagnostic contracts before their broader suites. Choose these
from real consumers; a blanket compile-only pass cannot detect stale runtime
assertions. For a full application check, once sources are stable, finish the
remaining stages:

```sh
make check-default-remaining check-proving check-lifecycle check-codec
```

`check-default-remaining` requires still-valid earlier compatibility evidence when
used for continuation. Use `check-default` for an independently requested complete
default suite.

After a failure, inspect its cause, fix it, and rerun the affected tests or stage.
Continue with the unfinished named stages. Repeat a passed stage when changed code,
fixtures, dependencies, features, profiles or build inputs invalidate its evidence;
assess relevant consumers rather than treating every edit as a full restart.
A fixture correction or test-module relocation can often use focused revalidation;
a payment ownership or network-policy change may affect wider suites. Resolve
material findings before building on the affected decision.

Retain commands, source revision and relevant input changes, feature/profile
selection, exit status and nonzero test counts in ignored `target/` logs or private
external evidence. For results carried across a fix, record why the earlier
selection remains applicable and what was revalidated. Account for every required
stage before reporting the full check complete. Describe assembled stage results
as a completed check with focused continuations, rather than claiming one
successful `make check` invocation. Named targets always execute when requested;
there is no persisted result cache or automatic skip decision.

Keep sources fixed for the validation run and executables fixed while their tests
execute. Run feature configurations sequentially: subprocess tests share the built
executable path. Coordinate separate Make/Cargo invocations and editor-triggered
Cargo checks before starting validation;
`.NOTPARALLEL` serializes one Make invocation, not other processes. Keep profiles
consistent when reusing evidence, and let active builds finish or arrange an
explicit handoff. See the [entrypoint reference](../scripts/README.md) for arguments.

Building dependencies/proving parameters can require network access; normal runtime
fixtures use localhost and deterministic unfunded keys, never a developer's `.env`
or mainnet wallet. Ignored child-process helpers invoked by their parent tests
differ from ignored optional live tests.

Base fixture modules `rotation::base::tor_tests` and
`rotation::base::tor_ownership_tests` and `rotation::base::tor_inactivity_tests`
exercise actual read consumers through fake
SOCKS/TLS/HTTP2: endpoint-first scope ordering, validated promotion, pending receipts,
fresh whole-view fallback, retained owners, cancellation and absent financial side
effects. Direct transport fixtures remain in `rotation::base::transport::tests`.
`network::tls_retry_tests` verifies TLS recovery and exhaustion in catalog,
unsigned API and Base while retaining certificate checks and signed no-replay.
Its body-read probe releases corrupt bytes only after the client accepts response
headers; a semaphore establishes that transport phase without real-time waits.
The fixture retains partial-header progress before releasing that semaphore.
Corrupted-byte acceptance and observed delivery counts share a synchronous
publication lock. Socket-free writer controls verify pending, failed and partial
writes without counting attempted mutations as delivered corruption.
Consumer cases retain retry ownership with corruption after server-written headers,
which can surface during dispatch or body collection.
Manual POST/PUT/PATCH/DELETE and ordinary PUT cases distinguish failed-handshake
recovery from an executed write whose first response TLS record is corrupted:
the latter must fail after exactly one execution, without signing or replay.
These prove routing and credential boundaries, not live Tor circuits or settlement.

### Test executable startup

On macOS, Make's Rust test commands automatically run private copied-and-ad-hoc-
signed harnesses through the [shared launcher](../scripts/test-runner.sh). This
works around pre-main `_dyld_start` stalls like
[OpenAI Codex issue #23802](https://github.com/openai/codex/issues/23802).
The symptom can also occur in ordinary terminal windows; the precise macOS cause
is unconfirmed. Other platforms retain direct execution. See the
[entrypoint reference](../scripts/README.md) for runner precedence and scope.

Copies preserve arguments, working directory, environment, stdio and process
exit/signal status. They remain available for child helpers. Copy or signing
failures fail the test command instead of falling back to a potentially stalled
original. CLI subprocesses launched through compiled `CARGO_BIN_EXE_*` paths still
use the original application executable.

Ad-hoc signing changes executable bytes. Record copied execution in development
evidence and do not substitute copied binaries for hash-pinned live qualification
or reproducible-release evidence. Sources and original build artifacts must still
remain fixed during tests. This workaround changes no production transport,
wallet or transaction authority.

### Verified TLS fixtures

Local provider fixtures use verified TLS with the public fixture CA in
`tests/fixtures/tls/ca.pem`. Independent MCP clients and caller-supplied low-level
payment clients must explicitly trust that CA; process trust does not flow into
separately constructed clients. Install a fixture's direct or Tor policy once,
before constructing its harness, and retain the same policy through all calls.
Tests that need different immutable policies run in separate child processes;
their parents verify successful execution and a nonzero child test count.
Executable fixtures pass the public CA through `trusted_ca_files` in their network
configuration. A SOCKS connection alone does not prove a verified origin request;
serving fixtures must observe the HTTP handler as well as proxy selection.
Negative TLS controls use an independent context without the fixture CA and an
intentionally untrusted certificate matching its fixture key. Keep that pair in
sync when renewing fixture material. Assert captured HTTPS path/query and
authority separately, preserving exact body, signature and isolation checks.
Directory fixtures explicitly distinguish local TLS from public-route testing;
changing a fixture URL's scheme must not change its destination authority.
Registry restart fixtures gracefully stop their HTTP server and await connection
and service release before dropping the registry owner and reopening. An aborted
accept task, delay or ignored lock error does not prove ownership has drained.
Provider help, paid API and remote catalog fixtures use TLS even when their router
also serves an infrastructure RPC. Separate infrastructure HTTP fixtures continue
to cover cleartext infrastructure support. Deferred-request barriers must also
observe an early client failure, so a rejected request cannot leave the test
waiting for an origin event that will never occur.

### Sandbox permissions

Choose execution permissions for the selected tests before starting them. Offline
fixtures can still create real localhost sockets. When the current sandbox is
known to block those sockets, request socket-capable execution through the runner
up front; a failed attempt is not a prerequisite. An environment that already
permits the required operations needs no extra approval.

| Selection | Runtime requirements |
| --- | --- |
| `crypto_vectors`, `headers`, `provider_catalogs`, `managed_config`, `network_audit`, `cli_layout` | Currently socket-free complete test targets: local computation/files and, for CLI inspection, child processes. Run in the ordinary sandbox. |
| `network`, `startup`, `cli_contracts`, `config` | Representative targets containing localhost fixtures. Use socket-capable execution for the complete target. |
| `make test`, `make check` | Include socket fixtures and require localhost socket access. |

Names above are Cargo test targets (`--test NAME`). For filtered tests, follow
helper and child-process calls: `config` and `cli_contracts` mix socket-free
inspection with real servers. A filename or search for direct bind calls alone
cannot establish requirements. Inspect unclassified selections before choosing
permissions, and reassess these examples when fixtures or helpers change.

These are runtime capabilities, not a guarantee that Cargo can build inside a
particular sandbox. Dependency/proving-parameter downloads and cache writes have
separate network/filesystem requirements; see [build tooling](../scripts/README.md).
Keep real wire, SOCKS, gRPC and lifecycle coverage intact rather than replacing or
skipping it to avoid a permission request.

Manual raw-header regressions live in `discovery::raw::tests`, discovery endpoint
fixtures and payment header/handoff/best-effort/hint/relay tests. They exercise sensitive
values, strict validation, exact key delivery, private relay exclusion, initial-payer
continuity and absence of new replay authority. A barrier-controlled payment regression overlaps two
manual calls before challenge delivery and checks their distinct credentials,
quotes and keys in unsigned and signed dispatch, verifying values never cross
calls. The isolated directory fixture
exercises real MCP stdio/HTTP calls, malformed frames, invalid inputs, body-reader
errors and upstream credential echoes. It captures application targets at TRACE
and checks that their diagnostics omit manual values; it also observes SDK
activity without asserting SDK privacy. The accepted upstream logging/error
limitation is documented under [manual raw headers](agent-sources.md#manual-raw-headers).
Initialization errors have a socket-free executable unit test.
`payment::header_capacity_tests` checks shared routed/manual overflow handling,
JSON and runtime-header headroom, and real HTTP/2 challenge/signing exchanges near
the map capacity. Cover padding skips those oversized header lists under its
existing byte policy; normal-sized exchanges verify actual padding delivery.
Direct managed-payment admission tests also fill a caller-built request's header
map and verify rejection without a journal entry, balance change or signed dispatch.
These are synthetic local contracts;
they establish neither vendor deduplication nor live payment/authentication qualification.

Managed payload validation uses one local SDK signature to check refusal of changed
payment identities, amount, accepted terms and validity before journal acceptance.
The Search relay contract uses a synthetic response validator and three authored
providers to check status/body rejection, retained possible charges and cached reuse
without replay. Pure URL and cover configuration tests cover password-only userinfo
and reserved padding-header namespaces without creating network transports.

## Test boundaries

| Area | Principal tests / contract |
| --- | --- |
| Catalog/config/providers | Generated schemas, selected operations, routing, pricing descriptions, composition and pinned local catalogs. |
| MCP/deployment/discovery | Independent local/remote/raw authority, directory subsets and native grouped responses, failure-before-I/O checks, immutable invocation snapshots, update/remove races, persistent idempotency, cancellation, pagination, shutdown and accepted paid-child ownership. |
| Payments/managed rotation | Real local signing, cap/asset validation, journal-before-disclosure, captured-wallet traversal only after typed insufficient funds, retained chargeable attempts, concurrent admission, exact reserve promotion, independent initial funding, and separately requested canonical resolution. |
| Treasury/store/funding | Encrypted persistence, legacy-state rejection/preflight, snapshot ordering, reservations and qualification limits, uncertain source sends, refund/expiry rules, backups and filesystem protections. |
| Network | Production HTTP/gRPC through authenticated fixture SOCKS, remote hostnames, identity/pool boundaries, participating Tor discovery/API/Base retries, `Retry-After` handling, conditional promotion, proxy failure, TLS, redirects and public-destination enforcement. |
| Output/resource limits | Exact/over-bound behavior, visible errors and logs, no partial cache success, no retry of paid oversized responses, typed inline image/mapping behavior. |

Tests should establish the absence of forbidden side effects as well as the
expected result: no signature for a rejected payment, no publication for a denied
registration, no source send before persistence, no connection outside the network
factory. Fixed-chain RPC fixtures capture block timestamps when the chain snapshot is
created; rereading wall time for each request can fabricate contradictory
latest/finalized/confirmed headers across a second boundary. Deliberate chain-time
changes belong in the fixture's explicit chain state.
Use barriers and controlled clocks to expose concurrency boundaries;
sleeps alone do not prove ordering. Retain positive fixture controls so a broken
fixture cannot make every negative test pass.

CLI contract fixtures count catalog/API requests and assert inspection output,
credential/error order and the absence of unsolicited pricing requests. Authored
prices and credit conversions retain focused rendering coverage. SocialFetch
credit inheritance uses synthetic source/endpoint subsets, so curated deployment
provider selections do not own that regression. Actual-call
challenge tests cover passive hint validation and wallet selection. Stdio and
shutdown suites exercise accepted-call ownership.

Capacity lifecycle fixtures call `consumed_reserve` with claim, retained-wallet,
retirement, vacant-replacement and allocated-replacement matrices. They pin exact
funding correlations, readiness support and first-error order, including existing
schema permissiveness. These structural validators certify recorded evidence;
passing their synthetic fixtures does not qualify live settlement.

Ordinary retirement changes require fixtures through the serving or finite-sync
producer and canonical publication path, rather than only direct mutation tests.
Cover independent per-group defaults/overrides, inclusive integer thresholds and
both directions of confirmed/latest disagreement. Candidate authorization paging
must cross 64 records, preserve still-valid unused signatures, use confirmed chain
time for expiry and avoid unrelated history reads or payment replay. Preserve
never-credited predecessors while original delivery remains possible, late usable
credit, current/archived source blockers, reserve generations and exact atomic
evidence through races and reopen. A completed never-credited job can close its
destination guard only when every original operation has proven expiry or never
accepted calculation/submission; test both safe closure and contrary source facts.
Malformed optional historical balance support must not become a manual-repair
gate: retain disputed history and acquire fresh proof. Contrast this with malformed
original authorization identities or missing accepted bytes, whose scoped failure
must remain visible. Exercise malformed history at the current canonical block,
including an earlier malformed row followed by a valid same-block contradiction;
ignoring optional corruption must neither require a new block nor hide that conflict.
Replaced checkpoints and authorization resolutions must
rebuild through cancellation/restart without replay or discarded original liability.

Scheduling evidence must count RPCs across idle wakes and disposable cache eviction:
latest-high completes even with a low saved minimum, whereas confirmed-high/latest-low
retains progress. Exercise independent backoff, demand during acquisition, cancelled
waiters, owner replacement, restart and unrelated concurrent work. Include finite
sync with unresolved work, paused auto-funding, Store-only serving, disabled groups,
qualification mutation denial and the startup SOCKS prerequisite with no direct
fallback. Isolated diagnostics must show committed zero/dust retirement at INFO,
safe blocker visibility and no repeated reminder on each poll. These unfunded
local fixtures establish ownership and scheduling contracts, not live settlement,
Tor privacy or the permanent absence of future external credit.

Catalog input matrices compare complete schemas, routes, body-name mappings and
body presence. They preserve duplicate-parameter behavior, collision error order,
required alternatives and recursive sanitization; public routing fixtures check
that exposed arguments reach the original wire fields.

Provider-semantics matrices pin full outcomes, assertion indices and error order,
including ambiguous JSON, nested error flags and scan exhaustion. Registry
dependency tests verify that classification affects admission without rewriting
payment evidence. Synthetic Tor audit sequences pin credential inheritance,
permissions, normalized destinations, circuit ownership, success counting and
required-target completeness. Complete sanitized outputs and rejection order are
checked; these fixtures establish evidence interpretation, not live isolation.

`tests/foreground_capacity.rs` holds seller headers and bodies with more than 128
retained owners across authenticated MCP listeners. Disconnects retain work
through signing, body collection and drain while cheap and paid direct calls
progress. The managed counterpart checks unique journaled signatures and absence
of extra funding. Qualification context is captured when work is polled, including
when its future was constructed outside the active case. These fixtures use
deterministic release points and establish no throughput or aggregate memory bound.

The [coverage map](../tests/COVERAGE.md) identifies regression boundaries and
optional qualification methods.

Progress-policy regressions use fragmented unary and streaming gRPC responses
lasting over twenty seconds, alongside silent headers, stalled bodies and sibling
stream traffic. Reconnect fixtures delay SOCKS establishment beyond read inactivity
but within the connection allowance, then verify silent headers still fail.
Zero-stream-capacity fixtures cover stalled dispatch in direct and Tor modes.
Saturated-stream fixtures verify sibling traffic cannot renew readiness and
cancelled calls cannot dispatch when capacity returns. A subprocess test exercises explicit force with a live blocking
worker, independently of the static-wallet HTTP shutdown fixtures. Wallet and
bootstrap drain subprocesses exercise the shared first-signal cancellation and
second-signal force path. A Unix handoff fixture holds drain before its first
force-receive poll: an independent signal witness confirms the second SIGTERM was
broadcast before the parent releases the barrier. The retained receiver must
still force exit, with the existing five-second fixture assertions unchanged.
Deployment fixtures keep listeners and Base reconciliation
available after a funding stop; optional remote catalog fixtures isolate missing
or invalid inferred base URLs while keeping authored URL errors fatal.
Real-process shutdown fixtures exercise responses beyond the old
ten-second cutoff, explicit force, HTTP disconnect, stdio EOF and a signal with
the parent input pipe still open. Treasury fixtures cover incremental scanning without local age/lag gates,
read-only proposal checks and retained unknown sends; managed
fixtures exercise advisory selection and post-failure balance refresh. A blocked
or failed balance read does not prevent signing or delay another wallet attempt.
New failures during active reads require follow-up; cancelled waiters retain the
reconciliation owner, and low-balance status survives restart. Submission
fixtures reject failed preflight without intent and tolerate an unsigned tip outage
after successful preflight without adding another read before sending. Concrete
funding-backend tests distinguish quote expiry before command dispatch from
uncertain preparation. Optional catalogs never suppress authored selector errors.
Optional-outage fixtures also check inherited prefixes and ambiguous listener
selectors. Birthday fixtures advance the tip between observations. Nonce-outage
fixtures retain original authorizations and explicit canonical reporting proofs;
nonce outages are not payment admission gates.
Calculation-owner fixtures hold the real accepted blocking task with a per-owner
test barrier; its Drop releases the task on assertion failure. Publication fixtures
replace only the existing minute wait with an explicit test retry-release channel,
then use the same worker, exact snapshot and idempotent SQL boundary. They do not
qualify elapsed cadence. Actual MCP progress coverage starts the authenticated application HTTP server on
the same current-thread runtime as preparation. An external controller issues a
real cached unsigned help tools/call only after the actual accepted SDK hold;
its existing fixture watchdog remains runnable if that serving runtime blocks.
It releases and joins held work before assertions, including failed HTTP or a
pre-handoff preparation failure. This is functional progress, not a latency claim.
Normal-runtime-drop coverage holds no external Store
sender and observes a real async task's abort before releasing accepted CPU work.

Treasury sync transition regressions use the existing observation failure state:
a real refused offline scan announces INFO once, repeated failure preserves
backoff, successful completion permits a later transition, and shutdown remains
quiet. The startup SOCKS owner fixture verifies command-first failure followed
by periodic retry shares that transition without creating financial work.

Synthetic proving checks actual-fee affordability and exact saved bytes through
failed publication, lost acknowledgement, cancellation and reopen. Actual Ironwood
proofs verify recipient, principal, fee and expiry under the same SDK proposal.
Negative tests reject legacy-pool selection and unsupported wallet shielding
before inputs, bytes or network side effects. The required Sapling prover adapter
has no parameter initialization; actual Ironwood calculations run through it
without invoking an unsupported proof. SDK build-time parameter acquisition is a
separate unavoidable pinned-dependency input, not runtime transfer support.
The optional synthetic-chain lifecycle exercises managed and direct recipients
through the real treasury actor, scan reuse, confirmation depth, restart,
cancellation after submission intent and funded same-height reorgs. These tests
do not establish proving latency, consensus validation or live settlement.
The production funding worker also runs against local bridge/Base fixtures through
range-aware nominal selection, saved-choice reuse, provider-floor redraw before the
next request, executable EXACT_INPUT route-minimum errors without a USDC redraw,
concurrent quote acquisition,
refund derivation, bootstrap completion and signed payment-triggered replacement
funding, with scan-reuse and no-replay assertions.
Reopened insufficient-funds diagnostics cannot block bootstrap or serving before
the current worker rechecks them. A synthetic deposit regression verifies that
readiness accepts a previously saved low estimate without a balance gate. Actual
actor preparation then scans a newly confirmed deposit and calculates with SDK
inputs without broadcasting.
Default fixtures cover concurrent
replacement allocation/credit with retained old payment liabilities and paged,
scoped bootstrap completion after reopen; see [LIFECYCLE.md](../tests/LIFECYCLE.md).
These are unfunded/local tests, not evidence of live
1Click delivery or a resolution of its Zcash quote-deadline contract.

Managed-payment fixtures exercise advisory selection, one saturating estimate
decrement per authorization, structured insufficient-funds traversal beyond a page,
cloneable POST bodies, exhaustion and multiple retained authorizations. Refresh is
independent and coalesced; failed reads preserve hints and low markers. In Tor mode,
payment, managed and discovery fixtures also cover participating unsigned status
retries (`400`, `403`, `408`, `429`, `500`, `502`, `503`, `504`), conditional winner promotion, retained
challenge-to-signed scope across competing promotion and cache eviction, combined
insufficient-funds handoff with per-wallet unsigned circuit retry and qualification
accounting, and
`SelectionPolicy::Legacy` preservation for internal discovery relay calls.
`payment::tor_wait_tests` covers actual Retry-After waits, independent progress,
retained disconnected waiters (including winner retention after response, text
and configured-output waiters disconnect), explicit owner termination and authority
expiry during the wait. `payment::tor_cover_tests` exercises API promotion and competing
recoveries with actual HTTP/2 streams, separate SOCKS pools, scope-matched cover,
and eviction/reinsertion. Body-failure fixtures reject partial, oversized and
inactive outputs without promotion or generic paid replay. Exhaustion fixtures
preserve each eligible status despite stalled or oversized final error bodies,
with no signature, promotion or final Retry-After wait. Unfollowed 302/307 catalog
fixtures preserve complete JSON validation and existing relay fallback without
extra origin attempts. Unrepresentable Retry-After fixtures exercise catalog,
pricing, relay continuation and API ownership: no early dispatch, unrelated
progress and explicit cancellation. These local fixtures
do not establish real Tor exit diversity or settlement. Funding
fixtures cover irreversible first-positive claims, exact accepted-work ownership,
scoped observation/backoff, cancellation and restart. Qualification charges each
accepted attempt under its original case allowance; a later success erases none.

Managed payment crash fixtures use SIGKILL on an owned signing subprocess at
private-signature, journal-acknowledgement and signed-dispatch boundaries. They
reopen the real SQLite state, retain exact committed authorizations and their one estimate debit without
replay, and verify independent peer-pool service. These use unfunded temporary
wallets and loopback providers; they do not establish live settlement outcomes.

SQLite fault fixtures install a non-default test VFS on temporary stores to fail
real WAL writes/fsyncs and database checkpoint writes/fsyncs. They also exercise
`SQLITE_FULL` using SQLite's page limit. Reopen checks require atomic preparation,
retained input claims/payment journals, atomic estimate updates, exact committed bytes, database integrity and no
provider dispatch after a failed payment journal acknowledgement. A sync error is
allowed to leave an entire recoverable commit; it never proves no work occurred.

The SIGKILL and test-VFS scenarios above are Unix-only; native Windows crash
and SQLite-I/O recovery work is deferred by user decision. Shared payment-admission
and canceled-waiter ownership fixtures remain enabled on all platforms.

Offline refill fixtures compare exact prepared bytes and transaction facts before
and after reopening the store. Restart qualification also binds transaction facts
and the SHA-256 digest of the immutable encrypted prepared record across observed,
drained and reopened snapshots. Historical evidence lacking these fields stays
readable but cannot establish that continuity. The digest proves record identity,
not transaction validity or settlement. Combined cover/payment fixtures retain
unique managed authorizations through concurrent calls, held range responses and
store reopen; seller success and cover progress do not release payment exposure.
These checks do not qualify a live funded restart or Tor privacy.

Production storage preflight uses compact old/current/future boundary fixtures:
schema 23 opens and older/future schemas reject without source mutation. Current
fresh/reopen/status/backup fixtures cover interrupted empty-history completion
without rewriting journals; current corruption tests preserve strict observation
and transaction-fact failures. Private disposable recovery of committed WAL and
hot rollback journals preserves accepted obligations through failed reopen. These
checks cannot supply missing immutable destination-readiness evidence or qualify
a private conversion or live migration.
Funded lifecycle, privacy and combined cover/payment acceptance retain the limits
listed under [qualification status](#qualification-status).

Lifecycle projection tests compare every published field with an independent
baseline assembly over small synthetic Store states, including complete pools,
claim generations, job order/phase/health, retirement facts, source history and
exact encrypted-record hashes. Direct persisted and runtime-cache observations
retain their different evidence fields. A pinned read-transaction fixture commits
another writer's changes between collections and rejects mixed snapshots. Required
phase/facts/raw-record/health errors remain failures; corruption only in diagnostics
not exported by narrow lifecycle cannot obstruct it, while full Status/export keep
their required contracts. Existing real-certificate and receipt fixtures verify
compatible/unavailable runtime evidence and retained receipts/charges after actual
lifecycle failure. No offline projection fixture qualifies live settlement.

Bootstrap readiness tests pass current Store-produced status 6 and lifecycle 2
through readiness and restart consumers independently of database schema 23 and
bootstrap summary 3. Eligible funded READY inventory needs no ACTIVE predecessor.
They distinguish captured-claim completion from ordinary serving readiness,
including paged vacant inventory, captured completion after
current generations advance, recovery after retained historical attempts and assessment
without writes. Store-level tests cover claim history and obstruction dispositions;
owner tests cover cancellation and drain. These are complementary checks, not an
exhaustive enumeration of concurrency schedules.
The retired-participant restart regression reopens saved empty, retired inventory,
retains destination routing, and schedules a new small reserve from historical
credit alone, without first-use demand or source preparation. Its saved retirement
fixture does not exercise retirement proof acquisition or live settlement.
Recovery eligibility and startup actionability remain separate: an accepted quote
can keep startup waiting while the worker may recover its original operation.
Bootstrap also distinguishes treasury-wide pending source work from the captured
operation's own source liability when explaining spendable-fund shortages.

Runner orchestration fixtures use real registry reservations, local MCP requests,
completion records, settlement inspection and owned child close/reopen. Test-only
external observations let them exercise two rounds, refusal before dispatch,
rotation/settlement/restart failures and final readiness without funding. They
assert retained reservations, no extra requests, unchanged expiry/funding authority
and the same execution clock across restart. Registry and real-process runner
regressions separately cover no replay, concurrent completion and cancellation
while draining. Outage preparation refusal has a local fixture; non-promotion
after a round error is established by review of the unchanged completion gate.
These fixtures do not establish chain settlement, successful Tor outage
qualification or the deferred automated funded lifecycle acceptance.

Registry report composition tests compare complete JSON for all-runs and selected
runs, configuration-only and prepared catalogs, revised authorization, global
charged reservations and funding usage. They cover ordered corruption failures,
nonfatal application/debit evidence, immutable pin revisions, selected event
filtering and exact case/pin/event and complete-document bounds. Component and
summary tests complement these fixtures. Read helpers preserve query order and
repeated evidence reads; selected-run sections retain their dependency order and
fatal versus incomplete error handling. These synthetic reports do not establish
live settlement or qualification.

Credit-pricing fixtures compare complete descriptions for fixed, bounded, metered
and batch tariffs, ordered surcharge and normalization suffixes, and unknown
metadata fallback. They retain zero and maximum values, checked-arithmetic overflow
rejection and unusual accepted metadata. Caller tests cover original billing text,
overrides, warnings and absence of pricing probes. Tariff conversion remains a
display estimate and never grants payment authority.

MCP inventory fixtures record exact eager-listing and discovery request sequences,
including separate cursor protocols, shared duplicate detection, page/tool and
response-byte boundaries, and prepared-inventory validation after traversal.
Failure and acknowledged cancellation tests assert that no later request starts.
A real keyless-runner test exercises the inventory gate before case execution.
Full schemas and descriptions must still match the pinned catalog; these local
fixtures do not qualify a live provider or prove remote cancellation.

Reliability report fixtures compare complete outputs and qualification results for
multiple ordered groups, malformed evidence, missing observations, canonical debit
requirements and mixed statuses. Timestamp matrices preserve the distinction
between invalid timestamped observations that advance the previous timestamp and
missing or unattempted observations that do not. Window, spacing, checked-add and
UTC-day boundaries are exercised alongside composed registry and summary tests.
The report interprets saved dispatch evidence; it does not establish provider health
or attribute failures to Tor.

Qualification planner fixtures compare complete serialized plans for all scenario
variants and both start modes, retaining invalid combinations as rejections. They
cover ordered dependency/case validation, wallet bindings, cap fallback, reservation
and allocation boundaries, reachable arithmetic overflow, and exact first errors.
Preset, selection, registry and real preparation consumers complement these tests.
The offline wrapper is checked without catalog credentials or wallet state. Planning
still grants no execution authority; synthetic plans do not establish live funding
feasibility. Infeasible billion-pool overflow fixtures are not manufactured: the
unchanged checked guards and manifest bounds receive independent review.

Wallet argument-resolution tests run the real resolver in isolated subprocesses
so process-global network policy and environment do not leak between cases. They
inspect every command's fields without executing it, including configuration and
network installation order, path defaults, identity assertions and birthday endpoint
selection. Missing/malformed state and configuration have explicit expectations.
Real executable fixtures separately cover Clap conflicts, config placement,
catalog-free administration and preservation of existing state. Resolution-only tests do not
claim to exercise funded command execution.

Resumption fixtures compare complete eligibility output and ordered failures for
case inventories, every event timestamp, original deadlines, prior financial states
and phase windows. Missing start observations remain distinct from expired runs;
expired windows cannot bypass unresolved prior work. Tests preserve manifest/phase
ordering, partial lifecycle refusal and the registry's ownership of canonical proof
and skipped-suffix validation. Registry and real-process cancellation/reopen and
window tests cover the caller boundary. Eligibility remains offline information,
never renewed authority or permission to replay attempted work; these checks do not
exhaustively enumerate concurrency schedules.

Current treasury reporting fixtures retain immutable principal/fee facts, encrypted
preparation identity, unresolved work and advisory wallet estimates. Deleted budget
and refund projections are unavailable rather than invented zero. Unsupported
historical report formats are refused without rewriting saved evidence or releasing
execution reservations. Baseline, attribution, registry ownership/reopen and report
fixtures preserve original qualification charges and unknowns. These summaries
never supply current spending authority. Current lifecycle 2, exact balance 2,
advisory admission balance 3 and ownership 1 are distinct contracts. Current intent
marker 2 and original per-attempt canonical observations remain required for earlier
runs, including single-attempt cases; aggregate case outcomes cannot replace them.
Current restart fixtures pin the selected reserve claim while allowing unrelated
inventory and readiness progress. Unsupported saved admission presentation leaves
original attempts, exact support and ordinary reconciliation intact.

Deployment validation fixtures pin complete static and managed wallet resolutions,
ordered contextual failures, listener address uniqueness and port-zero exceptions,
independent Host/auth settings, and treasury/funding prerequisites after assignment.
They run without listener creation or credentials; deployment and assignment
integration tests separately cover real local listeners and offline inspection.
The characterization protects validation orchestration, not all runtime interleavings.

Shared catalog/price regressions live in `src/http_cache/price_tests.rs`, existing
catalog cache fixtures, `src/payment/hint_tests.rs` and catalog-state description
and direct-target fixtures. They cover joint persistence/lifetime, revision and
content invalidation, alias sharing, received-order updates, delayed writes and
unchanged concise rendering/per-call offers, without live funds or price probes.
Forced warm tests cover unconditional catalog and configured-pricing acquisition,
normal serving reuse, complete-snapshot retention and provenance-family retirement.
`src/catalog_refresh/`, `src/discovery/refresh.rs` and
`src/discovery_relay/list_tests.rs` cover final unsigned target removal signals
after recovery, signed-target and relay-service exclusions, decoded relay
destinations despite signed relay fees, source-generation coalescing, continued
tools/search/hints during refresh and catalog failure, replacement preparation
while current tools serve, repaired tool invocation before optional pricing
completion, retained challenge observations after advertisement arrival, retired
generation fences against late enrichment, and usable replacements after optional
pricing failure. They also cover accepted signed work, cancelled callers, registration
races and shutdown ownership. Deferred refresh
fixtures cover combined publication failure, endpoint selection diagnostics and
unrelated endpoint usability. These are synthetic local contracts, not live
provider availability or financial qualification.
The managed relay fixture retains a depleted first wallet: local bootstrap catalogs
have no shared hint, so the actual relay offer earns an unsigned sibling handoff.
It pins the exact unchanged relay requests, signing owners and authorization journal
while retaining the absence of target signing, RPC, funding and allocation.

## GitHub Actions and coverage

[CI](../.github/workflows/ci.yml) runs on pushes to `main` and `ci_coverage`, and
pull requests targeting `main`. The `ci_coverage` push trigger supports pre-merge
bootstrap; it can be removed once this work is integrated. Linux checks run
`make check` in its existing order on Ubuntu 24.04, with the repository's pinned
Rust, Python 3.13 and full Git history for comparison with `origin/main`.
Each existing `check-*` stage is a separate sequential Actions step, with the
same ordering and fail-fast selections as `make check`. Stage output, elapsed
seconds and exit status are retained in the `check-diagnostics` artifact, and
outcomes are appended to the job summary even when a stage fails. Test concurrency
remains eight; compiled-artifact caching requires evidence from hosted timings.
The independent default-coverage job runs `make coverage` with matching LLVM
tools and cargo-llvm-cov 0.9.1. Actions and octocov's binary version/checksum are
pinned. Cargo download inputs are cached; application builds, financial state and
private local reports are not cached or uploaded. Hosted resource suitability
requires actual run evidence; this workflow is not release reproducibility or
live settlement qualification. The remaining advisory automation and Linux
acceptance work is owned by [release follow-up D2](plans/deferred/release_followups.md).

[Octocov configuration](../.octocov.yml) consumes the filtered LCOV report, writes
a job summary and generates a **default line coverage** SVG. The denominator
excludes vendor code, integration harnesses and operational tools; inline unit-test
helpers may contribute. Optional proving, consensus, Tor and funded qualification
are not included in this percentage. There is no percentage or test-time gate.
Coverage collection, export and publication failures fail the coverage job;
octocov comments and summaries are best-effort reporting. Linux checks remain a
separate check, and must be selected as a required check in repository settings
if merging should be blocked on their result.

Open the branch's Actions run or pull-request checks to see results. The
`default-line-coverage` artifact contains `html/index.html`, the filtered LCOV and
summaries, provenance and generated `coverage.svg`; download it for branch badge
preview. Collection logs are retained separately as `coverage-diagnostics`,
including on failed collection. Initial Cargo compilation/listing output is
retained in `tests-*.txt` alongside the execution log. Only selected resolved
files are uploaded, since
the local `target/coverage` path is an absolute symlink to its successful run.
For a branch-specific CI badge, use
`ci.yml/badge.svg?branch=ci_coverage&event=push`. The README badges describe main.

Main runs store octocov comparison reports in Actions artifacts. Pull requests
can show coverage deltas once a main baseline exists; missing or expired baseline
artifacts leave absolute coverage available without a historical comparison.
Fork pull requests retain read-only tokens: summaries and downloadable reports
remain available, while posting PR comments may be skipped. The workflow does not
use `pull_request_target` or run fork code under a privileged follow-up workflow.
Octocov's optional external viewer is disabled in favor of GitHub artifacts.

After both jobs succeed on a main push, a separate publisher updates `coverage.svg`
and `source-revision.txt` on the dedicated `badges` branch. Repository Actions
policy and branch rules must permit that job's `contents: write` token to push.
The permanent badge does not exist until this first successful publication. Only
publication is serialized: running test jobs are not cancelled, but GitHub may
replace a superseded pending publisher. A publisher checks current main and skips
an older revision, then uses a normal non-force push. The badge identifies its
published source revision; a failing or still-running main revision can leave
the prior successful coverage badge visible, alongside the current CI status.

## Coverage and complexity

[Catalog startup qualification](../tests/STARTUP.md) documents rolling-concurrency
and isolation tests, an opt-in local provider replay, and measurement methodology.

`make coverage` produces default line, region and function reports, plus a filtered
`lcov.info` for line-coverage reporting, with
compiler/source/lockfile provenance. Generated reports stay under ignored `target/`;
failed collection must not replace the latest successful report. Application
filters exclude external/vendor/test/example paths, although inline test helpers
can still contribute to the application denominator. Compare identical feature
sets and filters, and explain denominator changes before attributing a percentage
change to better tests.

A condition-only baseline/final evaluation uses the pinned
[condition analysis workflow](../tests/COVERAGE.md#condition-analysis), with only
necessary focused financial supplements. Other coverage modes are separate capabilities.

Branch collection is an optional separate invocation using a compatible nightly
compiler and matching LLVM tooling:
`RUSTUP_TOOLCHAIN=nightly make coverage-branch`. Stable region coverage is not
branch coverage. Optional proving,
consensus and Tor results remain distinct from default report percentages.
The development guide's [coverage section](development.md#local-coverage) documents setup and
artifact locations; its [complexity section](development.md#local-complexity-metrics)
documents the local rust-code-analysis workflow.

Review uncovered regions by risk: financial release boundaries, grants, transport
fallback, durable publication, error handling and concurrency matter more than a
repository-wide score target. Do not remove guards or mirror implementation details
in tests solely to raise a number. The small
[mutation pilot](plans/deferred/mutation_testing.md) remains deferred; no mutation
score is implied by coverage results.

Policy-removal regressions should exercise remaining authority boundaries through
normal application paths. Change advisory estimates or cached prices independently
of current caps and actual offers; assert permitted work proceeds and actual
over-cap work still cannot sign. Multi-attempt tests retain every original journal
entry and distinguish ambiguous protocol records from the exact retry category.
Owner-replacement tests interrupt a scheduler while accepted child work still
holds its lease, then check exclusion, release, restart, shutdown and retained
backoff. Count completed scans directly when proving single-scan expiry recovery;
the authenticated original signed expiry, burial and final unchanged Ready
snapshot coverage must still be checked. Snapshot revisions also count
intermediate checkpoints. Missing, relocated or differently spent old inputs must
not become negative-expiry gates; current SDK state owns spendability. Test bridge
terminality and current authority separately from local source closure. Diagnostic prose, workflow phases and aggregate
balances alone do not prove these properties.
Verify that a changed financial test is reachable through its module declarations
and actually runs with the intended feature/platform gates. A successful filtered
Cargo command can run zero tests; a complete complexity report can analyze a file
the compiler never reaches. Source metrics do not establish test execution.

Focused recovery tests and real SDK Ironwood lifecycle tests cover same-Store
reconstruction, task replacement, cancelled waiters, accepted input ownership,
qualification denial, lost submission response and restart without duplicate
calculation. Unsupported pool/input/receiver paths and transparent address RPCs
reject without acquiring new inputs, creating bytes or disclosing addresses.
Financial scenarios use genuine managed or native Ironwood transactions; SDK raw
state remains unchanged. These fixtures use public deterministic keys and do not
establish consensus or live-funded settlement qualification.

## Optional proving, consensus and live work

- [LIFECYCLE.md](../tests/LIFECYCLE.md) covers synthetic-note proof and exact-byte
  restore assertions without mainnet funds.
- [REGTEST.md](../tests/REGTEST.md) covers mined local Ironwood deposits, reorgs, unsupported-shielding rejection,
  expiry proof and recovery CLI behavior.
- [TOR.md](../tests/TOR.md) covers controlled real-Tor circuits and per-process OS
  confinement using an independently installed Tor binary. A local SOCKS fixture
  cannot prove actual Tor circuit assignment or host firewall behavior.
- [Live integration walkthrough](../tests/live/INTEGRATION.md) is the single funded
  provider workflow; [its reference](../tests/live/INTEGRATION_REFERENCE.md) defines
  assertions, registry accounting, concurrency, lifecycle and Tor evidence.

## Qualification status

| Boundary | Evidence and limitation |
| --- | --- |
| Catalogs, MCP, payments, permissions, durable funding and cancellation | Offline fixtures cover named invariants and interleavings, not exhaustive race freedom or real settlement. |
| Synthetic proving and local consensus | Optional suites exercise saved-byte recovery, mined deposits, reorg refusal, refunds and expiry. See LIFECYCLE and REGTEST; these results are separate from default coverage. |
| Fresh HTTP scope recovery and qualification mapping | Local fixtures cover retry credentials and private scope-to-owner/origin handoff, strict STREAM audit and session isolation. The changed recovery paths have not been requalified with a real Tor daemon or funded provider run. |
| Real Tor and confinement | Exercised on macOS with an installed Tor process, authenticated control events and OS egress controls. Local SOCKS tests do not prove this; other operating systems and all-network anonymity remain outside the claim. |
| Funded provider workflow | Historical funded runs exercised treasury bootstrap, a selected provider sweep, help caching, canonical debit reporting and the former startup-pricing check through Tor. Current runs require no startup-pricing stage. Failed or omitted providers remain failed or unobserved; this is not universal provider compatibility or current health. The canonical GenuineGood POST migration has only offline evidence; historical funded GET results do not qualify it. |
| Shared/separate-wallet concurrency and refill service | Manual live tests exercised overlapping paid calls and continued service during pending refill. Runner assertions have offline coverage; manual evidence does not qualify every automated lifecycle preset. |
| Automated lifecycle/restart | Full confirmed-refill/two-round restart acceptance remains deferred. Promotions or service during treasury insufficiency alone are insufficient. |
| Cover with paid traffic | Unsigned cover checks are separate. A fully successful combined cover/payment qualification remains deferred; range refusal or missing samples cannot be hidden by payment success. |
| Mutation and branch coverage | No mutation score is claimed. Branch collection requires a compatible nightly/LLVM setup; stable region coverage is not branch coverage. |

The [scenario presets](../tests/live/integration/SCENARIO_PRESETS.md) retain explicit
wallet scope, single-use cases, finite authority and observation windows. The
[provider preset](../tests/live/integration/PROVIDER_PRESETS.md) retains 23 providers,
22 independently reviewed paid requests, one unsigned directory call and 40 help
calls. Presets are inputs, not proof of live feasibility or permission to spend.
Coverage records describe this selected scope; adding a bundled provider requires
neither a paid case nor an exclusion record. Offline preset validation uses
explicit captured inputs for its selected remote sources and resolved production
local catalogs, independently of generated catalog snapshots.
[Deferred acceptance](plans/deferred/live_integration_acceptance.md) specifies the
remaining lifecycle and cover gates.

Qualification argument certification preflights every schema branch before checking
values. Unknown assertions, custom vocabularies and unsupported dialects fail
closed, including in absent properties and alternatives. OpenAPI `discriminator`
and `x-` documentation metadata remain opaque and do not bypass type, required or
composition checks. The exact `propertyNames: { type: "string" }` assertion is
accepted because JSON object names are strings; other property-name constraints
remain unsupported. This offline subset is not a general JSON Schema evaluator.

## Runner regression map

| Contract | Maintained tests |
| --- | --- |
| Recorded scope has valid, unique configs and matches the reviewed paid preset | `tests/provider_catalogs.rs::reviewed_provider_scope_matches_explicit_paid_preset` |
| Qualification schema rejection order, recursion/node boundaries and shared alternative work limits | `tests/live/runner/schema.rs` tests through public `validate` |
| Exact reviewed requests, caps, schemas and wallet arrangements | `tests/live/runner/presets.rs` and independent request fixtures |
| Authenticated MCP single-attempt fixtures sign exactly once | `tests/server.rs::authenticated_mcp_paid_call_signs_once_and_delivers_the_provider_result`, `tests/payments.rs` |
| Crash/failure cannot replay or refund a charged attempt | Runner registry tests and `tests/integration_preparation.rs` real-process tests |
| Prior permits and liabilities survive new runs | `src/qualification/funding/registry_state.rs` and immutable-baseline tests |
| Complete frozen inventories, typed loading failures and exact snapshot byte boundary including newline | `tests/startup.rs`, `tests/integration_preparation.rs`, `tests/snapshot_boundary.rs` |
| Identity/origin scope and confined egress | Managed/unsigned identity-map tests, `tests/network_audit.rs`, optional `tests/TOR.md` procedure |
| Rotation output, missing evidence, settlement precedence and original replacement-job bindings | `tests/live/runner/rotation_report.rs` tests; fixtures certify report logic only |
| Parent death, malformed-control drain and unsigned refusal before disclosure | `tests/supervision.rs` |

MCP success, useful provider content, seller receipts, consumed nonces and verified
canonical debits are different facts. Missing proof is not a zero fee. Financial
ledgers, attempted case IDs and failure evidence remain private and immutable;
removing a Git report never resets authority or permits a retry.

## Generated artifacts

Write live reports, metrics and audit inventories under ignored `target/` or an
operator-selected private external directory. Preserve raw evidence locally with
its exact build/configuration pins. Commit durable findings as tests, provider
configuration, maintained documentation or focused unfinished plans. Do not append
dated run logs, handoffs, balances or report copies to these documents.

Managed insufficient-funds fixtures exercise real `PaidClient` signing, separate
identity-bound challenges, unchanged GET/POST arguments and retention of both
authorizations. Their pinned seller forms are in
[`insufficient-funds.json`](../tests/fixtures/x402/insufficient-funds.json). These
local fixtures do not qualify a deployed seller or establish that the first attempt
cannot charge. Qualification reservations cover every authorization within the
existing per-case permit; reports correlate each receipt/debit by attempt ID and
preserve incomplete earlier outcomes. Saved admission estimates do not prove solvency.

V1 fixtures cover static and managed signing from header and body challenges,
the local SDK version-metadata adapter, and configured challenge limits above the
default. Legacy receipt failures can traverse existing wallets; successful,
malformed, duplicate and mixed-version receipt headers cannot earn another attempt.
Receipt projection retains legacy transaction/payer claims under the canonical
Base network for recovery. Direct managed header challenges discard their unused
bodies, while body-only challenges retain the default collection bound. These
checks use local sellers and unfunded keys, not live settlement.
