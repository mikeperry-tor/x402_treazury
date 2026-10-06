# Live integration reference

Start with the [provider walkthrough](INTEGRATION.md). This reference describes
case assertions, evidence interpretation and advanced execution safeguards.

[Scenario presets](integration/SCENARIO_PRESETS.md) provide matching deployments
for single-wallet smoke, shared/separate-wallet concurrency, rotation/refill,
two-round lifecycle, treasury-only bootstrap, unsigned discovery and cross-day
reliability. All calls and budgets are explicit and placeholders block execution.

The [reviewed provider preset](integration/PROVIDER_PRESETS.md) supplies 63 explicit
cases covering the established 23-provider baseline: paid requests, unsigned help
fetch/hit pairs and a keyless directory call. Its placeholders prohibit execution
until deployment, treasury, authority and windows are reviewed. It retains known
body-quality failures and the AgentUtility manual-review gap.

## Case selection

`select-cases` writes a new manifest from an already reviewed provider-sweep,
unsigned or reliability manifest, without catalogs, wallet keys, registry access
or requests. It resolves local TOML composition and validates both plans. Use a
new run ID and an existing owner-only output directory:

```sh
cargo run --locked --example live_integration -- select-cases \
  --manifest private/reviewed.toml --run-id selected_providers \
  --source socialfetch --source brazilayer --exclude-flagged \
  --output private/selected.toml
```

`--source` and `--case` include their union; with neither, all reviewed cases are
candidates. `--exclude-source`, `--exclude-case`, `--exclude-reliability-tag` and
`--exclude-flagged` then exclude matches. Names are exact, case-sensitive source
IDs/case IDs/configured tags, never inferred from tool prefixes; typos fail. Tag
names must occur on at least one source represented in the input manifest.
The JSON output lists every omitted case and reason, removed empty phases, hashes
and the selected reservation total. Preserve it with your review evidence.

Selection preserves requests, assertions, case order, windows, all wallet/pool
bindings, funding limits and registry authority. Empty output, removed dependencies,
invalid reliability groups, and lifecycle/concurrency inputs fail. The output uses
resolved absolute paths so moving the file does not silently change deployment
inputs. It never overwrites files or resets registry reservations. Review `plan`
and prepare/authorize the new run through the ordinary flow before execution.
This filters **calls only**: unselected sources still load, pricing checks still
apply, and all declared managed pools retain their configuration. To exclude a
provider from startup, author and review a separate deployment explicitly.

## Provider content assertions

Provider content is assessed separately from MCP delivery and payment settlement.
Declare reviewed checks on each case when data semantics matter:

```toml
[[cases.checks]]
kind = "json_exists"
pointer = "/results"

[[cases.checks]]
kind = "json_equals"
pointer = "/degraded"
value = false
```

`text_contains` with a nonempty `text` is also supported, for example for a help
document. JSON checks use one unambiguous JSON body from structured content or
parsed MCP text blocks; identical copies are deduplicated. They do not parse
arbitrary strings nested inside JSON as new documents. Multiple different JSON
bodies or unsupported content yield `manual_review_required` for JSON checks.
An object containing boolean `isError: true` anywhere in parsed JSON is an
observed semantic failure, even if the outer MCP result succeeded. An ordinary
`error` property has no automatic failure meaning. Use reviewed checks for
provider-specific degradation indicators.

Provider-sweep and reliability cases require declared content assertions to pass
their semantic gate. Other scenarios retain their MCP/payment assertion scope
when no provider checks are declared; detected nested errors still fail. Missing
required content checks are incomplete, not successful. Failed/manual required
checks block dependent phases, while independent phases continue. Reports evaluate
the retained, hash-checked response without rewriting its original MCP result or
payment state, including for older runs. Validated unused rotation suffixes remain
not executed. Interpretation does not purchase a replacement call.

Checks are bounded to 32 per case and 4096 serialized bytes each. Content scanning
is bounded to 1000 MCP blocks, 100000 JSON nodes and 64 levels; a reached bound is
explicitly reported as unassessed with the limit category, never silently clipped
or treated as successful. Markdown shows safe categories without provider prose.

## Offers and fees

Single-run reports include `fee_observations`: observed challenge offers, sampled
admitted Base USDC amounts and independently verified canonical debits, with
separate strict `> 10000` atomic ($0.01) flags. Challenge observations precede
selection and therefore include rejected or unselected offers; they grant no
payment authority. Admission is pre-signing evidence, not proof of spending.
Missing observations remain null; invalid application correlation withholds offer
and admission amounts. These samples neither establish provider-wide maxima nor
prescribe a minimum wallet size. Markdown uses anonymous case numbers.

Challenge capture records at most two observations per case (the initial challenge
and a permitted unsigned payer-change retry), at most 65536 encoded header bytes
and 128 offers per observation. Reaching a bound emits a warning and explicit
report evidence, never a partial offer list. Duplicate/malformed headers and absent
headers are separate outcomes. Headerless legacy body offers remain unobserved.
Only amounts, fixed scheme/asset classifications and a header hash are retained;
provider prose, extensions and payee addresses are omitted. Case/session/order
checks bind observations to application claims before admission/completion. This
instrumentation cannot select an offer, sign, retry payment or change reservations.

## Help cache assertions

Unsigned help cases may set `help_cache = "fetch"`, `"hit"` or `"shared"`.
Preparation verifies that the selected tool is actually a help tool. `fetch` means
this call ran the initializer, `hit` means the cache was ready at invocation, and
`shared` means an initially empty cache was populated by another call. A successful
response with the wrong cache path fails the assertion. Missing completion or
invalid correlation remains incomplete and blocks dependent phases. These checks
are independent of optional content `checks` and never authorize another request.

For example, add these reviewed cases and an unsigned phase to an existing
manifest with a `main` window and a selected SocialFetch help tool:

```toml
[[cases]]
id = "docs_first"
server = "b"
source = "socialfetch"
tool = "socialfetch_help"
arguments = {}
reserve_usdc = "0"
reviewed_read_only = true
unsigned = true
help_cache = "fetch"

[[cases]]
id = "docs_cached"
server = "b"
source = "socialfetch"
tool = "socialfetch_help"
arguments = {}
reserve_usdc = "0"
reviewed_read_only = true
unsigned = true
help_cache = "hit"

[[phases]]
id = "documentation"
window = "main"
cases = ["docs_first", "docs_cached"]
pools = []
required = true
[phases.scenario]
kind = "unsigned"
```

Sequential calls in this phase use the same process cache. Application restart
or a new reviewed run starts a new cache; do not expect an old process's hit to survive.
Concurrent `shared` behavior must be observed, not assumed from parallel dispatch.
A failed download is not cached and a later separately reviewed case may fetch it;
the driver does not replay the failed case. Reports retain retrieval status,
HTTP failure code when available, cache path and output byte count without the
document or URL. An interrupted lookup retains a start without a fabricated result.
Help cache observations do not establish Tor isolation or connection reuse.

## Startup pricing assertions

Startup pricing appears separately in `pricing_stages`, once per source and
application session. A completed discovery records selected/skipped/eligible/capped
endpoint counts, cache initialization/hit/shared counts, expired results, available
prices, fixed outcome categories and numeric HTTP codes. Cached failures retain
their original outcome; a cache observation is not a fresh HTTP request. Transport
failure, non-402 status, missing/malformed challenge and unusable first offer stay
distinct. Aggregate stage summaries contain no URLs, challenge prose or provider
response bodies. Private runtime events additionally retain constructed price lines
and their method/path keys to reproduce expected tool descriptions; treat the raw
JSON as private. These are not complete challenge bodies.

Optional manifest assertions apply to every application session in that run:

```toml
[pricing_checks]
api_without_probes = "disabled"
api_with_empty_discovery = "empty"
api_with_discovered_prices = "nonempty"
```

Use actual source IDs and omit assertions that the run does not need. `disabled`
requires the pinned source policy to disable probes; `empty` and `nonempty` require
enabled discovery with zero or positive available prices respectively. An empty
map may result from ineligible tools, an endpoint cap, failures or expiry; its
counts explain which occurred. Passing an empty-map assertion does not qualify the
provider's health. Source/session mismatch or inconsistent counters fail validation;
an interrupted start or missing session observation remains incomplete. These
assertions affect the final result, not payment authority, and never enable probes,
refresh cache entries or buy another API call. Live payment challenges remain the
only input to payment admission. Fresh application sessions have fresh caches.

The report bounds pricing rows at 10000 source/session pairs and explicitly refuses
a larger projection instead of omitting rows. A source's private pricing event is
limited to 16 MiB; exceeding it logs a warning and fails publication explicitly. Unselected sources can have no
pricing stage; request an assertion only for a source that will be priced. Catalog
inspection (`--check`/preparation) still does not probe. The real-executable suite
in `tests/integration_preparation.rs` shares the runner entry code in
`examples/live_integration/driver.rs`, preventing module/CLI drift.

## Report output

For a shareable offline summary of a single retained run:

```sh
target/debug/examples/live_integration report --state-dir /private/state \
  --run RUN_ID --format markdown --output /private/new-report.md
```

JSON remains the default and contains private evidence. Both formats use the same
registry report, including validated canonical API debit amounts by case. Markdown
uses numbered cases/assertions in JSON order and omits identifiers, addresses,
transaction references, paths, credentials and provider content. It distinguishes
selected-run charged reservations, registry-wide reservations and verified debit
totals. Missing proofs are not zero-cost claims; invalid proofs suppress totals.
Files publish atomically and cannot overwrite earlier evidence. Unknown statuses
are visibly unclassified, and size/row bounds refuse the report without truncation.
Tor/cover qualification and actual treasury fees/balances still require their own
evidence; this projection does not assert overall suite success.

Single-run JSON includes a versioned `summary` built from the same validated facts
used by the detailed report. Its provider rows keep catalog, help, pricing,
execution, MCP success, provider semantics and canonical payment separate. Missing
proof amounts are null, never zero. Pricing retains all HTTP-status/outcome counts:
a completed discovery stage may still contain failed probes. Empty or unavailable
observations stay explicitly not-attempted, unobserved or incomplete.

Markdown leads with the corresponding numbered provider-stage table and then the
detailed accounting. Private identifiers and provider prose remain omitted. Older
archives without full source inventories are labelled incomplete and show known
case sources only. Source accounting is a saved observation; source/job reservations
are budget bounds, not fees. Lifecycle results retain their original validators.
The network field states direct, external proxy-only or owned-control-audit-required;
use `audit-tor` for authenticated control evidence. Tor-only failures cannot establish
Tor causation. Reporting makes no requests and grants no execution authority.

## Execution and existing-pool adoption

`examples/live_integration.rs` provides offline planning and a durable registry.
It prepares frozen catalogs and supervises registry-reviewed MCP cases through
a pinned executable. Static deployments are strictly unsigned. Managed public-swap
deployments may use existing funded wallets with zero new funding authority, or
bootstrap from the treasury with explicit positive authority and `run --allow-funding`;
the child installs registry checks before pool setup and records payment admission
before signing. MCP results alone leave settlement PENDING. The runner observes
durable canonical authorization outcomes (USED or EXPIRED_UNUSED) after batches
and shutdown. A generic historical RESOLVED row is insufficient. NOT_SIGNED
requires application completion and drained store ownership, so in-flight
journaling cannot be mistaken for an unsigned result. These observations do not
establish seller receipt validity or the exact transferred fee. Managed signed
responses also journal a bounded seller receipt classification and transaction
reference, correlated to the admitted case. Missing, duplicate, oversized,
malformed, rejected and successful receipts remain distinct from transport
uncertainty. Successful claims must name Base and any supplied payer must match
the admitted wallet. Signatures and vendor error prose are never retained; a
header hash supports private correlation. These are unverified seller claims,
not transaction-log verification.

`report --run RUN_ID --eligibility` inspects case windows and the unstarted-run gate without execution. A started or reserved run can never be
launched again, even when some cases remain untouched. `report` and `audit-tor`
can inspect historical continuations; those records grant no execution authority.

To run untouched work after a stopped session:

1. Inspect its report and canonical payment/source accounting. Never copy reserved,
   dispatching, uncertain or completed cases into a retry manifest.
2. Make a new reviewed manifest with a new run ID and case IDs, retaining the same
   treasury, registry and all configured pools. Use `start.mode = "funded_pools"`
   and zero new-job/source authority unless separately reviewing new funding.
3. Select only genuinely untouched requests, with fresh explicit windows. Prepare
   and run it normally. Old API charges and source permits remain charged to their
   original run. New-run IDs do not restore budget or transfer funding authority.

Production startup rejects unresolved payments, unknown jobs, lost baseline
history and pending source work owned by another run. A known unprepared refill
may remain paused during a zero-funding run; its original permit is not adopted.
Otherwise reconcile existing work through production/read-only utilities before
starting a new run. This is explicit use of existing pools, not crash recovery.

## Reliability windows

The `reliability` scenario compares at least two identical cases across reviewed
absolute windows. Each observation has its own case ID and reservation; server,
source, tool, arguments, unsigned mode and content assertions must match. Reports
check dispatch-intent timestamps against windows, minimum spacing and any required
later UTC day. Missing observations or unresolved accounting remain incomplete;
completed failures remain failures. These are end-to-end observations, not automatic
provider-versus-Tor fault attribution or predictions of future reliability.

Overall `run_seconds` may be 1–604800 seconds (seven days); call, phase and cleanup
limits remain capped at one day. No daemon schedules future windows or automatically
continues paid runs. For later-day reliability observations use separately reviewed
runs and compare their dated results; historical multi-window reports stay readable.

## Funding and pool adoption

A fresh `funded_pools` run with zero job/source limits can keep using active
wallets after an earlier refill paused before preparation. A ready standby is
not required for this mode. Only known ALLOCATED/QUOTED jobs with their original
registry permits and pool/wallet bindings are retained; unreleased source
preparation reservations, prepared/pending transactions and unresolved payments
still refuse startup. Earlier permits remain charged and cannot be used by the
new run. This supports service after treasury insufficiency without implicitly
repairing the pool or transferring funding authority. A positive-funding run
may also retain such an unprepared job outside its selected pool scope. Its
selected pools still require their active and ready wallets at fresh startup;
new allocation authority applies only to those selected pools. The unselected
job stays paused under its original permit and cannot be prepared by this run.

## Canonical payment evidence

The Base RPC adapter's `verify_transfer` check independently verifies a successful
confirmed receipt against the admitted payer, payee, amount and nonce, requiring
both USDC Transfer and [AuthorizationUsed](https://eips.ethereum.org/EIPS/eip-3009#event)
logs. It rechecks canonical block hashes, uses payer isolation and only configured
read-only fallbacks, and leaves absent/unconfirmed receipts pending. Automated
verification runs outside the payer lock during managed background reconciliation.
Each pass checks one queued receipt per pool; pending receipts rotate to the back
and backlog size is logged. Proofs persist against the exact case and attempt.
Reports reject mismatched or duplicate attribution. A consumed authorization
without a debit proof cannot qualify a run or unlock dependent phases; a
successful paid case that never pays likewise cannot qualify a funded test. Startup validation reserves all missing pools' bootstrap pairs in one registry transaction before
allocation; a rejected batch leaves no partial reservations.
Funding-enabled launches additionally require `auto_fund=true`. Treasury-only
starts wait for production bootstrap before dispatch, with progress logs and the
phase/run deadline. An adopted bootstrapped pool may have a partly spent active
wallet; the runner does not refill it artificially. Positive funding remains
bounded by the registry and ordinary production limits on every allocation and
preparation. Zero job authority retains the deny-all restriction even when the
flag is supplied. Single-rotation/refill-service and two-round lifecycle dispatch are available;
funded acceptance remains an explicit live qualification step.
The owned-Tor backend selected by `run` supports unsigned discovery and registry-bound
managed payments, combining confinement, Tor identity/destination evidence and the
same canonical debit requirements. Positive funding requires its `--allow-funding`
flag as well as registry authority. `prepare` explicitly permits catalog inspection;
it verifies build metadata, selected tools and arguments, then reloads the frozen
configuration to verify an unchanged inventory. Preparation does not certify balances
or runtime/Tor safety. Full catalog snapshots and their private capture artifacts
have a separate 128 MiB bound, shared by the executable and runner. Response
documentation is preserved, including large catalogs such as SocialFetch; frozen
reload must still reproduce the inventory exactly. The ordinary 16 MiB evidence
bound and each manifest's smaller API result limit remain unchanged. Oversized
catalog evidence fails explicitly; split the deployment into smaller reviewed runs
rather than truncate a spec. Keep historical private financial ledgers unchanged.

```sh
scripts/zcash.sh build --example live_integration
target/debug/examples/live_integration plan --manifest /private/path/run.toml
scripts/zcash.sh test --example live_integration
```

## Planning and concurrency assertions

Planning resolves the normal deployment TOML with `Deployment::show_config`, so
it does not fetch catalogs, inspect wallet state, read signing secrets, execute
the supplied binary or contact Tor. JSON output contains exact expanded cases,
resolved wallet assignments, API reservation sum/headroom, bootstrap/refill slots,
network policy and explicit unverified prerequisites. It never grants execution
authority. Catalog selection/schema/argument validation belongs to preparation;
current balance, swap-minimum and fee feasibility require subsequent live evidence.

Use `integration/{short,standard,extended}.example.toml` as starting points. They
have invalid placeholder treasury IDs and zero execution windows deliberately.
Each case occurs once in one phase. Reliability repetitions need distinct case IDs
and absolute windows. Concurrency batches cover their phase cases exactly once.
Concurrency reports require overlapping MCP and application intervals from one
application session. Paid batches also require the reviewed wallet bindings,
distinct payer/nonces, canonical debit proofs, admission-time balance accounting,
and valid admission-to-response and signed-request timing. Accounting records the
confirmed chain block/balance and unresolved exposure before and after the current
reservation. The remaining spendable amount must equal balance minus total
exposure, without underflow or overflow. This is captured inside the same
store-worker command as admission, while the pool gate is held; later balance
deltas cannot substitute for it. Reports distinguish overlap at each level and total the
verified fees by wallet. Signed-request future intervals include connection setup
and final headers; they are not socket-byte or signature-computation measurements.
Missing legacy timing leaves a batch incomplete; sequential execution fails the
overlap assertion. Existing paid cases are never replayed to obtain new timing.

## Rotation and lifecycle assertions

Managed admission and receipt events include public lifecycle observations for
rotation/refill assertions. They contain pool roles/generations, funding-job
identities/phases and source-operation submission states, without keys, signed
bytes, quote bodies or upstream error prose. Response observation failure retains
the seller receipt with an explicit unavailable marker and warning. Pending-state
bracketing measures the signed-request future through final headers, not the full
MCP response-body interval. Rotation and lifecycle execution use these observations;
lifecycle additionally verifies one graceful application restart during its first refill.
The phase order is topological: dependencies must name earlier phases. Scenario
fields live in `[phases.scenario]` and inappropriate fields are rejected.
Rotation and refill-service scenarios require one `[[phases.scenario.rounds]]`;
lifecycle requires two rounds for the same pool. Each round names `pool`,
`expected_price_usdc`, `depletion_cases` and `service_cases`. The concatenated
depletion/service lists, in round order, must exactly match the phase's cases.
Depletion cases repeat the same reviewed request through the selected pool;
the first separate service case must use that pool, while later service cases may
use other selected pools to observe independent progress. Every case is paid and
has its own single-use ID and reservation. The expected price is a positive planning
estimate, not a payment override or proof that depletion is feasible. All declared
deployment pools remain visible. Single-rotation scenarios obtain fresh balances,
reject infeasible depletion, stop at promotion and dispatch the separate service
calls before waiting for confirmed refill readiness. Depletion calls settle
canonically before the next depletion call; after promotion the service calls run
immediately without waiting for refill or the triggering receipt. Uncertain calls stop further
paid dispatch. A partially dispatched rotation is observation-only; the runner
cannot restart it to replay or manufacture missing evidence.

The registry's promotion boundary atomically records validated before/after pool
and replacement-job snapshots and marks the unused depletion suffix
`SKIPPED_TARGET_REACHED`. Only never-reserved cases qualify. Attempted calls retain
their charges, including failed or uncertain calls. Reports keep the declared
`reservation_atomic` and separately show `charged_reservation_atomic`; skipped
cases are `NOT_EXECUTED`, cannot be replayed, and are never counted as paid successes.
Reports revalidate the boundary, generation sequence and absence of reservation or
application evidence for skipped cases. This durable mechanism alone does not
qualify restart or exhaustion. Single-rotation reports additionally require exact
payer/generation bindings and canonical paid service, pending-refill overlap for
each service wallet, a READY replacement holding its target, and exactly one
confirmed source submission. Fast refill leaves `not_observed` overlap and an
incomplete run. Dependent phases require these assertions before dispatch.
Provider failures remain visible as failed cases and an overall failed result,
while `rotation_status` separately retains verified promotion, service and refill
assertions. A failed response is never counted as a successful service sample.
Later lifecycle rounds require these rotation assertions and resolved accounting;
failed depletion responses cannot manufacture payment or overlap evidence.

For `lifecycle`, `restart = "queued_refill"` requires observing `ALLOCATED` before
preparation, while `restart = "submitted_deposit"` requires `DEPOSIT_PENDING` with
one acknowledged source submission. The runner observes the checkpoint after its
separate service calls. If funding has already advanced past it, the checkpoint
is explicitly unobserved and execution stops; the runner never stalls funding to
manufacture a window. It drains the owned application through its parent pipe,
retains the same Tor process, records complete process/cover output, verifies
funding continuity, rechecks pins and starts a new registered application session.
The new session retains the same configuration, registry, funding limits and
absolute expiry. Reopened MCP inventories must match before continuing. Each
application session needs complete cover evidence; one session cannot hide another's
missing samples. Cancellation preserves any in-progress drain and its original
cleanup deadline. A partial lifecycle cannot automatically resume paid dispatch.
Funded rotation, restart and exhaustion acceptance remain outstanding.

Fresh pool observations use an explicit registry request and the application's
normal background reconciliation. The application captures the request before
reading RPC state and records the verified view's wallet balances/block identity,
unresolved exposure and lifecycle snapshot while still holding the pool gate.
Only one response is accepted per request and session; a new request is required
for a new observation. Failures warn and leave the request unanswered, while
ordinary reconciliation continues. Existing wallet-isolated RPC and Tor policy
remain in force. The pure depletion estimator requires an active wallet and funded
standby included in that fresh view, no unresolved authorizations, and enough
expanded calls for `floor(active_balance / reviewed_price) + 1`. The final call
triggers promotion; it is not free. Price estimates do not authorize new calls or
override caps, and the controller must authenticate and consume observations promptly.

Amounts are decimal strings (USDC six decimal places, ZEC eight), with checked
integer sums. Zero job/source budgets mean no funding. A paid case must reserve
at least its effective wallet cap. No hidden retry/depletion iterations or automatic
repair allocations exist. The extended example declares twelve depletion attempts
and two service calls per round, with an independent wallet participating in
service. Actual balance/price observations must still establish feasibility;
larger route minima require more explicitly reviewed cases before preparation.

See [qualification limits](../../docs/testing.md#qualification-status). Full automated lifecycle/restart
and combined cover/payment qualification remain in the
[deferred acceptance plan](../../docs/plans/deferred/live_integration_acceptance.md).

## Wallet setup and readiness

Use the production CLI for wallet administration. Default builds include Zcash;
these operations need no runner, registry or matching source checkout.

```sh
target/debug/treazury wallet init \
  --state-dir state/new-integration-wallet \
  --key-file secrets/new-integration-wallet.key \
  --network-config examples/network/tor.toml
target/debug/treazury wallet addresses \
  --state-dir state/new-integration-wallet \
  --key-file secrets/new-integration-wallet.key
# Set treasury references and network policy in deployment.toml, then synchronize:
target/debug/treazury wallet sync --config deployment.toml
# With serving stopped, save an owner-only backup including its key:
target/debug/treazury wallet backup \
  --state-dir state/new-integration-wallet \
  --key-file secrets/new-integration-wallet.key \
  --treasury-id TREASURY_UUID --destination /private/new-backup-directory
```

New wallets discover their birthday through the configured indexer; imports require
an explicit historical birthday. Use saved unified receive addresses for funding.
Tor's SOCKS listener must already be running for these standalone commands. A
successful sync reports confirmed funds; a failed sync exits nonzero and must not
be interpreted as a zero balance. Setup and backup grant no spending authority.

Review the source input-plus-fee bound per allocation and number of allocations
before funding. Their product is an upper-bound estimate, not a live quote. During
a treasury-only run, the existing production application synchronizes and funds
pools under explicit authority; the runner waits for bootstrap within its original
run/phase deadline. It logs progress and recorded funding errors, refuses dispatch
on cancellation or missing readiness, and never manufactures balances or retries
API payments. Production wallet supervision still checkpoints sync on parent loss.

## Registry authorization and preparation

Use the command sequence in [the walkthrough](INTEGRATION.md). Authorization and
reporting are offline; `prepare` can fetch catalogs using the configured network
policy. `plan` reads only configuration, without executing the supplied binary,
loading wallet state or fetching catalogs.

### Ownership and authority

`authorize-registry` requires an existing stopped treasury and exclusive ownership.
It captures public pools, jobs, operations, payment/source liabilities and hashes of
historical SQLite/JSON/TOML evidence, locking legacy ledgers during capture. It does
not decrypt the wallet or mutate those ledgers. Review cumulative API/source/job
ceilings in `integration/authorization.example.toml`. Zero grants no authority;
new authorization IDs retain consumed reservations and the original baseline.

The owner-only `<state>/live-integration/registry.sqlite` stores manifests, plans,
absolute windows, immutable pins, batch reservations, separate execution/semantic/
settlement states and events. A supervisor lock excludes competing writers and is
required even for read-only reports. Symlinks, hard links, permissive paths and
unsafe SQLite sidecars are refused. Stop the active supervisor before reporting.

All members of a batch reserve atomically before dispatch. Failures, lost responses,
process exits and expired windows retain charges. Changed binaries/configuration/
catalogs need a new reviewed run ID under the same cumulative authority; they
cannot overwrite pins, reset budgets or replay a reserved case. Historical records
remain readable without enabling execution.

Funding permits bind immutable input-plus-fee bounds to stable job IDs before
bootstrap, promotion and preparation. Unlinked reservations remain charged. The
guard rechecks session authority, pins, windows and selected pools at each operation.
It validates the baseline, retained identities, source reservations and correlated
liabilities under treasury ownership. All configured pools remain declared. Known
current-run funding can recover during a lifecycle restart without new permits;
unknown liabilities refuse startup. Positive funding needs `--allow-funding`,
registry authority and production auto-funding. Zero authority installs the
no-new-funding restriction on every reopen.

### Catalogs and build provenance

`prepare` creates a fresh owner-only evidence subdirectory and refuses duplicate
run IDs. Failed evidence remains available and is never overwritten. Preparation
supports unsigned static or exclusively managed public-swap deployments; source
management is refused. Listeners use fixed literal loopback addresses/ports.
Build the executable and runner together, with matching features.

`treazury build-info` embeds application/runner/vendor/manifest/lockfile/toolchain
hashes, compiler, target and feature set. Preparation checks these against current
inputs and pins the executable bytes. This detects stale trusted local builds;
it is not attestation of an arbitrary binary. Full frozen catalogs must reproduce
exact tool inventories and preserve source policy, request schemas and documentation.
Catalog preparation is unsigned and probe-free, not payment or Tor qualification.
Owned Tor preparation retains its own control/confinement evidence separately from
execution; external SOCKS alone cannot establish isolation.

Arguments use a strict offline JSON Schema subset. Unknown assertions fail even in
unused branches. Numeric validation is limited to ±(2^53−1), nesting to 64 levels
and validation to 100,000 nodes/operations, with explicit errors. Never remove
constraints or normalize submitted values to make preparation pass.

Catalog/capture streams have a shared 128 MiB bound; ordinary evidence uses 16 MiB
and API results use the manifest limit. Overflow, collector loss, timeout and forced
cleanup invalidate inspection. Minimal-environment children have finite deadlines
and are reaped by owned handles. Source observations record completed/failed/cancelled/
not-started states, typed failure stages and numeric HTTP status, capped at 10,000.
Initial inputs and frozen reloads are separate; local files and coalesced aliases
do not establish independent remote availability. Missing old observations stay unknown.

Qualification inspection continues independent loads within the rolling bound;
aliases share failures without retries. Ordinary serving fails fast. Failed snapshot
commands retain a failure envelope and exit nonzero, never a partial executable
inventory. Execution still requires complete validated preparation. Pricing stays
probe-free during preparation and preserves source policy; runtime descriptions are
validated against that session's production price observations as described under
[startup pricing](#startup-pricing-assertions).

### Dispatch and supervised lifetime

The child receives fresh bearer tokens, minimal configured network environment
references and a private `--qualification-binding`. Ambient keys, dotenv and proxy
settings are not inherited. Before provider I/O, the handler verifies the registered
session, pins, treasury, current authority/window, exact listener/tool/arguments
and durable DISPATCHING state. A unique SQLite claim commits once per run/case.
Case IDs stay on the authenticated MCP side, never in provider/payment material.
Managed admission correlates to the case before signing and independently checks
its cap. Unsigned cases refuse every 402 before signing, even a zero-price offer.

Completion after expiry/revocation only records previously accepted work; it cannot
start another call. Cancelled waiters do not undo accepted blocking journal work.
Missing application completion remains incomplete. Independent provider failures
may allow other eligible cases to proceed; ambiguous payments are never replayed.
The parent pipe drives deliberate bounded drain on EOF/SIGINT/SIGTERM. Preserve an
in-progress drain across cancelled waiters, with its original deadline. Forced kill
or missing output remains explicit incomplete evidence. Per-session private output,
process records and results supplement the durable registry.

`run` exits 0 for qualified success, 2 for completed failures, 3 for incomplete work
and 4 for safety/configuration/process-evidence errors. `report --eligibility` exits
0 for a generated report even when execution is refused. A started/reserved run
cannot launch again. Planning, preparation, process exit or reporting success alone
never establishes payment, content, Tor or scenario qualification.

### Saved accounting and report bounds

After clean managed shutdown, exclusive treasury ownership permits a private
snapshot correlated to that registered session. Reports recompute consumed source
principal/fees, outstanding reservations, refunds, unresolved payment exposure and
anchored balances by role. Refund shielding consumes its fee only. Retired funds
are not active capacity; unanchored default zeros are unobserved. Saved observations
are not fresh chain queries or costs attributable exclusively to the selected run.
Missing/crashed sessions have no final snapshot.

Run attribution uses immutable operation-permit bindings captured with the snapshot,
never later mutable releases. Separate conservative registry ceilings from treasury
reservations and consumed costs. Missing budget evidence does not prove zero cost.
Compare against the hash-verified authorization baseline, which can span runs;
reservation/balance differences do not prove debits. Old baselines without wallet
anchors cannot establish aggregate USDC changes. Contradictory costs, transactions,
refunds or wallet ownership fail comparison. Historical private evidence stays intact.

Structured records/reports are limited to 16 MiB, accounting snapshots to 10,000 per
run, report cases/pin revisions to 10,000 and runtime events to 50,000. Baseline
traversal is limited to depth eight and 256 files, each hashed file to 2 GiB.
Select an individual run when aggregate reporting exceeds a bound. Publish files
atomically without overwrite; bounds produce explicit errors, never partial success
or financial cancellation.

## Production restriction for supervised existing-wallet runs

The production executable accepts the internal serving-only option
`--qualification-no-new-funding` with `--config`. It uses the same
`Deployment::bind_restricted` path available to fixtures and installs the
restriction before managed pool configuration/allocation, independently of
`funding.auto_fund`. A missing managed pool fails startup without creating its
wallets or funding jobs. All declared managed pools receive the restriction.

The store rejects new bootstrap/replacement allocations, promotions, source budget
reservations and new transaction preparation, including preparation using a
reservation inherited from an earlier process. Active-wallet calls with sufficient
available balance still work. A denied promotion preserves wallet roles,
generation and outbox and does not admit/sign the payment; trusted Base
reconciliation is retained. Both stderr logs and the tool error carry
`qualification_funding_denied` and explain the zero limit. Funding-worker denials
also retain that reason in wallet status and leave the job quoted, before
`PREPARING`, so refusal cannot masquerade as an interrupted calculation. A cheaper call can
still use the same active wallet after a denied promotion.

The restriction lasts for the store owner's lifetime and cannot be relaxed on that
owner. Every supervised launch/restart must supply it again. Ordinary serving has
no such restriction. Existing prepared transaction bytes and liabilities remain;
normal reconciliation, confirmation and explicitly permitted saved-byte submission
are still possible under their existing guards. The option is therefore not a
promise that earlier accepted work cannot settle.

This primitive does not bind a child to registry authority, prevent API signing,
or grant permission to launch it. The runner still cannot execute paid cases. Complete
supervisor qualification and registry binding before enabling execution; positive
funding requires separate allocation permits.

## Keyless child and parent lifetime

The executable's internal `--qualification-unsigned` option requires
`--qualification-parent-stdin` and `--config`. It serves the normal authenticated
MCP inventory with keyless clients: configured static key references are not read,
treasury state is not opened, and any declared managed wallet, `auto_fund=true` or
source-management configuration is refused. It also rejects `--env-file`; the
supervisor must provide only the explicit environment needed for the child.

Free API calls and lazy help retain ordinary routing and response bounds. Every
HTTP 402 produces an agent-visible `qualification_payment_denied` error and warning
before parsing the challenge, irrespective of amount or protocol version. No signer
exists and no paid retry occurs. Requests use the normal network factory and
origin-scoped discovery identity, preserving strict TLS/HTTP policy, Tor remote
DNS and refusal of direct fallback. This is unsigned discovery traffic, not evidence
of EVM-address isolation or settlement. Read-only review is still necessary: unsigned
HTTP requests can have provider-side effects.

The supervisor keeps a piped stdin write end open for the child's lifetime and
closes it to request shutdown. Stdin is a control pipe, not an MCP transport;
unexpected bytes cause a visible protocol error and deliberate shutdown. A null
stdin, regular file or terminal is refused. EOF during catalog loading/binding
cancels startup. During serving it invokes the existing ten-second drain and logs
why it is waiting. Parent process death also closes the pipe; the supervisor must
not share its write end with other processes. SIGINT/SIGTERM still work with the
pipe open. The watcher uses a nonblocking Unix pipe so there is no uncancellable
stdin reader thread preventing exit. Other platforms reject this mode explicitly.

Local executable tests cover authentication, free/paid calls, in-flight drain,
startup cancellation, malformed control input, abrupt parent exit, SIGTERM,
forbidden configuration and SOCKS identity/refusal. These are process/transport
fixtures, not real-Tor qualification. The runner wires these controls to prepared
binary/catalog/schema pins, registry-bound cases, bounded output, deadlines/reaping
and result correlation. Managed `run` uses funding restrictions and canonical
payment checks; unsigned `run` has no signer. Internal flags and successful fixture
tests alone do not confer execution authority.

A planned Tor outage currently requires a separate keyless deployment with no
declared managed wallets and automatic funding disabled. Selecting only unsigned
cases inside a managed deployment does not remove its treasury/funding workers.
The runner refuses that combination before execution and again before stopping
Tor. Outage qualification is an independent keyless test; do not stop Tor under
a funded child. Use `treazury wallet backup` for ordinary financial backups.

## Offline Tor re-audit

`reconcile-receipts --state-dir STATE --run RUN_ID --evidence-dir NEW_PRIVATE_DIR`
recovers canonical debit evidence for recorded successful seller receipts after
the serving process has stopped. It holds registry and treasury ownership, checks
the original frozen catalogs and payment journal, then starts dedicated Tor and a
confined, keyless RPC observer. Supply the original RPC environment references;
it uses the frozen network policy and configured RPC fallback order. No provider
requests, signing, funding, case dispatch or reservation release occur. The new
observer binary is recorded separately from the original execution pins.

The observer has a 900-second total deadline and the production per-RPC deadlines.
Output is bounded at 16 MiB; overflow or incomplete process/control evidence
prevents import. Each observed payer identity must pass authenticated Tor stream
checks before proofs are atomically added with the original case/session/attempt
correlation. Existing proofs are skipped. Exit 3 means a receipt remains pending
or unavailable; repeat observation with a new evidence directory, never replay a
paid case. This qualifies the new read-only observation session only; it cannot
repair missing isolation evidence from an earlier payment session.

`audit-tor --state-dir STATE --run RUN_ID --output NEW_PRIVATE_FILE` reinterprets
retained Tor events without launching processes, opening keys, making network
requests, replaying cases or changing reservations. Supply the original configured
endpoint environment variables; reconstructed discovery/treasury targets must match
the initial identity export. The treasury must be stopped. The command validates
pinned catalog artifacts and inventory, but intentionally does not authorize a new
executable to resume a paid run. It writes a separate result and preserves the
original qualification record.

A clean owned-Tor stop publishes `control-completion.json` with the event count
and log hash after the authenticated reader finishes. Re-audit requires this marker
for a whole-stream completeness claim. Older records without it can pass observed
stream checks but return exit 3 with `observed_streams_only`; a missing marker is
never reconstructed from human log prose. A present mismatched marker is an error.
Even a complete stream audit alone does not qualify payment settlement, scenario
completion or complete unlinkability.

## Owned-Tor qualification

`prepare --state-dir STATE --manifest MANIFEST` and `run --state-dir STATE --run RUN_ID`
select owned Tor from the manifest. Preparation owns Tor for catalog inspection;
execution owns a fresh Tor process using the same persistent state and frozen
catalogs. Each confines its child with macOS `sandbox-exec` and has separate
control evidence. Preparation alone makes no full-session isolation claim.
It requires `tor_mode = "owned"`, `confinement = "macos_sandbox"` and
`require_isolation_evidence = true`. It does not use Tor Browser's data directory
or stop an externally owned Tor. Dedicated data persists at
`STATE/live-integration/tor-state`; each run has new private control/cookie/event
artifacts. Create the manifest's evidence root with owner-only permissions before
launch; each run creates its own new subdirectory. CLI-relative Tor binary/state
paths are resolved before changing the child working directory. Bootstrap has one
300-second budget, without exit cycling or retries.

Unsigned owned-Tor qualification may omit outage testing. It still requires the
normal confinement, identity/destination evidence, complete child/Tor cleanup and
successful requested case assertions. Reports label the outage `not_requested` and
make no stopped-Tor cache claim. Managed outage is unsupported; use a separate keyless deployment. When requested in a keyless deployment, the final
outage phase declares its cache boundary explicitly:

```toml
[[phases]]
id = "outage"
window = "now"
cases = ["cached", "uncached"]
pools = []
required = true
depends_on = ["connected"]
[phases.scenario]
kind = "tor_outage"
warm_case = "warm"
cached_case = "cached"
uncached_case = "uncached"
```

All three are separately reviewed, unsigned cases. The warm case belongs to the
earlier dependency; warm and cached cases address the exact same listener/source/
help tool and arguments. The uncached case must use a distinct help URL that no
other case invokes. Preparation checks these bindings against the actual catalog.
After Tor stops, its SOCKS listener must explicitly refuse connections. The same
MCP process must still list its complete inventory and return identical cached
help. The uncached result must report an error accompanied by a typed application
`http_connect` event; an arbitrary provider error, driver timeout or malformed
response cannot qualify the outage. SOCKS libraries may hide the underlying OS
error, so the closed-port proof comes from the independent supervisor check.
Its raw semantic result remains `FAILED`, while the separate outage assertion
records the expected failure. This does not qualify a stalled in-flight request.

Before launching Tor and again after the planned outage, the runner exercises
unconfined positive and confined negative TCP/UDP controls on IPv4 and IPv6.
Only the SOCKS endpoint is permitted for outbound application connections; MCP
ports are inbound exceptions. No proxy-only downgrade is permitted. Control
loss/output overflow stops application progression and leaves incomplete evidence.
Raw Tor credentials and targets stay in private evidence, while the qualification
summary reports per-identity observations and circuit separation without them.
Unsigned execution proves discovery identity behavior only. Managed execution
exports initial and final identity maps; the latter includes replacement addresses
created during the run. Treasury traffic is confined to configured indexer/submission
hosts. Declared pool identities may reach funding/RPC hosts and the API origins
assigned to that pool; undeclared pool identities remain unpermitted. Each admitted
payer must have a successful stream to its reviewed provider origin, so RPC-only
traffic cannot pass a paid-wallet check. The audit retains the original remote DNS
hostname even when later Tor events report a resolved IP. It verifies separation
and permitted destinations, not complete unlinkability or individual HTTP/2
requests on a reused SOCKS stream. Payment correlation and canonical debit evidence
are independent requirements.

A reproducible free test fixture fetches Exa's public OpenAPI catalog and uses
Cloudflare's trace document and the Tor Project homepage (explicitly allowing HTTP/1 for that site's ALPN behavior). It creates an empty **synthetic treasury, never a wallet to
fund**, with zero API/source/job authority. The fixture writer is explicitly
ignored in normal test runs and itself performs no network requests:

```sh
scripts/zcash.sh build --bin treazury --example live_integration
TREAZURY_M4_FIXTURE_DIR=/private/tmp/treazury-m4-new \
  scripts/zcash.sh test --test integration_preparation \
  prepare_real_tor_qualification_fixture -- --ignored --exact --nocapture
# Review run.toml/deployment.toml; adjust the installed Tor path if necessary.
target/debug/examples/live_integration prepare \
  --state-dir /private/tmp/treazury-m4-new/state \
  --manifest /private/tmp/treazury-m4-new/run.toml
# Use the run_id saved in run.toml:
target/debug/examples/live_integration run --state-dir /private/tmp/treazury-m4-new/state --run RUN_ID
```

The directory must not already exist. Default fixture ports are SOCKS 19950 and
MCP 19877; adjust the deployment before preparation if occupied. Live requests
occur only in the final command. Both executable and runner must be rebuilt from
the same source/features, and source changes during the test invalidate preparation.
Keep private evidence under the fixture directory; do not publish raw responses,
SOCKS credentials, cookie files or control transcripts.

The loopback-only confinement gate can also be run independently:

```sh
target/debug/examples/live_integration qualify-confinement \
  --evidence-dir /private/tmp/treazury-confinement-new \
  --socks-port 19950 --mcp-port 19877
```

That command qualifies denied egress only. Successful confined MCP startup and
exchanges in owned-Tor `run` separately establish inbound operation.

## Unsigned cover experiment

Cover configuration and limitations are in [the cover guide](../../docs/cover-traffic.md).
The local opt-in matrix drives real MCP calls through a TLS-over-SOCKS fixture,
without Tor, live providers or funds:

```sh
scripts/zcash.sh test --lib cover::matrix_tests::cover_distribution_matrix -- --ignored --exact
```

It writes `target/cover/matrix.json`: no cover/ranges/padding/combined × five
samplers × concurrency 1/2/3 × two repetitions, with fixed fixture response sizes
and delays, test-only seeds, API latency and aggregate cover counts. This compares
correctness and overhead, not privacy or Internet performance. Separate raw h2
fixtures verify physical reuse, never-indexed HPACK, faults and stream priority.

Generate a private real-Tor fixture with no network traffic:

```sh
scripts/zcash.sh build --bin treazury --example live_integration
target/debug/examples/live_integration cover-fixture \
  --directory /private/tmp/treazury-cover-qualification \
  --binary "$PWD/target/debug/treazury" \
  --tor-binary '/Applications/Tor Browser.app/Contents/MacOS/Tor/tor' \
  --profile ranges --distribution log-normal --concurrency 2 --samples 3
```

The output contains a synthetic registry, a **never-fund** synthetic key/state,
local specs, reviewed zero-spend manifest and deployment. This does not create a
fundable Zcash wallet. The executable and runner must be built from the same
current inputs/features; rebuilding after modifying sources is required. The
fixture defaults to nghttp2.org/httpbin: `/httpbin/json` is the unsigned API
resource and `/httpbin/range/16384` provides bounded same-origin ranges.
`--resource rfc` selects RFC Editor; a server that ignores Range and returns 200
fails cover qualification. Help-cache/outage controls use
Cloudflare's public trace document. Review
these destinations and the generated files before the explicit live command:

```sh
target/debug/examples/live_integration prepare \
  --state-dir /private/tmp/treazury-cover-qualification/state \
  --manifest /private/tmp/treazury-cover-qualification/run.toml
target/debug/examples/live_integration run --state-dir /private/tmp/treazury-cover-qualification/state --run RUN_ID
```

The installed Tor binary gets dedicated persistent data under the synthetic state;
Browser data/circuits are not used. The run has 600 seconds, 300-second phases,
240-second calls, one real call at a time, 20-second cleanup and 1 MiB result bounds.
The explicit `post_batch_wait_ms = 10000` interval lets bounded cover tails finish
before another call preempts them or the outage phase stops Tor. It applies after
connected batches (including the last), consumes the existing phase/run/window
budgets, responds to cancellation and is logged and retained as evidence. Ordinary
manifests default to zero; values above 60000 or the phase budget are rejected.
MCP latency intervals exclude this settling time. Default three API samples request at most 16 KiB cover per episode, eight range
requests and ten seconds per episode, additionally bounded by global limits.
Existing confinement, control evidence and outage assertions remain mandatory.
Use a new fixture/run for additional samples; never replay a reserved case.

Profiles `none`, `ranges`, `padding`, `combined`, five `--distribution` choices,
`--concurrency 1..3`, and `--samples 1..16` support reviewed comparisons. Ranges are
the live default. Public range support is not authorization for arbitrary ignored
headers: enable live padding only for a compatible, explicitly reviewed resource.
Local tests qualify its mechanics independently. Fresh Tor circuit-learning state
is adequate for a bounded correctness check, not a performance comparison.

The runner requires one complete `TREAZURY_COVER_REPORT` stderr summary when cover
is enabled, records `cover_observed`, and includes `cover_evidence` in session
`results.json`. Missing/duplicate summaries, truncated process output, inconsistent
counts or unclean streams invalidate evidence. `at_least_one_range_qualified=false`
is explicitly unqualified range coverage even if the real calls succeeded.
Requested modes with no successful samples make qualification incomplete; the
ordinary API results and refusal evidence are still retained. Keep
outcome reasons, consumed counts and degraded samples. Counters measure reader
body bytes and uncompressed padding values; actual wire bytes and production
connection affinity are unobserved. Neither this fixture nor more traffic variance
establishes privacy, live EVM isolation, paid receipts or rotation correctness.
