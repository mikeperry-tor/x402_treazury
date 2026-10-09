# Catalog startup qualification

`tests/startup.rs` checks rolling replenishment, the default two-request catalog bound,
failure and caller cancellation closing active requests, untouched queued sources,
serial/parallel inventory equality, compatible alias sharing (concurrent and queued),
strict-limit/timeout separation, shared-download cancellation and fresh loads,
per-alias overrides/filters/wallets, strict configuration, and JSON stdout with
progress on stderr. Optional remote failures allow serving with explicit unavailable
status, while required sources, inspection and qualification stay strict. Its isolated SOCKS subprocess verifies that concurrent
catalog and pricing requests retain origin-based discovery identities: same-origin paths
share credentials, different origins do not. It does not require real Tor.

Run the focused regression suites sequentially:

```sh
scripts/zcash.sh test --test startup --test deployment --test provider_catalogs --test pricing --test network_audit
scripts/zcash.sh test --no-default-features --test startup --test deployment --test provider_catalogs --test pricing --test network_audit
scripts/zcash.sh build
```

The ignored SOCKS child is exercised by its parent. Pricing barriers additionally
assert a shared 16-request cap, lower per-source limits, rolling progress past a
stalled endpoint, duplicate-URL coalescing, and cancellation releasing permits.
The deployment suite verifies concurrent source discovery and complete, ordered
priced inventories before serving.

## Reproducible measurement

```sh
scripts/zcash.sh test --test startup_benchmark -- --ignored --nocapture
```

This opt-in test replays the 20 committed provider catalog cases plus two aliases
through localhost, introducing 100 ms of delay per request. All 22 sources have
distinct tool prefixes, with 20 unique spec URLs. It compares complete inventories
across three runs at each concurrency level, then measures unsigned pricing
discovery using provider settings and a fresh process-local cache. Possible
probe destinations are checked to be local before discovery. No external service,
Tor daemon, secret, funded wallet or signature is involved.

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
tools. Issue-tagged providers may be explicitly excluded from a benchmark; record
every exclusion without changing runtime selection.

The catalog default reflects a working theory that leaving headroom below an
anticipated three prebuilt Conflux sets helps transfer tails when a set is unavailable.
This is not an observed Conflux assignment or proven optimum. For rigorous live
comparison, alternate limits against fixed catalogs, retain failures, observe
actual circuit/Conflux assignments and use dedicated persistent Tor state with
mature circuit-build learning. Record changes in that learning and connection
quality. Keep Tor warm; use controlled NEWNYM between comparable runs when needed
rather than restarting it or resetting unrelated state. Local replay cannot
establish live Tor latency, memory costs or anonymity.
