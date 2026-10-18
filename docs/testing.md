# Testing and qualification

Use offline regressions to establish behavior; use optional consensus and live
runs for the integration boundaries they actually exercise. A coverage percentage
or successful HTTP response does not substitute for financial, permission or
network-isolation assertions.

When changing tests, preserve full schema/description contracts and meaningful
assertions; do not trim documentation or loosen expectations to obtain a pass.
Discovery tests inject public-URL documents and localhost fixtures only under
`cfg(test)`. Never add a production loopback bypass for fixture convenience.

## Default development checks

The default build includes Zcash. `scripts/zcash.sh` supplies the pinned Cargo
invocation and Cargo-managed protoc; do not append a second `--locked` flag.

```sh
scripts/zcash.sh test --all-targets -- --test-threads=1
cargo fmt --check
scripts/zcash.sh clippy --all-targets -- -D warnings
```

Reduced-feature suites require an explicit request. Run requested feature
configurations sequentially: subprocess tests use the built executable
path. Prefer focused tests for a change, then the relevant broader suite.
`scripts/check.sh` is the full qualification wrapper, including compatibility and
vendor verification. Building dependencies/proving parameters can require network
access; normal runtime fixtures use localhost and deterministic unfunded keys,
never a developer's `.env` or mainnet wallet. Ignored child-process helpers invoked
by their parent tests differ from ignored optional live tests.

## Test boundaries

| Area | Principal tests / contract |
| --- | --- |
| Catalog/config/providers | Generated schemas, selected operations, routing, pricing descriptions, composition and pinned local catalogs. |
| MCP/deployment/discovery | Auth, scope and grants, immutable invocation snapshots, update/remove races, persistent idempotency, cancellation, pagination, shutdown and ownership release. |
| Payments/managed rotation | Real local signing, cap/asset validation, journal-before-disclosure, captured-wallet traversal only after typed insufficient funds, retained chargeable attempts, concurrent admission, exact reserve promotion, independent initial funding, and separately requested canonical resolution. |
| Treasury/store/funding | Encrypted persistence, explicit conversion preflight, snapshot ordering, reservations and qualification limits, uncertain source sends, refund/expiry rules, backups and filesystem protections. |
| Network | Production HTTP/gRPC through authenticated fixture SOCKS, remote hostnames, identity/pool boundaries, proxy failure, TLS, redirects and public-destination enforcement. |
| Output/resource limits | Exact/over-bound behavior, visible errors and logs, no partial cache success, no retry of paid oversized responses, typed inline image/mapping behavior. |

Tests should establish the absence of forbidden side effects as well as the
expected result: no signature for a rejected payment, no publication for a denied
registration, no source send before persistence, no connection outside the network
factory. Use barriers and controlled clocks to expose concurrency boundaries;
sleeps alone do not prove ordering. Retain positive fixture controls so a broken
fixture cannot make every negative test pass.

CLI contract fixtures count unsigned probes and assert inspection output and
credential/error order; stdio and shutdown suites exercise accepted-call ownership.
Recorded-pricing fixtures reconstruct full inventories from session-bound events,
reject route/schema drift and check that only descriptions change. The real startup
regression verifies reconstruction introduces no extra probes.

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
independent and coalesced; failed reads preserve hints and low markers. Funding
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

The narrow conversion utility has synthetic fixtures for strict source-shape and
proof validation, rejection before mutation, repeated/reopened inspection,
exclusive ownership, and committed WAL inspection without changing source sidecars.
The operator-identified real state was inspected read-only and was **not converted**:
legacy completion records lack the required immutable destination-readiness support.
Synthetic checks do not supply that missing proof or qualify a live migration.
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

`scripts/coverage.sh` produces default line, region and function reports with
compiler/source/lockfile provenance. Generated reports stay under ignored `target/`;
failed collection must not replace the latest successful report. Application
filters exclude external/vendor/test/example paths, although inline test helpers
can still contribute to the application denominator. Compare identical feature
sets and filters, and explain denominator changes before attributing a percentage
change to better tests.

Branch collection is an optional separate invocation using a compatible nightly
compiler and matching LLVM tooling: `RUSTUP_TOOLCHAIN=nightly scripts/coverage.sh
--branch`. Stable region coverage is not branch coverage. Optional proving,
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
| Real Tor and confinement | Exercised on macOS with an installed Tor process, authenticated control events and OS egress controls. Local SOCKS tests do not prove this; other operating systems and all-network anonymity remain outside the claim. |
| Funded provider workflow | The reusable runner has exercised treasury bootstrap, a selected provider sweep, help caching, canonical debit reporting and a separate startup-pricing check through Tor. Failed or omitted providers remain failed or unobserved; this is not universal provider compatibility or current health. |
| Shared/separate-wallet concurrency and refill service | Manual live tests exercised overlapping paid calls and continued service during pending refill. Runner assertions have offline coverage; manual evidence does not qualify every automated lifecycle preset. |
| Automated lifecycle/restart | Full confirmed-refill/two-round restart acceptance remains deferred. Promotions or service during treasury insufficiency alone are insufficient. |
| Cover with paid traffic | Unsigned cover checks are separate. A fully successful combined cover/payment qualification remains deferred; range refusal or missing samples cannot be hidden by payment success. |
| Mutation and branch coverage | No mutation score is claimed. Branch collection requires a compatible nightly/LLVM setup; stable region coverage is not branch coverage. |

The [scenario presets](../tests/live/integration/SCENARIO_PRESETS.md) retain explicit
wallet scope, single-use cases, finite authority and observation windows. The
[provider preset](../tests/live/integration/PROVIDER_PRESETS.md) retains 23 providers,
22 independently reviewed paid requests, one unsigned directory call and 40 help
calls. Presets are inputs, not proof of live feasibility or permission to spend.
[Deferred acceptance](plans/deferred/live_integration_acceptance.md) specifies the
remaining lifecycle and cover gates.

## Runner regression map

| Contract | Maintained tests |
| --- | --- |
| Every provider is selected or explicitly excluded | `tests/provider_catalogs.rs::live_provider_scope_accounts_for_every_bundled_definition` |
| Qualification schema rejection order, recursion/node boundaries and shared alternative work limits | `examples/live_integration/schema.rs` tests through public `validate` |
| Exact reviewed requests, caps, schemas and wallet arrangements | `examples/live_integration/presets.rs` and independent request fixtures |
| Authenticated MCP single-attempt fixtures sign exactly once | `tests/server.rs::authenticated_mcp_paid_call_signs_once_and_delivers_the_provider_result`, `tests/payments.rs` |
| Crash/failure cannot replay or refund a charged attempt | Runner registry tests and `tests/integration_preparation.rs` real-process tests |
| Prior permits and liabilities survive new runs | `src/qualification/funding/registry_state.rs` and immutable-baseline tests |
| Complete frozen inventories, typed loading failures and exact snapshot byte boundary including newline | `tests/startup.rs`, `tests/integration_preparation.rs`, `tests/snapshot_boundary.rs` |
| Identity/origin scope and confined egress | Managed/unsigned identity-map tests, `tests/network_audit.rs`, optional `tests/TOR.md` procedure |
| Rotation output, missing evidence, settlement precedence and original replacement-job bindings | `examples/live_integration/rotation_report.rs` tests; fixtures certify report logic only |
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
