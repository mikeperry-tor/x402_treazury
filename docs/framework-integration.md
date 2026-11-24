# Framework integration

Frameworks that wrap `x402_treazury` should treat catalog preparation,
financial bootstrap, MCP listener availability, and API call success as
independent states. Catalog preparation can complete with partial coverage, and
a healthy MCP handshake means the listener is reachable rather than that all
catalogs loaded or wallets are funded.

## Catalog preparation

Run `catalog warm --config FILE` under the deployment's network and payment
configuration. See
[catalog warming](configuration.md#warming-a-selected-source-including-directly-for-a-tor-deployment)
for source selection, caching, and `--direct`. Warming never starts listeners
or funding workers, though relay fallback may make paid calls when configured.

Each selected source runs independently: remote fetch failures, invalid remote
documents, unsupported operations, and upstream inventory changes are reported
per source without aborting healthy siblings. A compatible partial catalog can
be accepted, whereas an invalid or wholly unsupported replacement cannot
overwrite an existing cached snapshot or learned prices. If optional external
pricing fails, valid API tools remain available with unknown prices.

| Result | Exit status | Stdout JSON |
| --- | --- | --- |
| Every selected source is ready with no degradation | `0` | `outcome: "ok"` |
| Attempts finished with unavailable sources, omitted operations, unmatched selectors, missing pricing, or incomplete cache writes | `0` | `outcome: "degraded"` |
| Invalid local config, failed required initialization, cancellation, internal worker failure, or JSON write failure | Nonzero | None |

A run where every remote source is unreachable still exits `0` with
`outcome: "degraded"`, because exit `0` means the requested warming pass
completed. Strict inspection (`catalog tools`, `config check`) and
qualification keep their completeness checks and fail on missing tools.

### Preflight and error handling

Preflight rejects malformed configuration or selectors, unknown source IDs,
cache opt-outs on selected sources, invalid network settings, and missing
required state or wallet configuration. During execution:

- **Unsupported operations** are recorded as per-source omissions for both
  local and remote catalogs.
- **Local fatal errors** (unreadable local files, malformed local documents,
  structural-safety violations, and invalid config) fail the command after
  draining any sibling work already in flight.
- **Shutdown** cancels unsigned fetches and drains in-flight paid relay calls
  before closing the wallet owner.

Both `serve` and `catalog warm` accept empty tool selections for local and
remote catalogs: missing tool names, missing tags, contradictory name filters,
and empty opted-in `recommended_tools` report reduced coverage without
broadening filters. Selector syntax and field types are still validated.

## Warming JSON

Stdout emits one JSON document and stderr carries logs. The top-level fields
are `direct` (boolean), `outcome` (`"ok"` or `"degraded"`), and `sources` (one
entry per selected source ID, sorted by ID).

Default `INFO` logs report warming start, per-source outcomes, and completion
(after paid relay calls drain and wallet cleanup finishes). `RUST_LOG`
overrides the default filter; fetch and cache details log at `DEBUG`.

```json
{
  "direct": false,
  "outcome": "degraded",
  "sources": [
    {
      "source": "healthy",
      "status": "ready",
      "catalog_cache": "fresh",
      "pricing": {"status": "not_configured"}
    },
    {
      "source": "offline",
      "status": "unavailable",
      "failure": {
        "stage": "headers",
        "reason": "connection_failed",
        "message": "Could not connect to the catalog server or configured proxy"
      }
    }
  ]
}
```

### Source entry fields

Every source entry includes `source` and `status`. Optional fields appear only
when populated (empty `omissions` and `selection_notices` arrays are omitted):

| Field | Meaning |
| --- | --- |
| `status: "ready"` | Source is ready with no omitted operations or unmatched selectors. Pricing or cache issues can still make the overall `outcome` degraded. |
| `status: "partial"` | Source has callable tools alongside `omissions` or `selection_notices`. |
| `status: "unavailable"` | Fetch failed or filtering left zero tools. Failed fetches include `failure` and omit `catalog_cache` and `pricing`. |
| `catalog_cache` | Storage state after the pass: `fresh`, `fresh_via_relay`, `requires_revalidation`, `not_stored`, or `local_file`. An unavailable source may still have an older cached snapshot on disk. |
| `omissions` | Objects with `method`, `path`, and `reason` for each unsupported operation that matched selection. Unavailable sources can also report omissions when rejecting an invalid replacement. |
| `selection_notices` | Objects with `setting` and unmatched `selector`. |
| `failure` | Sanitized fetch or processing failure object. |
| `pricing` | Pricing result with `status` and optional `failure` or `catalog_cache`. |

`requires_revalidation` and `not_stored` mark the overall `outcome` as
`degraded` while keeping valid tool definitions available. Cache lifetimes and
validation rules live in
[automatic discovery disk cache](configuration.md#automatic-discovery-disk-cache).

### Pricing statuses

`pricing.status` is one of:

- `not_configured`: no pricing source configured.
- `catalog`: price hints embedded in the accepted catalog.
- `ready`: external pricing metadata processed (also includes its own
  `catalog_cache` field; `not_stored` or `requires_revalidation` degrades the
  overall outcome). Matching gaps simply leave individual tool prices unknown;
  runtime x402 challenges determine charges.
- `unavailable`: optional pricing fetch or parse failed (never makes a working
  source `unavailable`).
- `not_attempted`: the source produced no eligible API tools.

### Omission reasons and selection notices

Omission `reason` codes describe compatibility issues without upstream error
text: `cookie_parameter`, `parameter_serialization`, `request_media`,
`input_reference`, `input_schema`, `schema_reference_siblings`,
`header_contract`, `header_name`, `header_reserved`, `header_missing_name`,
`header_duplicate`, `header_serialization`, `idempotency_key_schema`, and
`header_argument`.

`selection_notices` identify settings such as `include_tools`,
`recommended_tools`, `operation_filters`, and `send_tools`. A `tool_filters`
notice with selector `*` indicates that filters produced an empty inventory
with no single missing positive selector.

### Failure stages and reason codes

Each `failure` object contains `stage`, machine-readable `reason`, display
`message`, and optional numeric `http_status`. Branch on `stage`, `reason`, and
`http_status`, not on `message`. Display unrecognized reason codes safely
alongside their `message` rather than rejecting the JSON summary.

- **Stages:** `headers`, `body`, `local_read`, `parse`, `configuration`,
  `generation`, `selection`, `publication`, `acquisition` (relay failures
  without a finer document stage), and `pricing` (external pricing failures
  without a finer stage). Local fatal errors fail the command rather than
  appearing as completed source failures.
- **Shared reason codes:** `http_status`, `download_limit`,
  `unsupported_operations`, `incomplete_document`, `connection_timeout`,
  `response_inactivity`, `connection_failed`, `request_failed`,
  `local_read_failed`, `invalid_json`, `configuration_failed`,
  `generation_failed`, `selection_failed`, `publication_failed`, `body_failed`,
  and `acquisition_failed`.
- **Pricing and reference codes:** `pricing_document_invalid`,
  `pricing_inventory_unavailable`, `external_reference_unsupported`,
  `unresolved_reference`, and `cyclic_reference_unsupported`.
- **Relay reason codes:** `tls_failure`, `read_inactivity`, `connection`,
  `body`, `decode`, `request_construction`, `transport`, `partial_response`,
  `incomplete_delivery`, `response_rejected`, `invalid_relay_envelope`,
  `catalog_complexity`, `catalog_parse`, `origin_request`,
  `target_payment_required`, `unsolicited_not_modified`,
  `unsupported_response`, `no_representable_relay`, `provider_unavailable`,
  `qualification_authority`, `funding_denied`, `payer_changed`,
  `unsupported_payment`, `duplicate_payment_challenge_headers`,
  `conflicting_v1_offers`, `missing_pinned_v1_challenge`,
  `missing_pinned_v2_challenge`, `price_limit`, `payment_pending`,
  `wallet_not_ready`,
  `funding_unavailable`, `payment_outcome_unknown`, `chain_recovery_required`,
  and `payment_or_response` (plus the shared HTTP, timeout, and limit codes). A
  relay payment refusal stays a source failure; missing required wallet
  initialization fails before acquisition.

## Framework preparation and activation

Follow these rules when orchestrating `x402_treazury` from an outer framework:

- **Accept degraded warming:** Treat exit `0` with valid `ok` or `degraded` JSON
  as completed catalog preparation. Surface unavailable sources, omissions, and
  pricing or cache warnings to the user instead of requiring 100% ready sources
  or a minimum tool count. Treat invalid JSON as a protocol failure.
- **Keep financial bootstrap separate:** Catalog degradation never resets
  completed bootstrap state, changes wallet identities, reopens funding claims,
  or invalidates pending financial work.
- **Configure tolerant remote sources:** Set `allow_catalog_failure = true` on
  remote sources that should not block endpoint startup during outages, while
  keeping `wait_for_catalogs = true` so endpoints publish settled inventories.
  An endpoint whose remote sources all fail can still serve `initialize` and
  `tools/list` while explaining unavailable coverage in its MCP instructions.
  Standalone `--provider` serving remains strict.
- **Understand permissive selection in `serve`:** Both synchronous and deferred
  catalog loads accept empty or partially filtered inventories without needing
  `allow_catalog_failure` (which governs fetch and admission failures).
  Filters and routing order stay intact, tool-name collisions still omit
  conflicting bindings, empty-inventory notices stay visible in MCP
  initialization instructions even when `instructions_text` is set, and
  unmatched positive selectors or empty inventories emit `ERROR` diagnostics
  after catalogs settle and when a replacement catalog publishes.
  See [catalog startup](configuration.md#catalog-loading-and-listener-startup).
- **Probe endpoints safely:** After starting `serve`, run authenticated MCP
  `initialize` and `tools/list` probes on each endpoint; never use paid API
  calls as health checks. Auth, listener, and protocol errors fail that
  endpoint's activation, while unrelated MCP servers and the outer framework
  stay available. Rely on CLI exit codes, JSON summaries, and MCP responses
  rather than treating every `ERROR` log line as a fatal endpoint crash.
- **Recognize terminal endpoint-failure logs:** Because `serve` stays alive
  when one endpoint fails catalog startup or publishes invalid tool selections,
  process exit status alone does not report single-endpoint failures. Non-fatal
  `ERROR` logs (such as permissive-selection notices or transient Tor SOCKS
  waits) keep serving, whereas these two `ERROR` messages identify terminal
  endpoint unavailability requiring configuration repair and a restart:
  - `MCP endpoint unavailable; catalog startup failed; correct the reported cause and restart`
  - `MCP endpoint tools are unavailable because its tool selection is invalid; correct the configuration and restart`
- **Never auto-retry financial or network changes on failure:** On an
  interrupted or failed step, do not automatically rerun `wallet bootstrap` or
  potentially paid `catalog warm`, switch from Tor to direct mode, rotate
  wallets, delete state or caches, or raise spend caps. Preserve financial
  state and let `x402_treazury`'s own draining and recovery handle in-flight
  work.

## Offline inspection and container runtime

Frameworks commonly inspect state before starting `serve` using
`X402_TREAZURY_CONFIG` (or `--config FILE`):

- `config show` emits JSON by default (exposing resolved `network`, `treasury`,
  `servers`, `wallets`, `wallet_bootstrap`, and `base_rpc_policy`).
- `build-info` emits JSON by default (`source_revision`, `source_dirty`, and
  build metadata).
- `wallet status --json` reads public SQLite metadata without opening
  `key_file` or decrypting the Zcash snapshot. Follow it with `wallet addresses`
  (JSON by default) when you also want to verify encrypted snapshot readability.
- `wallet preview --json` emits `schema_version: 1` and exits nonzero if
  `startup_conflicts` is nonempty.

When packaging `x402_treazury` in a container:

- **Writable `/tmp`:** Provide writable temporary space so offline wallet
  inspection can reconstruct encrypted snapshots safely.
- **`flock` and SQLite WAL support:** Mount `state_dir` on a filesystem that
  supports `flock` advisory locks and SQLite WAL writer exclusion.
- **Extended `SIGTERM` grace period:** Configure an extended or unbounded
  container stop grace period so `SIGINT` and `SIGTERM` draining can finish
  accepted Zcash transactions instead of being killed after a short default
  timeout.
- **Tor Unix sockets:** When connecting over a Tor Unix domain socket
  (`socks_endpoint = "unix:/path"`), configure Tor's `SocksPort` with
  `IsolateSOCKSAuth` (and `GroupWritable` and `RelaxDirModeCheck` as needed for
  container user permissions).
