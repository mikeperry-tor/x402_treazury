# Vendored dependency patches

The application and its SDK and payment integration tests use the root
workspace's patches and committed `Cargo.lock`. Vendored upstream lockfiles are
provenance inputs only.

## Current reasons and removal evidence

| Patch | Purpose and consumer | Evidence required before removal |
| --- | --- | --- |
| `reqwest-transport.patch` | Post-connect header inactivity, nonempty body progress, disabled recovery of unstarted pooled requests, retained local SOCKS connection causes, and SOCKS-over-Unix transport for `src/network.rs`. | Equivalent unpatched behavior passing `src/network/http_timeout_tests.rs`, `src/network/http_queue_tests.rs`, local vs. target SOCKS classification tests, and Unix HTTP, gRPC, and startup fixtures. |
| `hyper-transport.patch` and `h2-transport.patch` | Discard unopened queued HTTP and gRPC streams (including signed payment `HEADERS`) on cancellation before peer capacity returns. | Equivalent unpatched behavior passing queued timeout and cancellation regressions in `src/network/http_queue_tests.rs` and `tests/network.rs`. |
| `pepper-sync-progress.patch` | Preserve progressing sync streams, loader and worker drain on shutdown, busy-before-dispatch publication, and safe resource-status diagnostics. | Equivalent response inactivity, drain, cancellation, restart, and task-publication behavior through the full sync chain; see [Zingo integration](../docs/zingo-integration.md). |
| `pepper-sync-reorg.patch` | Repair pruned rollback boundaries, rescan beyond the 1,000-block window after proven divergence, keep shorter unsigned tips safe, and repair future shard metadata while preserving accepted inputs and signed bytes. | Equivalent shorter-tip, deep-reorg, and interrupted-scan recovery while preserving accepted work and note spendability. |
| `pepper-sync-diagnostics.patch` | Preserve typed pool and operation context for `src/treasury/diagnostics.rs` without exposing upstream prose. | Equivalent sanitized diagnostics and regression tests. |
| `pepper-sync-manifest.patch` | Standalone workspace manifest matching the pinned Zingo cohort. | Incidental to retaining the checkout; retire alongside Pepper. |

None of these patches implements local funding budgets or historical refund
accounting; they enforce transport timing, cancellation, chain reconciliation,
and sanitized diagnostics. When changing a pin or retiring a patch, review all
consumers and regression tests and update this ledger alongside
[network egress](../docs/network-egress.md) and
[wallet rotation](../docs/wallet-rotation.md).

## Checkout retirement tracking

Prioritize removing entire maintained crate directories over trimming patch
lines. Review the full blocker set before investing in upstream PRs or local
changes; moving a fork to another repository does not retire maintenance.

Four patched directories remain: `hyper`, `h2`, `reqwest`, and `pepper-sync`
(excluded from workspace test selection). `x402-chain-eip155`, Alloy
primitives, and `zcash_proofs` use unpatched registry sources under the
[advisory disposition](../docs/development.md#locked-graph-advisory-disposition).
`zingolib` and `zingo-netutils` use the pinned upstream Git cohort, and
`lightwallet-protocol` 0.4.0 supplies Ironwood fields without a Git override.
See [vendor retirement plan](../docs/plans/vendor_retirement.md) for open
assessment items.

| Checkout | Blocker set | Next action | Upstream status |
| --- | --- | --- | --- |
| `hyper` (1.12.0) | Pending-open cancellation (`hyper-transport.patch`). | Retain until unpatched pending-open cancellation passes through `src/network.rs` alongside patched `h2`. | [1.12.0 client task](https://github.com/hyperium/hyper/blob/v1.12.0/src/proto/h2/client.rs) still waits for `h2` readiness before handling pending-open cancellation ([#4042](https://github.com/hyperium/hyper/pull/4042) covers dispatched body tasks only). |
| `h2` (0.4.20) | Pending-open queue cleanup and scheduler wakeups (`h2-transport.patch`). | Qualify zero-capacity cleanup and explicit and implicit cancellation independently of Hyper; recheck [#962](https://github.com/hyperium/h2/pull/962). | [0.4.20 prioritization](https://github.com/hyperium/h2/blob/v0.4.20/src/proto/streams/prioritize.rs) stays capacity-gated; [reset paths](https://github.com/hyperium/h2/blob/v0.4.20/src/proto/streams/send.rs) still need unopened-`HEADERS` cleanup and wakeups. |
| `reqwest` (0.13.5) | Post-connect header timing, body inactivity, disabled unstarted-request recovery, retained local SOCKS causes, and SOCKS-over-Unix transport. | Identify upstream hooks or a local adapter covering every hunk (including exposing `hyper-util` 0.1.21's hidden SOCKS connector cause). | No retirement candidate qualified. |
| `pepper-sync` (0.7.0) | Stream drain, task publication, typed diagnostics, rollback and rescan recovery, standalone manifest, and constructible sync payload for `tests/sdk_compat.rs`. | Establish a complete upstream path covering transport, diagnostics, and recovery while preserving lifecycle and merge regressions. | Zingolib v8 (`81d799775f11e882896e8710e8af12d6d4d29a41`) still needs these contracts; upstream [task-publication fix](https://github.com/zingolabs/zingolib/commit/3bc86bec73db704e5fba242dc6221497da73e429) addresses only the race. |

Note that `src/network/indexer.rs` owns the 19 `Indexer` and
`TransparentIndexer` trait methods (with transparent queries disabled under the
Ironwood-only policy), and `src/treasury/prover.rs` owns the fail-closed
`UnsupportedSapling` prover stubs; keep copied-code attribution in
`src/network/indexer.rs`, consumer tests, and SDK Git descriptor release inputs
intact.

Update rows whenever dependency work or upstream releases change status,
recording exact issue or PR URLs, revisions, blockers, and next actions (keep
detailed build logs under ignored `target/`). A checkout is retired only when
`Cargo.toml` and `Cargo.lock` use the qualified unpatched source, the local
directory and patch entries are deleted, and all regressions pass.

## Combined dependency compatibility

The root workspace combines registry Alloy core 1.7.3 (`sha3` 0.11.0 ->
`digest` 0.11.3) with the pinned Zingolib v8 cohort (`bip32` 0.6.0 -> `hmac`
0.13.0 -> `digest` 0.11.3) with no hashing manifest override. Independent Keccak
and EIP-712 vectors, payment signing, and `make check-compat` verify this
combination.

## HTTP transport patches

### `hyper` 1.12.0 and `h2` 0.4.20

Exact published crates.io archives patched by `hyper-transport.patch` and
`h2-transport.patch`:

- Pending-open HTTP/2 requests observe response-waiter cancellation and reset
  the stream before peer capacity returns, discarding queued `HEADERS` and
  preventing body transmission.
- `h2`'s scheduler discards cancelled unopened streams (both explicit and
  implicit cancellation, including head entries at zero peer capacity) without
  emitting an illegal `RST_STREAM` on an idle stream.
- Verified by `src/network/http_queue_tests.rs` and `tests/network.rs` over
  local TLS and SOCKS fixtures.

### `reqwest` 0.13.5

Exact published archive patched by `reqwest-transport.patch`:

- **Inactivity timing:** HTTP/1 and HTTP/2 header inactivity begins at
  per-request Hyper connection capture (after DNS, SOCKS, and TLS connect);
  body inactivity ignores empty `DATA` frames and lets backpressured readers
  drain available bytes before timeout rejection.
- **No hidden pooled retries:** Disables Hyper's automatic retry of unstarted
  pooled requests alongside `src/network.rs`'s `retry(never())` so retry
  ownership stays in application layers and replacement connections cannot run
  inside an already-started header timer. See
  [retry ownership](../docs/network-egress.md#retry-ownership). (Zcash tonic
  gRPC does not use reqwest; HTTP/3 is unused.)
- **Local SOCKS cause preservation:** Wraps `hyper-util` 0.1.21's SOCKS
  connector to mark local proxy connection failures with
  `SOCKS local proxy connection failed` while preserving the underlying error
  chain, keeping local proxy refusal distinct from transient target or
  handshake errors.
- **Unix socket SOCKS (`socks_proxy_unix_socket`):** Connects Unix-socket Tor
  endpoints through `hyper-util`'s SOCKS5 connector with remote DNS,
  authentication, timeouts, and local-failure classification matching TCP
  SOCKS.

## Source verification boundaries

Run the verifier whenever maintained vendor sources, patches, provenance
manifests, verifier scripts, or lockfile resolution of vendored crates change;
reuse prior evidence while those inputs are unchanged. `make check-static` runs
both verifiers and `make check-compat` runs `verify_pepper.py` (`make build`
does not run them).

```sh
python3 vendor/verify.py
python3 vendor/verify_pepper.py
```

- `vendor/verify.py` (via `verify_transport.py`) checks `transport-sources.json`
  patch hashes, reverses each patch in temporary copies, and matches every file
  against published crates.io archive hashes.
- `vendor/verify_pepper.py` checks `pepper-sync-sources.json`, rejects
  unexpected files in `vendor/pepper-sync/`, reverses all patches (including
  manifest patches), and compares every byte against the pinned upstream Git
  tree.

These checks verify source provenance and patch reversibility; behavioral
verification belongs to
[combined workspace verification](../docs/development.md#combined-workspace-verification).
