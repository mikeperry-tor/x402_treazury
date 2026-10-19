# Zingo connector injection

`zingolib/`, `zingo-netutils/` and `pepper-sync/` contain source from zingolib revision
`c6381534f802b1022041beda4b01c106ad132329` (v6.0.0). Upstream licenses are
included. `zingo-connector.patch` records the connector Rust source changes:

- `GrpcIndexer::from_channel` wraps an application-owned tonic channel.
- `GrpcIndexer::from_transport` retains application middleware across all clones.
- `LightClient::set_indexer` installs that indexer without creating a direct socket.

`zingo-progress.patch` applies after the connector and diagnostic patches. It
adds injectable service middleware (constructed only by `src/network.rs`), removes
complete-message timers on sync streams, replaces sync-start polling/abort with
an owned first-poll acknowledgement, and drains loaders/workers with periodic
stage diagnostics instead of forced timeout success. Application transport owns
header/body inactivity; legacy constructors retain library request metadata.
Read retries, protobuf size bounds, mempool drain and financial authority are
unchanged. The added Tower utility dependency supports cloneable injected services.

`pepper-sync-diagnostics.patch` records the additional sync diagnostic changes.
Returned shard-tree errors retain their pool and operation (scan merge, reorg
rollback, or pool-recovery rollback) in a typed error. Existing recovery
recommendations are preserved. No tree insertion, truncation, retention,
serialization, request, or retry policy changes. The application extracts fixed
labels and discards the underlying error before logs and saved status; no tree
positions or hashes are exposed. The synthetic `scan_merge_conflict_preserves_pool_context`
unit fixture exercises actual conflicting merges in all three shielded pools,
without wallet keys or network requests. Pepper Sync retains its upstream unit-test
dependencies for this fixture.
The diagnostic patch also removes raw `err` events from latest-block and tree-state
fetch instrumentation and places those spans at debug level. The application owns
sanitized failure reporting and describes retries according to its caller's schedule;
no upstream status prose is emitted by those spans, including at debug level.

`pepper-sync-tip-reorg.patch` fixes same-height fork verification. Once the stored
chain-tip hash is known to disagree with the indexer, sync truncates that block's
wallet data and verifies the replacement tip against its retained predecessor.
Deeper forks use the existing backward verification/truncation path and reorg
allowance. Starting with a wide range could span a whole fork with matching seams
and retain orphaned confirmed transactions. The application's funded synthetic
lifecycle covers both a tip-only and a deeper same-height fork, retained source
accounting, blocked new work and canonical reappearance. Apply this patch after
the diagnostic/progress patches above.

`zingo-mock-chain.patch` fixes the optional synthetic-chain indexer fixture: its
`GetLatestBlock` hash comes from the current stored branch, matching `GetBlock`
and tree states after a same-height reorg. It does not change production indexer
behavior. Apply it after the patches above.

Every existing RPC, cloned indexer and pepper-sync stream uses the supplied channel.
The application constructs both direct and authenticated SOCKS channels through
`network.rs`; it creates LightClient offline and installs that indexer before sync
or proposal work. It does not use upstream online constructors, URI setters,
migration transmitters, price fetchers or optional nym workers.

Cargo manifests expand workspace-inherited dependencies, pin sibling Git crates to
the same revision and omit upstream standalone test/example/bench targets and dev
dependencies, except Pepper Sync’s unit-test dependencies. Library sources
(including testutils used by our consensus suite),
build scripts and licenses are preserved. The combined compatibility manifest uses
these same patches. Run `python3 vendor/verify_zingo.py` to check all
reviewed files against `zingo-sources.json`; compare `zingo-connector.patch` with the
pinned upstream commit when updating; check `pepper-sync-diagnostics.patch` and
`zingo-progress.patch` too.
Never edit a reference checkout or Cargo's Git cache to implement these patches.

Sapling parameters are obtained and embedded by the upstream build script, not
fetched by runtime wallet operations. Generated `zcash-params/*.params` files are
ignored and excluded from snapshot verification. Build-time network acquisition is
outside runtime Tor routing. Pre-populate the standard Sapling parameter cache for
an offline first build.
