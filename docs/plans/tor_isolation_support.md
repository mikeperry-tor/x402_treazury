# Optional Tor transport with wallet-scoped isolation

## Objective and scope

Add optional Tor routing to the Rust executable using an operator-provided SOCKS5
listener. Use one network architecture, payment state machine, funding worker and
sync pipeline in both direct and Tor modes. Direct mode remains the default.
Connection ownership, isolation identities, client factories and lifecycle rules
apply in both modes; only the outbound connector policy changes.

In Tor mode every runtime outbound connection owned by the application must go
through the configured SOCKS listener, with destination names resolved remotely.
No proxy failure, unsupported SDK operation, retry, redirect, background task or
shutdown recovery may fall back to a direct connection. Incoming MCP listeners
remain ordinary local listeners; they are not routed through Tor. Cargo downloads,
build-time proving-parameter acquisition, external browsers and unrelated programs
are outside this runtime guarantee. Audit SDK runtime parameter downloads separately:
route them through the factory or require preinstalled parameters and fail before
spending. Do not silently let an SDK fetch them directly.

The feature must work with static EVM wallets, managed rotating wallets, standalone
source serving, multi-server deployments and wallet CLI commands. Python is outside
scope. No bundled Tor daemon, control-port dependence, NEWNYM calls, onion-service
hosting or automatic Tor installation is required.

## Privacy contract

Distinct isolation identities must never share an HTTP connection pool, gRPC channel
or Tor SOCKS isolation token. Activity for one ephemeral EVM address retains its
identity across funding, use, retirement, reconciliation and process restart.
Different named profiles using the same actual chain/address intentionally share
that address identity. A profile's replacement address gets a different identity.

Tor circuit isolation is not a guarantee of distinct exits, disjoint relay paths,
or application-level unlinkability. Public swaps expose transaction relationships;
NEAR sees quote recipients, Base RPC providers see queried addresses, and shared
API credentials or request contents can connect requests. One treasury's sync
necessarily covers multiple funding jobs. The checkable application guarantee is
routing and isolation of transports, not elimination of these relationships.

An application can audit and test all its known network constructors, but cannot
prove that arbitrary future dependency code never opens a socket. Document an
optional OS firewall/container setup that permits the executable to contact only
the Tor SOCKS listener for deployments requiring an enforced egress boundary.
The SOCKS listener must honor stream isolation; a generic SOCKS proxy is not enough.

## Current implementation and required changes

Use these entry points to locate the work; inspect their current definitions before
editing rather than assuming line numbers:

- `src/main.rs`, `deployment.rs`: build reqwest clients for serving and catalogs;
  deployment sources currently retain source-specific HTTP clients.
- `src/payment.rs`: `PaidClient` holds an HTTP client independently of payer state;
  static calls snapshot a payer, and managed calls select/admit after the 402.
  `with_http` and `replace_payer` must become identity-aware.
- `src/rotation/manager.rs`: `ManagedPool::pay` refreshes Base state, admits a
  payment, may promote the standby, signs and journals before submitting once.
- `src/rotation/base.rs`: `BaseRpc::view` queries multiple wallet addresses and
  pending authorizations through one client. Split those requests by address while
  preserving the common canonical block anchors and reconciliation transaction.
- `src/rotation/near.rs` and funding worker: a shared NEAR client currently handles
  asset discovery, quote creation and status requests. Identity must be explicit
  and recoverable from the durable job throughout those operations.
- `src/catalog.rs`, `pricing.rs`, `server.rs`: spec loading, startup pricing probes
  and lazy help requests must use discovery identities and shared cache rules.
- `src/treasury/{birthday,server,mod,send,submission,refunds,expiry}.rs` and
  `wallet_cli.rs`: direct gRPC creation, zingolib indexer configuration, background
  scanning, capability checks, transaction lookups and submissions all need coverage.
- `Cargo.toml` pins zingolib, zingo-netutils and pepper-sync to revision
  `c6381534f802b1022041beda4b01c106ad132329`; verify the resolved lockfile as well.
  The SOCKS helper in that stack covers selected operations and has no credential
  parameter. Enabling its feature alone does not proxy the ordinary full sync path.

Audit other outbound paths, examples and optional features, including implicit SDK
fetches. Produce a maintained constructor inventory alongside the network module.
Production consumers must not create their own clients or directly dial sockets.

## Configuration and command surface

Use a strict, reusable network policy structure. Example deployment TOML:

```toml
[network]
mode = "tor"
socks_endpoint = "127.0.0.1:9150"
isolation_secret_file = "../../secrets/tor-isolation.key"
isolation_namespace = "x402_treazury"
socks_auth = "tor_extended"
connect_timeout_seconds = 30
```

`mode` accepts `direct` or `tor`; omission means direct. `9150` is an example Tor
Browser endpoint, not an auto-discovered or guaranteed port. Validate a literal
loopback IP and port for the initial implementation, including bracketed IPv6.
This avoids DNS lookup of the proxy itself and exposing SOCKS credentials over a
remote cleartext link. No automatic port scanning or launching Tor Browser.

In Tor mode require `socks_endpoint` and `isolation_secret_file`. Namespace defaults
to the exact string `x402_treazury`. Authentication choices are `tor_extended`
(default) and `legacy`. Reject Tor-only fields with direct mode rather than leaving
operators with an apparently configured but unused proxy. Validate all policies
before any spec download, listener startup, wallet sync or funding operation.
Unknown fields fail. File paths resolve relative to their declaring config file.
The connect timeout is positive and bounded; retain existing operation deadlines,
quote-validity checks, cancellation and spending limits. Timeouts may require
operator tuning for Tor, but never extend a financial deadline automatically.

A separate file containing this same `[network]` table is accepted through
`--network-config FILE` for standalone source mode, wallet commands and utilities.
Meta-config supplies the table itself; reject simultaneous meta-config and an
external network policy rather than inventing precedence. Provider/source files
cannot override transport mode, proxy or isolation identity. Agents cannot choose
these settings through tool arguments. Resolve root parsing and wallet subcommand
parsing so `wallet ... --network-config FILE` works consistently.

Add `network init --network-config FILE` to explicitly create a random 32-byte
isolation secret with exclusive creation, owner-only permissions, a preexisting
parent directory and symlink rejection. Never overwrite it. The command works
without the Zcash feature, prints no secret and performs no network requests.
Document generating it before launching Tor mode; do not silently create it while
loading catalogs or running inspection. Direct mode does not need this file.

`--show-config` displays effective mode, endpoint, namespace, auth format, secret
file path and identity-assignment rules, never secret bytes or derived tokens.
It remains file-composition-only and does not unlock keys or contact the proxy.
Local `--check`/inventory stays credential-free; a remote spec requires the network
policy's isolation secret in Tor mode but never an EVM signing key. Explain that
transport credentials and wallet credentials are different requirements.
Offline wallet status/backup/address inspection does not connect to Tor. New-wallet
birthday lookup uses the policy before any treasury state is created. Explicit
birthday initialization remains offline. A missing secret in an operation that
needs Tor is a clear actionable error, never permission to use direct mode.

## Isolation identities and credentials

Define a typed, canonical `IsolationId`, never an arbitrary caller-supplied password:

| Operation | Identity |
| --- | --- |
| Unsigned API challenge and paid retry | `evm(chain_id, address_bytes)` |
| Base balance, allowance or authorization lookup | EVM identity of the queried owner |
| NEAR quote and status for a funding job | EVM identity of that job's immutable recipient |
| Zcash deposit broadcast and job-specific inclusion checks | Destination EVM identity from the durable job |
| Refund-address-specific lookups and shielding broadcast | Original funding job's destination EVM identity |
| Shared treasury scan, witnesses, tree queries, birthday/capability checks | `treasury(treasury_uuid)` |
| Birthday before treasury creation | Fresh random bootstrap identity for that invocation |
| OpenAPI, llms.txt/help, unsigned pricing discovery | `discovery(canonical_origin)` |
| Global NEAR asset metadata | NEAR origin's discovery identity |
| Base chain ID/header data without an address | Base RPC origin's discovery identity |

Origin means canonical scheme/hostname/effective port, not URL userinfo or query
secrets. Keep discovery separate from actual unsigned requests made as part of a
paid call. The latter can carry identifying inputs and belong to the payer.
Local files need no identity or transport. Static mode supports address identities
without a treasury. Shared sync remains treasury-scoped even when collecting notes
or transparent refunds associated with individual jobs; do not claim all observations
of those transactions are exclusively EVM-scoped.

Derive tokens with HMAC-SHA-256 using the independent persisted isolation secret,
a versioned domain separator, namespace and a length-delimited identity encoding.
Normalize EVM addresses to 20 bytes and include the chain ID; checksum spelling
must not change identity. Encode the result as bounded ASCII hex. Never use wallet
private keys, mnemonic material or the encryption key as the HMAC key. A plain
address hash permits address guessing; a keyed hash avoids directly revealing the
address to the SOCKS service. Tokens are local transport identifiers, not Tor
account credentials and not HTTP authentication headers.

For `tor_extended`, use username `<torS0X>0` and password
`x402_treazury:v1:<token>` (substitute the validated namespace). For `legacy`, use
username `x402_treazury` and password `v1:<token>`. Bound namespace length so both
fields fit RFC 1929's 255-byte limits; reject invalid encoding. The extended format
is Tor's recommended encoding and has compatible isolation behavior on older Tor
implementations. Never downgrade automatically if negotiation fails. Operators
using legacy must enable `IsolateSOCKSAuth` on the listener. Document and test the
configuration needed for the chosen Tor Browser/daemon; do not infer isolation
support merely from a successful TCP connection.

Changing the secret/namespace creates new isolation identities. Require restart
for such changes, including switching direct/Tor mode; do not migrate live handles.
Back up the secret separately for stable identity across restart. Loss does not
lose funds but prevents reproducing previous transport identities; replacement is
an explicit operator action. Never log tokens, proxy credential URLs or secret data.

## One transport architecture

Introduce a `network` module with an immutable `NetworkPolicy`, `IsolationId`,
`NetworkContext`/factory and cloneable identity-bound transport handles. Direct
and Tor use the same handles, callers, request construction, retries and state
machines. Only the connector implementation differs. Keep typed identity information
available even when direct mode does not need a SOCKS token.

The factory owns HTTP client and gRPC channel caches. Keys include policy generation,
isolation identity, endpoint/origin and relevant transport options (TLS, timeout,
redirect policy and credential scope). Never share a pool across identities simply
because URLs match. Do not put API secrets in printable cache keys. Either separate
clients by auth scope or attach sensitive headers per request; avoid credentials
leaking between a provider, NEAR and Base. Source-specific timeouts and headers must
survive the refactor. Sharing an identity does not require sharing one connection
across all destinations.

Use reference-counted handles: retiring a wallet stops allocating new paid work to
it, but existing calls retain their original signer and transport. Background
reconciliation can acquire that old identity again. Bound cache retention with idle
expiry/weak references or an LRU that never interrupts live handles. Eviction can
recreate a connection with the same identity. Do not pin one client forever for
every retired address. Shutdown cancels/drains through the existing bounded lifecycle.

For HTTP enable reqwest SOCKS support and use `socks5h` semantics: send destination
hostnames to SOCKS, with no prior local resolution. Explicitly disable ambient
`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY` influence in both modes;
network policy is authoritative. Direct means direct, Tor means the explicit proxy.
Preserve HTTPS certificate verification and SNI for the original endpoint. Keep
redirects disabled where currently disabled; other redirects must retain identity,
follow existing scheme/auth restrictions and never bypass proxy policy. No QUIC/UDP
or SDK-managed alternative transport is enabled in this feature.

Loopback HTTP destinations used in tests must go through the fake proxy in Tor
mode; there is no implicit localhost or NO_PROXY escape. Tor itself may refuse
private-address destinations. Document that as an unsupported destination in strict
Tor mode, not a reason to dial directly. MCP inbound loopback connections are exempt
because they are listener traffic, not remote API egress.

## Zingolib and gRPC integration

Implement authenticated SOCKS transport at the common zingo-netutils connector
boundary and pass it through zingolib/pepper-sync constructors. Prefer a small
reviewable upstream contribution pinned to a revision; if necessary maintain an
explicit vendored patch with verification similar to the existing Alloy patches.
Do not modify reference checkouts or Cargo cache sources as the product solution.
Update the normal and combined compatibility manifests/lockfiles together.

The connector must implement remote hostname SOCKS CONNECT with username/password,
then establish ordinary TLS and HTTP/2 to the original gRPC endpoint. Preserve
certificate validation, original SNI, streaming, cancellation, flow control and
request deadlines. Support the full Indexer method set, including streaming compact
blocks, subtree roots, transparent queries, mempool, tree state, tx lookup and send.
Do not grow a second hand-coded subset of the lightwallet protocol for Tor mode.

Identify and eliminate internal `GrpcIndexer::new`/direct socket construction that
bypasses supplied policy, including `set_indexer_uri`, sync restart, proposal
construction, rescan and go-online transitions. A rebuilt internal channel must
retain its identity and connector. The configured treasury sync connector is
always treasury-scoped; transaction submission is a separate identity-bound handle.
A missing injection point is a blocker for claiming Tor support, not an acceptable
partial exception. Do not implement an unauthenticated local forwarding proxy as
a workaround: that would add a parallel network service and lifecycle.

Expose one connector-backed API to our own birthday, Ironwood-capability,
submission and expiry modules too. The existing pre/post-sync Ironwood checks and
chain/network checks remain mandatory and follow the same transport policy.

## Payment admission, rotation and concurrency

Represent a selected payer as a handle containing the actual address, pool generation,
signer access policy and bound network identity. Static payer replacement swaps the
whole handle atomically. Existing requests retain the old handle through completion.
No request may take a signer from one generation and a client from another.

Managed calls select a candidate before their unsigned API request, but do not sign
or reserve money until a validated challenge supplies the amount. Admission receives
that expected wallet/generation and checks it atomically with the existing budget,
balance and exposure rules. It cannot silently return a different payer.

When admission discovers insufficient funds or a concurrent promotion, it may
atomically promote a funded standby and enqueue its replacement as today, but it
returns a typed `payer_changed_before_payment` result without signing for the new
wallet against the old wallet's challenge. Release only unsubmitted provisional
state; preserve all previously journaled liabilities.

For GET/HEAD, permit at most one fresh unsigned challenge request using the new
payer handle, then admission/signing/submission under that same identity. Do not
reuse the old challenge, headers, connection, cookies or request extensions. For
other methods return an actionable retryable MCP error before any signature is
sent; the next caller attempt uses the new active wallet. Do not automatically
replay arbitrary POSTs merely because the first response was 402. This narrowly
bounded behavior is identical in direct and Tor modes. Document the occasional
retry needed at a rotation boundary for non-idempotent tools.

Once authorization is durably journaled, preserve the current one-submission rule:
a timeout, rejected paid response or uncertain settlement never triggers a fresh
wallet, new signature or replay. Retain exposure and reconcile normally. Holding a
transport handle alone is not financial admission and must not reserve funds or
prevent a pool from retiring an insufficient candidate. Bound retries and queues
under rapid concurrent promotions; do not hold a global network lock across calls.

## Funding and background reconciliation

Bind NEAR requests to the durable job's allocated EVM address, never the pool's
current active address. Quote refresh, public/confidential authentication, status
polling, refunds and recovery reuse that identity even after the recipient retires.
Resolve it through immutable wallet/job IDs; missing or ambiguous association fails
closed instead of using a shared default. No changes to quote/slippage/spending
limits, serialized treasury operation ownership, or signed-byte recovery rules.

Per-address Base reads use separate identity-bound handles. Never batch multiple
wallet addresses or authorization owners into one RPC request. Fetch global headers
through a discovery handle, then use the same chosen block hashes for each identity's
balance/authorization reads. Preserve final anchor revalidation and commit one
consistent reconciliation result only when all required reads succeed. Proxy failure
must not publish partial balances or promote an unverified standby.

The shared treasury scanner uses its own identity for wallet-wide synchronization
and proposal witnesses. Job-specific Zcash broadcast, explicit rebroadcast, refund
shielding broadcast and inclusion queries use the recipient identity. Store/recover
the association before creating signed bytes; operator recovery after restart must
not infer identity from current pool roles. Keep existing separate indexer and
submission endpoint configuration; both travel through the network factory.

Retain startup-only pricing behavior: success and failure caching, global concurrency
limit and no TTL-triggered network refresh. Cache keys must include discovery
identity and existing full-URL semantics. Lazy help caching remains process-lifetime.
No network fetch may accidentally inherit the active EVM client because it is handy.

## Diagnostics and failure behavior

Use concise errors such as `tor_proxy_unreachable`, `tor_socks_auth_failed`,
`tor_destination_unreachable`, `tor_connect_timeout`, `network_identity_missing` and
`payer_changed_before_payment`. Preserve causes useful to operators without raw
credential-bearing URLs, wallet keys, authorization payloads or SOCKS tokens.
Errors remain on stderr/MCP error results, never mixed into successful JSON output.
Avoid automatic direct reachability probes when diagnosing a proxy failure.

Show selected transport mode and semantic identity scope in startup/status diagnostics.
Do not claim a circuit ID or exit IP without evidence; no extra public IP-check
service is contacted automatically. Tor Browser stopping should produce bounded
errors, retain pending financial journals and recover through normal retries when
the proxy returns. Retry only operations whose existing semantics permit it.

## Verification and release acceptance

Build an in-process authenticated SOCKS5 test server recording credentials,
destination address type/hostname/port and TCP connection identity. It must forward
HTTP and HTTP/2 fixtures and simulate refusal, bad authentication, truncated replies,
timeout and midstream disconnect. Use deliberately unresolvable destination names
that the fixture maps internally to prove remote DNS handling. Tests must not use
real funded wallets or depend on a running Tor instance.

Required coverage:

1. Default/direct behavior uses the same factory and caller paths. Config validation,
   relative paths, explicit secret initialization, missing/unsafe secret files,
   CLI policy propagation and secret-free `--show-config` work without Zcash.
2. HTTP and gRPC negotiate the expected credentials; different address spellings
   normalize identically, different addresses differ, and independent domains
   (treasury/discovery/EVM) never collide. Token derivation is pinned by vectors.
3. Same-identity requests can reuse connections; different identities cannot.
   Source-specific settings, auth scopes, redirects, concurrent use, retirement,
   cache eviction and restart retain the intended boundaries.
4. Exercise all inventory paths: remote spec, pricing, help, static/managed calls,
   Base reads, NEAR metadata/quote/status, birthday, Ironwood checks, full sync,
   proposal preparation, broadcast, refund handling and recovery. Test all relevant
   enabled SDK runtime download paths or verify their explicit offline requirement.
5. Poison ambient proxy/NO_PROXY settings and make the Tor proxy unreachable.
   A directly reachable fixture must receive zero direct requests. Destination DNS
   must not resolve locally. Run an OS-restricted egress test where available to
   catch internal SDK sockets that a fake-proxy assertion alone cannot observe.
6. Concurrent rotation proves initial challenge and paid submission use the same
   payer identity; changed-generation GETs re-challenge at most once; POSTs fail
   before signing with a retryable result. A paid failure never moves to a new payer.
   Include static replacement and a wallet shared by multiple sources/listeners.
7. Restart while funding is pending proves quote/status/broadcast identities stay
   attached to old jobs. Reconcile multiple addresses through separate connections
   at common canonical anchors; partial failures cannot update admission state.
8. Run isolated Zebra/Zaino consensus tests through the fake SOCKS connector with
   Ironwood active. Include receive, spend, refund shielding, transaction recovery,
   sync checkpoint/restart and proxy loss. Do not weaken pool-capability checks.
9. Regression suites, feature combinations, vendored-patch verification and the
   combined zingolib compatibility suite pass. Audit constructor inventory using
   an automated repository check with narrow test-only exceptions; keep manual
   dependency review because a text search is not a security proof.

An opt-in real-Tor smoke suite can verify public spec fetch, birthday and empty
wallet sync, remote failure handling and Browser-port compatibility without sending
funds. Circuit separation requires a controlled Tor instance with isolation enabled
and observable stream/circuit assignments, not merely seeing SOCKS credentials in
our fixture. Control-port use belongs only in this optional test harness. Live funded
swap/payment qualification remains a separately authorized step. Never start funded
work as an automatic test of networking configuration.

## Implementation sequence and completion criteria

Implement as reviewable commits, with each stage compiling and testing:

1. Add strict configuration, identity types, secret initialization and transport
   factory. Refactor direct mode to use the factory everywhere; retain behavior
   except documented identity-bound admission semantics introduced in stage 3.
2. Add authenticated SOCKS HTTP connector and deterministic proxy fixtures. Cover
   discovery and address-scoped Base/NEAR clients; keep Tor unavailable for serving
   until all required gRPC paths are covered (a clear unsupported error is sufficient
   during development, not a partially private runtime mode).
3. Bind payment transport to payer generations and implement bounded pre-signature
   promotion handling. Preserve financial admission/journaling invariants in both modes.
4. Integrate the pinned upstream gRPC connector change across full zingolib sync and
   our treasury commands, including runtime-fetch audit and compatibility builds.
5. Carry immutable funding identities through all recovery paths, finish end-to-end
   fail-closed and consensus tests, and remove temporary Tor startup restrictions.
6. Document direct/Tor configuration, Tor Browser/daemon requirements, setup/backup,
   JSON diagnostics, privacy limits and optional OS enforcement in README/AGENTS.
   Complete the constructor audit and opt-in unfunded Tor smoke qualification.

Done means one operational stack supports both modes, every runtime remote network
path is accounted for, per-wallet connection isolation survives rotation/restart,
and proxy loss cannot leak traffic or weaken financial accounting. A proxied reqwest
client alongside direct zingolib sync does not satisfy this plan.

## References

- [Tor SOCKS extensions and isolation credential encodings](https://spec.torproject.org/socks-extensions.html)
- [Tor stream isolation and circuit sharing](https://spec.torproject.org/path-spec/stream-isolation.html)
- [Reqwest proxy API](https://docs.rs/reqwest/latest/reqwest/struct.Proxy.html)
- Pinned zingolib source and reference checkouts under `reference_repos/zingolib/`;
  verify proposed APIs against the actual Cargo-resolved revision.
- [Zcash rotation architecture](zcash_rotation.md), including existing admission,
  outbox, refund, expiry and persistent-state invariants that this work must retain.
