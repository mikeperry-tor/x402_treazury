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

## SC1 — Evidence-based invalidation and refresh

First have an independent, read-only design reviewer identify a reliable existing
producer and challenge the necessity and scope of the hook. Until that evidence
exists, this work remains unfinished.

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
