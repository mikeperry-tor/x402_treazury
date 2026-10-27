# Testing and qualification

Use offline regressions to establish behavior; use optional consensus and live
runs for the integration boundaries they actually exercise. A coverage percentage
or successful HTTP response does not substitute for financial, permission or
network-isolation assertions.

When changing tests, preserve full schema/description contracts and meaningful
assertions; do not trim documentation or loosen expectations to obtain a pass.
Discovery tests inject public-URL documents and localhost fixtures only under
`cfg(test)`. Never add a production loopback bypass for fixture convenience.

## Provider regressions

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

## Default development checks

The application always includes Zcash. The [Makefile](../Makefile) verifies pinned Cargo
and supplies Cargo-managed protoc; do not append a second `--locked` flag.
See the [entrypoint reference](../scripts/README.md) for focused test arguments.

`make check` runs the fixed stages below in order and stops at the first failing
stage, including with `make -k`. Every stage can also run independently; it does
not implicitly rerun earlier stages. `CARGO_ARGS` and `TEST_ARGS` customize
`make test` / `make clippy`, but never narrow a `check-*` stage.

| Stage | Selection |
| --- | --- |
| `check-static` | Formatting, tooling regressions, analyzer tests and CogC/CycC report, vendor provenance |
| `check-clippy` | Production library/binary compiler check, then all-target Clippy with `zcash-regtest`; emitted compiler warnings from both application and dependencies fail the stage |
| `check-compat` | SDK wallet, payment, MCP and cryptographic integration tests using the application lockfile |
| `check-default` | All default application targets, including SDK restore/scan-merge regressions and examples |
| `check-proving` | `zcash-testutils` treasury proving tests |
| `check-lifecycle` | `zcash-testutils` synthetic-chain treasury lifecycle |
| `check-codec` | `zcash-testutils` address-codec isolation regression |

Default application harnesses use eight libtest threads. `TEST_THREADS=1` selects
serial execution for `make test` and `make check-default`; other positive counts
can be measured the same way. `TEST_ARGS` still replaces the whole harness argument
list for focused `make test` runs, while `check-default` retains its complete target
selection. The lifecycle stage uses eight threads; proving and codec stages keep
one thread. Feature builds remain sequential.

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
without the fixture's scoped subscriber. All severity, count, privacy and financial
assertions stay in that child; the parent verifies successful execution of one
test. Other fixtures remain parallel; this is not a reason to serialize an entire
harness.

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

### Validation order and continuation

Start with design review and `make check-static`. Complete applicable independent
[complexity review](../tests/COMPLEXITY.md#review-discipline), including disposition
of findings that could require refactoring, before broad runtime suites. Generating
a report does not complete that review; `make check` runs automatically without a
human review pause. Reuse a verified source-matched report when its inputs have not
changed.

Run `make check-clippy check-compat` before long runtime suites to expose lint,
shared-test compilation and dependency compatibility failures early. Run focused
tests for the changed behavior and affected inexpensive configuration, authored
example, schema or diagnostic contracts before their broader suites. Choose these
from real consumers; a blanket compile-only pass cannot detect stale runtime
assertions. Once sources are stable, finish the remaining stages:

```sh
make check-default check-proving check-lifecycle check-codec
```

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
captures real MCP stdio/HTTP SDK logs at TRACE, including malformed frames, invalid inputs, body-reader errors and upstream
credential echoes; initialization errors have a socket-free executable unit test.
`payment::header_capacity_tests` checks shared routed/manual overflow handling,
JSON and runtime-header headroom, and real HTTP/2 challenge/signing exchanges near
the map capacity. Cover padding skips those oversized header lists under its
existing byte policy; normal-sized exchanges verify actual padding delivery.
Direct managed-payment admission tests also fill a caller-built request's header
map and verify rejection without a journal entry, balance change or signed dispatch.
These are synthetic local contracts;
they establish neither vendor deduplication nor live payment/authentication qualification.

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
factory. Use barriers and controlled clocks to expose concurrency boundaries;
sleeps alone do not prove ordering. Retain positive fixture controls so a broken
fixture cannot make every negative test pass.

CLI contract fixtures count catalog/API requests and assert inspection output,
credential/error order and the absence of unsolicited pricing requests. Authored
prices and credit conversions retain focused rendering coverage; actual-call
challenge tests cover passive hint validation and wallet selection. Stdio and
shutdown suites exercise accepted-call ownership.

Capacity lifecycle fixtures call `consumed_reserve` with claim, retained-wallet,
retirement, vacant-replacement and allocated-replacement matrices. They pin exact
funding correlations, readiness support and first-error order, including existing
schema permissiveness. These structural validators certify recorded evidence;
passing their synthetic fixtures does not qualify live settlement.

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
second-signal force path. Deployment fixtures keep listeners and Base reconciliation
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
Synthetic proving checks actual-fee affordability and exact saved bytes through
failed publication, lost acknowledgement, cancellation and reopen. The optional synthetic-chain lifecycle exercises two
managed recipients through the real treasury actor, scan reuse, confirmation depth,
restart, cancellation after submission intent and funded same-height reorgs.
The production funding worker also runs against local bridge/Base fixtures through
concurrent quote acquisition, refund derivation, bootstrap completion and signed
payment-triggered replacement funding, with scan-reuse and no-replay assertions.
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

Offline refill fixtures compare exact prepared bytes and transaction facts before
and after reopening the store. Restart qualification also binds transaction facts
and the SHA-256 digest of the immutable encrypted prepared record across observed,
drained and reopened snapshots. Historical evidence lacking these fields stays
readable but cannot establish that continuity. The digest proves record identity,
not transaction validity or settlement. Combined cover/payment fixtures retain
unique managed authorizations through concurrent calls, held range responses and
store reopen; seller success and cover progress do not release payment exposure.
These checks do not qualify a live funded restart or Tor privacy.

Production storage preflight and historical-schema regressions cover rejection
without mutation, private disposable recovery of committed WAL and hot rollback
journals, and preservation of accepted obligations through failed legacy reopen.
These checks cannot supply missing immutable destination-readiness evidence or
qualify a live migration.
Funded lifecycle, privacy and combined cover/payment acceptance retain the limits
listed under [qualification status](#qualification-status).

Bootstrap readiness tests distinguish captured-claim completion from ordinary
serving readiness, including paged vacant inventory, captured completion after
current generations advance, durable recovery-count boundaries and assessment
without writes. Store-level tests cover claim history and obstruction dispositions;
owner tests cover cancellation and drain. These are complementary checks, not an
exhaustive enumeration of concurrency schedules.

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
Real executable fixtures separately cover Clap conflicts, aliases, catalog-free
administration and preservation of existing state. Resolution-only tests do not
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
and refund projections are unavailable rather than invented zero. Historical
reports remain readable. Baseline, attribution, registry ownership/reopen and report
fixtures preserve original qualification charges and unknowns. These summaries
never supply current spending authority.

Deployment validation fixtures pin complete static and managed wallet resolutions,
ordered contextual failures, listener address uniqueness and port-zero exceptions,
independent Host/auth settings, and treasury/funding prerequisites after assignment.
They run without listener creation or credentials; deployment and assignment
integration tests separately cover real local listeners and offline inspection.
The characterization protects validation orchestration, not all runtime interleavings.

## Coverage and complexity

[Catalog startup qualification](../tests/STARTUP.md) documents rolling-concurrency
and isolation tests, an opt-in local provider replay, and measurement methodology.

`make coverage` produces default line, region and function reports with
compiler/source/lockfile provenance. Generated reports stay under ignored `target/`;
failed collection must not replace the latest successful report. Application
filters exclude external/vendor/test/example paths, although inline test helpers
can still contribute to the application denominator. Compare identical feature
sets and filters, and explain denominator changes before attributing a percentage
change to better tests.

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
backoff. Count completed scans directly when proving two-scan recovery; snapshot
revisions also count intermediate checkpoints. Manifest tests use actual SDK
transactions and selected inputs, including a missing small input when the
remaining value covers the fee. Diagnostic prose, workflow phases and aggregate
balances alone do not prove these properties.
Verify that a changed financial test is reachable through its module declarations
and actually runs with the intended feature/platform gates. A successful filtered
Cargo command can run zero tests; a complete complexity report can analyze a file
the compiler never reaches. Source metrics do not establish test execution.

The R01/R02 owner follow-ups add focused default-build recovery tests and real
SDK synthetic-chain shielding tests under `treasury::lifecycle_tests::shielding`.
They cover same-Store reconstruction, actual task replacement, cancelled waiters,
per-address ownership, fee-only cost accounting, rejection of zero-net proposals
before accepting inputs, qualification denial, lost submission response
and restart without duplicate calculation. The pinned SDK may promote shielding
change from Orchard to Ironwood after an upgrade; ownership checks use decrypted
account-zero notes across supported shielded pools. These tests use public unfunded
keys and do not establish consensus or live-funded settlement qualification.

## Optional proving, consensus and live work

- [LIFECYCLE.md](../tests/LIFECYCLE.md) covers synthetic-note proof and exact-byte
  restore assertions without mainnet funds.
- [REGTEST.md](../tests/REGTEST.md) covers mined local deposits, reorgs, refund
  shielding, expiry proof and recovery CLI behavior.
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
| Parent death, malformed-control drain and unsigned refusal before signing | `tests/supervision.rs` |

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
