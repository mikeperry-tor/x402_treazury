# Local resource limits

Every local limit, cap, cache lifetime, and queue bound exists to protect a
concrete memory, storage, concurrency, or external protocol boundary. Do not
add arbitrary limits or whole-work deadlines; let valid work finish while it
makes progress.

For network timeouts, retry counts, polling cadences, and protocol expiry
windows, see the
[timer and budget inventory](network-egress.md#remaining-timers-and-budgets).

## Rejection and truncation

Any limit that rejects or truncates content must stay visible to the consumer
and operator. Never publish a partial document or silently slice completed API
responses.

- **Rejections:** Oversized API responses, help documents, remote OpenAPI
  specs, MCP HTTP bodies (HTTP `413`), images, Base and NEAR RPC bodies, and
  catalogs exceeding the structural work budget fail closed with a `WARN`
  naming the resource, setting, and numeric bound (speculative origin probes
  log byte rejections at `DEBUG`).
- **Truncation:** Completed API tool outputs have no character-length truncation
  (the obsolete `max_response_chars` setting fails config parsing).
  Local JSON field omissions (`response_mappings`) disclose every removed path
  and count in the tool result advisory.
- **Protobuf and gRPC codes:** `ResourceExhausted` and `OutOfRange` emit `WARN`
  at production consumers with fixed stage and numeric status codes (without
  inferring local vs. peer origin from error prose).
- **Queue and pool saturation:** Full queues emit `DEBUG` and wait with the
  original command. Base query and caller saturation emits `WARN`; active-RPC
  waiting emits `DEBUG`. Cache LRU evictions emit `DEBUG` without aborting
  in-flight work.

## Memory and document bounds

| Boundary | Limit | Behavior when exceeded |
| --- | --- | --- |
| Incoming MCP HTTP body | 4 MiB (`MCP_REQUEST_BYTES` in `src/limits.rs`, enforced in `src/server.rs`) | Rejects with HTTP `413` and `WARN`. |
| API response body | 16 MiB default (`max_response_bytes`) | Checked on `Content-Length` and stream; rejects whole with `WARN`. |
| Help and usage documents | 4 MiB default (`max_help_bytes`) | Rejects whole with `WARN` (probes log `DEBUG`). |
| Remote OpenAPI specs | 32 MiB default (`max_spec_bytes`) | Rejects whole with `WARN`; local spec files are exempt. |
| Catalog structural budget | Depth 64, 500,000 nodes, 64 MiB traversal and copy work | Shared across local, remote, cached, and relayed OpenAPI parsing; rejects whole with `WARN`. |
| Dynamic management requests and catalogs | 64 KiB management request, 10,000 operations, 256 KiB per tool schema, 64-byte source name, 512-byte tool name, 128 MiB live documents, 256 MiB SQLite registry | Enforced in `src/discovery/`; rejects whole with `WARN` rather than truncating or loading partial registrations. |
| MCP image attachments | 4 images, 4 MiB per image, 8 MiB total by default (`image_limits.max_images`, `max_image_bytes`, `max_total_bytes` in `src/output.rs`) | Rejects oversized or excess images with `WARN`; never silently drops or resizes. |
| Tool descriptions | `max_description_chars` (default unset) | [Marks truncated descriptions and logs compile-time truncation](configuration.md#observed-prices-and-description-overrides). |
| Base RPC response body | 16 MiB fixed (`src/rotation/base.rs`) | Rejects without retrying that endpoint; may fail over to a fresh view. |
| NEAR JSON response body | 2,000,000 bytes fixed (`src/rotation/near.rs`) | Rejects with `WARN`; never parses truncated JSON. |
| Protobuf and gRPC frames | Tonic default 4 MiB per decoded message and Zcash bounds | Rejects with `WARN` at unary and stream consumers. |
| API HTTP headers | HTTP library map capacity | Fallible insertion checked up front; rejects unrepresentable sets with a value-free `WARN`. |
| Foreground work ownership | Unbounded task count | Accepted MCP and client calls drain on shutdown; no aggregate count cap or replacement gate. |

## Caches and storage limits

| Store or cache | Limit | Behavior and eviction |
| --- | --- | --- |
| Discovery disk cache (`http-cache/discovery-v1.sqlite`) | 4,096 entries, 128 MiB payload and metadata, 16 KiB metadata per entry, 144 MiB SQLite file | Private owner-only SQLite; LRU pressure eviction logs `DEBUG`; I/O errors log `WARN` and fall back to origin. |
| Catalog and price-hint TTL | `catalog_cache_ttl_seconds` (default `"indefinite"`) | Local TTL governs retention; reads and price observations never extend expiry; `0` disables disk caching. |
| Outbound HTTP and gRPC transport caches (`src/network.rs`) | 256 entries each per `NetworkContext` | LRU eviction logs `DEBUG` and drops cache ownership only; active handles finish their work. |
| Dynamic import completed cache | 4 completed entries (`300s` success retention, `1s` failure backoff) | Trims only completed entries on lookup and completion; active coalesced imports never evict. |
| Funding scheduling metadata | 256 records per observation kind (1,024 max) | Evicts only due inactive records (`DEBUG`); active work and unelapsed backoff stay pinned. |
| SQLite busy timeout | 5s (Store) and 2s (HTTP disk cache) | Contention failures log `WARN` before replying (retained transaction-publication retries log `DEBUG`); never silently replays a mutation. |
| Outgoing snapshot retention | Unbounded (immutable audit evidence) | Unreferenced old snapshots are pruned; snapshots referenced by outgoing operations stay preserved. |

## Queues, pagination, and concurrency limits

| Boundary | Limit | Purpose |
| --- | --- | --- |
| Store and treasury command queues | 32 Store and 8 treasury | Bounds queued commands; full queues log `DEBUG` and wait with the original command. |
| Deployment catalog Tor concurrency | `startup.catalog_concurrency` (default `2`, range `1..=64`) | Bounds concurrent active Tor SOCKS scopes for deployment catalog, pricing, and relay fetches; direct egress and cache hits bypass it. |
| Dynamic mutation jobs | 16 concurrent mutations | Dynamic registration saturation logs `WARN` and rejects; static publication logs `DEBUG` and waits. |
| Base RPC resource pool | 64 query owners, 32 active RPCs, 128 RPC callers | Bounds concurrent Base reads; active permits release during retry backoff. |
| Base and funding inventory pages | 64 rows per page (no total-history cap) | Pages wallet, authorization, and funding scans with keyset cursors instead of full snapshots. |
| Advisory and retirement maintenance | 4 wallet acquisitions per group | Bounds concurrent background Base balance and retirement proof checks. |
| Funding observation lanes (per store) | 4 credit, 4 status, 4 unsigned quotes, 1 source step, 1 quote prerequisite | Serializes treasury source mutations while keeping independent observations concurrent. |
| URL discovery probes (`base_url`) | 4 concurrent GET probes | Bounds speculative OpenAPI and help path checks. |
| Search and browse pagination | Default 25 (`x402_treazury_tools_search` and local browse); 1-20 per remote directory (default 5); `tools/list` unpaginated | `tools/list` warns above 500 advertised definitions suggesting `discover_on_demand = true`. |
| Optional cover traffic | Bounded per-episode range and process budgets | Caps optional padding traffic without delaying or aborting the paid API call. See [cover traffic](cover-traffic.md). |
