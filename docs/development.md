# Development and measurement

See [testing and qualification](testing.md) for test boundaries and acceptance limits.
Commands run from the repository root.

## Portability

Target Linux and macOS. Prefer standard Rust or existing cross-platform
dependencies over native OS calls. Isolate necessary platform behavior behind a
small shared abstraction with equivalent correctness and security contracts.
Remove unnecessary responsibilities before adding abstraction layers.

Do not hide missing implementations behind successful no-ops or silently weaker
permission, recovery or network checks. An unavailable required capability must
produce an explicit error scoped to that operation. Prefer portable fixtures and
verify native behavior before claiming support; compilation alone does not
validate filesystem or process semantics.

Windows has no support claim. Runtime, storage, supervision and native acceptance
remain explicitly deferred in the [release follow-ups](plans/deferred/release_followups.md).
Preserve existing portable improvements; Windows validation is not a prerequisite
for the selected Linux/macOS cleanup.

Inherited macOS ACLs can defeat a mode-only privacy check. The shared private-storage
boundary creates files/directories atomically without inherited grants on macOS,
and validates reopened descriptors. Linux owner-only modes constrain POSIX ACL
masks; default ACLs can remain. SQLite copies the private database mode to its
sidecars. macOS SQLite parents reject file-inheriting grants; non-inheriting parent
grants remain valid. Private files/directories themselves reject granting macOS
ACLs. Inspection errors and
unsupported privacy operations fail explicitly. The [storage assessment](plans/portability_fixes.md)
retains the separate Unix namespace-authority question; privacy checks do not prove
configured-name authority or detect authenticated-state rollback.
Ordinary application support and optional qualification are separate:
[release reproducibility](reproducible-builds.md) and
[real-Tor confinement](../tests/TOR.md) have macOS-specific evidence.

Qualification connectivity probes use Rust ErrorKind to distinguish permission
denial, refusal and timeout. These portable reporting controls do not establish
OS confinement. Strict byte-exact vendor reconstruction and portable test fixtures
remain in use. Current dependency-security evaluation belongs to
[NU7 release readiness](plans/nu7_release_readiness.md); Linux CI automation and
future platform acceptance belong to the [deferred release plan](plans/deferred/release_followups.md).

## Review decisions and unfinished assessments

No blanket SQL/archive, public-visibility or deployment-loader refactor is planned.
Archive INSERT differences preserve one-time versus idempotent transitions and
actual reauthorization; public Rust items have library/test/tooling consumers and
do not themselves expose MCP capabilities; loader differences follow their distinct
authority and failure contracts. Counts and duplication alone do not establish a
functional defect. Inspect real consumers before any future focused change.

Test-hook attribute counts alone do not establish production/test divergence.
A blanket replacement of test hooks with injected traits is not selected work;
inspect the affected production and test behavior before choosing a focused change,
and add an abstraction only when its concrete responsibility justifies it.

The supported compiler baseline is Rust 1.99.1, matching the pinned toolchain.
Release builds enable integer overflow checks throughout the dependency graph;
explicit checked, wrapping and saturating operations retain their semantics.
Financial code must return checked-arithmetic errors rather than rely on panic
recovery. Other hygiene observations require individual evidence;
their presence alone does not establish a functional defect.

No replacement of `fs2` based solely on age, blanket dependency unpinning, source-hash
removal or count-driven `unwrap`/`expect` rewrite is planned. File locks protect
exclusive ownership; dependency pins preserve a reviewed compatible cohort; source
hashes identify release inputs; panic counts alone establish no reachable failure.
Actual dependency-security updates have their own owner below, and concrete bugs
remain ordinary maintenance. The default adjacent wallet key is an explicit
[storage threat boundary](wallet-cli.md), with separate-key storage supported; it
is not a promise of protection from theft of the entire default wallet directory.
The [deferred release plan](plans/deferred/release_followups.md) owns the specific
correction of archive builds' unknown-versus-dirty provenance.

Provider-output assessment and snapshot visibility have concrete owners in the
deferred release plan. Snapshot pruning is not selected work. Current contracts remain in
[tool documentation](agent-sources.md#tool-documentation) and
[source recovery](wallet-rotation.md#funding-and-source-safety).

## Dependency vendoring

Before introducing a new vendored package, tell the user the concrete need,
why upstream releases or application-level alternatives are insufficient, and
the maintenance cost. Logging or diagnostic changes alone generally do not
justify a new checkout; document upstream limitations where practical. Existing
patches are not precedent for adding another. Track necessary patches and their
removal evidence in the [vendor ledger](../vendor/README.md).

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
make build
target/release/x402_treazury --help
```

Bare `make` and `make build` default to release mode and set
`CARGO_INCREMENTAL=0`, including for the protoc helper. Use `PROFILE=NAME`
to select another profile with incremental caches disabled. The Makefile requires
GNU Make 3.81+ and a POSIX shell; see the [entrypoint reference](../scripts/README.md)
for prerequisites and argument forwarding.
Release compilation takes longer, but produces optimized code without debug info
and with integer overflow checks enabled.
This ordinary build is separate from the qualified
[reproducible release workflow](reproducible-builds.md).

Development and test builds use `debug = "line-tables-only"` throughout the
application workspace. This retains file/line
backtraces but omits type and variable information needed for richer debugger
inspection. Enable incremental development builds explicitly:

```sh
make build-dev
# Development profile without incremental caches:
make build PROFILE=dev
```

`make build-dev` selects the dev profile and sets `CARGO_INCREMENTAL=1`.
Build targets select their incremental setting even when a value is inherited
from the shell. Select profiles through `PROFILE`, rather than passing Cargo
profile flags in `CARGO_ARGS`; `build-dev` always uses dev. `test`, `cargo-check`,
and `clippy` retain normal Cargo profiles and respect the inherited environment.
To disable incremental compilation for tests:

```sh
CARGO_INCREMENTAL=0 make test
```

For full debugger information, set `CARGO_PROFILE_DEV_DEBUG=2` with `make build-dev` or
`CARGO_PROFILE_TEST_DEBUG=2` for tests. Profile changes can require dependency
recompilation and leave previous artifacts on disk. Disabling incremental
compilation does not delete existing caches or other dependency/test artifacts.

With builds stopped, the regenerable incremental caches can be removed directly:

```sh
rm -rf target/debug/incremental
```

The next compilation will rebuild any needed cache. Preserve the rest of
`target/`: it also contains retained live qualification evidence, pinned
executables and reports. Do not use blanket `cargo clean` or delete `target/`
as routine disk cleanup.

### Compilation parallelism

Cargo already schedules independent crates and targets concurrently. Its default
job count is the number of logical CPUs; `CARGO_BUILD_JOBS` changes that compiler
budget independently of libtest's `TEST_THREADS` setting. For example:

```sh
CARGO_BUILD_JOBS=8 make cargo-check CARGO_ARGS='--all-targets --timings'
make test CARGO_ARGS='--all-targets --no-run --timings'
```

Cargo writes the concurrency and compilation timeline under
`target/cargo-timings/`. Compare unchanged sources, features, profiles and warmed
inputs before choosing a job count. See Cargo's
[job control](https://doc.rust-lang.org/cargo/reference/config.html#buildjobs) and
[timing reports](https://doc.rust-lang.org/cargo/reference/timings.html).

The application, its integration tests and SDK regression share one lockfile,
workspace profile, patch table and target directory. This permits reuse of
matching compiled units; different features and target kinds can still require
separate units. Keep their Cargo invocations and feature configurations sequential
while validating tests, since CLI fixtures need a fixed executable. Separate
Make commands do not isolate artifacts or bypass Cargo's build-directory lock.

## Verification

Run `make check` for the application, test-feature Clippy, compatibility
checks and the local synthetic-chain treasury lifecycle. Run feature configurations
sequentially because CLI tests share the executable path. See
[development tools](../scripts/README.md).
Individual commands:

```sh
make test
make clippy
make fmt-check
```

The fixed `make check-clippy` stage checks production library/binary compilation
before all-target test-feature Clippy and rejects emitted compiler warnings from
dependencies too. Ordinary `make clippy` only denies warnings in selected targets.
Review successful command output as well as status under
[compiler diagnostics and review](testing.md#compiler-diagnostics-and-review).

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
`make check` runs the audit by default. New or worsening hotspots above CogC
20 or CycC 40 receive advisory review warnings against the merge base with `main`;
scores do not fail the check. Use `--base REF` or `COMPLEXITY_BASE_REF` for another
review target. Incomplete analysis fails visibly. Apply the
[review discipline](../tests/COMPLEXITY.md#review-discipline) to act on warnings;
see [definitions and findings](../tests/COMPLEXITY.md)
for syntax qualification, macro/feature limitations and the independent tool build.

### Local coverage

Install `cargo-llvm-cov` (`cargo install cargo-llvm-cov --locked`) and LLVM tools
matching `rustc -vV` (or `rustup component add llvm-tools-preview` for rustup).
Run `make coverage` for the default Zcash build, all test targets and examples.
It supplies protoc and detects Homebrew LLVM; `LLVM_COV` and `LLVM_PROFDATA` can
select another matching installation. Rust 1.99.1 from Homebrew uses LLVM 23.1.3.

Open `target/coverage/html/index.html`; text and machine-readable summaries are
`target/coverage/summary.txt` and `target/coverage/summary.json`. The denominator
covers application source, excluding integration tests, examples, operational tools and vendored
code; inline unit-test helpers can still count. Stable Rust reports line, region
and function coverage, not branch coverage. Optional proving/Docker and live
funded tests are outside this run. Reports contain local paths and are gitignored.

Use the isolated [condition analysis workflow](../tests/COVERAGE.md#condition-analysis)
for a condition-only baseline/final comparison. It pins the verified compiler revision,
cargo-llvm-cov and matching LLVM tools, preserving condition outcomes and exact
source hashes. Existing coverage entrypoints do not authorize collecting multiple
modes. Follow [testing efficiency](testing.md#testing-efficiency); compare ordinary
timings on unchanged inputs separately from instrumented runtime, and prefer
focused profiles over a suite-wide per-test matrix. Production and normal check
pins remain unchanged.

## Logging and diagnostics

All application logs go to stderr; stdout carries the MCP wire or requested CLI
JSON. Process `RUST_LOG` overrides the default `warn,x402_treazury=info` filter for
every command.

Manual raw-call headers use sensitive HTTP values outside the serializable routed
business-header map. Never log, serialize into previews, cache or persist their
values, including arbitrary innocuous-looking names; validation errors must be
value-free. This diagnostic rule does not redact generic upstream response bodies.
See [manual raw headers](agent-sources.md#manual-raw-headers). These guarantees
apply to application-owned diagnostics. The unmodified `rmcp` SDK can log raw
arguments, malformed frames, results and error prose, including manual auth
headers or upstream credential echoes. Its HTTP transport errors can also expose
parser or body-reader prose. Do not enable verbose SDK logging when handling
secrets; embedded library consumers must also configure their own subscribers.
The executable separately sanitizes stdio initialization failures.

### Log severity

Choose the level from the outcome and scope of the owning capability or logical
operation, after its applicable retry and fallback paths:

| Level | Use for | Examples |
| --- | --- | --- |
| ERROR | Required service or endpoint functionality is unavailable and has no recovery remaining in the current lifecycle. | Program failure; an MCP endpoint cannot bind or cannot publish its required catalogs. Other endpoints may remain healthy. |
| WARN | A requested operation cannot complete under its existing recovery and authority, or functionality is unexpectedly lost even when the remaining operation succeeds. | Document retrieval fails with no usable fallback; a potentially chargeable payment fails; one-pass sync fails; selected catalog operations become unavailable. |
| INFO | Meaningful user-visible lifecycle, progress, delay or recovery changes. | Program startup; catalog ready; MCP endpoint ready; funding progress; entering fallback or waiting for a retrying dependency; recovery of that dependency. |
| DEBUG | Routine outcomes, expected negative results, individual attempts and troubleshooting details. | Ordinary successful calls; optional probes finding no document; no search matches; routine input/authentication rejection; background polling; cache activity. |
| TRACE | Fine-grained execution or protocol detail needed for deeper diagnosis. | Internal transitions and detailed transport sequencing, subject to the same sanitization rules. |

“Unrecoverable” is scoped to the current service lifecycle or logical operation,
not to whether an operator could fix configuration and restart, or a later request
could succeed. An exhausted operation can WARN while its endpoint remains usable;
a terminal endpoint failure can ERROR while other endpoints remain usable. An
operation that cannot safely proceed without new authority or reconciliation has
no automatic recovery merely because another provider exists.

Intermediate failures with an allowed retry or fallback remaining must not emit
WARN/ERROR. Use INFO when they explain a meaningful delay, fallback or degradation;
keep repetitive attempt details at DEBUG/TRACE. Mere elapsed time or repetition
does not promote a recoverable condition. The owner that knows the final outcome
emits the terminal diagnostic; lower layers retain the safe facts needed to
explain it without announcing each failed attempt as an operation failure.
Unexpected catalog truncation or unavailable selected tools emits WARN at the
public catalog construction boundary, including library callers and successful
partial inventories. Log only safe aggregate omission counts and fixed reasons.
Strict conversion of that same build does not repeat the construction warning;
its terminal command/source/endpoint owner may separately report the broader
impact. Internal naming reprojections and speculative parsing do not announce
functionality loss. Ordinary library validation returns errors to its caller,
which owns terminal severity; this does not exempt explicit resource rejection
or public catalog construction from their diagnostic contracts.
Authored source operation/path/tag exclusions designate expected non-exposure
before compatibility checks. They need no loss warning; tool-name/listener filters
cannot retrospectively hide unexpected source omissions. Document compatibility
reasons in provider caveats and revisit exclusions when support changes. Do not add
blanket warning suppression or suppress structural-budget rejection.

The executable reports terminal `serve` failure at ERROR and failed inspection,
administration or other explicit CLI work at WARN. Typed command scope selects
severity; validators do not each log the same terminal failure. Tracing carries
only safe categories and available typed causes, never the raw error chain.
Matching fixed local messages solely to choose a safe diagnostic label does not
grant financial authority. No blanket rewrite of diagnostic-label matching is
planned; permission and recovery decisions must retain their actual typed facts
and established blockchain/SDK evidence.
The existing adjacent `error:` cause-chain display remains CLI output on stderr
and is not a substitute for that severity event. Invalid logging directives use
the default subscriber to report their failure. Clap help/usage handling remains
unchanged. Explicit forced shutdown is WARN with unknown-outcome guidance, not an
unexpected service ERROR, and never releases accepted financial work.

An optional initial Base destination read that fails while funding continues
logs at DEBUG; failure to publish a positive observation still defers preparation.
Catalog structural-budget rejection is an explicit exception: the shared guard
always emits WARN with fixed reason and numeric usage/bounds, even if a caller
catches the error or another catalog candidate remains. Never include catalog
text, submitted names or URLs; import callers do not duplicate that warning.
NEAR header/body timeout attempts also log at DEBUG with a safe stage and reason;
the owning funding or CLI operation determines any terminal severity.

A terminal automatic-funding owner failure emits ERROR immediately, before its
caller handles service shutdown, with a fixed cause category and available typed
SQLite codes, I/O kind or JSON category. It never prints arbitrary error or panic
prose. Accepted financial work stays recorded; this diagnostic adds no restart or
replay authority. A funding-owner panic with existing retry remaining reports its
recovery at INFO; an already-retained owner remains DEBUG.

Retried local SDK reconstruction and completed-transaction publication report
individual failures at DEBUG, with fixed snapshot categories, SQLite error codes
or I/O kinds when available. Arbitrary error prose is never logged. These
retries preserve accepted work and do not imply an operator-required stop.
Store-worker termination is different: its exact handle is supervised and emits
one immediate ERROR with a fixed panic/cancellation cause and affected capability,
without formatting the panic payload into that diagnostic. This does not suppress
the existing Rust panic hook, which may print a closure panic to stderr.
Dead-Store operations fail instead of retrying SQL.
Completed unpublished transactions emit their own ERROR explaining that exact
bytes remain retained and graceful shutdown cannot finish. A forced shutdown can
lose those bytes; the original accepted inputs remain uncertain, never replayable.

The treasury scan owner emits INFO when its existing observation schedule first
changes from successful/unattempted to failed, including scans requested through
commands or pending jobless shielding. Repeated failures remain DEBUG; the existing
successful-scan INFO makes recovery visible and resets the transition. This adds
no readiness gate, scan, retry or financial authority. One-pass CLI callers can
observe this capability transition before their terminal WARN. Shutdown cancellation
does not announce a new recovery state or reset the existing backoff.
Background treasury sync and shielding attempts with automatic observation/retry
remaining log failures at DEBUG, including fixed safe categories and available
sanitized details. Per-address shielding preparation failures must also reach
that diagnostic while healthy SDK ownership permits trying the next address.
Preparation diagnostics identify the preparation stage and a fixed safe reason,
separate from pending-transaction observation. Empty/uneconomic selection,
fee-limit and fee-only proposals remain expected DEBUG outcomes; typed SDK
proposal failures retain only safe categories, never private SDK prose or sources.
Proposal fee refusals retain numeric fee/limit detail at DEBUG; the caller owns
any terminal failure report. Explicit one-pass sync failures remain WARN because that
command has no automatic retry. Unreconstructible original transaction/input
records requiring operator review remain WARN. Current SDK spendability and unfinished transaction observation remain separate.
Missing, moved or spent old inputs cannot veto buried finite expiry of the exact
authenticated original. Established SDK confirmation or finite-expiry closure
requires no indexer corroboration. A wallet scan behind a lookup's newly included transaction
is ordinary pending confirmation. Logs never release financial ownership or
authorize another submission.
SDK chain recovery does not reopen completed source work or settled swaps.
Unrecoverable original evidence for unfinished work needs
the operation identity and a safe failure category; do not discard the cause or
log original signed bytes or credential-bearing upstream messages.

Expected negative results, routine caller rejections and normal cancellation are
not automatically operational failures. Cancellation does not end accepted
financial work or erase its eventual terminal diagnostic. Recommendations and
recoverable degradation use INFO only when useful to the user, otherwise DEBUG;
they do not justify WARN by themselves. Routine successes and per-request
completion messages normally belong at DEBUG/TRACE. Normal capacity waits,
retained observation deferrals and disposable cache eviction belong at DEBUG,
even when they delay work or discard advisory cached data.

Financial uncertainty must remain explicit in terminal diagnostics. A level or a
successful HTTP response never establishes settlement, grants replay authority or
changes recovery policy. All levels obey the same privacy and sanitization rules.

Successful payment-protocol compatibility adjustments, including challenge
description truncation, are DEBUG diagnostics. Actual call failures retain their
terminal WARN and correlated call ID; an adjustment observed during that call
does not by itself establish the cause of failure.

Apply this policy to new diagnostics and correct violations encountered in the
affected development work, including excessive INFO, unclear messages and
inadequate failure detail. Update owning documentation and affected diagnostic
tests with the correction. Existing logs and tests do not establish exceptions;
unrelated work does not require a broad logging audit.

### Message clarity and diagnostic detail

INFO, WARN and ERROR message strings must explain what happened in plain language
that a user can understand without reading source code or decoding internal stage
names, enum variants or error codes. Structured fields supply identities, counts
and other context; they do not replace a meaningful message. Keep implementation
detail at DEBUG/TRACE unless it helps explain the user-visible event or failure.

WARN/ERROR diagnostics must provide enough available, safe detail to identify the
failed operation or unavailable capability, its affected scope, the known cause
or failure category, and the resulting impact and recovery status. Include a
useful next action when established. If the cause or outcome is uncertain, say
so; do not invent a root cause, promise recovery or imply that a payment is safe
to repeat. Requirements scale with the failure: include relevant facts rather
than a fixed inventory of fields on every event.

Diagnostic context may be in the message, structured fields or an explicitly
referenced, correlated WARN/ERROR event. For example, an endpoint ERROR can name
the unavailable endpoint and point to its source's catalog WARN for the cause.
The basic explanation must remain available at WARN filtering; DEBUG/TRACE may
support deeper investigation. Preserve existing sanitization contracts: sufficient
detail means safe reasons, status codes, stages and correlation, never credentials,
upstream prose, raw error chains or other prohibited content.

### Diagnostic context

Catalog file reads, origin fetches, cache configuration/policy, cache hits and
writes, shared downloads and retained catalog-load handoffs are DEBUG. These
implementation details do not announce readiness. Per-source catalog readiness,
initial loading summaries and endpoint waiting/ready transitions remain INFO.
When remote loads remain pending, the initial summary says catalogs are still
loading and waiting endpoints will start when their required catalogs are ready;
it does not announce that startup discovery has finished.
Catalog cache-hit messages confirm a readable JSON document; tool generation and
listener validation must still finish before a source is ready. Catalog failures
report the authored source ID, `failure_stage`, a fixed plain-language `reason`,
and numeric `http_status` when available. Shared download failures retain the same
typed cause and diagnostics. Stages distinguish configuration, acquisition,
JSON parsing, tool generation, listener selection and publication. Reasons can
identify unsupported header contracts or invalid provider tool filters without
printing header names from the document, schemas, URLs, upstream prose or raw
error chains. A failed source stays unavailable; a required source failure emits
ERROR for each dependent waiting endpoint, identifying the source and directing
the operator to its catalog warning. Publication failures retain the existing
inventory for ready endpoints and fail endpoints still waiting for catalogs.
For failures without potentially chargeable work, correct the reported cause and
restart serving to retry a failed startup catalog. A possibly charged acquisition
instead retains explicit payment uncertainty and the prohibition on automatic replay.

Unsigned document acquisitions carry an ephemeral `document_id` and trusted
`request_kind`, shared by origin and paid relay spans. Origin failures report safe
HTTP status, a plain-language reason, direct/Tor route, typed failure/transport
categories and final attempt/maximum counts. `relay_fallback_eligible` describes
the origin failure, not configured relay availability, payment authority or
successful recovery.
Explicit document origin failures and selection of their configured paid fallback
are INFO. Catalog, API and directory-search fallback owners also report selection
at INFO. Selection does not promise a new request, payment or successful retrieval:
preparation, existing authority, readiness and possible cached work still apply.
Individual relay exchanges, unsigned relay failures and routine payment completion
are DEBUG. Optional discovery probes keep their acquisition and fallback detail
at DEBUG, including successful paid relay calls.
The complete document acquisition emits WARN only when no usable paid fallback is
available or retrieval ends in failure after fallback. This includes lazy source
refresh preparation and authority loading failures. Speculative OpenAPI, directory
and documentation URL probe failures remain DEBUG, including missing paths and
blocked responses. Enable `RUST_LOG=warn,x402_treazury=debug` for these probe
diagnostics; oversized origin candidates also use DEBUG with resource, setting
and byte limit, while terminal explicit document rejection retains WARN. The tool
response retains per-URL outcomes. No URLs, hosts, arguments or upstream error
prose are logged. Terminal ordinary payment failures and terminal potentially
chargeable relay failures retain WARN and possible-charge facts, including when
an optional probe's payment fails. These levels do not grant retry or payment authority.
Payment WARN events retain fixed provider-reported categories for settlement
failure, invalid request bodies, temporary lookup unavailability and failed inline
data refreshes. They include
the last observed payment-challenge/receipt header state (absent, duplicate,
malformed or decoded object), allowlisted x402 error codes, an optional seller
success claim, and the failed response body's shape, including empty objects.
Header facts describe the last response, which can precede a later signed transport
failure; seller claims never establish settlement or replay authority. Unknown
codes are reported as unrecognized without their values. The exact code vocabulary
includes the pinned Rust SDK and the public x402 v2 specification's error codes;
substring matching of provider prose is never used. Omitted managed-payment
extensions accumulate known-category flags (Bazaar, builder code, payment identifier,
Sign-In-With-X and other) across payer attempts on the same call, including signing
or transport failures. Neither omission nor HTTP status proves the root cause.
Upstream messages, arbitrary extension names, request IDs, credentials, request
arguments and response bodies remain excluded at every application log level.
Internal failed-call errors retain the upstream error body and the unique
payment receipt's string-valued `errorReason` and `errorMessage`, even when the
code is not recognized for logging. Other receipt fields are not copied into the
error detail. Duplicate receipt headers across both aliases, malformed headers
and non-string fields supply no private receipt detail. MCP API-error presentation
uses typed categories and status facts rather than this diagnostic chain. This
ephemeral error projection is untrusted provider prose and must never be logged; it grants no
settlement or replay authority, including when a provider reports `charged:false`.
The Coinbase directory search owner reports a terminal WARN after origin and
fallback recovery ends, including safe cause/status and possible-charge facts;
individual unsigned relay failures remain DEBUG.
A successful relay HTTP call does not establish usable target documentation or
settlement; consumers still validate the document.

Source distribution selector failures retain the selection stage and a safe reason
identifying the source's `send_tools` rules in both immediate and deferred catalog loads.
A readable catalog with invalid distribution is not a network acquisition failure;
detailed selectors remain in the inspection error chain, outside serving logs.
Endpoint selection diagnostics distinguish a missing exact listener selector from
an empty inventory without discovery capabilities. A reserved static namespace has
a distinct source-admission reason. Serving logs and endpoint status use fixed
reasons with repair guidance; detailed selector/routing evidence stays in inspection
errors.
Deployment `send_tools` wildcards, shadowed distribution rules and final listener
wildcards use advisory DEBUG diagnostics. They do not indicate a failed operation
or endpoint and must not emit INFO/WARN merely because they match no current
tools. Missing exact selectors remain selection errors.

A `tools/list` request with more than 500 advertised definitions emits an advisory
WARN whose message states the endpoint name and current tool count and suggests
`discover_on_demand = true`, without URLs, descriptions or credentials. It observes
advertised inventory, not model context; each listing may repeat it. It does not
reject work, change mode or perform network/wallet work.

An unreachable configured local Tor SOCKS endpoint during serving startup emits
a shared startup-wait diagnostic with the local address, safe I/O kind, retry
cadence and reminder interval. While startup recovery remains pending, the
condition is INFO; repetitive probe details belong at DEBUG. The existing probe
schedule and reminder interval do not grant terminal failure severity. Recovery
emits INFO describing TCP reachability, without asserting Tor bootstrap or remote
service health. No upstream prose, credentials or destination URLs are included.

Managed extension omission is diagnostic detail (`bazaar` discovery metadata also
remains DEBUG). It reports challenge preparation before offer validation or
signing, not a failed payment. Payment-call spans carry an ephemeral `call_id`, configured source and
HTTP method; catalog calls also carry tool identity. Dynamic catalog calls use
their registered source identity. Direct managed-library calls have their own
call ID without claiming a configured source. No request URL, arguments, raw
challenge, extension values, signed bytes or upstream error prose is logged.

Accepted payment work logs its terminal result inside its retained owner, even
after caller cancellation. Routine completion belongs at DEBUG; terminal failure
is WARN. Warnings include a fixed plain-language reason derived from the typed
transport or payment category, with explicit uncertainty when no more specific
cause is available. Provider error detail is never formatted, even when stored in
a static string. A completed error body with the exact structured JSON
`error.code` value `payment_settlement_failed` adds a fixed `provider_failure`
category and explanation. This records an unverified provider claim, without
logging its message or granting replay authority; other body text remains
unclassified. An earlier unsigned HTTP status cannot explain a later signing
or output failure by itself. Diagnostics retain a fixed failure kind, execution
stage, unsigned attempt number/maximum when available, and
`authorization_may_charge`. Counts describe dispatches in the latest payer's
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
diagnostics cover returned response headers, while body consumption remains the
caller's responsibility; expected negative statuses follow the severity policy
above. Diagnostics
never authorize replay or change payment execution.

Optional cover diagnostics distinguish fresh range failures from cached refusal
and episode shortfalls. Capability loss or fallback that explains meaningful
user-visible degradation belongs at INFO; ordinary shortfalls, cached refusals and
individual attempt facts belong at DEBUG. Optional cover degradation alone does
not establish failure of an agent operation. See
[cover diagnostics](cover-traffic.md#cover-diagnostics) for fields, deadline
cleanup and safe resource labels.

Disposable HTTP cache read/write failures retain complete origin responses. Their
WARNs identify the operation, safe filesystem kind or SQLite code when available,
or explicitly unavailable local-validation detail. Worker panic/cancellation uses
fixed cause text; paths, database error prose and response content are excluded.

An unexpected balance reconciliation task exit reports its pending retry at INFO
with the affected pool; routine observation attempts remain DEBUG.

Each funding owner first logs a job's saved/current local phase explicitly as
such; later phase changes use a separate progress message. Neither implies a
fresh provider observation. Each validated NEAR status enum logs at DEBUG with
its operation correlation. Observation unavailable/recovered transitions use INFO
and existing job records; repeated failures remain DEBUG. Terminal provider
outcomes log only after persistence. `FAILED` does not prove no funds were
received, and `REFUNDED` is a provider claim, not an SDK-observed refund receipt.
Quote diagnostics at DEBUG may include validated quote IDs, swap mode, amounts,
execution floor, slippage, estimated completion time and quote age. Status refund
reasons use a fixed whitelist. Neither estimated time nor a quoted output floor
establishes faster execution, a smaller route minimum or refund-free settlement.
Do not log unparsed refund amounts, addresses or remote prose. These diagnostics add no
persisted state, polling work or financial authority.

Waiting for existing ZEC to become spendable and preparation deferred by unresolved
source work use DEBUG. After an insufficient-funds rejection from SDK input
selection, an incomplete current-owner scan reports at INFO that funds could not
be assessed, without deposit advice. Pending SDK transactions produce a DEBUG
wait; they do not establish that enough funds will become available. Otherwise,
SDK-observed confirmed funds, including dust and unreserved transparent funds
(also immature confirmed coinbase outputs), at least equal to the SDK's required
amount produce a DEBUG spendability wait. Only an observed
shortage produces an INFO message with the existing shielded receive address,
asking the operator to add ZEC and allow confirmations. Identical repeated
assessment failures and shortages remain DEBUG. These explanations add no new
spending checks or polling; SDK input selection remains the spending authority.
If receive information is unavailable, it directs the operator to `wallet status`
with the serving configuration. Address lookup does not derive a new address or
change funding authority; the address is not added to persisted failure categories.
Their retry backoff and stored failure conditions remain intact.
Treasury sync diagnostics retain safe connection reasons and direct/Tor route
details when available. Recoverable background failures use INFO for the affected
recovery state and DEBUG for individual attempts. Background diagnostics report
`automatic_retry=true` and `retry_after_seconds` from the existing observation
schedule (90, 180, then 300 seconds after consecutive scan failures). Retry
continues while serving and sync is needed; SDK reconstruction can delay it.
One-pass `wallet sync` warnings report `automatic_retry=false` and instruct the
operator to rerun the command. Pepper Sync fetch spans are debug-only and do not
emit raw returned errors; the sanitized application diagnostic owns failure
reporting. Base RPC attempts and successful provider failover are DEBUG events
with provider ordinals, including an exhausted view acquisition. The requesting
workflow owns its outcome: retrying retirement reports a meaningful delay at INFO,
while an operator-required stop warns once with the affected wallet/group and a
safe record-failure category for unreconstructible original records or authority.
Recoverable canonical-history changes and derived checkpoints continue through
the existing observation schedule; they do not request manual repair. SDK/indexer
disagreements retain useful height and scan context while further observations
resolve them. A true record failure preserves obligations and asks for inspection;
later checks of the same stopped request do not repeat the warning.
Malformed optional historical balance support is retained as disputed audit data;
fresh canonical proof can replace its current role without an operator-only stop.
Diagnostics must distinguish that recovery from malformed original authorization
identities or missing accepted bytes, which current chain reads cannot recreate.
Configured discovery relay selection and meaningful paid-recovery transitions
use INFO at the owning operation. Individual relay exchanges, unsigned relay
failures and payment completion use DEBUG. Other unsigned API and catalog/pricing
Tor recovery messages use INFO when they explain user-visible delay, with attempt
and maximum counts where available; repetitive attempt details belong at DEBUG.
The complete acquisition owns terminal failure warnings.

Sanitize Zingolib errors before logging or persistence. Retain fixed categories
and typed reasons, numeric gRPC codes and fixed status names when available, never
upstream prose, metadata, wallet identifiers or credentials. Shard-tree diagnostics
retain the fixed pool and operation labels from the pinned Pepper Sync patch.
Keep ordinary CLI errors legible even with backtrace environment variables enabled.

## Dependency compatibility

The application pins MCP SDK 3.9.0 and uses direct SHA-2 0.11, alongside the
stable Zingolib v8 cohort. Protocol cancellation/authentication/output tests and
stable identity/hash vectors qualify those consumers. The protoc helper resolves
its independent graph separately; provenance-only vendor lockfiles are unchanged.
The development analyzer retains its qualified Tree-sitter 0.20.9/grammar cohort
because its analyzer API is incompatible with an independently selected 0.27.

### Security update boundaries

The root lockfile selects Tokio 1.54.1 and ctutils 0.4.8, containing the fixes for
[Tokio I/O safety](https://rustsec.org/advisories/RUSTSEC-2026-0386.html) and
[constant-time mask generation](https://rustsec.org/advisories/RUSTSEC-2026-0385.html).
Preserve those fixes when resolving dependencies. Tokio 1.54.0 is yanked for a
separate [timer race](https://rustsec.org/advisories/RUSTSEC-2026-0384.html);
the 1.54 series requires 1.54.1 or later.

Alloy's signer/network cohort is 2.5.0, alongside core crates 1.7.3. The application
declares Rust 1.99.1 as its supported baseline, matching the pinned build
toolchain. This is a support choice, not evidence that every older compiler fails. The independent payment vectors and SDK/payment integration tests
qualify this combination.

The [crypto-bigint timing advisory](https://rustsec.org/advisories/RUSTSEC-2026-0387.html)
still matches `crypto-bigint` 0.5.5 through `elliptic-curve` 0.13.8 and `k256`
0.13.4. Alloy 2.5.0 requires that cohort; forcing an incompatible 0.7.6 override
is unsupported. The previous `PrivateKeySigner` path performed affected integer
arithmetic on signing secrets through k256 scalar reduction and nonce inversion.
Production EVM key generation, public-key derivation and signing now use the
existing native `secp256k1` dependency (libsecp256k1), through the shared
`payment::signer::EvmSigner` adapter implementing the SDK's `SignerLike` trait.
It covers static v1/v2 exact and upto clients and managed funding/payment keys;
raw 32-byte key storage and authorization formats remain unchanged. Each adapter
owns its private key and erases that storage in place on drop using the dependency's
supported API; decoded/exported buffers are zeroizing. This is best-effort erasure:
Rust moves/copies and native internals can leave other copies, as the dependency
documents. Alloy's opaque native credential lacks in-place erasure, so it is not
used to hold production secrets.

The residual k256 cohort handles public signature parsing/recovery in those paths;
this is a scoped mitigation, not removal of the affected package or a universal
constant-time claim. Do not convert production signers back with `to_k256` or
`into_k256`. The independent Python vector must match the production signature
exactly, alongside address/key roundtrip, invalid-scalar rejection and full payment
compatibility checks. Any new secret consumer requires renewed exposure analysis.
Complete graph findings and release disposition remain owned by
[NU7 release readiness](plans/nu7_release_readiness.md); Zcash compatibility alone
does not resolve signing or build-only TLS advisories.

### Reproducible local advisory evaluation

Use the [local advisory tool](../scripts/README.md#local-dependency-advisory-assessment)
with an explicitly pinned RustSec checkout to assess both active lockfiles and
all path/Git package sources. Preserve its lockfile/source hashes, original-source
mapping and raw reports under ignored `target/`. Ordinary RustSec lockfile scans
skip nonregistry packages, so their absence from the standard report is not a
clean assessment. The supplementary normalized inventory matches advisory package
names and exact versions; review the actual source and local patches to determine
applicability. Preserve separate runtime, build-only, optional-test and protoc-helper
dispositions. Provenance checks and compatible compilation do not resolve findings.

### Locked-graph advisory disposition

The local replay covers both active lockfiles and explicitly mapped path/Git
sources. It remains a findings report, not a clean-graph claim. The current
remaining vulnerability matches have these scopes:

| Match | Consumer and disposition |
| --- | --- |
| `crypto-bigint` 0.5.5, RUSTSEC-2026-0387 | The production secret-signing path is replaced as described above; residual public parsing/recovery remains in the upstream EVM cohort. Keep the advisory visible and recheck any new secret consumer. |
| `rustls` 0.21.12, RUSTSEC-2026-0346 / 0349 | Build-only `minreq → zcash_proofs(download-params) → zingolib` parameter acquisition. The downloader uses the built-in session store, not custom session-secret Debug logging. Its old classical key exchange provides no post-quantum guarantee. Runtime transport uses patched rustls 0.23.46. |
| `rustls-webpki` 0.101.7, RUSTSEC-2026-0098 / 0099 / 0104 / 0382, and `webpki-roots` 0.25.4, RUSTSEC-2026-0381 | The same public-parameter downloader retains legacy certificate verification/roots. It configures no CRLs; the CRL-parser panic has no identified consumer. Name-constraint and emSign verification risks remain in that build-time path. Accepted parameters still require upstream fixed lengths and BLAKE2b digests. |

The supported release workflow verifies prefetched public parameters and denies
build network access. Ordinary online first builds can still suffer unavailable
or intercepted downloads; the digest check does not fix TLS. Retiring that graph
requires an upstream downloader migration or separately reviewed patch, rather
than incompatible version overrides. See [SDK parameter acquisition](zingo-integration.md#build-time-parameter-downloads).

Maintenance notices are also retained: `derivative` through `ark-ff`, `fxhash`
through the development analyzer, `json` through Zingolib, `paste` through Alloy/ark,
and `pin-utils` through `wasmtimer`. These are upstream responsibilities with no
compatible lockfile removal established. Do not vendor replacements merely to
suppress informational findings. Reassess them with their owning cohort updates;
any reported unsoundness or vulnerability needs its own exposure analysis.

### TLS security updates

The application and its SDK/payment integration tests use the root lockfile;
update that lockfile for runtime TLS security fixes. For the
[emSign intermediate-key compromise](https://rustsec.org/advisories/RUSTSEC-2026-0382),
runtime verification requires `rustls-webpki` 0.103.16 or later in the 0.103
series, together with `webpki-roots` and `webpki-root-certs` 1.0.10 or later.
The verifier rejects compromised intermediate public keys; the root-bundle
update removes the affected emSign root. Host trust-store updates remain
necessary for platform verification. Rebuild and replace deployed binaries to
consume locked dependency changes. Vendored upstream lockfiles are provenance
inputs, not the dependency resolution used by the application workspace.

The build-only legacy TLS dependency and parameter-integrity boundary are
documented in [the Zingo integration guide](zingo-integration.md#build-time-parameter-downloads).
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

The root workspace includes the application and complexity analyzer. Ordinary
commands select only the application through `default-members`; the analyzer is
selected explicitly for development checks. Vendored dependencies remain
nonmembers. All application tests use the root lockfile, patches, profile and
build cache. The SDK fixtures in `tests/sdk_compat.rs` exercise public SDK
restore/address continuity alongside Ethereum payer construction, and actual
three-pool merge conflicts through public sync traits. Payment signing, MCP server
and independent cryptographic-vector tests remain in their integration targets.

The merge fixture in `tests/support/sdk_merge.rs` preserves typed Sapling, Orchard
and Ironwood `ScanMerge` conflict context, retry refusal and absence of fetch
requests. Pepper exposes a constructor for the payload already accepted by its
public sync trait. This keeps the regression in the application test graph without
building Pepper's separate unit-test executable or enabling its development
features. Run the fixture directly with:

```sh
make test CARGO_ARGS='--test sdk_compat' TEST_ARGS='sdk_merge::scan_merge_conflict_preserves_pool_context --exact'
```

Run source verification and the combined suite with the reviewed vendored patches:

```sh
python3 vendor/verify.py
make check-compat
```

`check-compat` verifies the retained SDK source hashes, obtains Cargo-managed
`protoc` through `tools/protoc`, and selects the `sdk_compat`, `payments`, `server`
and `crypto_vectors` test targets with the root lockfile. These targets also run
in the complete default suite. First builds fetch Cargo dependencies and zingolib's public Sapling proving parameters;
zingolib embeds those parameters from the standard cache during its build. Tests use temporary
offline wallets and localhost payment servers. No wallet sync, funded payment, approval or broadcast
occurs.

The [vendored inventory](../vendor/README.md) records each remaining patch's
current reason, consumers, removal evidence and source-verification scope.
The shared graph uses unpatched Alloy core 1.7.3 with the pinned Zingolib v7 cohort:

```text
Alloy → sha3 0.11.0 → digest 0.11.3
Zingolib → bip32 0.6.0 → hmac 0.13.0 → digest 0.11.3
```

`lightwallet-protocol` 0.4.0 supplies the required Ironwood fields without a
separate root Git override. All workspace packages use ordinary upstream Zingolib/netutils
Git sources at the same pinned revision, with no local checkout overrides. The
[Zingo integration](zingo-integration.md) uses public RPC traits, direct Pepper sync
and backend calculation over the application transport. Pepper's retained patches
own progress/drain behavior and sanitized diagnostics. Upstream v7 handles
same-height reorg truncation and synthetic branch hashes; the application keeps
its lifecycle regressions. HTTP transport patches preserve the
[network inactivity/cancellation contract](network-egress.md#timeout-budgets-and-concurrency).
`vendor/verify.py` checks HTTP transport sources; `verify_pepper.py` checks the
remaining Pepper snapshot, patches and copied-code notice, then reverses the
patches and compares its complete tree with upstream file hashes. Upstream license
texts are included. Deliberate source/extra-file mutations must still fail that
verification. SDK Git revision, tags, dirty status and actual generated descriptor
are release inputs; see [release qualification](reproducible-builds.md).

The independent hash vectors are regenerated with
`uv run --script scripts/generate_crypto_vectors.py`; normal Rust tests consume
the committed vectors without Python. The combined suite runs the shared
payment/MCP/crypto targets plus offline wallet tests. The root manifest and
lockfile select one dependency graph for all of them. These checks use temporary
offline state. SDK upgrade coverage includes a public
v6 snapshot, retained derived addresses and next-address continuity under v7.

This focused compatibility suite tests dependency coexistence and offline wallet
round trips. The application suites separately cover sync, proving and consensus
recovery. Neither suite qualifies mainnet NEAR settlement. All tests use the
reviewed vendored patches; no reference checkout is required. The
[wallet architecture](wallet-rotation.md) describes treasury ownership,
admission, funding, persistence and recovery.

`x402-chain-eip155` also requires its `telemetry` feature with `client` in this
release: the `upto` implementation otherwise references an unavailable
`tracing` dependency. The application enables both.
