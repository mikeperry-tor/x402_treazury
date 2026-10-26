# Catalog startup qualification

`tests/startup.rs` checks rolling replenishment, the default two-request catalog bound,
failure and caller cancellation closing active requests, untouched queued sources,
serial/parallel inventory equality, compatible alias sharing (concurrent and queued),
strict-limit/timeout separation, shared-download cancellation and fresh loads,
per-alias overrides/filters/wallets, strict configuration, and JSON stdout with
progress on stderr. The CLI visibility regression keeps readiness at INFO and
local reads, cache policies, origin fetches and alias sharing at DEBUG, with the
complete tool inventory on stdout. The synchronous unsigned-serving loader can omit optional
remote failures with explicit unavailable status; required sources, inspection and
qualification stay strict in these synchronous tests. Its isolated SOCKS subprocess verifies that concurrent
catalog requests retain origin-based discovery identities: same-origin paths
share credentials, different origins do not. It does not require real Tor.

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
worker/publication failures. Failure logs distinguish HTTP status, tool-generation
and listener-selection failures without upstream prose; publication failures explain
the retained inventory. Startup unit tests cover a fresh cached JSON document
whose unsupported header schema prevents generation, and shared failures preserving
safe acquisition stages and status. Existing accepted-relay-payment cancellation and
panic fixtures also run with default waiting and verify drain/Store ownership.
The waiting-endpoint regression also checks that the INFO startup summary
identifies catalogs still loading instead of announcing discovery completion.
Publication preserves authored prices, tool order and existing callable snapshots.
Listener selector diagnostics are covered by `deployment::selection_diagnostics`
and the deferred readiness fixtures: pending catalogs do not produce missing-tool
warnings, terminal source outcomes trigger one report per affected listener, and
unrelated completions do not repeat it. Synthetic catalogs verify source name-filter
causes, opt-in recommendations, listener tags and unchanged exact-selector errors.
Deferred collision
regressions cover opposite completion orders, three owners, exact selectors,
unique tools, listener isolation and captured bindings; strict loading still rejects
duplicate names.
The catalog capacity subprocess saturates both slots with indefinite server delays
and verifies a queued healthy source publishes a callable tool. It also exercises
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

## Reproducible measurement

```sh
make test CARGO_ARGS='--test startup_benchmark' TEST_ARGS='--ignored --nocapture'
```

This opt-in test uses three independent synthetic OpenAPI documents: small
request contracts with nested arrays and a path template, a larger lookup
inventory, and embedded-price/GET-POST variants. Five sources include two aliases
sharing three unique spec URLs while retaining distinct prefixes. Localhost adds
100 ms per request. The test compares complete inventories across three samples
at each concurrency level. Aliases must not duplicate catalog requests, and no
endpoint pricing requests are made. The server rejects payment-signature headers.
No provider fixture, external service, Tor daemon, secret, funded wallet or signature is
involved. Compiling this ignored test does not run a measurement or establish
performance results.

Candidate preparation and tool generation are synchronous. Measure queue, headers,
body, parsing and tool construction separately; full startup also includes
wallet binding. Record peak memory, request counts, failures and identical tool
inventories, not only successful elapsed times. Keep results under ignored `target/`.

## Defaults and working theory

Catalog concurrency defaults to 2 (configurable 1..64). Compatible catalog aliases
share one download per load, while local files and separate loads remain independent.
A separate live comparison must declare its selected provider workload and
retain failures without changing runtime selection to improve the result.

The catalog default reflects a working theory that leaving headroom below an
anticipated three prebuilt Conflux sets helps transfer tails when a set is unavailable.
This is not an observed Conflux assignment or proven optimum. For rigorous live
comparison, alternate limits against fixed catalogs, retain failures, observe
actual circuit/Conflux assignments and use dedicated persistent Tor state with
mature circuit-build learning. Record changes in that learning and connection
quality. Keep Tor warm; use controlled NEWNYM between comparable runs when needed
rather than restarting it or resetting unrelated state. Local replay cannot
establish live Tor latency, memory costs or anonymity.
