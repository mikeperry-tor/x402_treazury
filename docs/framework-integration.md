# Framework integration

Treat catalog preparation, financial bootstrap, MCP availability and successful API
execution as separate results. Catalog preparation can finish with reduced coverage;
a running process or a successful MCP handshake does not establish complete coverage,
available funds or a working paid API.

## Catalog preparation

Run `catalog warm --config FILE` under the deployment's existing network and payment
authority. See [warming configuration](configuration.md#warming-a-selected-source-including-directly-for-a-tor-deployment)
for source selection, cache behavior and the explicit unsigned `--direct` option.
Configured relay warming may pay; warming never starts listeners or funding workers.

A valid warming command consumes every independent selected source result. Exhausted
remote acquisition failures, rejected remote documents, unsupported operations and
remote inventory changes are reported without dropping healthy sibling work. A
compatible partial catalog may be accepted; wholly unsupported or invalid replacement
documents cannot replace or renew previously accepted cache state or learned prices.
Optional pricing failure leaves usable API definitions available with unknown prices.

| Result | Exit status | Completed JSON |
| --- | --- | --- |
| Every selected source is usable without reported degradation | 0 | `outcome: "ok"` |
| Attempts finish with unavailable sources, omissions, missing selections, unavailable pricing or incomplete cache storage | 0 | `outcome: "degraded"` |
| Invalid local configuration, required initialization failure, cancellation, internal worker failure or failure to write the final JSON | Nonzero | No completed-success summary |

All selected remote sources may be unavailable and still produce a completed,
degraded zero exit. Zero means the requested attempts finished; it does not guarantee
cache persistence, complete tool coverage or future availability. Strict inspection
and qualification retain their completeness requirements.

Preflight rejects malformed configuration/selectors, unknown selected source IDs,
selected cache opt-outs, invalid network settings and missing required state or
wallet authority. Unsupported-operation omissions are coverage results for local
and remote catalogs; local read, malformed document, structural-safety and
configuration failures remain fatal even when found
during execution; sibling work already started is drained before returning the
failure. Shutdown stops unsigned acquisition and drains accepted paid work before
closing its financial owner. No new retry, direct fallback or whole-work deadline
is implied by a degraded result.

Ordinary serving and warming accept empty selections for local and remote catalogs.
Missing names, missing tags, contradictory name filters and empty opted-in advice
select no additional tools; they report reduced coverage instead of failing
preparation or endpoint selection. Selector syntax and field types remain validated.
Strict inspection and qualification retain inventory and completeness checks.

## Warming JSON

Stdout contains one JSON document; diagnostics go to stderr. `direct` is a boolean,
`outcome` is `ok` or `degraded`, and `sources` contains exactly one result for each
selected authored source ID, sorted by ID.

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

Each source has `source` and `status`. Other fields are present when applicable;
empty omission and selection-notice arrays are omitted.

| Field | Meaning |
| --- | --- |
| `status: "ready"` | Usable source without omitted operations or missing selections. Pricing/cache degradation may still make the overall outcome degraded. |
| `status: "partial"` | Usable tools remain alongside `omissions` or `selection_notices`. |
| `status: "unavailable"` | Acquisition failed or selection leaves no tools. A rejected acquisition has `failure` and omits `catalog_cache` and `pricing`. |
| `catalog_cache` | State of accepted storage after the attempt: `fresh`, `fresh_via_relay`, `requires_revalidation`, `not_stored` or `local_file`. An unavailable source can retain an earlier accepted snapshot; this field does not certify the new candidate. |
| `omissions` | Objects containing `method`, `path` and a fixed `reason` for each unsupported selected operation. Unavailable sources can also report omissions without accepting an unusable replacement. |
| `selection_notices` | Objects containing the authored `setting` and missing `selector`; no unrestricted tool fallback is authorized. |
| `failure` | Safe acquisition failure metadata described below. |
| `pricing` | Optional pricing result with `status` and, on failure, `failure`. |

`requires_revalidation` and `not_stored` degrade the summary without making usable
definitions unavailable. A stored document is not evidence that its API works.
`fresh_via_relay` describes catalog storage provenance, not a promise about future
relay availability. Cache lifetimes and acceptance are owned by
[configuration](configuration.md#automatic-discovery-disk-cache).

Pricing statuses are `not_configured`, `catalog` (advertisements embedded in the
accepted catalog), `ready` (external metadata processed successfully) and
`unavailable`, plus `not_attempted` when the source has no eligible API tools.
External pricing with `ready` also reports its independent `catalog_cache` using
the same storage vocabulary as catalogs. `not_stored` or `requires_revalidation`
degrades the summary even when pricing metadata was valid. `ready` does not mean
every tool has an advertised price. Matching
ambiguities and absent records leave prices unknown; runtime challenges determine
actual payment requests. Pricing failure never changes a usable source to
`unavailable`.

Omission `reason` values are `cookie_parameter`, `parameter_serialization`,
`request_media`, `input_reference`, `input_schema`, `schema_reference_siblings`,
`header_contract`, `header_name`, `header_reserved`, `header_missing_name`,
`header_duplicate`, `header_serialization`, `idempotency_key_schema` and
`header_argument`. They describe compatibility, without upstream error prose.
Selection notices identify provider/distribution settings such as `include_tools`,
`recommended_tools`, `operation_filters` and `send_tools`. `tool_filters` with
selector `*` reports an empty filtered inventory without a specific missing
positive selector. The selector remains an
identity, not evidence that an unrestricted tool was selected.

A failure contains `stage`, stable machine-readable `reason`, authored display
`message` and optional numeric `http_status`. Branch on codes/statuses, not message
text. Catalog stages are `headers`, `body`, `local_read`, `parse`, `configuration`,
`generation`, `selection` and `publication`; remote relay failures without a more
specific document stage use `acquisition`. Optional pricing also uses `pricing`
when acquisition supplied no more specific stage. Local fatal errors do not become
completed source failures merely because this vocabulary can describe them.

The shared failure codes are `http_status`, `download_limit`,
`unsupported_operations`, `incomplete_document`, `connection_timeout`,
`response_inactivity`, `connection_failed`, `request_failed`, `local_read_failed`,
`invalid_json`, `configuration_failed`, `generation_failed`, `selection_failed`,
`publication_failed`, `body_failed` and `acquisition_failed`. Optional pricing can
also report `pricing_document_invalid` or `pricing_inventory_unavailable`.
Authored reference-validation codes, where applicable, are
`external_reference_unsupported`, `unresolved_reference` and
`cyclic_reference_unsupported`. Relay acquisition preserves existing safe reason codes: `tls_failure`,
`read_inactivity`, `connection`, `body`, `decode`, `request_construction`,
`transport`, `partial_response`, `incomplete_delivery`, `response_rejected`,
`invalid_relay_envelope`, `catalog_complexity`, `catalog_parse`, `origin_request`,
`target_payment_required`, `unsolicited_not_modified`, `unsupported_response`,
`no_representable_relay`, `provider_unavailable`, `qualification_authority`,
`funding_denied`, `payer_changed`, `unsupported_payment`, `price_limit`,
`payment_pending`, `wallet_not_ready`, `funding_unavailable`,
`payment_outcome_unknown`, `chain_recovery_required` and `payment_or_response`,
as well as shared HTTP/timeout/limit codes. A source-specific relay payment
refusal remains an acquisition result under existing authority; it grants no replay
or increased payment allowance. Required wallet/authority initialization failures
remain fatal before acquisition. Consumers should display unfamiliar codes safely
alongside their message rather than reject an otherwise valid summary.

## Framework preparation and activation

Accept a zero exit with valid `ok` or `degraded` JSON as completed catalog
preparation. Display unavailable sources, omissions, pricing and cache limitations;
do not require a minimum source/tool count, all-ready statuses or fresh stored
entries. Invalid JSON is a protocol failure. ERROR diagnostics identify source
capability loss and do not independently imply process or endpoint failure.

Keep financial bootstrap separate. Catalog degradation does not erase a completed
bootstrap, change wallet identity, reopen funding claims or invalidate accepted
financial work. Preparation records describe historical completion, not current
health.

For deployments that should retain remaining tools through remote outages, opt
remote sources into `allow_catalog_failure = true`. Retain `wait_for_catalogs = true`
when endpoints should publish their settled available inventory. Required sources
retain their stricter readiness choice. An otherwise valid endpoint emptied by
remote loss can initialize and list tools, while explaining unavailable coverage.
Standalone single-provider serving retains its separate strict startup boundary.
Ordinary deployment serving applies the same permissive selection policy to
synchronous and deferred loads, local and remote catalogs. The source flag controls
acquisition/admission failures; accepted catalogs publish their available selection,
even when empty or wholly unsupported, without requiring the flag. It does not hide API call errors
or authorize retries.

Missing endpoint inclusion and distribution names, missing operation tags and
empty intersections retain their authored filters and routing order. No namespace
ownership inference or typo detection gates ordinary serving or warming. Shared
or nested prefixes do not make an absent name fatal; actual tool-name collisions
still omit every conflicting binding. An empty static inventory is visible in
initialization instructions even with custom instructions. Unmatched positive
selections and empty endpoint inventories emit ERROR diagnostics after their
catalogs settle or a replacement is published; these describe reduced coverage,
not endpoint failure. No inventory history is required to distinguish a typo from
provider drift. Inspection and qualification remain strict.
See [catalog startup](configuration.md#multiple-mcp-ports-from-one-configuration)
for endpoint readiness settings.

After starting serving, probe authenticated MCP initialize/list on every configured
endpoint. Authentication, listener and protocol failures still fail the affected
activation. Do not call paid APIs as readiness probes. Preserve unrelated MCP
registrations and keep the framework available when its treasury integration is
unavailable. Use native command results and actual endpoint responses; do not treat
every ERROR log line as a terminal endpoint event.

A failed, interrupted or missing result grants no replay authority. Do not
implicitly rerun bootstrap or potentially paid warming, switch to direct egress,
rotate wallets, delete state/caches or loosen payment limits. Retain the same
financial state and native accepted-work draining/recovery. Integrations must build
and package a treasury revision implementing this contract; preparation records
alone cannot establish the behavior of a different executable.
