# Catalog startup qualification

`deployment::store_loss_tests` checks that store-only ownership observes the exact
worker failure, dependent calls fail and close reports failure while a separate
Store remains usable. The shared Store supervisor reports loss even without a
new request; it does not stop independent listeners or restart financial work.

`deployment::direct_swap_tests` separates wallet-independent catalog inspection
and unsigned startup from signed history recovery. Temporary incompatible state
must not affect inspection; actual binding restores only authenticated owned
status, retains removed-listener background work and preserves identity/schema
guards. The public-demo inventory fixture rewrites authored paths into temporary
state before loading either profile, so it never opens operator wallet state.
Its isolated local-preflight diagnostic fixture checks Base, NEAR and treasury
causes without an SDK wallet or network calls. Warnings show only safe top-level
configuration errors, omit endpoint/header values and retain scoped degradation
without allocating funding jobs.

The treasury command-sync reuse fixture can emit static phase breadcrumbs with
`X402_TEST_SYNC_DIAGNOSTICS=1` and `--nocapture`, covering initial scan, invalidation
and restart scans, delayed/failed/cancelled probes and final close. These are
diagnostic observations, not assertions. The focused Linux
[hang workflow](../.github/workflows/treasury-sync-diagnostics.yml) preserves the
whole target's eight-thread scheduling and captures owned native stacks before
terminating a diagnostic run; see [limits and artifacts](../docs/testing.md#github-actions-and-coverage).

`tests/startup.rs` checks direct downloads bypass Tor isolation capacity, the
default two-scope setting, failure and caller cancellation closing every active
direct request, inventory ordering independent of completion and the configured
Tor capacity, compatible alias sharing,
strict-limit/timeout separation, shared-download cancellation and fresh loads,
per-alias overrides/filters/wallets, strict configuration, and JSON stdout with
progress on stderr. The CLI boundary regression parses the complete tool inventory,
checks its tool count, and retains meaningful stderr progress without exposing
endpoint addresses. A malformed catalog makes `catalog tools` exit unsuccessfully
with empty stdout and no endpoint-address disclosure on stderr. A separate
qualification-snapshot case checks structured parse failure, independent source
completion and absence of partial inventory, without matching lifecycle log text.
Readiness remains INFO; routine local reads, cache policies,
origin fetches and alias sharing remain DEBUG under the logging policy. The
synchronous unsigned-serving loader can omit optional
remote failures with explicit unavailable status; required sources, inspection and
qualification stay strict in these synchronous tests. Its isolated SOCKS subprocess verifies that concurrent
catalog requests retain origin-based discovery identities: same-origin paths
share credentials, different origins do not. It does not require real Tor.

`rotation::funding::scheduler::opportunity_tests` checks deferred ordinary allocation,
prompt accepted-job recovery and explicit bootstrap using paused time and unfunded
stores. Pure injected interval tests cover activity-independent scheduling, all-due
rearming and skipped missed opportunities. These tests do not qualify live funding
time or process/power savings.

`deployment::bootstrap::actor_tests` also exercises explicit wallet bootstrap's
pending monitor through the real funding worker and local Base adapter. One
captured claim is already credited; the other completes while its original
source remains pending. Successful supervisor shutdown and Store reopen retain
the exact source bytes, attempt and claim identities. This needs no transaction
proving or synthetic chain; separate worker-drain tests cover blocked accepted work.

`deployment::startup::price_alias_tests` verifies first-startup revision capture for
all compatible aliases, shared acquisition despite API destination/prefix changes,
and invalidation when the second alias changes on the next load. Catalog cache
fixtures cover direct-warm/relay provenance, joint price renewal on HTTP 304,
expiry, replacement and stale writes; no API probes or wallet creation are needed.

Run the focused regression suites sequentially:

```sh
make test CARGO_ARGS='--test startup --test deployment --test provider_catalogs --test network_audit'
make build
```

The application includes Zcash. The ignored SOCKS child is exercised by its parent.
Relay ownership fixtures cover capacity retained after waiter cancellation and
queued serialized fetches leaving unrelated request capacity available.
The deployment suite verifies concurrent source discovery and complete, ordered inventories through synchronous loading. The `deployment::deferred::tests`
unit tests cover incremental deployment serving (`wait_for_catalogs = false`):
listeners bind before slow remote catalogs finish, available tools remain callable,
and a required remote failure becomes unavailable while strict inspection still
rejects it. `deployment::deferred::readiness_tests` uses real MCP HTTP requests to
verify default waiting per endpoint, publication before initialize/tools/list,
independent local/immediate peers, optional failure, ERROR plus socket closure for
required failure, local-only/management-only endpoints, cancellation while waiting, and
worker/publication failures.
Runtime refresh coverage holds other catalog downloads pending and verifies that
published selector failure resolves the affected endpoint's readiness immediately,
while eager peers retain their valid tools and old handles retire.
Failure logs distinguish HTTP status, tool-generation
and listener-selection failures without upstream prose; publication failures explain
the retained inventory. Startup unit tests cover a fresh cached JSON document
whose unsupported header schema prevents generation, and shared failures preserving
safe acquisition stages and status. The cached-generation diagnostic runs in an
exact child process so subscriberless sibling tests cannot disable its shared
tracing callsite; freshness, typed issues and privacy checks remain in that child.
Discovery-help restart coverage waits for final Manager destruction after HTTP
shutdown and accepted-work drain, before reopening its exclusive registry. A
retained-owner control proves that teardown and registry unlock cannot finish
while another service still owns that Manager.
Existing accepted-relay-payment cancellation and
panic fixtures also run with default waiting and verify drain/Store ownership.
`deployment::lifecycle_tests::funding_failure_keeps_listeners_and_base_reconciliation_available`
retains both real MCP listeners and the independent scalar RPC after funding
failure. It waits for the wallet's published `balance_due`, advances only that
owned cooldown and one maintenance wake, then resumes time for real HTTP and
Store work. A server handler count alone cannot anchor that advancement.
`deployment::readiness::tests` verifies the once-only funded startup announcement
waits for catalog success, every listener and sufficient current estimates across
all bound pools, including an earlier pool spending while another awaits credit.
Closed readiness channels and cancellation release Store ownership. The store
selection regression covers exact cap equality, split balances, low/unknown or
malformed estimates, disabled pools and ineligible payer roles without financial
mutation. The aggregate catalog signal fixture also covers eager endpoints,
required failure and allowed optional failure without changing serving readiness.
The waiting-endpoint regression also checks that the INFO startup summary
identifies catalogs still loading instead of announcing discovery completion.
Publication preserves authored prices, tool order and existing callable snapshots.
Advisory listener/distribution selectors emit DEBUG diagnostics; missing exact
selectors retain terminal failure diagnostics. Listener selector diagnostics are
covered by `deployment::selection_diagnostics` and the deferred readiness fixtures:
pending catalogs do not produce missing-tool diagnostics, terminal source outcomes trigger one report per affected listener, and
unrelated completions do not repeat it. Synthetic catalogs verify source name-filter
causes, opt-in recommendations, listener tags and unchanged exact-selector errors.
Deferred collision
regressions cover both legacy attachments and authored distribution rules in
opposite completion orders: three owners, exact selectors, unique tools, exclusion,
fanout, listener isolation and captured bindings. Synthetic prefix reconstruction
retains and validates `send_tools`; excluded operations remain in source inventory but
never publish. Strict loading still rejects duplicate names.
Source distribution uses potential bindings declared before catalog acquisition.
Routing regressions must preserve pending/optional/required source isolation and
`wait_for_catalogs` behavior for every declared target, including targets with no
current tools or shadowed rules. Static-only empty routed destinations and invalid
legacy listener selectors must not discard a shared catalog's valid tools on peers.
Immediate local and deferred remote catalogs use the same selection path; eager
endpoints publish safe failure instructions and unchanged failures do not repeatedly
log on unrelated completion. Shared-payer cover ownership still validates across
listeners: conflicting profiles reject admission, while matching profiles preserve
both endpoints. Late namespace rejection omits only the invalid source, retaining
healthy aliases and captured calls; both completion orders verify a safe selection
reason and a single paid catalog exchange. Strict namespace rejection happens
before wallet initialization;
reused managed-owner cleanup is exercised by an actual occupied-listener failure
after initialization and funding restriction. Both outcomes reopen the owner and
assert no funding jobs or wallet addresses were created.
Exclusion-only sources add no readiness dependency.
Strict inspection and frozen reload must agree with publication and wallet
bindings; routing explanations include exclusions without adding catalog reads or
funding work. See the maintained
[distribution contract](../docs/configuration.md#source-tool-distribution).
The catalog capacity subprocess exercises indefinite server delays and verifies
a healthy source publishes a callable tool while delayed attempts hold no active
isolation slot. It also exercises
strict loading and warming, which download the healthy source while remaining
pending for completeness; cancellation cannot dispatch delayed retries.

Wallet discovery uses the [wallet observation policy](../docs/wallet-rotation.md#wallet-observation-policy).
`network::startup_socks::tests` covers a late local SOCKS listener, shared and
cancelled waiters (including initializer handoff to an existing waiter), sanitized ERROR/recovery INFO, minute ERROR reminders during
a continuing outage, completed observation without a
permanent monitor, and fresh catalog cache hits while the proxy is absent.
`deployment::deferred::startup_socks_tests` opts into incremental availability and
verifies that a bound authenticated MCP
listener and local tools remain available, then remote catalogs and captured Base
balance discovery and an idle treasury's initial scan resume in the same process
when a fake SOCKS endpoint appears after a confirmed startup connectivity ERROR,
without signed requests or new financial work. These are local TCP/SOCKS fixtures,
not live Tor bootstrap qualification.
`rotation::manager::maintenance::tests::startup_socks` verifies a requested Base
refresh runs while startup waits for an absent SOCKS port, preserves its failure
cooldown through proxy recovery and notification bursts, then succeeds after the
cooldown. `treasury::actor::tests::startup_socks_preserves_commands_and_failed_scan_cooldown`
checks command responsiveness during that outage and that a failed explicit scan's
backoff survives proxy recovery before the pending startup scan resumes. Both use
isolated Tor-policy subprocesses and assert that discovery creates no financial work.
`rotation::manager::maintenance::tests` exercises finite Base discovery, successful
zero refresh completion, notification bursts at 59 seconds, cancellation/restart,
retired-wallet exclusion, startup inventory failure and first-positive initial
claim consumption. `tests/treasury_sync.rs` checks finite available-treasury startup,
shared scan/failure cooldowns, accepted jobless ownership, independent explicit-sync
asset scopes and exclusive administration ownership.
The explicit-sync scope regressions also preserve configured wallets, generated
identities and accepted removed recipients despite invalid unrelated templates,
spending settings or bindings, without creating allocations or funding work.
The deployment Store-only fixtures preserve usable Base service when the SDK/source capability is unavailable;
that runtime performs no ZEC discovery. Listener-binding tests retain the ordering
that tools become available before these background observations complete. These
local fixtures do not qualify live settlement or Tor.

Finite Base discovery consumes its captured pass after exhaustion or a transient
inventory read failure; a failure is reported without extending or retrying that
frontier. Recurring requested observations retain their independent retries. Pass
completion does not imply SOCKS readiness or completion of already accepted reads;
explicit reconciliation still drains those reads and checks unfinished retirement.

## Reproducible measurement

```sh
make test CARGO_ARGS='--test startup_benchmark' TEST_ARGS='--ignored --nocapture'
```

This opt-in test uses three independent synthetic OpenAPI documents: small
request contracts with nested arrays and a path template, a larger lookup
inventory, and embedded-price/GET-POST variants. Five sources include two aliases
sharing three unique spec URLs while retaining distinct prefixes. Localhost adds
100 ms per request. The test compares complete inventories across three samples
at each configured Tor capacity. Its direct requests bypass that capacity, so
these runs measure direct acquisition and inventory stability rather than Tor
admission or a concurrency speedup. Aliases must not duplicate catalog requests, and no
endpoint pricing requests are made. The server rejects payment-signature headers.
No provider fixture, external service, Tor daemon, secret, funded wallet or signature is
involved. Compiling this ignored test does not run a measurement or establish
performance results.

Candidate preparation and tool generation are synchronous. Measure queue, headers,
body, parsing and tool construction separately; full startup also includes
wallet binding. Record peak memory, request counts, failures and identical tool
inventories, not only successful elapsed times. Keep results under ignored `target/`.

## Default capacity and Conflux rationale

Deployment catalog Tor isolation capacity defaults to 2 (configurable 1..64).
Catalog, optional pricing and relay requests with the same effective SOCKS
credentials share one slot and run in parallel while distinct scopes wait.
Different transport/retry scopes count separately even on one domain. The last
full-body holder releases capacity before retry/handoff delays. Direct egress,
including explicit direct warming, bypasses the gate; idle Tor circuits and pooled
sockets do not count. Compatible catalog aliases share one download per load,
while local files and separate loads remain independent. Runtime dynamic imports
outside this deployment acquisition owner retain their existing scheduling.
A separate live comparison must declare its selected provider workload and
retain failures without changing runtime selection to improve the result.

Tor normally targets three fresh prebuilt multipath Conflux sets. Admitting two
active isolation scopes leaves headroom for replenishment, making Conflux use
more likely for catalog downloads. The operator's live Tor performance testing
also identified two as a sweet spot. This motivates the default without proving a
universal optimum or guaranteeing a particular Conflux assignment. For rigorous live
comparison, alternate limits against fixed catalogs, retain failures, observe
actual circuit/Conflux assignments and use dedicated persistent Tor state with
mature circuit-build learning. Record changes in that learning and connection
quality. Keep Tor warm; use controlled NEWNYM between comparable runs when needed
rather than restarting it or resetting unrelated state. Local replay cannot
establish live Tor latency, memory costs or anonymity.
