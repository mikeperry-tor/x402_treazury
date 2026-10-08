# AGENTS.md

Guidance for code agents working on **x402_treazury**, a Rust application with
executable **x402_treazury** and library crate **x402_treazury**.

## Purpose and layout

Expose x402-paid HTTP APIs as plain MCP tools. The server owns payment credentials
and handles challenge/sign/retry; agents receive ordinary tool results. Reusable
TOML providers compose into sources and multiple authenticated HTTP listeners.
A single Zcash treasury can fund rotating Base USDC wallet pools through NEAR
Intents. Optional Tor uses the same network/state machinery as direct mode.

- `src/main.rs`, `src/cli.rs`, `src/wallet_cli.rs`: executable command tree and treasury commands.
- `src/output.rs`: bounded typed image/text results and explicit JSON image mappings.
- `src/catalog.rs`, `config.rs`, `pricing.rs`: OpenAPI tools, TOML composition,
  startup-only pricing discovery.
- `src/server.rs`, `deployment.rs`, `payment.rs`: MCP transports, multi-listener
  lifecycle, payment signing and identity-bound admission.
- `src/network.rs`: all production outbound HTTP/gRPC constructors and isolation.
- `src/cover/`: optional bounded unsigned ranges, request padding, sampling and scoped evidence.
- `src/rotation/`: pool assignment, durable SQLite state, Base/NEAR adapters and funding.
- `src/treasury/`: embedded Zingolib, serialized commands, sync, proving and recovery.
- `providers/`: 27 TOML definitions; curated OpenAPI catalogs in provider subdirectories.
- `examples/`: deployment, public-demo and Tor configurations; Rust development utilities.
- `tests/`: offline Rust suites, local protocol fixtures and pinned provider contracts.
- `vendor/`: reviewed Alloy/Zingo patches and provenance verification.
- `compat/`: independent dependency compatibility and Cargo-managed protoc workspaces.
- `docs/`: network inventory, funded-test runbook and architecture plans.

## Live integration runner

`examples/live_integration.rs` and `tests/integration_preparation.rs` share
`examples/live_integration/driver.rs`; do not duplicate CLI/module wiring. The
[runbook](tests/live/INTEGRATION.md) describes supported commands, evidence bounds
and scenario assertions; [testing](docs/testing.md#qualification-status) states
qualification limits. Full automated lifecycle and combined cover/payment remain [deferred](docs/plans/deferred/live_integration_acceptance.md).
Retained evidence never grants spending authority or permission to replay cases.

- Use production wallet commands for init/addresses/sync/backup. `plan` is offline;
  `prepare` freezes complete catalogs and validates exact listener/source/tool
  arguments; `run` selects direct/external/owned Tor from the manifest. Keep exact
  executable/config/catalog pins during execution. Read-only evidence inspection
  and wallet administration must not require an unchanged development checkout.
- Started or reserved runs are observation-only, including `report --eligibility`. New reviewed
  runs use the same registry and retain cumulative API/source/job charges. Never
  replay attempted/uncertain work, reset deadlines/budgets, adopt unknown liabilities
  or transfer old permits. Preserve explicit source/pool/listener scope on selection.
- Production admission and durable funding permits remain authoritative. Zero-new-
  funding is installed before pool setup and every supervised reopen; it permits
  safe use/reconciliation of existing assets, never allocation or preparation.
  Prepared bytes retain ordinary recovery obligations. Positive funding needs
  explicit run authority, `--allow-funding` and production auto-funding policy.
- A unique durable case claim precedes provider I/O. Only reviewed arguments and
  current authority can start work. Completion after expiry records already
  accepted work; it grants no new start. Bound private child output and drain by
  owned process handle on cancellation/parent EOF. Preserve an in-progress drain
  across cancelled waiters; never replace it with a PID-based kill/restart.
- MCP success, provider semantics, seller receipts, canonical authorization
  resolution, canonical debit proofs and saved balances are distinct facts. Nested
  `isError:true` fails provider semantics; arbitrary `error` properties do not.
  Missing assertions/proofs remain incomplete. Do not infer zero fees or readiness.
- Concurrency assertions use one application clock, exact wallet bindings, unique
  authorizations and admission-time balance/block/exposure. Promotion, pending-
  refill service, confirmed replacement credit and real restart are separate gates.
  Only structurally verified untouched depletion suffixes can be skipped without
  charge. Every attempted case retains its reservation.
- Catalog/help/pricing instrumentation observes real production paths; no extra
  probes or signed retries. Preserve typed catalog failure stages and HTTP codes.
  Continue independent qualification catalog loads within the normal rolling bound;
  aliases share failures without retries. No partial executable inventory. Complete
  catalog snapshots have a shared 128 MiB bound; ordinary evidence is 16 MiB.
- Observe the actual help OnceCell initializer: fetch/hit/shared are distinct and
  failures stay uncached. Pricing remains one-shot startup discovery. Frozen reload
  inventories must match; reconstruct price descriptions only from that session's
  recorded production prices. Do not loosen schema assertions or trim documentation.
- Owned Tor uses installed software and dedicated persistent state, separate from
  Tor Browser. Preparation and execution have separate confined process/control
  evidence. Required payer streams must reach provider origins; RPC-only traffic
  cannot qualify them. Unknown credentials, changed destinations, local DNS, circuit
  sharing or incomplete capture invalidate isolation evidence. Never derive traffic
  authority from observed tokens. External SOCKS alone cannot qualify isolation.
- Outage qualification is a separate keyless deployment with no declared managed
  wallets, funding or source management. Keep the same keyless process/cache across
  stopped SOCKS: listing/cached help succeeds, fresh help has typed connection
  failure. A stopped listener does not qualify stalled in-flight timeouts.
- Keep historical private evidence immutable/readable. SQLite events mix plain
  text and JSON: guard extraction with `json_valid` inside the expression, not only
  WHERE clauses; malformed relevant financial events fail explicitly. Read-only
  receipt recovery imports only exact original canonical correlations, never new
  execution authority. Saved accounting/attribution uses exclusive ownership and
  clean process evidence; refund shielding consumes its fee, not the returned
  output twice. Retired funds are not active capacity.
- The versioned report summary joins validated observations, preserving failures,
  unknowns and unattempted stages. JSON is private; Markdown omits raw identifiers,
  credentials, provider prose and paths. Tor-only outcomes do not identify causation.
  Preserve explicit qualification limits in maintained guides.

Treasury sync accepts at most three blocks of tip advancement after scanning a
freshly observed target, identically for direct and Tor. The starting observation
age includes sync time; never renew freshness at completion. Persist the final
`observed_tip_height` separately from scanned `height`; confirmations use the latter.
Expiry recovery requires equality on both syncs before releasing reservations.
The treasury is exclusively spent here; accepted lag appears at debug level and in CLI output.

## Configuration and wallet workflow

`serve --config FILE` starts a deployment; `--meta-config` is its compatibility alias.
`serve --provider FILE` starts standalone serving. `catalog tools`, `catalog tags`,
and `catalog route` inspect tools; routing previews accept standalone providers only.
`config show` is offline file inspection; `config check` fetches and validates
catalogs for deployments or standalone providers. No command means help, never
serving. Keep these modes in the shared Clap tree; only `serve` accepts listener,
authentication and supervised funding options. Wallet commands
accept the same deployment file without loading catalogs or requiring listener
tokens. `wallet bootstrap` also consumes wallet assignments/funding settings and
funds initial managed pairs; other wallet administration never starts funding
workers. Do not combine deployment configuration with standalone location/network overrides.
User-facing deployment examples live in `examples/deployments/`; network-only examples in
`examples/network/`. Preserve user-owned local copies when changing layouts.

An omitted treasury ID resolves from existing state at runtime; a supplied ID is
an assertion, never permission to adopt another wallet. Offline `config show`
reports that resolution policy without opening state. An omitted key path resolves
to `wallet.key` inside the owner-only state directory; the whole directory contains
sufficient decryption material. External key paths remain supported. Never replace
existing state on init. Endpoint defaults are shared with birthday lookup; explicit
missing environment references fail, and submission otherwise follows the indexer.

Managed serving completes initial pool bootstrap before discovery and defaults to
automatic replacement funding, within configured USDC budgets and ZEC/fee safeguards. `auto_fund=false` pauses it; build features, inspection,
wallet initialization and sync never authorize automatic funding. Qualification
restrictions remain authoritative, and bounded live demos retain explicit opt-outs.

## Commands and test discipline

HTTP listeners require bearer authentication by default. Host validation is independent: `allowed_hosts` replaces loopback defaults;
`disable_host_check=true` or an empty list disables the allowlist with a startup
warning. Reject an explicit list combined with the disable flag. Standalone HTTP
uses `--allowed-hosts` or `--disable-host-check`; both may accompany `--no-auth`.
Explicit `servers.NAME.auth=false`
omits `bearer_token_env`; standalone HTTP uses `--no-auth`. Warn at startup when
disabled. Never log authentication headers or token values. Auth failure warnings
are bounded per listener/category, with explicit suppression notice/counts; HTTP
401 bodies distinguish missing, malformed and incorrect-token failures. Live
qualification must reject unauthenticated listeners.

`scripts/check.sh` validates only the default Zcash build, including all-feature
Clippy and dependency compatibility. See `scripts/README.md` for optional vector
regeneration. Individual commands:

```sh
scripts/zcash.sh build
scripts/zcash.sh test --all-targets -- --test-threads=1
scripts/zcash.sh clippy --all-targets -- -D warnings
cargo fmt --check
python3 vendor/verify.py
python3 vendor/verify_zingo.py
python3 scripts/check_compat.py
```

Run commands from the repository root. `scripts/zcash.sh build` (or no command) defaults
to release with incremental caches disabled, including the protoc helper.
`build --developer` enables an incremental development build; it cannot accompany
`--release` or `--profile`. Explicit build profiles disable incremental caches.
Tests, checks and Clippy retain normal Cargo profiles and inherited environment. `rust-toolchain.toml` pins the development
compiler; the Zcash/check wrappers enforce it for non-rustup installations too.
Run `sh scripts/check_toolchain.sh` before plain Homebrew Cargo commands.
Release reproducibility uses `python3 scripts/reproducible.py` on the version-checked
macOS profile; see `docs/reproducible-builds.md`. It builds committed HEAD only,
requires prefetched dependencies/parameters and blocks build-time network access.
Do not weaken the sandbox or silently update toolchain/profile pins to make a run pass.

`target/release/x402_treazury serve --help` lists serving
options; `x402_treazury wallet --help` lists treasury commands. Default builds include
the embedded Zcash wallet; `--no-default-features` disables it. Plain Cargo builds
require protoc; the Zcash wrapper supplies Cargo-managed protoc and respects
`--no-default-features`. `scripts/coverage.sh` measures default-build application
coverage with matching LLVM tools and writes HTML/JSON/text under `target/coverage`. First builds
may fetch public proving parameters; Cargo/build traffic is outside runtime Tor policy.

Tests use temporary state and public unfunded deterministic keys. Never load real
wallet keys or seeds into default tests. Local fixtures require localhost binding.
Do not run no-default-feature builds, tests or Clippy unless the user explicitly requests
that configuration. Zcash is included in the standard build; static-wallet deployments
simply do not configure a treasury. Use focused tests for changes and avoid repeating
full suites after a localized fix. If explicitly requested, run feature suites
sequentially because their CLI tests share the same executable path.
`tests/stdio.rs` tests the real process; provider snapshots test full tool contracts.
Python is optional development tooling, never an application/runtime dependency.
Consensus tests need Docker and explicit ignored-test invocation; see `tests/REGTEST.md`.
Live-funded qualification is a separate operator-authorized step.

Keep `.env`, `state/`, `secrets/`, local example overrides, and reference checkouts
untracked. Do not inspect or print secret values unnecessarily. User-owned `TODO`,
`run_servers.sh`, `FUCK.md` and editor files are not part of automatic cleanup.
All logs go to stderr: stdout is the MCP wire or the requested CLI JSON output.
Process `RUST_LOG` overrides the default `warn,x402_treazury=info` filter for all
commands. Info reports catalog/pricing completion, listener startup and funding
progress; download/parse/protocol details and accepted tip lag are debug. Warnings
indicate failures, degraded operation or actionable configuration issues. Sync
failures log fixed categories, never upstream prose.
Keep ordinary CLI errors legible even with backtrace environment variables enabled.

## Agent source management

`src/catalog_state.rs` holds immutable deployment-wide snapshots; `src/discovery/`
implements optional meta-config-only registration, imports, permissions, SQLite
persistence and management tool definitions. See [docs/agent-sources.md](docs/agent-sources.md)
and [the runtime architecture](docs/architecture.md).

- One named `source_management.wallet` serves all dynamic sources by default. Only
  explicit listener overrides split wallet sharing. Never run automatic per-source
  pool assignment or trigger allocation/funding from registration. The README's
  arrangement table describes deployment, listener and group sharing; preserve this
  boundary if adding higher-level agent privacy defaults.
- Listener ID through bearer-authenticated routing is the principal. Agent/session
  fields never select an owner. `source_management=true` enables endpoint-local
  registration only; static TOML sources are immutable. Registry configuration, not
  agent arguments, controls persistence. Explicit listener `wallet` overrides the
  shared dynamic wallet.
- Mutation tools are separately authorized under `x402_treazury_`; API filters cannot
  remove them. Dynamic names contain the full UUID under `dyn_`. Both prefixes are
  reserved in participating deployments. Every direct/fallback call captures one
  snapshot. Fallback validates an opaque endpoint/process/revision-bound tool reference before HTTP.
- HTTP is stateless: expose stable search/call tools, without advertising list-change
  notifications. Standalone stdio has no dynamic management. Cursors bind to process
  catalog instance, generation and caller/query. The five tools are sources_search,
  source_details, source_add, tools_search and tool_call under `x402_treazury_`.
  Directory search (browse or best mode) and details use embedded reviewed x402 List
  schemas and the endpoint's source-management wallet through normal payment
  admission and network policy.
  They require no static directory source or selector, do no startup catalog I/O,
  and expose only the management wrappers. Reject mixed-mode search arguments before
  I/O; dispatch once to /services or /best and use a directory slug for details.
  Keep their schemas aligned with the pinned directory contract; importing sources never grants a directory transport bypass.
- Agent imports require public HTTPS and bounded bytes/reference expansion. Direct
  DNS validation happens in the network factory's resolver; Tor preserves remote DNS
  and depends on Tor's internal-address rejection. Dynamic paid clients must retain
  `public_destinations()`. No pricing probes on registration or refresh.
- Durable mutations commit before publishing all listener views. A cancelled waiter
  does not undo an accepted blocking-worker job. Add deduplicates canonical spec URLs
  per endpoint, including races and restart. Revalidate endpoint enablement, local
  scope, quotas and format on restart; disable invalid records and retain tombstones.
  Registry, ownership sidecar and SQLite sidecars must not alias config/treasury paths.
- Source removal never retires wallets or deletes payment/funding/refund journals.
  `config show` never opens the registry; `sources inspect` is read-only and cannot
  create a missing registry. `sources refresh/remove` require exclusive registry ownership
  while serving is stopped; refresh fetches and validates without wallet credentials.
  Persistent metadata/specs can still reveal user interests.
- Tests in `src/discovery/tests.rs` inject public-URL documents/local fixtures only
  under `cfg(test)`; no production loopback bypass is permitted. Tests use unfunded
  keys/temporary state and cover actual HTTP management, caps, concurrency and recovery.
- `include_operations = ["GET /path", ...]` is an exact provider-level method/path
  allowlist, intersected with other filters. Keep the x402-list curated read surface
  closed to newly added upstream endpoints. It does not affect generated names.

## Payment and catalog invariants

- Base USDC is the supported asset. Disabling the amount cap retains the allowlist.
  Static Permit2/upto needs an externally provisioned allowance; do not auto-approve.
- v2 uses PAYMENT-REQUIRED / PAYMENT-SIGNATURE / PAYMENT-RESPONSE; v1 remains
  supported in static mode. Sanitize challenge resource descriptions to 500 chars
  before signing because facilitators reject longer echoed descriptions.
- No replay after a journaled signed submission. A seller response alone does not
  release managed exposure; use confirmed chain evidence.
- Preserve vendor descriptions whole by default. Any text limit must be explicit
  (`max_description_chars`, `--max-response-chars`). Authored overrides apply last.
- Provider/source `include_tools` and `exclude_tools` use the same case-sensitive
  `*`/`?` patterns as listeners, after path/operation/tag selection and collision
  naming. They also select help tools; unknown exact names fail, unmatched patterns
  warn, and empty inventories fail. Listener filters can only narrow source tools.
- Path filters respect segment boundaries; query/body collisions rename the exposed
  body argument and restore its original body key on the wire. Header parameters
  are not agent-settable. Normalize OpenAPI boolean exclusive bounds to numeric ones.
- Startup pricing is unsigned GET-only, bounded and cached including failures;
  expiration never triggers refresh. `catalog tools --discover-pricing` explicitly
  reuses that path without signers, wallet access or listeners; ordinary inspection
  never probes. Respect source probe opt-outs. Distinguish advertised estimates,
  observed probe prices, metered maximums and unknown prices in descriptions.
  Help is lazy and cached after success.
- Optional provider `credit_pricing` converts structured credit tariffs into per-tool
  USDC display estimates using an explicit decimal rate. Preserve original billing
  prose and estimate provenance; unsupported metadata must remain visibly unknown.
  Conversion never changes payment authority. Instruction overrides and tag filters
  retain estimates; authored tool description overrides still apply last.
- `src/http_cache.rs` persists only header-supported remote catalogs and derived
  pricing estimates under existing treasury `state_dir/http-cache/`. Respect
  `http_cache_enabled=false`, network/transport scope, HTTP freshness and validators;
  never persist payment challenges or serve stale data on origin failures. Cache
  setup must not create treasury state. Qualification bypasses disk persistence.
  `catalog warm --config FILE --source ID --direct` is a dedicated unsigned-only
  process: direct-warm entries retain separate provenance and the target configured
  network policy. Normal loading accepts them only while fresh, warns on reuse,
  never sends their validators over Tor and never refreshes them directly. Warming
  opens no wallet with `--direct` and cannot run during qualification.
- Optional deployment `[discovery_relay]` names a local Curl bootstrap provider and
  optional wallet override, enabled separately for serve/warm. Otherwise choose
  one effective assigned profile per source by deterministic SHA-256 rendezvous
  selection; config show exposes discovery_wallets. Keep the discovery-wallet-v1
  selection domain stable. Shared sources do not multiply paid discovery across
  listeners; different wallet scopes cannot coalesce relay requests or relay cache data.
  Relay-service failure/cancellation disables the shared relay across all wallets.
  Attributed target failures are cached without disabling other targets. Target discovery stays
  unsigned GET-only; only relay payment uses PaidClient. Reuse existing wallet caps
  and byte/time limits. Serialize calls, coalesce successful targets, and disable
  the relay for the run on relay-service failure or cancellation before any retry can spend.
  Every failed remote catalog fetch or unsuccessful pricing probe can use the
  configured relay once; no origin-error allowlist. Log source, discovery stage,
  fixed failure category and available HTTP status without URLs or upstream prose.
  Relay cache entries retain separate provenance and explicit origin freshness;
  never revalidate through Curl, use outer relay headers or persist challenges.
  Serving initializes wallets before relay catalog I/O; warming denies funding and
  opens only resolved wallets for selected sources. Ordinary managed serving with
  auto_fund bootstraps initial pairs before discovery via the same supervised path
  as `wallet bootstrap`; qualification restrictions preserve their existing lifecycle.
  Ordinary inspection remains unsigned; qualification cannot invoke paid relays.
- HTTP MCP is stateless and JSON-response based, with bearer auth enabled by default.
  Disabling the gate requires the explicit configured or standalone opt-out.
- Typed results flow through `PaidClient::execute_response`, `BoundTool::invoke_output`
  and `Server::invoke_output`, including dynamic fallback. PNG/JPEG/WebP bytes and
  explicit method/path JSON base64/data-URI mappings produce MCP image blocks.
  Preserve metadata with visible attachment markers; never lossy-decode binary
  API success bodies, silently truncate base64, fetch media URLs, or retry paid output
  conversion failures. `image_limits` bound decoded bytes/count in addition to
  the HTTP response-byte limit. Text-only convenience methods reject attachments.
  Format signatures are checked, not complete image decoding. See `tests/media.rs`.
- Provider caveats and intentional exclusions live in `providers/CAVEATS.md`.
  Use TOML providers instead of adding provider-specific server implementations.

Provider reliability metadata lives in authored TOML `reliability_tags` and
`reliability_note`, independently of OpenAPI selection tags. Keep warning categories tied to observed behavior; do not infer Tor causation from Tor-only tests.
`src/provider_status.rs` defines accepted tags and stderr warnings. CLI serving
warns before catalog I/O, once per listener-bound source/tag; inspection is quiet.
Metadata never filters tools or changes payment/retry policy. Preserve these
properties when adding tags or changing startup. `slow_pricing` identifies slow
unsigned pricing discovery, independently of catalog or paid-call performance;
its note must not assert Tor causation. Live performance benchmarks can explicitly exclude all authored
issue tags and must record the exclusions; this does not change runtime selection.

## Configuration, treasury and financial ownership

Managed profiles use `funding_amount_usdc`, `max_funding_amount_usdc`,
`max_api_payment_usdc` and integer `max_conversion_overhead_percent`. An omitted
maximum funding amount permits no increase above the target. Funding-level
`daily_funding_limit_usdc` and `total_funding_limit_usdc` are optional gross
allocation budgets across all pools, not API-spending budgets. Managed deployments
require at least one aggregate budget (daily USDC, total USDC or daily ZEC). Reserve accepted
quote outputs alongside source reservations in the serialized store worker;
derive history from original authenticated quotes and source journals, including
archived operations. Missing relevant history fails closed. Pending allocations
survive UTC rollover; refunds do not replenish total USDC allowance. Preserve
canonical expiry/unprepared recovery and never reset budgets on restart or rename.
Native safeguards are `max_funding_spend_zec` (optional per wallet transfer),
`daily_treasury_spend_limit_zec` (optional treasury-wide),
`max_funding_transaction_fee_zec` and `max_refund_shielding_fee_zec` (both default
0.0003 ZEC). Network-fee settings reject proposals; they never override standard
wallet fee calculation. Current TOML rejects obsolete option names; historical
qualification evidence remains readable without gaining new authority.

See [wallet rotation](docs/wallet-rotation.md), [architecture](docs/architecture.md)
and README wallet-sharing tables for implementation details. TOML source wallet
bindings override server defaults, followed by automatic assignment. Named profiles
control sharing. Generated `auto_v1_` pool names are persistent contracts; renaming
scope/identifiers can create new pools. Inspect without opening state or secrets.
All declared managed pools initialize, including those unused by selected tools.

- Treasury commands serialize under exclusive ownership. Persist encrypted wallet
  snapshots and outgoing operations atomically; revision CAS prevents stale saves.
  Never use upstream plaintext saves or quick-send helpers. Failed/cancelled
  preparation or persistence requires owner teardown/reopen, not reuse.
- Calculate-only preparation checks current quote, exact output/fee/expiry and
  aggregate budget. Commit prepared bytes and costs before broadcast intent;
  submission consumes a single-use capability. Unknown sends remain possibly
  broadcast. Only explicit reconciliation may rebroadcast identical saved bytes.
- Absence, expiry or seller responses never release financial exposure. Recovery
  needs fresh canonical evidence; expired-source recovery verifies unspent inputs
  against retained preparation. Refund shielding charges only its fee, not returned
  principal twice. Reorgs quarantine accounting instead of silently repairing it.
- Double buffering promotes once under the pool gate, retains old liabilities and
  queues a fresh recipient. Admission reserves before signing and requires current
  balance/block/generation. Late credit must not overwrite newer roles/evidence;
  completed funding credit is idempotent. Retired balances are not active capacity.
- `PreparationDeferred` alone certifies that preparation never began; only this
  typed result plus no outgoing/budget record can rewind PREPARING. Missing bytes
  or error prose cannot establish retry safety.
- Base RPC failover restarts a complete read-only view, never mixes providers or
  retries signed work. Data-validation/TLS failures do not trigger fallback.
  Immutable original operation identities govern recovery, not current pool roles.
- Logs/status expose bounded fixed failure categories and numeric codes, never
  upstream credential-bearing prose. Drain deliberately for transaction safety;
  tell operators promptly why shutdown is waiting.

Catalog loading uses rolling scoped futures, deterministic output and a load-scoped
alias cache keyed by URL, timeout, limit, transport and discovery identity. Failures
or cancellation cannot publish a partial inventory. Pricing is process-cached
startup-only discovery with independent request limits. See [startup](tests/STARTUP.md).
Provider TOML composition is one-level; omitted inherits, present values replace,
empty clears. Relative paths belong to their declaring file. Unknown fields and
bindings fail. Preserve exact operation/tag filters and request schemas. Provider
defaults omit diagnostic/account/fund-moving routes and use lazy help when useful.

## Rust network policy

`src/network.rs` is the only production egress factory. A process
uses one immutable policy: direct by default or strict authenticated Tor SOCKS5.
The explicit unsigned-only `catalog warm --direct` process is the cache-warming
exception described above; it cannot return a serving deployment.
Meta-config accepts `[network]`; standalone/wallet/example commands accept
`--network-config FILE`. They cannot override a meta-config's policy. No isolation
key file is used. Canonical typed identities produce deterministic SHA-256 tokens;
see `docs/network-egress.md` and `examples/network/tor.toml`.

Tor defaults to 120-second connection establishment and a 240-second complete
request floor (`request_timeout_seconds`). Effective HTTP timeout is the maximum
of the caller budget and this floor, including body download. Outer import,
application-owned treasury RPC and admission deadlines respect the same policy;
direct budgets, signed authorization expiry and funding safety deadlines remain
independent. Network inspection exposes the floor. Embedded sync RPC deadlines
remain upstream-owned; see `docs/network-egress.md`.

Provider HTTPS requires HTTP/2 and TLS 1.3. `Config.allow_http1` and
`allow_tls12` default false and independently permit older versions. Propagate
`Config::transport()` through catalogs, pricing, help and paid bindings; include
it in HTTP pool, catalog-download and pricing-cache keys. Agent imports and
agent-added tools use strict defaults. Existing cleartext HTTP and treasury/RPC/
gRPC policies are unchanged. Keep HTTP/2 explicit in reqwest features. Log actual
response HTTP versions with stage only; never claim TLS version was observed from
reqwest's response metadata. Test TLS-over-SOCKS multiplexing with concurrent
blocked streams, distinct wallets/discovery and strict/compatible pool separation.

Both modes use identity-bound HTTP pools and gRPC channels. Keys include the owning
Tokio runtime; sync releases channels before destroying its private runtime. Never
construct reqwest clients, tonic channels or SDK RPC providers outside the factory.
`tests/network_audit.rs` guards this boundary. Tor uses remote DNS, mandatory SOCKS
username/password negotiation, normal TLS validation, no direct fallback, no ambient
proxy/NO_PROXY influence, and no automatic transport-wide HTTP retries or redirects. The Base adapter alone
allows one bounded retry of unsigned read RPC transport failures on the same
client/identity/payload; signed API requests are never retried. Local MCP
listeners remain inbound-only. `config show` exposes policy, never derived tokens.

`PaidClient` owns payer state and source timeout, not arbitrary HTTP clients. Managed
calls select a candidate before the unsigned request and require that same wallet/
generation at admission. Promotion before signing returns `PayerChanged`: GET/HEAD
may fetch one new challenge; other methods return an actionable error. Signed paid
requests are never replayed. Static replacement preserves in-flight signer/transport.
Base address reads and NEAR quote/status use the actual immutable EVM identity;
shared scans use the treasury UUID; catalog/help/pricing use discovery origins.

Schema version 10 adds immutable `operation_network` recipient bindings for refund
shielding. Funding operations resolve identity through original job/recovery records,
not current pool roles. Missing/conflicting identities fail closed before outbound
lookup/submission. Vendored zingolib/netutils expose only channel/indexer injection;
verify with `python3 vendor/verify_zingo.py`. Do not modify reference
checkouts or Cargo caches. Build-time Sapling acquisition is not runtime traffic.
The opt-in `tor_consensus_lifecycle` test runs alone because it installs Tor policy.

## Optional cover traffic

See [docs/cover-traffic.md](docs/cover-traffic.md). The process switch defaults on
for Tor, off for direct; explicit false wins. Authored providers/sources can opt out
with top-level `cover_traffic_enabled=false`. Static sources without a profile use
a same-origin HTTPS catalog/help candidate; no path is guessed. Explicit profiles
can set one same-origin fallback URL. Agent registration never grants cover. Retain actual
attempt identity, exact origin, timeout and transport in owner/pool keys. Cover is
unsigned and cannot enter payment/funding paths. Real requests preempt cover until
final headers. Failures cannot replay signed requests. HPACK padding is sensitive /
never-indexed; HTTP/1-compatible profiles skip it explicitly. Initial qualification failure can try the configured fallback once within existing
budgets, except rate limits/deadlines/protocol failure/cancellation. Selected
resource and negative results persist for owner lifetime. Cover diagnostics go to stderr,
with listener/source attribution and explicit limits/refusals; never add diagnostic
MCP tools or cover advisories to API results. Optional-task shutdown precedes financial drain;
aggregate stderr evidence follows it. Pool reuse is best-effort, not a same-channel
or privacy guarantee. `cover_fixture` and the ignored distribution matrix provide
zero-spend qualification; see `tests/live/INTEGRATION.md`.

## Visible resource limits

Buffer bounding and output truncation must never be silent. When a bound rejects,
truncates or discards input/output, emit a clear log/output message and give the
relevant consumer explicit evidence: an agent-facing tool error or truncation
marker, an HTTP error, or a CLI error. Identify the affected resource, configured
limit and adjustment setting where applicable. Never return an incomplete document
as if it were complete. Do not log content, credentials or sensitive URLs merely
to explain a limit. Tests must verify both the consumer-visible evidence and logs.
Display truncation is separate from a download/memory bound. Oversized paid
responses may already have settled; do not retry them automatically.

## Documentation and repository changes

README.md is the user entry point and documents wallet-sharing arrangements.
`docs/README.md` indexes maintained architecture, wallet, network, agent-source and
testing guides. `docs/plans/` describes unfinished work only. When a plan is
implemented, migrate useful explanations to those guides, preserve concrete
remaining work in a focused plan, fix incoming links and remove the completed
plan. Do not add an implemented-plan archive; Git retains tracked design history.
Write generated reports, metrics and audit inventories under ignored `target/` or
private external storage. Commit durable findings as code, configuration, tests or
maintained guides, never dated journals, handoffs or report copies. Keep private
financial ledgers and failure evidence; Git cleanup never grants replay authority.
AGENTS.md describes current layout, commands and invariants; update affected sections
in place, without changelog entries or append-only migration notes. Preserve stable
state formats, generated pool names and Tor token domain separators when renaming
files or product labels. The `x402_treazury-tor-isolation-v1` domain and `X402_*` environment
variables are intentional compatibility contracts, not stale product branding.

Do not change reference checkouts or Cargo caches to implement dependency patches.
Do not use the TODO tool. Prefer targeted offline tests; never make live paid calls
as a substitute for missing fixture coverage.
