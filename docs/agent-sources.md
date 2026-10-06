# Agent API discovery and source management

Treazury can let an agent register public OpenAPI APIs at runtime. This is optional
and available in `--config` HTTP deployments. Standalone stdio/HTTP provider
commands remain static. Start with [the static-wallet example](../examples/deployments/agent-sources.toml)
or [the managed-wallet example](../examples/deployments/agent-sources-managed.toml).

Both examples use **one named `agent_shared` wallet** for all added APIs on both
listeners. A managed profile with a $5 deposit target has a $10 active-plus-standby
target regardless of source count. This is capital allocation, not a total spending
limit. All these APIs share payment identity; optional listener wallet overrides
provide explicit server/group separation. Existing payment caps and treasury budgets
still apply. Registration does not allocate pools, transfer funds, probe endpoint
prices, or wait for a wallet to become funded. Configured managed profiles initialize
and synchronize through normal startup; automatic funding still requires `auto_fund`.

## Run and inspect

```sh
cargo build --locked
# Offline composition and permissions; no keys, spec requests or registry access:
target/debug/x402_treazury config show --config examples/deployments/agent-sources.toml
# Checks the example's committed directory spec, without credentials or probes:
target/debug/x402_treazury config check --config examples/deployments/agent-sources.toml
# Serving needs EVM_PRIVATE_KEY and the two configured MCP bearer tokens:
target/debug/x402_treazury serve --config examples/deployments/agent-sources.toml --env-file .env
# Offline saved-record inspection; does not create a missing registry:
target/debug/x402_treazury sources inspect --config examples/deployments/agent-sources.toml
```

Use `scripts/zcash.sh build` for the managed example and initialize its treasury
as described in the [wallet documentation](../README.md). Static signing credentials
for future dynamic bindings are loaded at startup even if no current tool uses them.
Agent inputs cannot choose wallets, key environment variables, proxies or spend caps.

## Authority and scope

The configured listener ID is the principal. Everyone sharing a listener's bearer
token shares its grants and ownership; client names and session IDs do not identify
separate agents. Mutations can change only registrations owned by that listener.
Receiving another listener's source permits use, not modification. Static TOML sources
cannot be changed by these tools.

| Setting | Meaning |
| --- | --- |
| Global `source_management.wallet` | Required existing `[wallets]` profile; no template or automatic assignment |
| Global `registry_file` | Optional separate SQLite registry, relative to TOML; required for persistence grants |
| Listener `enabled = true` | Exposes preview/add/update/remove; requires `accept_sources = true` |
| Listener `accept_sources = true` | Receives dynamic tools and exposes list/search/call fallback |
| Listener `allowed_targets` | Explicit target IDs; defaults to the calling listener only |
| Listener `allow_process_scope` | Permits targeting all current receivers, provided every receiver is authorized |
| Listener `allow_persistence` | Permits durable registration; defaults false |
| Listener `wallet` inside `source_management` | Optional existing-profile override for dynamic calls on this listener |

Visibility and lifetime are independent:

| Input | Behavior |
| --- | --- |
| `visibility: "server"` | Calling listener; default; omit `targets` |
| `visibility: "servers"` | Explicit nonempty authorized `targets` |
| `visibility: "process"` | All participating receivers; omit `targets`; requires the extra grant |
| `lifetime: "process"` | Disappears on process exit; default |
| `lifetime: "persistent"` | Saves accepted bytes and bindings for restart |

Process visibility resolves a concrete target list when committed. Adding another
listener to TOML does not expand saved registrations. Explicitly update visibility
or targets to expand them. Existing listener tag and tool-name filters remain hard
limits on dynamic APIs, for both direct and fallback calls. Management tools use
separate grants and are not removed by provider filters. If using `include_tools`,
include `dyn_*` to allow dynamic APIs. `treazury_` and `dyn_` tool prefixes are reserved
in deployments with source management configured.

## Agent workflow

The example exposes five curated `x402_list_*` directory tools for service search,
details, rankings, categories and networks. Directory reads have a shared-IP free
quota and may require x402 payment after it, including on shared Tor exits. Ordinary
wallet caps apply. Results are leads; verify a published OpenAPI URL from the vendor.
Endpoint lists do not automatically supply usable request schemas, and directory
results are never registered automatically.

1. Call `treazury_source_preview` with a `candidate` object. This validates and fetches
   an OpenAPI 3 JSON document, builds tools, and reports wallet sharing and limits.
2. Call `treazury_source_add` with the same candidate, an `idempotency_key`, and optionally
   the returned `preview_id`. A preview is optional and is not an approval barrier.
3. Call `treazury_tools_search` for current tool IDs, input schemas and revisions.
4. Call `treazury_tool_call` with `tool_id`, `arguments` and `expected_revision`.
   It uses the same dispatcher and payment admission as a direct provider tool call.

Example preview input:

```json
{
  "candidate": {
    "name": "research_api",
    "spec_url": "https://api.example.com/openapi.json",
    "selection": {"tags": ["Search"], "exclude": ["/admin"]},
    "visibility": "servers",
    "targets": ["research", "analysis"],
    "lifetime": "persistent"
  }
}
```

Add uses `{"candidate": <same object>, "preview_id": "...", "idempotency_key": "add-research-1"}`.
Do not substitute the illustrative URL above for an actual verified spec URL.
Candidates optionally accept `base_url`; otherwise the first root OpenAPI server is
used, resolving relative URLs against the spec URL. Name is an ASCII label of at
most 64 letters/digits/underscores/hyphens. Source UUIDs and the full UUID-derived tool
prefix stay stable across label changes, filter changes and refreshes.

Selection supports `tags`, `exclude_tags`, `include`, `exclude`, `include_tools` and
`exclude_tools`. Path selection uses existing provider semantics, including stripping
matched include prefixes from generated names. Tool-name patterns apply to the full
`dyn_<uuid>_<operation>` name, so wildcard prefixes are useful. Exclusions win.
No candidate field accepts authentication headers, arbitrary config files, help URLs,
pricing probes, wallet definitions or timeout increases. Unknown fields fail.

| Tool | Additional details |
| --- | --- |
| `treazury_sources_list` | Visible/owned records, revision, filtered tool count, lifecycle and effective named wallets; optional `source_id`, `cursor`, `limit` |
| `treazury_source_preview` | Initial `candidate`, optional `limit`; subsequent pages use `preview_id`, `cursor`, `limit` without candidate |
| `treazury_source_add` | Candidate, idempotency key, optional preview ID; preview reuse commits exactly its accepted bytes |
| `treazury_source_update` | Source ID, expected revision and idempotency key; optional `name`, replacement `selection`, visibility/targets/lifetime, `refresh_spec` |
| `treazury_source_remove` | Source ID, expected revision and idempotency key; removes all bindings, retains persistent tombstone |
| `treazury_tools_search` | Optional text `query`, `source_id`, cursor/limit; returns full visible descriptions and schemas |
| `treazury_tool_call` | Expected source revision required; static tools use revision 0; cannot invoke management tools recursively |

Omitted update fields retain values; supplied lists/maps replace in full. URLs are
immutable: create a new registration to change them. `refresh_spec = true` explicitly
fetches new bytes; unrelated changes and restarts reuse the accepted document.
Concurrent updates fail with `source_revision_conflict`. Repeat a lost mutation with
the same key and identical input to get its original result; changed input is an
idempotency conflict. Keys are scoped to listener and operation and limited to 128 bytes.

Search/list pages default to 20 items, maximum 100. MCP `tools/list` pages contain at
most 100 tools on dynamic receivers. Cursors belong to one running catalog, listener,
query and generation; restart listing when stale. Preview pages belong to their
preview. Previews expire after five minutes and share an eight-entry/128 MiB cache.
There is no hidden description truncation; tool definitions retain accepted prose.

Stateless HTTP does not advertise `tools/list_changed` notifications. Clients that
cache their initial tool list can use the stable search/call tools immediately.
Clients supporting explicit refresh can discover normal dynamic tools with `tools/list`.
Direct calls use the current definition; fallback calls reject stale revisions before
HTTP. Already running calls retain their captured route and payer after update/removal.

## Limits and network policy

Defaults are 16 registrations, 8 owned per writer, 100 tools per source, 200 dynamic
tools per listener, 32 MiB spec bytes and a 30-second fetch deadline. Global limits
are bounded to 1024 sources, 10,000 tools/source or listener, 64 MiB/spec and 300 seconds.
Owned-source limits are bounded to 1024. Management requests are limited to 64 KiB
(fallback API-call arguments retain the normal API behavior). Serialized registry
state is limited to 256 MiB. Disabled records still occupy registration
quota. The process accepts at most four concurrent fetches, one per writer, and 16
outstanding mutation jobs. Busy errors are retryable with the same mutation key.

JSON/reference traversal is bounded to depth 64, 500,000 expanded nodes and 64 MiB of
expanded string/key content. Import rejects over 10,000 operations, generated schemas
over 256 KiB, names over 128 bytes, external/unresolved/cyclic refs and per-path/operation
server overrides. Accepted live document bytes total at most 128 MiB. At most 10,000
mutation receipts are retained; reaching that limit refuses new mutations instead
of silently pruning durable idempotency. Do not delete a live registry to clear limits.

Agent-selected destinations require public HTTPS without userinfo or fragments.
Local/special addresses, private names, redirects and local files are rejected. Direct
connections validate every DNS answer in the dialer's resolver; a private rebinding
answer cannot be dialed. Optional `allowed_origins` accepts only exact canonical HTTPS
origins and narrows both spec and API access. Undeclared cross-origin absolute API
routes are rejected. Imports make no pricing probes; actual calls validate fresh x402
challenges.

All traffic uses the existing network factory and pools. Specs use discovery identities;
API calls use the resolved EVM payer's identity. Tor uses authenticated remote-DNS SOCKS
with no direct fallback. With Tor, private DNS-answer rejection depends on Tor's
`ClientRejectInternalAddresses` protection; Treazury does not resolve destinations
locally or inspect answers hidden by the proxy. Keep that Tor protection enabled.
An arbitrary SOCKS proxy is not a substitute for this destination policy. Restrict
origins when only a known provider set should be reachable.

Vendor descriptions and directory responses are untrusted content. They do not authorize
changing grants, raising caps, registering further sources or contacting other origins.
Registration success is not a qualification of a vendor's payment or delivery behavior.
Wallet balance/readiness is checked during payment admission; an unfunded managed profile
can register tools but paid invocation may return `wallet_not_ready`.

## Concurrent agents

Authority and mutation keys are scoped to the authenticated MCP listener, not to
an individual agent connection. Agents using the same endpoint should choose
independent idempotency keys for independent operations. A retry must preserve
both the key and arguments, including any preview ID. Concurrent identical
requests cannot create duplicate registrations; successful retries return the same
receipt. An attempt rejected as busy must still be retried. Different arguments with
a previously accepted key produce an idempotency conflict. Another listener has
its own key namespace and cannot mutate the first listener's registrations.

The importer admits one in-flight request per listener and four process-wide.
Calls exceeding these bounds receive `owner_import_busy` or `imports_busy`;
`mutation_queue_full` similarly reports saturation of the 16-job mutation queue.
These errors do not silently drop or publish a partial registration. Retry after
other work completes, preserving the mutation key and arguments. Accepted preview
IDs let additions reuse already fetched bytes without another import.

Quota checks and catalog publication are serialized across listeners. Concurrent
sources cannot independently claim the same remaining registration or tool quota.
Each published snapshot contains all applicable listener views and filters;
already-running tool calls retain their captured binding while later calls use
the current catalog. Persistent receipts and registrations survive reopening the
registry, including operations that raced with other writers.

## Persistence and recovery

The registry is separate from treasury state and contains no wallet keys. It stores
accepted spec bytes/hashes, UUIDs, owners, fixed targets, revisions, timestamps, generator
format, persistent mutation receipts and tombstones. Files are owner-only, with an
exclusive `.owner.lock` sidecar. Registry and sidecars cannot alias configured provider,
deployment or treasury paths. Config-only inspection never opens the store.

Back up the SQLite registry while the process is stopped, separately from treasury/key
backups. Do not delete or replace its ownership sidecar while serving. Document URLs and
contents can reveal operator interests even though the registry contains no wallet keys.
`x402_treazury sources inspect` reports saved records without fetching or changing them;
`treazury_sources_list` reports the running state, including policy-disabled records.

Persistent changes commit before publication; accepted commit jobs finish even if the
caller disconnects. Restart rebuilds from saved bytes without vendor requests. Promoting
a temporary source persists it; making a persistent source temporary records durable
withdrawal so it cannot reappear after restart. Corrupt/unsupported stores stop startup.
Generator-format changes disable affected sources until explicit validated refresh.
Revoked owner/target/persistence permissions disable saved records. Reduced quotas disable
the conflicting restored set deterministically; owners may remove disabled records.

Removal changes API access only. Wallet balances, reservations, swap/refund records,
retired keys and background reconciliation remain with their existing wallet profiles.

## Implementation boundaries

`src/discovery/policy.rs` resolves listener authority, targets, named wallet grants
and limits. `import.rs` handles bounded public-URL retrieval and candidate validation;
`tools.rs` defines management schemas; `store.rs` owns persistent records/receipts;
`mod.rs` coordinates accepted mutations. `src/catalog_state.rs` supplies the immutable
views used by both ordinary and fallback tool execution.

Network fetch and candidate building happen outside serialized publication. Before
acceptance, recheck revisions, ownership, grants and quotas against current state.
A persistent mutation commits its registration/receipt before publication; a
cancelled caller cannot roll back an accepted worker job. On restart, durable
records rebuild views under current policy. Captured in-flight bindings remain
usable after an update/remove; subsequent calls use the new view. These boundaries
are exercised by permission, persistence, crash and concurrent-addition tests;
see [the testing guide](testing.md) and [coverage evidence](../tests/COVERAGE.md).
