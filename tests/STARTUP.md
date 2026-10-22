# Catalog startup qualification

`tests/startup.rs` checks rolling replenishment, the default two-request catalog bound,
failure and caller cancellation closing active requests, untouched queued sources,
serial/parallel inventory equality, compatible alias sharing (concurrent and queued),
strict-limit/timeout separation, shared-download cancellation and fresh loads,
per-alias overrides/filters/wallets, strict configuration, and JSON stdout with
progress on stderr. The synchronous unsigned-serving loader can omit optional
remote failures with explicit unavailable status; required sources, inspection and
qualification stay strict in these synchronous tests. Its isolated SOCKS subprocess verifies that concurrent
catalog and pricing requests retain origin-based discovery identities: same-origin paths
share credentials, different origins do not. It does not require real Tor.

Run the focused regression suites sequentially:

```sh
scripts/zcash.sh test --test startup --test deployment --test provider_catalogs --test pricing --test network_audit
scripts/zcash.sh build
```

The application includes Zcash. The ignored SOCKS child is exercised by its parent.
Pricing barriers additionally assert a shared 16-request cap, lower per-source limits, rolling progress past a
stalled endpoint, duplicate-URL coalescing, and cancellation releasing permits.
Pricing retry regressions additionally cover a delayed probe releasing a single
source slot, sixteen delayed sources permitting a queued provider's description
publication through the deferred runtime, and unchanged active request bounds.
Relay ownership fixtures cover capacity retained after waiter cancellation and
queued serialized fetches leaving unrelated request capacity available.
The deployment suite verifies concurrent source discovery and complete, ordered
priced inventories through synchronous loading. The `deployment::deferred::tests`
unit tests cover ordinary deployment serving: listeners bind before slow remote
catalogs or pricing finish, available tools remain callable, and a required remote
failure becomes unavailable while strict inspection still rejects it. Pricing
publication preserves tool order and existing callable snapshots. Deferred collision
regressions cover opposite completion orders, three owners, exact selectors,
unique tools, listener isolation and captured bindings; strict loading still rejects
duplicate names.
The catalog capacity subprocess saturates both slots with indefinite server delays
and verifies a queued healthy source publishes a callable tool. It also exercises
strict loading and warming, which download the healthy source while remaining
pending for completeness; cancellation cannot dispatch delayed retries.

Wallet discovery uses the [wallet observation policy](../docs/wallet-rotation.md#wallet-observation-policy).
`network::startup_socks::tests` covers a late local SOCKS listener, shared and
cancelled waiters, sanitized ERROR/recovery INFO, completed observation without a
permanent monitor, and fresh catalog/pricing cache hits while the proxy is absent.
`deployment::deferred::startup_socks_tests` verifies that a bound authenticated MCP
listener and local tools remain available, then remote catalogs and captured Base
balance discovery and an idle treasury's initial scan resume in the same process
when a fake SOCKS endpoint appears after a confirmed startup connectivity ERROR,
without signed requests or new financial work. These are local TCP/SOCKS fixtures,
not live Tor bootstrap qualification.
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
scripts/zcash.sh test --test startup_benchmark -- --ignored --nocapture
```

This opt-in test uses three independent synthetic OpenAPI documents: small
request contracts with nested arrays and a path template, a larger lookup
inventory, and embedded-price/GET-POST variants. Five sources include two aliases
sharing three unique spec URLs while retaining distinct prefixes. Localhost adds
100 ms per request. The test compares complete inventories across three samples
at each concurrency level, then exercises unsigned pricing with a fresh shared
process-local cache at each source concurrency. Its declared workload expects
34 unique probes: 32 larger-inventory lookups, one small lookup and one unpriced
GET. Embedded prices, path templates and POST-preferred twins are not probed;
aliases must not duplicate requests. Every possible probe destination is checked
to be local and the server rejects payment-signature headers. No provider
fixture, external service, Tor daemon, secret, funded wallet or signature is
involved. Compiling this ignored test does not run a measurement or establish
performance results.

Candidate preparation and tool generation are synchronous. Measure queue, headers,
body, parsing and price-description rebuild separately; full startup also includes
wallet binding. Record peak memory, request counts, failures and identical tool
inventories, not only successful elapsed times. Keep results under ignored `target/`.

## Defaults and working theory

Catalog concurrency defaults to 2 (configurable 1..64); pricing uses a shared
16-request limit plus per-source limits. Pricing probes inspect status/challenge
headers rather than consuming complete bodies, so evaluate them independently.
Compatible catalog aliases share one download per load, while local files and
separate loads remain independent. Empty discovered-price maps reuse original
tools. A separate live comparison must declare its selected provider workload and
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
