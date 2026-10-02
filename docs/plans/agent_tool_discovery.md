# Agent discovery and runtime API source management

## Objective

Let an operator optionally authorize agents to discover APIs and register OpenAPI
sources as tools in a running Treazure deployment. Additions can be visible on one
MCP server, a specified set of authorized servers, or every participating server
in the process. Independently, a registration can last until process exit or
persist across restarts.

The default and recommended arrangement is **one operator-selected named wallet
for all agent-added sources**, across all participating servers. Registering another
API must not implicitly allocate another managed wallet pool. Discovery, preview,
registration and tool listing do not make paid API calls or initiate funding.
Actual tool invocation uses the existing payment and funding machinery.

Reuse the Rust catalog, config validation, payment admission, pricing cache, wallet
registry and direct/Tor transport factory. Do not create an independent paid-call
stack or allow agents to write arbitrary deployment TOML. Disabled deployments
retain their current configuration, catalog, authentication and payment behavior.

This document specifies planned functionality, not existing CLI/config fields.

## Scope and terminology

| Dimension | Supported values | Meaning |
| --- | --- | --- |
| Visibility | `server` | Calling server only; default |
| Visibility | `servers` | Explicit authorized target server IDs |
| Visibility | `process` | All servers currently configured to accept agent sources, subject to caller authority |
| Lifetime | `process` | Registration disappears when this process exits; default |
| Lifetime | `persistent` | Accepted registration and catalog survive restart |

Persistence is not a third visibility level. A persistent registration can target
one server, and a process-wide registration can be temporary. Resolve `process`
visibility to a concrete target list when a mutation commits. Persist that list;
adding a new listener to TOML must not silently grant it existing registrations.
A later explicit update can expand a registration within current policy.

The authorization principal is the configured server ID through which a request
arrives. Callers sharing its bearer token share its permissions. Do not infer agent
identity from `clientInfo`, tool arguments or an MCP connection. Session-specific
source ownership and per-user credentials are outside the initial implementation.

Implement management in multi-server `--meta-config` mode first. Do not add these
fields to provider files or silently accept them in standalone `--config` mode.
The reusable catalog machinery should permit a later standalone extension.

## Existing implementation to reuse

| Area | Relevant files and constraints |
| --- | --- |
| Deployment and policy | `src/deployment.rs`: strict `MetaConfig`, listener filters, wallet resolution, atomic listener binding, bounded shutdown |
| Catalog and arguments | `src/catalog.rs`, `src/config.rs`: OpenAPI parsing, normalization, tool names, tags/path selection, argument routing and one-level TOML composition |
| MCP | `src/server.rs`: current immutable tool vector/binding maps, lazy help, bearer-gated stateless Streamable HTTP |
| Network | `src/network.rs`: immutable process policy, discovery-origin HTTP clients, wallet-scoped payment transport, authenticated remote-DNS SOCKS |
| Pricing | `src/pricing.rs`: bounded unsigned GET discovery, process-wide success/failure cache, no TTL-triggered refresh |
| Payments | `src/payment.rs`, `src/rotation/manager.rs`: identity-bound admission, generation checks, in-flight signer ownership, no replay after signed submission |
| Wallets | `src/rotation/assignment.rs`, `config.rs`, `store.rs`: explicit/shared profiles, automatic static-deployment assignment, durable pool identity and financial journals |
| Provider tests | `tests/provider_catalogs.rs`, `tests/config.rs`, `tests/fixtures/catalogs/`: reviewed contracts and settings |
| Transport tests | `tests/server.rs`, `stdio.rs`, `network.rs`, `deployment.rs`, `managed.rs`: actual protocol calls, authentication, isolation and payment behavior |

A provider catalog is not currently mutable. Refactor that boundary before exposing
source-management tools. Keep `get_tool`, `tools/list`, direct invocation and fallback
invocation on the same authoritative catalog and permission checks.

## Configuration and authority

Add strict optional `source_management` structures to `MetaConfig` and listener
configuration. Their absence disables registration. Validate all declared settings,
references and limits before network fetches or listener startup.

Recommended managed-wallet configuration fragment, inside an otherwise complete
deployment with treasury/funding settings:

```toml
[source_management]
wallet = "agent_shared"
registry_file = "state/agent-sources.sqlite"
max_sources = 16
max_tools_per_source = 100
max_tools_per_server = 200
max_spec_bytes = 33554432
fetch_timeout_seconds = 30

[wallets.agent_shared]
mode = "zcash_rotation"
deposit_size = "5.00"
max_price_usd = "0.05"
max_input_zec = "0.006"
max_fee_bps = 500

[servers.research.source_management]
enabled = true
accept_sources = true
allowed_targets = ["research"]
allow_process_scope = false
allow_persistence = true
max_owned_sources = 8

[servers.analysis.source_management]
enabled = false
accept_sources = false
```

The wallet can instead reference an existing static profile; source management must
work without the `zcash` feature or treasury state. Paths are relative to the
meta-config file. The example registry path is appropriate for a deployment file in
the repository root; files in `examples/` need `../state/...`.

Define these defaults and validation rules:

- `wallet` is required when any server enables management or accepts dynamic sources.
  It must name `[wallets]`, not a template. No fallback to deployment automatic
  per-source/per-binding assignment is allowed for agent registrations.
- `registry_file` is required only if any server grants persistence. It is a separate
  application-owned registry, never the treasury database, encryption key, config
  file or a caller-supplied path. Reject path aliasing with those files, including
  canonicalized existing parents and symlinks. Acquire exclusive ownership before
  serving. Memory-only management must not require an unrelated key/state directory.
- `enabled`, `accept_sources`, `allow_process_scope`, and `allow_persistence` default
  to false. Only `enabled` exposes mutation tools; `accept_sources` permits publication
  and exposes the stable search/call fallback tools.
- `allowed_targets` defaults to the calling listener only when management is enabled.
  Configured IDs must exist; publication additionally requires each receiver's
  `accept_sources`. Reject an unauthorized target instead of silently dropping it.
  `process` also requires `allow_process_scope` and authorization for every receiver.
- Default limits: 16 registrations globally, 8 owned per writer, 100 selected tools
  per source, 200 dynamic tools per receiving server, 32 MiB decoded spec bytes and
  a 30-second fetch deadline. Validate positive bounded values. Counts include free
  and help tools; management/fallback tools are reserved and counted separately.
  Disabled persistent records still count toward registration quotas until removed.
- Source limits, target limits and quota reservations are checked atomically across
  concurrent writers. Bound concurrent imports globally to four and one per writer.
- Optional operator `allowed_origins` narrows dynamic remote access to exact canonical
  HTTPS origins. Absence permits public HTTPS origins under the destination rules
  below; agents cannot modify this policy. Reject unsupported wildcard syntax.
- Optional listener `wallet` inside its `source_management` table deliberately overrides
  the shared default for bindings on that listener. It also names an existing profile.
  Multiple servers can reference the same profile to define an operator-selected group.

Existing source/listener tag and tool-name exclusions remain hard limits. Agent
selection can narrow them but cannot override them. Mutation tools use a reserved
`treazure_` namespace and are authorized separately from provider tool selection;
provider filters cannot unexpectedly remove the management interface. Document
this distinction and include it in configuration inspection.

`--show-config` reports grants, receiving targets, limits, effective dynamic wallet
bindings and registry path without opening the registry, reading keys or fetching
specs. Add an explicit offline CLI command `treazure sources inspect --meta-config
FILE` to inspect saved records without network or financial mutations. It must not
create a missing registry. Runtime `treazure_sources_list` reports active state.

## Wallet behavior and capital

All recommended examples use `source_management.wallet = "agent_shared"`; they do
not define one pool per newly discovered API. With a $5 deposit target, that named
managed pool has a $10 active-plus-standby target, regardless of how many APIs use
it. This is capital allocation, not a total spend limit or a promise that fees and
retired balances are zero. Existing per-payment caps, treasury daily/input budgets
and funding policies remain authoritative.

The configured shared managed profile already participates in normal startup pool
initialization and opt-in funding. Source registration does not initiate that work,
add a second pool, or wait for it. Report `wallet_not_ready` when appropriate;
free tools remain usable according to existing managed free-call semantics.

Resolve/load static wallet credentials authorized for future dynamic bindings at
startup, even if no static catalog currently uses those profiles. Do not discover
secret environment variable names from agent inputs during registration. Resolve
all effective dynamic payer handles from the same registry as static bindings.

An explicit listener override permits finer server/group isolation at a known
configuration-time cost. Reuse the same payer/managed-pool object for all bindings
that resolve to the same profile. Report effective sharing in preview and status:
API registrations on that profile are not wallet-unlinkable from each other.

Automatic per-agent-source or per-binding pool creation is not part of this first
release. A future opt-in template policy must add durable identities, an aggregate
allocation limit and add/remove churn protection before enabling those scopes.
The registry should retain stable source UUIDs so this extension need not redefine
source identity. Never reuse the static deployment assignment algorithm implicitly.

Removing or expiring a registration removes access to tools, not wallet state.
It must not discard pending authorizations, outgoing swaps, refund associations,
retired keys or background reconciliation. Shared profiles continue serving other
sources. A temporary registration can disappear while its financial liabilities
remain durable.

## Agent-facing tools

Expose a minimal, structured API. Unknown fields fail. Each mutation has an
`idempotency_key`; update/remove also require `expected_revision`.

| Tool | Input and behavior |
| --- | --- |
| `treazure_sources_list` | Optional source ID and pagination; visible registrations and manageable owned records, lifecycle, revision, targets visible to caller, selected tools, wallet readiness and allowed actions |
| `treazure_source_preview` | Candidate spec URL, optional API base URL, selections and desired visibility/lifetime; validate/fetch/build and return paginated tool summaries, limits, destination origins, effective wallet sharing and warnings; no persistence, probes, allocation or funding |
| `treazure_source_add` | Same candidate plus owner-local name and idempotency key; optionally consume a preview ID; validate and atomically publish the selected source |
| `treazure_source_update` | Owned source UUID, expected revision, idempotency key, replacement selections/targets/lifetime, optional `refresh_spec`; omitted fields retain current values and supplied lists replace in full |
| `treazure_source_remove` | Owned source UUID, expected revision and idempotency key; unpublish it from all its bindings and tombstone a persistent record |
| `treazure_tools_search` | Search caller-visible API tools by text/source with pagination; return namespaced tool IDs, descriptions, input schemas and catalog revision |
| `treazure_tool_call` | Tool ID, arguments and expected source revision; execute through the same dispatch and payment policy as a direct tool call |

Candidate selection supports tags/excluded tags, path include/exclude, and generated
tool-name include/exclude. The agent does not provide arbitrary `extends`, local
spec paths, headers, credentials, wallet/template IDs, pricing probes, timeout
increases, text overrides, filesystem output paths, network settings or spending
policy. The operator may preconfigure those through normal providers/static sources.
Do not expose arbitrary HTTP requests or wallet administration through these tools.

Registration initially accepts OpenAPI JSON documents, not legacy operations digests,
remote MCP servers, code, plugins, shell commands or arbitrary llms.txt-to-tool synthesis.
An optional base URL must pass the same destination policy as spec-derived servers.
External `$ref` fetching is unsupported; local references retain bounded cycle-safe
resolution. Do not silently derive tool definitions from directory endpoint metadata.

A preview is not approval. Add may validate and commit in one call within the granted
policy. If a preview ID is provided, it refers to a bounded process-local cache of
exact fetched bytes/hash, candidate settings and expiry; reuse those bytes, then
recheck current policy, quotas and conflicts. Cache previews for at most five minutes,
with eight entries and 128 MiB of accepted document bytes globally; evict without
publishing anything. Coalesce simultaneous fetches of the same canonical spec URL
under the same transport/options, but apply each caller's permission checks before
using cached data. An explicit refresh fetches new bytes; unrelated catalog mutations
do not. No second fetch with different contents may be represented as committing the
preview. Expired previews return a clear error.

Return structured JSON and legible text consistently with current MCP conventions.
Add/update returns source UUID/revision, catalog generation, committed targets,
selected tool count, readiness, lifetime and instructions for search/call or refreshing
client tools. Registration success does not imply a paid endpoint has been qualified.

The owner is the authenticated calling server, never an argument. A receiver may use
another server's published source but cannot update/remove it merely because it can
see it. Do not expose other servers' secret names, wallet addresses, hidden sources
or inaccessible target details through discovery/status/error messages. TOML sources
are immutable through management tools. Reserve source names and prefixes to avoid
collisions with both static providers and management tools.

## Dynamic catalog and client refresh

Introduce an immutable `CatalogSnapshot` containing tool definitions, source IDs and
revisions, route/base settings, payer handles, help-cache handles and selection results.
A deployment registry publishes an `Arc` snapshot through a short synchronous lock
or equivalent atomic read mechanism. A request clones a snapshot once, so it cannot
combine a schema from one revision with a route or signer from another. No network
request, SQLite operation or tool execution runs while holding that publication lock.

Use one deployment-wide generation containing all listener views. Publish multi-target
mutations with one swap after every target passes validation. `get_tool`, `tools/list`,
direct call, search and fallback call read views from this generation. Existing calls
retain their original snapshot. Removal prevents new calls after publication; already
admitted/in-flight work follows existing completion, cancellation and journaling rules.

Use stable source UUIDs and reserve a `dyn_` prefix for dynamic tool names, followed
by the full UUID without hyphens and the generated operation name. Reject names that
exceed protocol limits or collide; do not silently truncate. Owner-local labels are
human-readable and renameable; renaming must not replace UUIDs or payer identities.
Keep operation naming independent of listener identity. A source's definition can
therefore be shared across listeners while bindings and filters remain per listener.

Refresh can alter request meaning under an existing name. Fallback calls require the
expected source revision and fail before any HTTP request if it changed. Direct MCP
calls use the currently published definition because MCP calls do not supply our
revision automatically; document this and use change notifications/explicit refresh.
Source URLs/base URLs are immutable in this release; replacing them requires a new
registration. Selection changes and explicit spec refresh remain atomic updates.

Paginate dynamic listing/search and pin cursors to a catalog generation. Stale cursors
return a restart-listing error; never stitch pages from different generations. Do not
add a hidden description truncation limit: large results use explicit pagination,
summary fields and bounded source/tool counts, preserving accepted tool definitions.

MCP supports `notifications/tools/list_changed`, but current Treazure HTTP is stateless
JSON and does not retain notification connections. Stable search/call tools must be
advertised from initialization on all participating servers, so clients that cache
their initial tools can use newly registered APIs immediately.

For stdio, send list-change notifications through the existing peer when supported.
For HTTP, retain the default stateless transport and fallback behavior. Do not advertise
notification support it cannot deliver. Notification-capable HTTP sessions/SSE may be
added as an explicitly tested transport option, but are not a prerequisite for source
management. If implemented, retain bearer authentication on every request/stream,
bounded session cleanup and shutdown; transport sessions do not become authority IDs.
Return the appropriate refresh/fallback instructions based on actual capabilities.

## Fetching, caching and destination policy

Route spec/directory/help traffic through discovery identities and actual API calls
through their resolved payer identities. Use `network.rs` in direct and Tor modes;
no ad-hoc reqwest clients or SDK-owned network access. Preserve source timeouts, TLS
verification, remote DNS in Tor mode, no redirects and no direct fallback.

Treat agent inputs and document URLs as untrusted network destinations. Apply policy
to the spec, optional base URL, every accepted server/absolute operation URL and any
help/document fetch. Reject local files, URL userinfo, non-HTTPS schemes, malformed
hosts, loopback/private/link-local/unspecified/multicast IP literals and special-use
local names. Agent-defined tools must not reach the MCP listeners, metadata services
or local administration endpoints. Reject undocumented cross-origin API destinations
unless explicitly permitted by the operator's origin policy.

In direct mode, check resolved destination addresses at connection establishment and
bind dialing to the validated result; a prefetch-only DNS check does not prevent DNS
rebinding. Implement this through the common network factory without changing trusted
static-source behavior unnecessarily. Test IPv4, IPv6 and IPv4-mapped IPv6 cases.

In Tor mode, never perform local destination DNS for this check. The application can
validate literal/special-use names and origin allowlists but cannot inspect an address
resolved inside Tor. Supported Tor setup must retain internal-address rejection;
document that proxy-side boundary rather than claiming local DNS validation. An
operator needing stricter destination control can set exact allowed origins and
proxy/network enforcement. A generic SOCKS proxy without Tor's protections is not a
supported substitute. Do not silently disable Tor or loosen destination policy.

Bound decoded response bytes, total fetch deadline, JSON nesting, reference traversal,
operation count before filtering, generated schema size and preview-cache storage.
Reject over-limit documents rather than silently dropping tools/descriptions. Pin
explicit implementation constants/config bounds in tests; account for SocialFetch's
large vendor spec when selecting defaults. Limit error body capture and never return
credential-bearing URLs or authorization payloads in diagnostics.

Import/update performs no pricing probes by default. Dynamic tools use vendor price
extensions or the existing unknown-price description, and actual payments validate a
fresh challenge. Reuse the process cache if an operator-controlled probing extension
is added later; agent re-import/refresh must not bypass its success/failure caching
or create repeated rate-limited endpoint sweeps. Help remains lazy and success-cached.
Reuse unchanged help/cache handles across catalog mutations; namespace changed
content by source revision when its accepted URL/semantics change.

Spec descriptions and directory text are data, not authority to change configuration,
add tools, raise limits or contact other origins. Preserve the existing explicit-only
text-limit policy while keeping those fields separate from management instructions.

## Registry, ownership and recovery

Add a separate versioned SQLite registration store for persistence, with exclusive
process ownership and a bounded serialized mutation worker. It contains no private
keys, mnemonics or raw auth credentials and requires no isolation/encryption key.
Use owner-only file permissions; accepted documents and source URLs may still reveal
operator interests. Do not reuse or alter treasury snapshot encryption.

Persist source UUID, owner server ID, owner-local name, immutable spec/base URLs,
resolved target IDs, lifetime, selections, accepted bounded spec bytes/hash, source
revision, tool-generation format version, acceptance timestamps and lifecycle state.
Persist idempotency request hashes/results and tombstones. Store the exact accepted
input needed to rebuild tools; do not fetch the spec again merely because of restart.
On generator-version incompatibility, disable the record with an actionable reason
until an explicit validated refresh, rather than silently changing its API contract.

Idempotency is scoped to owner and operation. Repeating a key with identical input
returns the same result/UUID, including after a lost response; reusing it for different
input is a conflict. Bound retained result payloads. Persistent idempotency/tombstones
survive restart and must not be silently pruned in a way that resurrects removed
sources. Process-only idempotency can expire with the process.

Prepare/fetch/build outside the mutation worker; recheck expected revisions, grants,
quotas and collisions in the serialized commit path. For persistent operations, commit
SQLite before publishing the prepared in-memory generation; acknowledge only after
both. The accepted worker job completes despite caller disconnect. Keep publication
infallible after the transaction is accepted. A crash in between is recovered from
SQLite at next startup. Never expose a persistent add before its durable commit.

Changing lifetime from process to persistent saves the exact accepted state before
acknowledgment. Changing persistent to process records durable withdrawal before
publishing the temporary form, so restart cannot resurrect it. Update expected-revision
checks prevent remove/refresh races. Removal is idempotent with its original key.

At startup, acquire ownership, read the registry and revalidate current owner/target
permissions, wallet references, destination policy, quotas and catalog compatibility
before publishing. Missing owners, revoked persistence/receiver grants and incompatible
records become disabled with reasons; they do not regain authority from saved data.
Do not partially publish one record across its targets. If reduced quotas cannot admit
all saved records, disable the affected conflicting set deterministically and report
it instead of depending on SQLite row order. Preserve records for inspection/removal.

A corrupt or unsupported registry is a startup error when persistence is configured;
never silently replace it with an empty file. Config-only `--show-config` remains
available for diagnosis. Saved registrations cannot shadow static sources: reserve
prefixes and fail explicit static-name conflicts before serving. Publish active
registrations only after normal listener binding succeeds.

## Discovery provider

Implement `providers/x402-list.toml` using the generic source path, not a bespoke
network client or a hard-coded directory aggregator. Use the documented canonical
OpenAPI URL `https://x402-list.com/api/v1/openapi.json`; the shorter `/openapi.json`
redirects and Treazure disables redirects. Verify the current spec's actual paths,
parameters, base URL and generated names during implementation.

Expose a curated subset for service search, service details, rankings, categories
and supported networks. Exclude submission, ownership, feedback/suggestion writes,
paid assessment creation and unrelated operational endpoints. Set `probe_pricing =
false`. Include concise authored guidance on supported Base USDC payment and directory
limitations. Snapshot the required request schemas offline and pin selected tools,
parameter routing and excluded endpoints in Rust tests.

Directory reads are documented as unauthenticated, with a shared-IP free daily quota
and paid x402 reads after it. Do not label discovery as unconditionally free, especially
with shared Tor exits. Use the ordinary configured wallet/cap for metered directory
calls. A directory response is a lead, not proof that a service accepts our payment
scheme, publishes usable OpenAPI, is safe, or will successfully settle/deliver.

Directory details may not contain an importable OpenAPI URL. Expose documented fields
honestly; do not invent a spec location or treat endpoint lists as schemas. An agent
can supply a verified spec URL from provider documentation. Discovery-tool descriptions
should explain the preview/add workflow, rather than merely dumping service URLs into
the registration tool's description. Do not automatically register search results.

A normalized multi-directory `treazure_discover_apis` aggregator is outside the first
release. The provider config can be replaced or supplemented through ordinary TOML
composition. Keep internal `treazure_tools_search` distinct from external API discovery.

## Implementation order and commit boundaries

1. **Curated discovery provider.** Inspect the current x402-list spec, create the TOML
   provider and offline fixture, document metering, pin the selected read tools and
   routing. Extend provider inventory/snapshot tests intentionally, with reviewed
   expected counts. No management capability or funding changes in this commit.
2. **Dynamic catalog and client refresh handling.** Introduce the shared immutable
   generation/view abstraction, refactor all existing dispatch through it and add
   listener-scoped search/call fallback tools behind configuration. Test atomic
   replacement, stale revisions/cursors, filters and in-flight calls using injected
   catalogs. Add stdio notifications where supported and truthful HTTP capabilities.
   Keep source mutations unavailable until their policy is implemented.
3. **Temporary source management.** Add strict global/listener policy, shared named
   wallet resolution and explicit server overrides, bounded importer/destination
   checks, preview/add/update/remove/list, idempotency and process lifetime. Implement
   all target/ownership/limits before enabling tools. Add representative static and
   managed shared-wallet examples; registration must work while a managed wallet is
   unready and must not allocate/fund another pool. Reject persistent requests until
   the next stage rather than silently degrading their lifetime.
4. **Persistent registrations and recovery.** Add the independent registry, lifetime
   transitions, durable idempotency/tombstones, offline inspection, startup revalidation
   and fault-injection tests. Publish only after durable acceptance; preserve financial
   journals on source removal. Enable `allow_persistence` after recovery coverage passes.
5. **Documentation and qualification.** Update README, AGENTS.md, provider notes and
   examples in place. Document listener authority, shared-wallet capital/privacy,
   fallback invocation, client-refresh behavior, registry backup and policy revocation.
   Run complete feature/compatibility checks and the bounded unfunded validation below.

Each stage should be a reviewable commit with its relevant checks passing. Keep new
logic in focused modules, for example `src/discovery/{policy,import,registry,tools}.rs`
and a catalog-snapshot module. Continue using one application crate. Extend the
network constructor inventory and tests for any new connector behavior.

## Testing and validation

Use temporary files, public deterministic keys, local fake sellers/directory servers,
fake SOCKS and injected failure points. Tests must not read the developer's `.env`,
wallet seed/key, actual registration store or funded treasury. Give importer tests an
explicit test-only loopback fixture capability; do not introduce a production bypass
or allow tool inputs to grant private-network access.

| Area | Required assertions |
| --- | --- |
| Disabled compatibility | Existing catalogs, authentication, payment behavior and CLI inspection remain unchanged; management tools are absent |
| Config | Unknown/invalid fields, unresolved wallet/target IDs, unauthorized process scope and persistence without a store fail before network; show-config remains offline |
| Discovery provider | Curated read-tool names/schemas/routes, filter parameters, no write/assessment routes, no probes, actual paid-over-quota challenge respects the ordinary cap |
| Visibility | Server/set/process publication reaches exactly authorized receivers; nonparticipants receive nothing; input cannot spoof caller/owner |
| Lifetime | Visibility and lifetime compose independently; temporary records vanish at restart; accepted persistent bytes rebuild without vendor fetch |
| Shared wallets | Many sources on several listeners share one payer/pool and one $10 configured target at $5; listener override produces only the declared extra profile; preview/add/update/remove cause no key allocation/funding |
| Policy | Dynamic sources cannot choose wallets, spend caps, secrets, files, proxies or admin actions; source/tool/tag exclusions apply equally to direct and fallback calls |
| Ownership | Receivers cannot modify another owner's source; static TOML sources cannot be changed; unauthorized enumeration leaks no hidden records or wallet details |
| Catalog atomicity | No mixed definition/route/payer revisions, no partially published multi-target source, duplicate names fail atomically, in-flight calls finish on their captured generation |
| Client behavior | Actual stdio initialize/list/change/call, stateless HTTP list refresh, cached-client search/call fallback, truthful capabilities and bearer enforcement |
| Concurrent mutations | Revision conflicts, parallel add quota races, duplicate idempotency keys, lost responses and remove/update races produce one deterministic result |
| Fetching | Size/decompression/depth/ref limits, deadlines, invalid OpenAPI, unsafe absolute routes, redirects, userinfo, private IPv4/IPv6 and DNS rebinding are rejected without partial publication |
| Tor | New fetch/invocation paths use the existing discovery/payer identities, remote DNS and mandatory auth; hostile ambient proxies and proxy loss never cause direct fallback |
| Caching | Repeated previews/imports coalesce bounded fetch work; imports never run pricing sweeps; unchanged help and pricing cache behavior survives catalog updates |
| Payment | Free calls need no paid readiness; paid calls preserve caps, promotion/re-challenge rules and signed-attempt liabilities; fallback cannot bypass any check |
| Removal | Pending payments/funding/refunds remain journaled and reconcilable after unmount, shutdown and restart; deleting a source cannot reset a wallet budget |
| Persistence | Crash before/after durable commit/publication, full disk, malformed/incompatible registry, concurrent ownership and cancellation never acknowledge undurable state or erase saved records |
| Revocation | Removing owner/receiver permissions, wallet references or quotas disables affected records deterministically; saved registrations cannot restore revoked authority |
| Portability | Config and registry paths resolve relative to declaring files; examples inspect from another working directory without reading secrets; rejected alias paths cannot overwrite treasury/config files |

Run focused tests after each stage, then:

```sh
scripts/check.sh --zcash
```

This runs formatting, vendor verification, default tests, Zcash tests, all-feature
Clippy and the combined Zingolib compatibility suite sequentially. Update the script
only if new checks are necessary; normal checks must remain offline at runtime and
must not run ignored live/consensus tests automatically.

For end-to-end validation, run two local MCP listeners with separate tokens and a
local directory/API fixture. Discover, preview, add, search/call, update and remove a
source through real MCP JSON-RPC. Exercise server/set/process targets and a restarted
persistent deployment. Verify a cached-tool-list client works through fallback, and
that another token cannot exceed its listener's authority. Repeat the transport flow
through fake SOCKS; assert zero direct destination connections on proxy failure.

An optional live read-only check may fetch the public directory spec and a small
number of directory results using unsigned HTTP requests through the configured
network factory. Stop on 402/429; never pay merely to qualify discovery. Fetch/preview a selected public
spec without calling its paid tools, then remove the temporary registration. Real
funding, paid assessment, source subscription and mainnet swap tests require separate
operator authorization and are not acceptance requirements for this feature.

## Completion criteria

The feature is complete when an explicitly authorized agent can discover and import
an API, immediately invoke selected tools with a cached-list client, target only the
permitted listeners, and optionally survive restart without editing TOML. All paths
use existing wallet/payment/network policy. Recommended configurations use one shared
wallet; source count does not multiply funded pools. Revocation, removal, import
failure and crashes cannot expose partial catalogs, lose financial liabilities or
silently restore permissions. A provider file alone, or mutable tool listing without
a usable invocation/refresh path, does not complete this plan.

## References

- [x402-list API](https://x402-list.com/api): canonical spec location, service search,
  details, categories/networks, rankings and metering. Recheck the spec when implementing.
- [MCP tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools):
  tool listing, pagination and list-change capability/notifications.
- [Runtime egress inventory](../network-egress.md).
- [Wallet rotation architecture](zcash_rotation.md).
- [Tor isolation architecture](tor_isolation_support.md).
- [Development checks](../../scripts/README.md).
