# Dynamic registry efficiency (R2)

## Intent, baseline and preserved contracts

Remove repeated catalog construction and quadratic lookup work without weakening
atomic publication, persistence or endpoint ownership. Baseline
`1bd810eb`; original review baseline `3ceb88f2e41038c4f6ef6959494c2f501373f1cd`.
This plan authorizes no paid relay discovery or registration by itself.
Read [agent source authority/publication](../agent-sources.md),
[catalog handoff](../architecture.md#catalog-configuration-and-execution),
[source distribution](../configuration.md#source-tool-distribution),
[resource policy](../local-resource-limits.md), [testing](../testing.md) and
[discovery improvements](discovery_improvements.md). Coordinate shared snapshot
work with that plan and [catalog safety](catalog_safety_and_schema.md).
Preserve listener principal ownership, original accepted document/payment context,
generation-bound search, stable names, persistence-before-publication, crash/restart
recovery and help-cache compatibility. Registration allocates/funds no wallet.

The refreshed baseline includes `main` at `a626b998`: randomized nominal reserve
ranges and saved unsigned choices, removal of configurable per-transfer ZEC caps,
and qualification's running USDC API budget. Preserve current fee and accepted
operation authority; do not restore removed ZEC budgets. Historical qualification
registries remain read-only under the version-2 authority contract.

## Current costs and proposal

`src/discovery/mod.rs` rebuilds live records in `snapshot`; validation and building
repeat during registration and publication. Collision/help lookup uses linear
searches, and async readers share `inner`'s std mutex with construction/persistence.
Mutation commits already use `spawn_blocking`, a mutation permit and a 128 MiB
document guard: the review's uncapped-registry/async-build wording is incomplete.
Do not add registration caps, queues or new timeouts. Query-string identity is an
existing registration contract, not automatic permission to normalize identities.

Prefer retaining immutable built records for unchanged document/revision and
relevant selection/configuration inputs, plus local maps/sets for collision and
help reuse. Inventory all invalidation inputs before picking a cache key: source
revision alone cannot capture listener policy, base generation or effective bindings.
Avoid a second durable index or incremental transaction framework. Publish immutable
read views so ordinary async lookups do not wait on long construction/storage work;
keep the smallest serialized mutation boundary needed for durable atomic results.
Skip tag matching when there are no tag filters. Adjacent R9 regex/client/status
work is outside scope unless a measured dependency makes it necessary.
The merged qualification API-budget check adds both run and all-registry history
scans of payment JSON inside the serialized admission transaction. This worsens
R9's historical hot path for qualification; it is a separate registry and is not
fixed by caching source catalogs here. Assess it as a focused follow-up, preserving
atomic budget admission and durable uncertain charges; make no latency claim
without evidence and do not add parallel monetary accounting by default. R5 remote
probe policy stays advisory and retains separately accepted payment authority.

## Ordered commits and gates

1. **D1 — Remove repeated pure work and quadratic lookup.** Inventory registration,
   restore, update/remove and base-catalog rebuild callers. Reuse unchanged built
   results with complete invalidation; use snapshot-local lookup maps/sets, skip
   unfiltered tag traversal. Tests assert build reuse/invalidation and identical
   selections, names, help compatibility and failure rollback, using synthetic inputs.
   Do not add universal provider snapshots. Update `docs/architecture.md` and
   `docs/agent-sources.md` with actual ownership/invalidation rules.
2. **D2 — Separate read publication from serialized mutation.** Move async reads
   onto immutable published metadata or the smallest safe equivalent, retaining
   durable mutation ordering and one generation boundary for catalog/instructions/
   search. Cover racing duplicate registration, update/remove, reload, persistence
   failure, cancellation and crash after commit. Test readers continue while a
   mutation is deliberately held at a deterministic barrier and restart restores
   the same result. Update source/architecture guides and affected testing guidance.
   Measure representative synthetic scaling only if making quantitative claims;
   no new benchmark framework is required for functional build-count evidence.

**G1 after D1:** independent cache/invalidation review across all prior commits and
fixups, including base routing, source selection and restored records. Exit with
no stale authority or stale tools and demonstrably removed repeated work.
**G2 after D2:** combined concurrency/integration review of persistence, publication,
search/help and failure paths. Exit with no partial generation, orphaned accepted
work, identity change or async blocking on the expensive mutation section. Revisit
simpler representations before accepting new state synchronization machinery.

## Review, validation and handoff

Before substantive implementation, capture the current exact baseline and have an
independent read-only reviewer challenge the design, necessity of checks and
opportunities to remove responsibilities. Resolve material findings before dependent
work. The baseline above describes the proposal, not completed validation.

For each ordered commit, freeze sources, run affected cheap contract tests, then
applicable production compiler/all-target Clippy and compatibility checks before
broader runtime suites, following [testing](../testing.md#validation-order-and-continuation).
Use pinned Make entrypoints and public unfunded keys; record actual nonzero tests,
commands, source identities and limitations under ignored `target/`. Serialize
builds. Offline fixtures establish no live-funded or Tor qualification.
Independently review every exact committed SHA and its consumers; the reviewer
must not author it. Fix findings with `git commit --fixup=<introducing-SHA>`,
independently review each fixup, and retain unsquashed history through integration.

Each named gate examines all preceding plan commits and fixups alongside complete
call chains, tests and documentation. Resolve material findings before exit;
examine fixup causes, recurring assumptions and missing coverage, and decide
whether they require regression tests, a wider audit, simplification or a design
change. Earlier approvals do not exempt code; later material changes reopen gates.
For substantive Rust refactors and architecture gates compare CogC/CycC with the
confirmed baseline using [complexity discipline](../../tests/COMPLEXITY.md#review-discipline).
Review warnings, changed chains and unchanged high-risk hotspots; assess simpler
ownership/control flow rather than extract helpers solely to lower scores.

Update the named permanent guides with each implementation commit. Keep the
applicable [AGENTS.md](../../AGENTS.md) links pointing to those guides; add an
owning-guide link there if a new enduring rule needs one. Before removing this
completed plan, verify guide/reference handoff, remove its documentation-index
link, and independently review both the exact cleanup SHA and the final combined
branch after removal. Keep concrete unfinished work, no completed-plan archive;
documentation cleanup never deletes financial evidence or grants replay authority.
