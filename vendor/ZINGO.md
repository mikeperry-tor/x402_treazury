# Zingo connector injection

`zingolib/` and `zingo-netutils/` contain source from zingolib revision
`c6381534f802b1022041beda4b01c106ad132329` (v6.0.0). Upstream licenses are
included. `zingo-connector.patch` is the complete Rust source change:

- `GrpcIndexer::from_channel` wraps an application-owned tonic channel.
- `LightClient::set_indexer` installs that indexer without creating a direct socket.

Every existing RPC, cloned indexer and pepper-sync stream uses the supplied channel.
The application constructs both direct and authenticated SOCKS channels through
`network.rs`; it creates LightClient offline and installs that indexer before sync
or proposal work. It does not use upstream online constructors, URI setters,
migration transmitters, price fetchers or optional nym workers.

Cargo manifests expand workspace-inherited dependencies, pin sibling Git crates to
the same revision and omit upstream standalone test/example/bench targets and dev
dependencies. Library sources (including testutils used by our consensus suite),
build scripts and licenses are preserved. The combined compatibility manifest uses
these same patches. Run `python3 vendor/verify_zingo.py` to check all
reviewed files against `zingo-sources.json`; compare `zingo-connector.patch` with the
pinned upstream commit when updating. Never edit a reference checkout or Cargo's
Git cache to implement this patch.

Sapling parameters are obtained and embedded by the upstream build script, not
fetched by runtime wallet operations. Generated `zcash-params/*.params` files are
ignored and excluded from snapshot verification. Build-time network acquisition is
outside runtime Tor routing. Pre-populate the standard Sapling parameter cache for
an offline first build.
