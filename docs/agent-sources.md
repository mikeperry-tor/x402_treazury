# Agent API discovery and source management

Enable `source_management = true` on an HTTP deployment endpoint to let its agents
search a configured directory, add public OpenAPI APIs, inspect tool signatures and
invoke them. It defaults to false. Standalone provider serving remains static.
See the [README workflow](../README.md#let-agents-discover-and-add-apis),
[static-wallet example](../examples/deployments/agent-sources.toml) and
[managed-wallet example](../examples/deployments/agent-sources-managed.toml).

## Configuration and authority

```toml
[source_management]
wallet = "agent_shared"
registry_file = "../../state/agent-sources.sqlite" # optional persistence
# directory_tool = "x402_list_services" # default

[servers.research]
source_management = true
# Existing listen, bearer_token_env, sources and other server settings go here.
```

The shared block names an existing wallet and optionally a separate registry file.
A server's existing `wallet` setting overrides the shared dynamic wallet. No automatic
wallet allocation occurs per source. Managed wallet initialization and funding follow
the normal operator-authorized startup policy. Registration never funds wallets or
probes endpoint prices; paid invocation checks readiness and ordinary payment limits.

A listener's authenticated endpoint ID is the principal. All agents using its bearer
token share authority. Agent/session fields cannot choose an owner. Each registration
is visible only on the endpoint that added it; there are no publication targets,
receiver roles or process-wide grants. Two endpoints adding the same specification
get independent registrations, even if they share a payment wallet. Static TOML
sources cannot be modified by agent tools.

Persistence belongs to the operator. With `registry_file`, all additions are saved;
without it, additions disappear on exit. Agents cannot select lifetime, wallet,
timeouts, payment limits or visibility. Existing endpoint tag/tool filters still
constrain imported API tools. Include `dyn_*` when an endpoint has an explicit
`include_tools` allowlist. The `x402_treazury_` and `dyn_` namespaces are reserved in
participating deployments. Management tools have their own enable switch and are
not removed by API filters.

## Agent tools

| Tool | Arguments |
| --- | --- |
| `x402_treazury_sources_search` | The input schema of the configured directory tool; bundled x402 List supports `q`, `network`, `page`, `per_page` and other directory filters. |
| `x402_treazury_source_add` | Required `spec_url`; optional `name`. |
| `x402_treazury_tools_search` | Optional `query`, `source_id`, `cursor`, `limit`. |
| `x402_treazury_tool_call` | Required `tool_ref` and `arguments`. |

The directory must be a static GET API tool visible on the enabled endpoint.
`directory_tool` selects its exact generated name. Serving rejects an unavailable
or unsuitable binding. Search uses the same tool schema, response handling, payer
and payment admission as a normal invocation; it is not an unsigned bypass. The
bundled directory can charge after its shared-IP quota. Results are leads, not
payment/delivery guarantees, and may lack an OpenAPI URL. Verify the vendor's published
specification rather than inferring schemas from endpoint descriptions.

Add accepts a public HTTPS OpenAPI 3 JSON document. The API address comes from the
first root OpenAPI server, resolving relative addresses against the specification
URL. Optional names are ASCII labels of at most 64 letters/digits/underscores/hyphens;
omission produces a stable URL-derived label. Unknown fields fail. There is no
preview, scope, lifetime, selection or mutation-key argument.

Add validates before publication and returns a source ID, revision and filtered tool
count. The canonical specification URL identifies an existing registration within the
endpoint. A repeated add returns that registration without fetching again, regardless
of the supplied label. Concurrent additions recheck duplicates under the publication
lock. Busy errors can be retried with the same URL. An operator can remove a source
and subsequently allow it to be added again with a new source UUID.

Tool search returns complete descriptions and schemas plus an opaque `tool_ref`.
Pass that reference unchanged to tool call with arguments matching its schema.
References bind the endpoint, running catalog instance, tool name and source revision;
a stale or foreign reference fails before HTTP. Search again after restart. Tools
cannot use this path to recursively invoke management commands.

HTTP is stateless and does not advertise list-change notifications. Stable search/call
tools work with clients that cache their initial tool list. Clients that refresh
`tools/list` can also invoke normal `dyn_<uuid>_*` tools. Each call retains its captured
route and payer for its entire lifetime.

Search pages default to 20 and allow at most 100 entries. MCP tool-list pages also
contain at most 100 entries on enabled endpoints. Cursors bind the running catalog,
endpoint, query and generation; restart a search when its cursor becomes stale.
Descriptions and schemas are not silently truncated.

## Limits and network policy

Default operator limits are 16 registrations across the deployment, 100 tools per
source, 200 dynamic tools per endpoint, 32 MiB per specification and a 30-second fetch
deadline. These defaults need not appear in ordinary configuration. Advanced overrides
live in `[source_management]`: `max_sources`, `max_tools_per_source`,
`max_tools_per_server`, `max_spec_bytes`, `max_response_bytes`, `max_help_bytes`,
`fetch_timeout_seconds`, and `allowed_origins`.

Source counts are bounded to 1024, tool counts to 10,000, specifications to 64 MiB and
fetch deadlines to 300 seconds. Disabled registrations still occupy quota. Management
requests have a 64 KiB limit; forwarded API calls retain normal API behavior. Import
concurrency is four process-wide and one per endpoint; at most 16 mutation jobs can
be outstanding. Saturation returns an explicit busy error. Publication and quota
checks are serialized, so concurrent additions cannot publish partial sources.

JSON/reference traversal remains bounded to depth 64, 500,000 expanded nodes and
64 MiB of expanded string/key content. Import rejects over 10,000 operations,
generated schemas above 256 KiB, names above 128 bytes, external/unresolved/cyclic
references, and per-path/operation server overrides. Accepted live documents total
at most 128 MiB; serialized registry state is at most 256 MiB. Retained mutation
receipts are bounded to 10,000. Limits reject visibly rather than trimming documents.

Agent destinations require public HTTPS without userinfo or fragments. Local/special
addresses, private names, redirects and local files are rejected. Direct connections
validate DNS answers at dial time. Optional `allowed_origins` narrows specification
and API destinations to exact canonical HTTPS origins. Undeclared cross-origin API
routes are rejected. All production traffic uses the network factory; paid dynamic
clients retain public-destination restrictions.

Tor uses authenticated remote-DNS SOCKS with no direct fallback. Private DNS-answer
rejection depends on Tor's `ClientRejectInternalAddresses`; the application does not
resolve destinations locally or inspect proxy-hidden answers. Keep that protection
enabled. Vendor descriptions are untrusted data and grant no additional authority.

## Persistence and operator maintenance

The separate owner-only SQLite registry holds accepted specification bytes, hashes,
source UUIDs, endpoint ownership, revisions, receipts and tombstones. It contains no
wallet keys, but URLs and documents can reveal operator interests. Back it up while
serving is stopped. Never delete or replace its ownership sidecar while serving.
Registry files and sidecars cannot alias configuration or treasury paths.

`config show` inspects configuration without opening registry state or secrets.
`sources inspect --config FILE` reads saved registrations and never creates a missing
registry. It does not require listener tokens, wallet keys or catalog requests.

Stop serving before operator maintenance; exclusive registry ownership is enforced:

```sh
x402_treazury sources refresh --config FILE --server research --source-id SOURCE_ID
x402_treazury sources remove --config FILE --server research --source-id SOURCE_ID
```

Refresh fetches and validates new bytes through the configured network policy without
wallet credentials or pricing probes. Removal records a tombstone. Neither command
changes wallets, payment reservations, funding/refund records or reconciliation.
Temporary registrations need no registry maintenance; they disappear on exit.

Restart rebuilds from accepted bytes without vendor requests. Disabled endpoints,
nonlocal saved bindings, invalid formats and quota conflicts cannot regain access
silently. Corrupt/unsupported registries fail startup. Persistent additions commit
before publication; cancelling an accepted commit's waiter cannot undo the write.
