# Complete-document acquisition audit

The Coinbase integration review found an existing gap outside its new search and
explicit usage-document readers: lazy help in `catalog_state.rs`
(`BoundTool::invoke_output`) uses `error_for_status`, and dynamic OpenAPI import in
`discovery/import.rs` (`fetch_response`) accepts successful statuses. Neither
rejects unsolicited HTTP 206 or HTTP 200 with Content-Range. A partial body that
is valid text or structurally valid JSON can therefore appear complete.

Starting reference: Coinbase functional baseline `a919de52`; verify the current
consumers before changing them. New Coinbase origin/relay search and
`source_details(documentation_url=...)` reject explicitly partial responses.
Generic API/raw responses may legitimately use partial-response semantics;
speculative URL probes return location leads rather than accepted usage text.

Audit lazy help and import independently through acquisition, bounds, parsing,
caching, publication and output. Establish their complete-document contracts
before implementing a correction. Include static catalog acquisition and shared
relay decoding where they participate, and inspect cancellation/error consumers.
Prefer a scoped response-contract check over a global HTTP restriction or new
state. Do not add limits, timers, credentials or automatic paid retries.

Acceptance for a subsequent coherent fix: regressions for HTTP 206 and HTTP 200
with Content-Range, valid complete text/JSON, accepted empty help where supported,
visible errors without partial success, and no cached/published partial result.
Keep generic API response behavior intact. Update the owning
[agent source](../../agent-sources.md), [network](../../network-egress.md) and
[resource](../../local-resource-limits.md) guides where contracts change. Require
independent design review before implementation, exact-SHA/fixup reviews and a
combined call-chain/complexity review before completion, following AGENTS.md.

This is focused unfinished work, not a qualification claim or authority for live
paid calls or wallet access.
