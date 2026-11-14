# Deferred: stateless MCP catalog-change subscriptions

Status: deferred, pending verified support in Hermes or another identified major
agent. This is a proposed design and qualification plan, not implemented behavior
or authorization to begin implementation. Per the user's handoff request, this
draft remains uncommitted for copying into the active repository.

## Intent, baseline and governing guidance

Keep connected agents' advertised tools current after successful catalog changes,
using the modern stateless MCP subscription protocol. Do not add protocol-level
sessions, `Mcp-Session-Id`, or the legacy GET notification channel.

Draft inspection baseline: x402_treazury branch `main`, commit
`bb86a307a4ab30931892a27a0df988a1873a8e51`, with rmcp `3.9.0`.
Hermes reference: v0.23.1 source
`3b421578588243f920886e6fe79a6faa87ea026e`, using Python MCP SDK `2.3.0`.
Recheck the active repository, dependency pins and consumer versions on activation;
these identifiers describe inspected sources, not future execution baselines.

Follow [AGENTS.md](../../../AGENTS.md), the
[planning workflow](../../planning-workflow.md),
[architecture](../../architecture.md),
[configuration](../../configuration.md),
[agent sources](../../agent-sources.md),
[testing](../../testing.md), and the
[complexity review discipline](../../../tests/COMPLEXITY.md#review-discipline).
The actual planning workflow is `docs/planning-workflow.md`.

## Current behavior and concrete benefit

- `src/server.rs` serves stateless Streamable HTTP at `/mcp`, explicitly disabling
  legacy session mode and returning SSE for POST responses. Active-call streaming
  does not provide a notification channel between calls.
- rmcp supplies modern request metadata, discovery, subscription dispatch,
  acknowledgments, filtering and cancellation machinery. Its default
  `accepted_subscription_filter()` returns `None`, making `subscriptions/listen`
  unimplemented. x402_treazury overrides neither that hook nor `listen()`, and
  advertises tools without `listChanged`.
- `src/catalog_state.rs` publishes immutable catalog snapshots. `tools/list` reads
  current listener-visible definitions, but publication does not notify clients
  holding an older inventory. Deferred source loading, registration, refresh,
  removal and restore can therefore change the catalog without a connected agent
  learning about it until rediscovery.
- In direct-tool mode, notifications would let clients discover added tools, remove
  stale tools and replace changed schemas/descriptions without manual reconnects.
  This includes advertised price prose when it actually changes in `tools/list`;
  it does not update Hermes's separate configured `tool_costs` table.
- In `discover_on_demand` mode, stable search/call wrappers already consult the
  current catalog. Internal catalog changes that do not change advertised tool
  definitions need no tool-list notification. Notification benefits are smaller
  unless wrapper descriptions or directly advertised recommendations also change.
- Changing initialization-only server instructions is not a tool-list refresh.
  Do not promise clients will replace initialization instructions or consume
  vendor catalog metadata merely because they re-fetch tools.

The Hermes reference has a notification handler that re-fetches tools, and can
negotiate the modern protocol, but inspection found no subscription-opening call.
Upstream code searches and inspection during drafting likewise found no evidence
of `subscriptions/listen` integration. Python SDK support is explicit opt-in through
`mcp.client.subscriptions.listen(..., tools_list_changed=True)`; it is not started
by constructing `ClientSession`. These are source findings, not interoperability
qualification, and must be revalidated when considering activation.

## Activation gate G0: demonstrate a real consumer

Remain deferred until Hermes or another named major agent has released usable
modern tool-change subscriptions. An SDK upgrade, a notification callback, a
configuration key named `events`, or support for legacy sessions is insufficient.

Next action: inspect a named candidate release's complete connection and recovery
paths. Identify its agent source SHA, SDK version, relevant configuration and
negotiated protocol revision. Use an unfunded local fixture to show the actual
client sends `subscriptions/listen`, opts into `toolsListChanged`, receives the
acknowledgment and refreshes its advertised tool inventory after an idle change.
Check stream-loss recovery rather than relying only on happy-path delivery.

If another agent satisfies G0 first, record it as the activation consumer; Hermes
compatibility remains an explicit unqualified boundary until its own tests pass.
Do not silently claim full Hermes compatibility from another client's success.

On activation, establish the actual branch/baseline and preserve unrelated work.
Update this plan with the consumer evidence and resolved design decisions, and
commit the active plan before implementation under the normal planning workflow.
The present instruction to leave the transfer draft uncommitted is not a general
exception to the active repository's implementation workflow.

## Proposed design and preserved boundaries

The [2026-07-28 protocol changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog)
removes sessions and replaces the GET channel with a long-lived
`subscriptions/listen` POST. Retain that stateless model. Confirm exact supported
SDK APIs at activation rather than adding a protocol compatibility layer.

1. Advertise `tools.listChanged` only with working delivery. Implement rmcp's
   `accepted_subscription_filter()` for tool-list changes and `listen()` using its
   native `SubscriptionContext`. Let rmcp own acknowledgment, subscription IDs,
   filter intersection and wire framing. Unsupported notification categories are
   not accepted. Do not add a new configuration switch unless a concrete operator
   need remains after checking native capability advertisement.
2. Connect successful publication to subscribed listeners through the smallest
   process-local change signal. Compare the actual endpoint-visible tool
   definitions, including schemas and descriptions, rather than treating every
   global generation change as a change to every listener. Reuse the same
   definition/projection logic as `tools/list`; do not create parallel catalog
   truth, persistent notification history or a general event bus.
3. Publish before notifying, outside catalog locks and financial critical sections.
   Do not await slow clients while publishing. Coalescing is acceptable because a
   notification invalidates cached inventory, not a durable event history. Prevent
   the subscribe/list race: once acknowledgment permits a client to list tools,
   subsequent relevant publication must remain observable by that subscription.
   Verify whether native watch/version signals already satisfy this contract.
4. Preserve listener-local visibility and authority. Apply bearer authentication,
   Host/Origin rules and existing routing to subscription POSTs. Session or
   subscription identifiers never select wallets, source owners or permissions.
   A listener must not receive another listener's tools or change details.
5. Close subscription work through its request cancellation and server shutdown
   lifecycle. Idle streams are not accepted financial work and must not prevent
   graceful drain. Dropping a stream must neither cancel accepted paid work nor
   allocate wallets, initiate funding, renew signed authority or replay requests.
   Preserve `src/server/work.rs` ownership independently of delivery.
6. Reconnect recovery re-establishes the subscription and fetches a current list;
   no replay log is needed. Define client responsibility clearly: the server cannot
   repair a client that never subscribes or reconnects. Inspect native cleanup and
   cancellation guarantees before adding local registries, limits or timers.

Observed pricing hints can change descriptions without advancing catalog
generation. At design review, trace that path and decide whether it merits the
same small change signal or a documented advisory freshness limitation. Do not
change price authority, search cursors or catalog generation merely to drive
notifications, and do not claim all price-only changes are pushed without evidence.

## Ordered implementation boundaries and review gates

All steps below remain unstarted. Each implementation commit includes its focused
tests and owning documentation. Follow exact-SHA and fixup review requirements;
do not commit a failing intermediate state as a completed step.

### G1: independent design review before implementation

A fresh read-only reviewer inspects G0 evidence, current pins, publication and
pricing paths, listener projection, cancellation and shutdown consumers. Resolve
notification granularity, subscribe/list races, native SDK ownership, initial
capability advertisement and consumer negotiation before dependent work. Challenge
extra state, options, enforcement and unsupported compatibility claims. Reconcile
[discovery improvements](../discovery_improvements.md), whose proposed direct
recommendation advertisement can change `tools/list`, while retaining its separate
scope and manual instruction policy. Do not assume that proposal is implemented.

### P1: native subscriptions with catalog invalidation

Implement the complete usable capability together: server hooks, publication
signal, listener-scoped invalidation and subscription cleanup. Trace standalone
HTTP, deployment HTTP and stdio consumers; preserve existing stateless request
behavior and define any supported stdio subscription behavior explicitly.
Update architecture, configuration and agent-source guidance in the same commit.

Acceptance: focused fixtures demonstrate acknowledgment/filtering, idle delivery,
current tools after invalidation, race handling, isolation, cancellation and drain,
including forbidden financial side effects. Complete applicable CogC/CycC review
and selected compiler/compatibility checks before broad runtime qualification.

### P2: real-agent qualification and operational handoff

Add a reproducible unfunded harness for the activation consumer, including the
Hermes cases below when claiming Hermes support. Keep development-only client
dependencies outside runtime requirements; use pinned/source-identified clients
and disposable profiles. Update testing guidance with reproducible commands,
evidence locations, actual supported consumer versions and remaining limits.
Keep README changes limited to useful operator guidance and links. Existing
AGENTS.md links to architecture, configuration, agent sources and testing should
remain the owning entrypoints; add a link only for an enduring rule that lacks one.

### G2: final combined implementation review and correction loop

A fresh independent reviewer covers P1/P2, all exact commits and fixups, complete
affected call chains, required complexity evidence and qualification results.
Supply the baseline, source/build identities, findings and evidence paths. Reviewers
inspect recorded results and source rather than repeating passed suites absent a
specific suspected defect. Resolve material findings and selected simplifications;
use `git commit --fixup=<introducing-SHA>`, independently review exact fixups and
repeat affected checks/gates only when evidence is invalidated. Preserve unsquashed
history through integration and user review under the planning workflow.

## Focused qualification and Hermes acceptance

Use existing shared fixtures and the testing guide's selection/order rules, not a
new full validation matrix. Record commands, actual test counts, source/executable
identities and private report paths under ignored `target/`. Exercise affected
native behavior on Linux/macOS; unavailable platforms remain missing coverage.

- Protocol: modern discovery and metadata; existing clients can still list/call
  tools without subscribing; acknowledgment precedes events; only requested and
  advertised categories are honored. No legacy session header or GET channel is
  necessary. Authentication and Host/Origin rejection remain enforced.
- Catalogs: idle addition, removal, same-name schema/description replacement;
  deferred static publication and dynamic registration/refresh/removal/restore;
  failed or rejected publication leaves the old inventory usable. Exercise
  listener filtering and `send_tools`, multiple subscribers, on-demand wrappers
  and unrelated publications without cross-listener exposure. Cover the selected
  pricing-only behavior and any activated recommendation advertisement.
- Races/recovery: publication around subscription acknowledgment and initial list,
  change during an active call, bursts, slow/disconnected consumers, abrupt stream
  loss, graceful close, client restart, server restart and shutdown with an idle
  subscriber. After reconnect/list, the inventory converges to current state
  without blocking catalog publication or leaking subscription tasks.
- Financial boundaries: hold accepted fixture work while closing its response or
  subscription and while changing catalogs. Verify it completes under its captured
  binding, with no duplicate dispatch/signing, new funding or authority extension.
  A subscriber has no authority beyond the authenticated listener. Reuse unfunded
  payment/ownership fixtures; this plan grants no live-funded qualification.
- Hermes specifically: use an actual source-identified Hermes client integration,
  not only the Python SDK. Observe the modern negotiated revision and outbound
  `subscriptions/listen` filter. Assert an idle change causes a fresh `tools/list`
  and updates Hermes's registry, including replacement and removal. Exercise
  supported direct exposure and Tool Search paths, and check the next agent turn
  sees the changed tool schema without a manual reload or a paid model request.
  Distinguish registry updates from any native limitations for already-running
  turns or frozen editor manifests. Test re-listen plus refetch after stream loss
  and treasury restart, concurrent calls, and normal client shutdown. Where the
  pinned client cannot do this, record the exact missing client behavior and keep
  Hermes compatibility unqualified; do not compensate with legacy sessions.

Controlled fixtures do not qualify live settlement, model selection quality,
public ingress, Tor privacy or arbitrary client versions. Wrapper Docker/Podman
qualification belongs to Private Hermes if its deployment behavior changes; it is
not established by x402_treazury library or local-client tests.

## Completion, explicit deferrals and evidence status

Out of scope: legacy session support, persistent subscription/event storage,
notification replay, generic orchestration, automatic payments/retries, new spend
budgets, Hermes cost-table synchronization, prompts/resources subscriptions and
initialization-instruction hot reload. No new service or dependency fork is planned.

G0, G1, P1, P2 and G2 remain pending. No runtime qualification or implementation
review has passed. Keep this deferred plan while the consumer gate is unmet.
Once activated, retain intended steps and concise review/evidence dispositions
through the normal correction loop. After accepted implementation and user review,
move enduring rules into the owning guides, preserve genuinely unfinished work in
focused deferred plans, remove this plan, repair links and commit cleanup. Obtain
independent review of the exact cleanup SHA and final combined branch. Cleanup
does not erase private evidence or authorize pushing, merging or funded operations.
