# Shared catalog and price-hint caching

## User intent and implementation baseline

Implement a simple, best-effort cache shared by provider catalogs and their
per-operation price estimates. An imperfect shared estimate is preferable to
discarding it or showing no estimate. Optimize reuse and remove responsibilities.

The user explicitly rejected additional price-cache partitions for wallet groups,
listeners, payers, runtime store instances, API destination overrides and
authentication differences. Use the catalog cache's existing identity and sharing
rules for both datasets. Do not reintroduce those partitions as safety checks,
configuration fingerprints, private-price exceptions or fallback eligibility rules.
Method and original path template identify an operation within a catalog; they do
not introduce a separate provider-cache boundary.

**Do not pollute pricing hints with extra provisos, freshness warnings, limitations,
timestamps, provenance labels or ancillary explanatory wording.** Preserve the
current concise price presentation and existing fallback behavior. Do not add
agent-facing caveats, notices, notifications, new output blocks, help text or a
pricing-documentation cleanup as ancillary work. Internal cache housekeeping is
not a reason to expand what an agent sees. Keep maintainer contracts in owning
guides, with no repeated warning boilerplate.

This is a plan for implementation in a fresh conversation; creating it does not
implement the feature or authorize live paid calls. Starting application baseline:
`487b2f75b23de27a702949162d986e26225c4aaa` on `provider_updates`. Inspect intervening
work before implementation. Preserve unrelated local files, including the untracked
`state` entry; do not read real wallet secrets or qualification evidence.

Read [AGENTS.md](../../AGENTS.md), [configuration](../configuration.md#pricing-metadata-and-runtime-hints),
[catalog disk caching](../configuration.md#automatic-discovery-disk-cache),
[architecture](../architecture.md#catalog-configuration-and-execution),
[wallet selection](../architecture.md#wallet-selection-and-network-isolation),
[resource limits](../local-resource-limits.md),
[timer inventory](../network-egress.md#remaining-timers-and-budgets),
[best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[wallet observation](../wallet-rotation.md#wallet-observation-policy),
[reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy),
[testing](../testing.md#validation-order-and-continuation),
[complexity review](../../tests/COMPLEXITY.md#review-discipline) and
[documentation policy](../README.md) before changing their consumers.

## Current implementation and proposed behavior

At the baseline:

- `src/http_cache.rs` stores raw remote catalog responses in an existing treasury
  state's `http-cache/discovery-v1.sqlite`. Identity includes the catalog URL,
  network/transport and existing cache settings, with existing relay/direct-warm
  distinctions. Listener and wallet-group membership do not partition catalogs.
- `src/deployment/startup.rs` shares compatible catalog acquisitions; selection,
  overrides and listener tool distribution remain separate consumers.
- `src/pricing/runtime.rs` holds process-only hints with a 24-hour receipt lifetime.
  `src/payment.rs::operation_key` adds source/revision, wallet group and runtime
  store identity. `src/catalog_state.rs` reads hints for descriptions; managed
  payment selection reads them to rank existing wallets.
- Catalog retention defaults to 24 hours, configurable through
  `catalog_cache_ttl_seconds`. Disk expiry affects the next catalog load, not an
  automatic background reload of published tools. Actual calls obtain and validate
  their own payment offers independently of cached estimates.

Replace these separate cache policies with the following behavior:

1. **One cache identity.** Attach per-operation estimates to the existing catalog
   cache entry. Share them wherever that catalog entry is shared, including across
   research-desk endpoints and wallet groups. Reuse existing transport/relay cache
   distinctions; add no pricing-specific identity dimensions. Remove obsolete
   hint-only identity plumbing and partition tests rather than retaining a second
   compatibility layer.
2. **One seven-day lifetime.** Default the shared catalog/hint retention to
   `604800` seconds. Reuse `catalog_cache_ttl_seconds` for both; retain its existing
   override/disable meaning. Price observations replace the stored amount without
   extending catalog expiry. Ordinary reads and restart do not renew the entry.
   A successful existing catalog revalidation, including an unchanged HTTP 304,
   renews the joint entry and retains its prices. Do not keep an independent
   price-age gate or invent a separate pricing TTL setting.
3. **One invalidation lifecycle.** Provider/source configuration updates and
   replacement with changed catalog content invalidate the shared catalog and
   observations together. Ordinary unchanged loads, alias selection and listener
   publication must not clear prices. Detect configuration updates through the
   owning load/refresh lifecycle, including across restart, using the smallest
   necessary revision bookkeeping; it is not another lookup-key partition. Avoid
   alias differences causing repeated invalidation of a shared entry.
4. **Best-effort storage.** Keep exact raw catalog bytes intact. Store amounts and
   operation identifiers alongside them using the existing SQLite cache owner.
   Learn in memory immediately; persist outside the payment/response critical path.
   Failed writes, eviction, cancellation or incompatible old disposable cache data
   may lose estimates, but must not fail calls or create recovery obligations.
   Preserve existing cache eligibility, opt-outs, qualification bypass and handling
   when no existing state directory is available. Do not make local catalogs or
   dynamic imports newly disk-cacheable solely to persist their prices. Calls
   without a catalog cache entry retain their existing per-call price result.
5. **Minimal ordering protection.** Preserve latest-received observation ordering
   within an entry. A delayed older write cannot replace a newer observation or
   resurrect an expired, invalidated or replaced catalog entry. Reuse entry identity
   and a small generation check where necessary; do not add tombstone histories,
   ledgers, durable job queues, flush barriers or new financial ownership state.
6. **Useful error invalidation.** A concrete operation-removed/schema-changed signal
   should invalidate the affected catalog and hints for normal metadata refresh.
   Establish that signal from actual existing response/error consumers before
   building the hook. A generic 404, 400 or 422 is not sufficient evidence by itself.
   Coalesce through existing catalog acquisition/publication machinery; do not add
   provider dispatch, a speculative error framework or periodic probing. Preserve
   the original call outcome; metadata refresh never replays the API call.

Keep payment challenge validation, caps, journal-before-disclosure, accepted-work
ownership, wallet selection fallbacks and payer-bound connection/Tor isolation
unchanged. Sharing an estimate does not share a signer or connection. Keep the
per-call `x402_pricing` result tied to that call's validated offer; another call's
cached value must not replace it. Do not add price probes, wallet allocations,
funding waits, new cache caps or readiness gates. An expired cache does not create
a new background catalog scheduler; published tools retain their existing behavior.

## Ordered commits and independent review

Before substantive implementation, a read-only independent reviewer must challenge
the design against the user intent above. Resolve material findings before building
on them, particularly attempts to restore rejected partitions, independent price
aging or agent-facing warnings. The coordinator owns edits and builds; reviewers
stay read-only. Establish source-matched CogC/CycC at the application baseline.

### PH1 — Complete shared persistence and lifecycle

Deliver the coherent replacement in one implementation commit: common identity,
seven-day default, SQLite persistence/restore, synchronous in-memory learning,
configuration/content invalidation and minimal late-write protection. Include the
catalog acquisition, `BoundTool`, managed hint-selection and fresh-description
consumers together. Do not remove current hint behavior in one commit and defer
its necessary replacement or invalidation integration to another.

Primary implementation: `src/http_cache.rs`, `src/catalog.rs`,
`src/pricing/runtime.rs`, `src/payment.rs`, `src/catalog_state.rs`,
`src/deployment/startup.rs` and relevant discovery/registration consumers.
Remove `hint_group_scope` or other obsolete plumbing only where this change makes
it unnecessary; preserve unrelated store/financial evidence identities.

Include focused synthetic tests for restart restore; seven-day expiry and disabled
disk caching; sharing across groups/listeners and compatible aliases; existing
network cache distinctions; unchanged loads; configuration changes across restart;
changed catalog replacement; HTTP 304 joint renewal; price writes not renewing
expiry; newer observations superseding older ones; delayed writes after
invalidation/eviction; cache I/O failure; and original per-call pricing results.
Assert that these operations neither sign/pay/probe/fund nor add calls or authority.
Use current cache/hint/payment test helpers and small inputs, not vendor snapshots
or a new test framework. Assert the existing concise price rendering is unchanged.

Update permanent contracts with PH1: configuration's pricing and automatic-cache
sections, architecture's cache/selection/description sections, both resource/timer
inventories, and affected test ownership notes. Replace contradictory current
24-hour/process-only/group-scoped statements in place. Keep presentation changes
out of scope; no extra price provisos belong in these edits.

**Gate A after PH1:** independently review the exact commit and all its fixups,
alongside complete catalog read/revalidation → binding → hint learn/restore →
description/selection → actual challenge → journal/signing call chains. Compare
CogC/CycC and inspect unchanged relevant hotspots. Check that shared aliases do
not cause misses, stale writes cannot revive invalidated entries, and cache failures
remain disposable. Resolve material findings before PH2 builds on this lifecycle.

### PH2 — Evidence-based contract-error refresh

Identify a real producer of an operation-removed/schema-changed signal in current
API response/error handling. Implement the smallest hook into PH1 invalidation and
existing metadata acquisition, covering static and dynamic consumers that already
have a corresponding catalog. Keep unrelated sources available and preserve
accepted financial work. Include a faithful small positive signal and negative
resource-not-found/bad-argument cases; assert no API replay, duplicate active refresh,
payment, funding or extra pricing text. Update the owning architecture,
configuration and test guide with the implemented trigger and refresh behavior.

Resolve the signal question in design review before claiming PH2 implementation.
If no reliable generic producer exists, keep this as focused unfinished work;
deliver useful PH1 behavior without inventing provider-specific classifiers or
an unused signal abstraction. Do not report error-triggered refresh as implemented
merely because an invalidation function exists.

**Final integration gate after PH2, or after recording its unresolved trigger:**
independently review all plan commits and fixups with the full call chains above,
including source refresh, aliases, dynamic publication, relay cache acquisition and
`research_desk.toml` sharing. Inspect fixup causes and repeated assumptions; decide
whether findings require a wider audit, regression, simplification or design change.
Earlier per-commit approvals do not exempt code from this combined review. Recheck
complexity against the agreed baseline and verify the no-extra-wording requirement
on actual MCP definitions/search/results. Later material changes reopen affected
gate concerns and receive appropriate revalidation.

## Validation, commits and handoff

Review every exact committed SHA independently, including fixes made with
`git commit --fixup=<introducing-SHA>`. Keep unsquashed history through integration.
Give reviewers user intent, baseline, changed consumers, source-matched metrics and
actual validation counts; ask them to challenge unnecessary checks and cache misses.

Run affected cheap cache/hint/description/selection and deployment contracts first.
Then complete `make check-static COMPLEXITY_BASE_REF=<implementation-baseline>`,
independent complexity review, `make check-clippy` and `make check-compat` before
`make check-default-remaining check-proving check-lifecycle check-codec`. Use the
pinned Make toolchain/protoc and appropriate socket-capable execution up front.
Freeze source/executables during validation, run feature configurations sequentially,
reuse still-valid evidence and resume unfinished named stages after focused fixes.
Keep generated reports under ignored `target/`; no live funded qualification is
required or authorized by this plan.

Maintain enduring rules in the guides named with PH1/PH2. Existing AGENTS.md links
to configuration, architecture, network/resource inventories, wallet rules and
testing are their handoff; verify those anchors and add a direct owning-guide link
there only if an enduring rule otherwise becomes unreachable. Update the
documentation index when this plan's status changes. After implementation, verify
that handoff before removing the completed plan without an archive. If PH2 remains
unfinished, retain only that focused work instead of a completed PH1 narrative.
Independently review the exact cleanup SHA and final combined branch after cleanup
before declaring completion.
