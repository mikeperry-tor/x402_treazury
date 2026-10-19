# Zcash isolation recovery after the HTTP batch

## Scope and prerequisite

Deferred extension, not part of the first catalog/pricing, API and Base RPC batch.
Begin implementation only after that batch has passed its cumulative/final reviews
and merged. This unfinished extension does not block its cleanup or merge and does
not authorize live-funded qualification, transaction replay or a treasury rescan.

Before implementation, identify the merged HTTP batch SHA and current starting
baseline, inspect its exact transport/ownership interfaces, and independently
review this extension against the resulting code. The current reference baseline
for the observations below is `970f6bde1334eb42a540288ea1432381e1b9ae64`; it is not
an instruction to implement against stale pre-merge interfaces. Confirm permanent
HTTP selected-entry guidance and regression evidence were handed off before
removal of the HTTP plan. Use maintained guides and merged code as prerequisites,
not a link to a completed plan that will be deleted.

Owning guides: [network policy](../../network-egress.md),
[resource limits](../../local-resource-limits.md),
[wallet ownership/recovery](../../wallet-rotation.md),
[wallet observation](../../wallet-rotation.md#wallet-observation-policy),
[configuration](../../configuration.md), [development](../../development.md),
[testing](../../testing.md), [Tor evidence](../../../tests/TOR.md) and
[complexity discipline](../../../tests/COMPLEXITY.md#review-discipline).

## Existing transport and financial boundaries

Zcash uses `NetworkContext::grpc` and a separate cached `GrpcIndexer`/tonic channel,
keyed by Tokio runtime, canonical identity and full endpoint. SOCKS credentials
belong to the channel connector; cloning an indexer cannot change circuits. Reuse
the merged HTTP work's owner-bound scope, coherent record and conditional selected-
entry replacement principles where useful, but retain a separate gRPC adapter and
cache. Do not force gRPC through reqwest, Curl relay or an HTTP status retry loop.

Full scans in `treasury::SyncSession` create a private runtime, install an indexer
into the SDK and release channels before runtime teardown. Replacing a factory
entry does not move an active SDK scan; selected-channel retention naturally ends
with that runtime. Long-lived lookup/reuse-check owners can retain selection on
their own runtime. Do not add a cross-runtime winner registry to preserve circuits.

Keep treasury UUID identity for scans and exact durable recipient/owned-transparent
address identity for transaction operations. Transport scope never changes payer,
financial owner or prepared bytes. `GrpcSubmission` validates preflight, retains
its ready client and dispatches the journaled transaction once. Unknown send
outcomes remain unknown; no generic gRPC retry may replay `send_transaction`,
restart financial calculation, create another signature or consume additional
transaction-recovery authority merely because a read or connection failed.

Initial scope is connection establishment and explicit read-only network-info,
tip, capability and transaction-lookup boundaries. Preserve network/capability
validation, protocol expiry, response-inactivity/readiness meaning, private-runtime
cleanup and existing observation/backoff scheduling. Do not migrate or restart
progressing streaming scans. Mid-stream resumption is outside the initial delivery
and requires its own SDK/progress-ownership design if later requested.

## Proposed ordered delivery and evidence

These boundaries must be reconciled with the merged baseline during independent
design review before substantive edits. Keep coherent tests and permanent guide
updates with each commit; do not leave an unprotected submission path between them.

### Z1 — Coherent gRPC selection and channel lifetime

Introduce a canonical-owner/effective-scope channel record and explicit enrollment
policy. Implement private candidate channels and conditional cache-entry replacement
using exact acquired-record identity, without a winner/history table or counters.
Use fresh scopes for participating misses, including after runtime teardown/restart;
nonparticipating callers keep their existing behavior. Acquired old channels remain
valid and cannot resurrect evicted/replaced records. Do not enable automatic method
retries or broaden any financial authority in this commit.

Test with local trusted TLS/gRPC through fake authenticated SOCKS: distinct
credentials/connections, selected-record reuse, owner/endpoint/runtime separation,
concurrent promotion, eviction/reinsertion, retained active streams and unchanged
nonparticipating/submission behavior. Exercise `release_grpc` against canonical
owners and private-runtime cleanup; stale channels must not cross runtime lifetime.
Update network/resource guidance for the delivered primitive, with enrollment
still pending Z2. Independently review its exact SHA and substantive complexity
before read consumers depend on it.

### Z2 — Explicit read-only recovery and validated promotion

Enroll only the read-only boundaries above. Define eligible failures from typed
connection/gRPC status evidence at the method boundary; gRPC status/trailers are
not equivalent to the HTTP 400/403/429/503 policy. Respect external retry guidance
where applicable without new whole-work deadlines or an independent retry layer.
Use selected channel then finite fresh candidates before returning the read failure;
reuse the merged scope/selection machinery rather than duplicating retry state.
Retain failed/pending lookup meaning, network/capability checks and original
scheduler authority. A read outage cannot become non-inclusion evidence.

Promote after applicable read validation, before handing a selected indexer into
new SDK work where appropriate. Preflight/read recovery must finish before the
existing one-shot broadcast boundary; keep its selected ready handle pinned.
No whole-SDK retry wrapper may intercept transaction submission or scan mutation.

Test real birthday/network/tip/capability/lookup consumers with selected/fresh
success and exhaustion, malformed/wrong-chain data, pending versus unknown results,
silent responses versus progressing streams, unrelated concurrent progress,
waiter cancellation and owner/runtime termination. Assert exact channel/RPC counts,
retained old-stream progress and next-operation winner use. Reopen durable financial
fixtures after an ambiguous send: preserve exact bytes, input owner and unknown
outcome with no automatic rebroadcast or repeated calculation. Confirm treasury
cadence and recovery-attempt accounting are unchanged. Update wallet/network/
configuration, both resource inventories and testing guidance in this commit.

**Cumulative gate ZG1, after Z2:** independently review Z1/Z2, all fixups and full
birthday/sync/preflight/lookup/submission call chains alongside the merged HTTP
implementation. Require source-matched tests for channel races, private-runtime
lifetime and absence of financial replay. Compare original extension-baseline
CogC/CycC with the same pinned analyzer, review new/worsened warnings and unchanged
SDK/submission hotspots, and evaluate concrete simplifications. Examine fixup root
causes and recurring ownership assumptions; add regressions or widen the audit
where warranted. Resolve material findings before cleanup.

### Z3 — Documentation handoff and extension cleanup

Verify enduring rules and evidence limitations already live in owning guides and
AGENTS.md links still reach network, wallet ownership and complexity policy. Remove
this extension only when its initial read-only scope and validation are complete;
retain focused concrete unfinished work if needed, not a completed-plan archive.
Fix incoming links without deleting private evidence or changing runtime code.
Validate paths/anchors, delivered-versus-proposed claims and evidence source hashes.

**Final gate ZG2, after Z3:** independently review the exact cleanup SHA and combined
extension, all prior commits/fixups, relevant unchanged consumers and its interaction
with the merged HTTP batch. Require ZG1 findings resolved, documentation handoff,
source-matched validation/complexity evidence and no unexplained lifetime or financial
boundary changes. Material later changes reopen affected checks and reviews.

Follow AGENTS.md exact-SHA reviews by non-authors, introducing-SHA fixup commits and
independent fixup review; preserve unsquashed history through final integration.
Use independent read-only design review before edits and analyze recurring fixup
causes at both gates. Run focused default-Zcash tests through `scripts/zcash.sh`,
verify named tests execute nonzero cases, and run applicable formatting/lint checks.
Keep sources/builds coordinated and metrics under ignored `target/` or private
storage. Local fixtures do not qualify live Tor circuits or settlement. Real-funded,
Docker consensus and live Tor work retain their separate explicit authorization.
