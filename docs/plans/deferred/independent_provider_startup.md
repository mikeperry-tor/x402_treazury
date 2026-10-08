# Independent provider startup

Deferred until partial provider availability is a priority. The bounded eager
startup optimizations are implemented; this plan changes serving lifecycle and
agent-visible inventory behavior. The [live integration runner](../../../tests/live/INTEGRATION.md)
already supports provider testing; partial startup availability remains separate work.

## Objective

Make multi-listener startup independent of remote provider availability. An MCP
listener should serve its available tools while other sources are loading or
unavailable. Slow catalogs and pricing probes must not delay unrelated providers,
listeners, wallet reconciliation, or paid calls. A missing catalog must be visible
to both the operator and the agent; it must never look like a deliberately empty
inventory.

This work is separate from transport timeouts and payment admission. Tor uses a
120-second connection budget; both transports default to a renewable 60-second
HTTP read-inactivity budget without a total response deadline.
Managed admission releases its pool gate after durable authorization journaling,
before awaiting the signed HTTP response. Preserve those boundaries.

## Current architecture

`Deployment::load` in `src/deployment.rs` validates the deployment, installs network
policy, and loads sources through `src/deployment/startup.rs` with a rolling
concurrency limit (default 2, configurable 1..64). Every source must succeed
before listener selection and binding. A remote spec failure aborts startup.
The first observed load failure cancels pending futures; inventory ordering is
still deterministic. Phase timings, ready counts and ten-second waiting updates
go to stderr. Compatible remote aliases already share downloads and parsed documents within
a load; keys include exact URL, requested timeout, byte limit, transport policy and discovery
identity. Local files and separate loads remain independent. Background publication
is not implemented.
Serving also waits for configured startup pricing discovery, with rolling work
across up to 16 sources and a shared 16-request cap. Each source retains its
configured probe limit (default four); endpoint slots also roll without batch
barriers. Local specs avoid catalog fetches; the live qualification driver's
`freeze-catalogs` command is a reviewed workaround, not a general production cache.

Empty discovered-price maps reuse original tools; Kronos disables its slow
startup probes by default. Preserve configured per-source limits and cache
semantics. The [architecture guide](../../architecture.md) documents implemented
startup behavior. Historical measurements, the provisional two-slot Conflux
hypothesis and further performance qualification live in the
[startup testing guide](../../../tests/STARTUP.md); they are not prerequisites for
this availability feature.

`src/catalog_state.rs` already supplies immutable snapshots shared across
listeners. `src/discovery/` uses those snapshots for agent-added sources, but
static bindings, persistent dynamic registrations, permissions and wallet
assignment have different ownership rules. HTTP MCP is stateless and does not
advertise catalog-change notifications. Clients can retain an earlier tool list.

## Configuration and failure policy

Extend the existing strict deployment-only `[startup]` table with the proposed
`provider_loading` policy. Only `catalog_concurrency` is implemented today:

```toml
[startup]
provider_loading = "background" # default; "strict" retains eager fail-fast serving
catalog_concurrency = 2         # implemented; validate 1..64
```

`config check`, `catalog tools`, and `catalog tags` remain exhaustive, credential-free
inspection: await catalogs with bounded concurrency and fail visibly if any cannot
be validated. `config show` remains entirely offline. Standalone single-source
serving retains eager loading initially. Direct and Tor deployments use the same
startup machinery.

Invalid TOML, invalid provider composition, unknown wallet/source references,
unsafe registry paths, invalid binding policies and listener bind failures are
fatal. A remote fetch error or invalid remote catalog makes that source unavailable;
it does not terminate healthy listeners. Invalid local catalogs are fatal: parse
and validate those before serving. Distinguish these outcomes in tests and logs.

Unknown tool/tag selectors cannot be classified until the relevant catalog loads.
Validate syntax immediately; retain selectors for pending sources. Apply existing
selector diagnostics once each catalog is available. Never interpret an unavailable
source as an empty catalog to silently satisfy or reject its selectors.

## Source state and publication

Represent every declared source with stable source ID, configuration, resolved
wallet bindings and a runtime state: `loading`, `ready`, or `failed`. Record phase
(`catalog_fetch`, `catalog_parse`, `tool_generation`, `selection`, `pricing`),
attempt number, last transition time, effective deadline and a sanitized error.
Pricing is auxiliary: a pricing failure does not make otherwise valid tools fail.
Keep operator-authored configuration errors distinct from untrusted remote input.

Resolve wallet assignments once from declarations before loading providers. A
source becoming ready must not allocate a new wallet, change sharing scope, or
trigger additional bootstrap funding. Existing managed startup and auto-funding
policy still govern declared pools. Never defer treasury integrity checks until a
paid request. All listeners must bind successfully before any begin serving or
funding starts; binding rollback remains atomic.

Load remote catalogs through a bounded worker queue. Coalesce equal resolved spec
URLs with equal fetch policies within the process, but apply each alias's own
filters, prefix, base URL, help, wallet binding and overrides independently.
Never hold catalog publication locks, registry mutation locks or wallet admission
gates across a download or parsing task. Bound response bytes and reference
expansion using existing checks and surface every exceeded limit explicitly.

Build a complete candidate source off-lock, then publish all affected listener
views as one immutable generation. Serialize publication against dynamic source
registration/removal so a late static load cannot overwrite a concurrent agent
addition. Existing in-flight calls retain their captured snapshot and payer.
Selection and invocation must enforce identical filters and wallet bindings.

On cross-source tool-name collision, retain already published tools, mark the
incoming source failed with a selection error, and explain the conflicting IDs.
Never overwrite an existing route. Reserve management/status names before source
loading. Strict inspection must report the same collision as an error regardless
of completion order; background mode's first successful publication owns the name
until restart. Document that invalid colliding deployments have order-dependent
availability and should be corrected, rather than treating this as priority routing.

## Agent and operator visibility

Expose a read-only `x402_treazury_source_status` tool on background-loading listeners,
independent of agent source-management grants. It reports only sources declared
for that listener plus dynamic sources already visible under existing permissions.
Use source IDs, state, phase and sanitized errors; omit sensitive spec query
strings, credentials, wallet addresses, other listeners and hidden source IDs.
Status is not an authorization to add, refresh or remove sources.

MCP initialization instructions explain that sources may still be loading, name
the status tool, and tell clients to refresh `tools/list` after readiness changes.
Keep stateless transport semantics; do not advertise unsupported list-change
notifications. An invocation of a known unavailable source's previously published
tool returns a clear unavailable/loading error. Unknown tool names retain normal
not-found behavior. Never guess source ownership from an arbitrary tool prefix.

Logs identify source ID, phase, effective timeout, state transition and readiness
counts without secrets. Announce listener availability separately from provider
readiness and wallet readiness. Warn explicitly when a listener exposes only the
status tool. Status output uses existing visible pagination/limit conventions;
never truncate the list silently.

## Pricing, help and retries

Publish generated tools before optional pricing discovery completes. Run pricing
as bounded background work with the existing process-wide cache, success/failure
retention, GET-only eligibility and no refresh on TTL expiry. A slow probe may
occupy a probe slot, but cannot hold up catalog downloads or tool invocation.
Rebuild descriptions with authored overrides last and publish a new generation
without altering routes, schemas or wallets. Prevent a late pricing result from
resurrecting a replaced/removed source by checking source revision at publication.

Help stays lazy and cached on success; concurrent requests for the same help may
coalesce. A slow help request must not block unrelated help, catalogs or API calls.
Do not fetch help at startup.

Initially attempt each remote catalog once per process. Failed sources remain
visible until restart; no automatic polling, silent snapshot fallback, or new
agent refresh capability. A future operator-authorized refresh can reuse the
same state machine, with explicit retries and stale-catalog policy. Never retry
paid requests as part of provider recovery.

## Shutdown and qualification

Own loader/probe tasks in the deployment lifecycle. On shutdown stop scheduling,
cancel unsigned downloads, join workers and prevent later snapshot publication.
Continue the existing announced drain for signed payments and treasury work;
catalog failures never cancel unrelated paid calls. A failed listener still stops
siblings according to existing lifecycle rules.

Keep funded qualification on frozen catalogs until background startup has been
qualified separately. Its immutable catalog/schema hashes and experiment ledger
must not be bypassed to accommodate a source arriving late. Preparation must await
every reviewed case's schema or report why it cannot be prepared. Do not reset an
existing ledger to change pinned catalogs, budgets or timeouts.

## Implementation milestones

Each milestone should be a separately tested commit.

1. **State and policy:** extend startup configuration with loading policy, source-state types and
   pure validation. Test unknown fields, bounds, binding resolution without catalogs,
   eager inspection behavior, and no secrets/state access in `config show`.
2. **Atomic publication:** share an explicit publication coordinator with dynamic
   catalog updates. Test forced overlaps among static completion, pricing completion,
   registration and removal, including same-name conflicts and revision checks.
3. **Background loader and lifecycle:** bind listeners, launch bounded/coalesced
   remote loads and publish completed sources. Test a permanently stalled provider
   alongside ready sources across multiple listeners; prove healthy tools are callable
   before releasing the stalled fixture. Cover local failures, remote malformed
   specs, shared aliases, bind rollback and cancellation without orphan tasks.
4. **Status and MCP behavior:** implement restricted source status and initialization
   guidance. Test authenticated real HTTP `initialize`, `tools/list`, status and calls
   before/after publication; stale inventories, filters, wallet scope and absent
   source-management grants must never widen access. Verify logs and agent errors.
5. **Background pricing and documentation:** publish tools before probes, preserve
   cache/override semantics and update README/AGENTS. Test slow/failing probes and
   help alongside successful paid fixture calls, pricing generations and shutdown.

Run targeted offline suites for each milestone, then the default Zcash full suite,
Clippy, formatting and `network_audit`. Add authenticated SOCKS fixture coverage for
slow remote-DNS catalog streams and prove continued use of source discovery
identities, payer identities and no direct fallback. Real Tor qualification is
unsigned and optional; use local deterministic barriers for concurrency assertions,
not vendor availability or funded transactions. Record the remaining client-side
limitation that some MCP clients do not automatically refresh tool inventories.
