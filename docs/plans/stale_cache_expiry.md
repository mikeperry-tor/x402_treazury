# Stale cache expiry

## Intent and baseline

Refresh the affected catalog and its shared operation estimates when an API supplies
a concrete operation-removed or schema-changed signal. Preserve the original call
outcome and never replay the API call. Source baseline:
`4476090b6d43d7016f1d1ee36eb2040d9910b5cd`; inspect intervening changes before starting.
The shared cache contract lives in [configuration](../configuration.md#automatic-discovery-disk-cache),
[architecture](../architecture.md#wallet-selection-and-network-isolation) and the
[resource inventory](../local-resource-limits.md).

Current `payment::HttpFailure` and `payment::diagnostics::ProviderFailure` consumers
recognize settlement failure, invalid arguments, temporary unavailability and
provider data-refresh failure. None establishes that an operation was removed or
its schema changed. Dynamic-source refresh is explicit maintenance. A generic
HTTP 400, 404 or 422 cannot supply the missing evidence. Do not invent a
provider-specific classifier or unused signal abstraction to fill this gap.

## Sweep evidence and candidate indicators

The 2026-11-11 public sweep (`target/provider-updates/1794430761571364000`, ignored)
attempted all 70 bundled providers: 68 completed captures, `ax1` and
`coinmarketcap` were unavailable, `agproxy` and `curl` were semantically unchanged,
`horizonpulse` produced the only real candidate, and the remaining captures
established baselines. No provider returned an explicit operation-removed or
schema-version marker; `horizonpulse` produced a refreshed-document candidate, which
is evidence to review rather than an explicit removal/schema-change signal. Authored providers had no prior capture, so their remote
evidence cannot yet establish drift. These are candidate indicators, not refresh
authority:

| Indicator | Existing producer | What it can show | Limitation |
| --- | --- | --- | --- |
| Response validator drift (`ETag`, `Last-Modified`; `Date` is response time only) | Provider-update capture now records these safely beside exact bytes | A changed validator with unchanged bytes is representation churn; a changed body with a stable validator makes the validator unreliable | Neither proves an operation was removed or a request schema changed; `Date` alone has no discriminating power |
| Source generation metadata (`info.version`, `openapi`, vendor timestamps such as `lastUpdated` or `x-generated-at`) | Captured schema/registry/pricing documents | The vendor published a new document generation | Generation fields are optional and may not track operation/schema compatibility |
| Operation identity drift (added/removed method/path, generated-name change, GET/POST pair change) | Provider-update projection and `tool_changes` | A concrete catalog-contract difference exists and needs review | Offline evidence only; no runtime replay, and filtered tools can hide the change |
| Override and parameter-note target drift | Provider-update override diagnostics | An authored target no longer resolves, or a new operation reached a selector | Replacement text can mask upstream changes; unknown compiler targets are `null` |
| Pricing relocation or shape change | `pricing.relocation`, `pricing_metadata_operations`, `pricing.signals`, external pricing diff | The configured key/source moved, disappeared or changed shape | Advisory metadata; unknown is not free and no price or request authority follows |
| Conditional revalidation and cache age | Catalog DEBUG `revalidate`, `revalidated`, `disk_hit`, `requires_revalidation`, `fresh_via_relay`, configured TTL and acquisition age | The local snapshot is old, revalidated unchanged, or was served through a relay/direct-warm path | A 304 proves only that representation at that request; expiry is not failure and a hit does not prove compatibility |
| Runtime operation failure patterns (404/405/410, repeated `INVALID_REQUEST_BODY`, `InlineRefreshFailed`) | `HttpFailure` status and `ProviderFailure` codes | A provider may be rejecting a previously published contract | Generic status alone cannot distinguish removal, routing, argument or provider policy; never replay |
| Live challenge shape drift (version, scheme, network, asset, amount versus advertised hint) | Validated challenge and observed-offer facts | The executable payment contract or price differs from the cached advertisement | The challenge is authoritative for payment, not proof that the catalog is stale; gating and wallet policy also vary |

Indicators are corroborating evidence only. A future refresh hook needs at least one
concrete producer tied to the affected source, preferably an operation identity
change or an explicit provider error code. A generic HTTP status, age alone, a
pricing signal or a validator change is insufficient. Never replay a call, allocate
wallets, add probes or start a periodic scheduler to obtain corroboration.

A subsequent full capture the same day compared all 70 providers against the prior
one. Forty-nine reported no upstream evidence change, seven authored providers also
had no upstream change, three vendor-copy assets were unchanged, five authored
providers needed a first baseline review, one source was unavailable, and three
authored providers plus two external-pricing providers had reviewable differences.
Those differences were local documentation edits, timestamp-only churn (`2s`
registry `generatedAt`, `brazilayer` per-record `lastUpdated`), live directory
statistics (`x402-list` help), a dynamic Straits guide-page re-render, and a
`strale` capability-array reorder with no slug or field change. These are concrete
examples of indicators that must not be promoted to schema-change authority without
structural corroboration.

Provider-update packets now expose `response` metadata and distinguish a
semantically unchanged committed vendor asset from a real candidate. They also
report `pricing_metadata_operations` separately from usable estimates. These
instrumentation changes improve sweep evidence; they do not implement refresh
authority.

## SC1 — Evidence-based invalidation and refresh

First have an independent, read-only design reviewer choose and challenge a reliable
existing producer from the indicators above, including whether it can identify the
affected catalog and consumers without provider-specific dispatch. Until that
evidence exists, this work remains unfinished.

Deliver the producer, smallest invalidation/acquisition hook, affected consumers,
focused regressions and maintained documentation in one coherent SC1 commit.
Coalesce through existing catalog acquisition/publication for static and dynamic
consumers that already own a corresponding catalog. Preserve unrelated sources,
accepted financial work, shared cache eligibility and the existing concise pricing
presentation. Add no probes, periodic scheduler, new cache partition or API replay.

Use a small faithful positive signal and negative resource-not-found/bad-argument
cases. Assert original outcomes, no duplicate active refresh, no signing or funding
and unchanged MCP definitions/search/results. Update the owning architecture,
configuration and test guides with the actual trigger and refresh behavior.

Independently review the exact SC1 SHA and each introducing-SHA fixup, preserving
unsquashed history. The final integration gate must examine all preceding commits
and fixups with complete acquisition, binding, observation, description/selection,
challenge and journal/signing call chains, including aliases, source refresh and
relay consumers. Resolve recurring causes and coverage gaps; compare source-matched
CogC/CycC against the starting baseline and review relevant unchanged hotspots.
Follow [AGENTS.md](../../AGENTS.md) and the
[validation order](../testing.md#validation-order-and-continuation).

After acceptance, verify the permanent documentation handoff, update the index and
remove this plan without an archive. Independently review that exact cleanup SHA
and the final combined branch. This plan authorizes no live paid calls.
