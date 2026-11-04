# Treasury proving execution (A4)

## Intent, baseline and preserved contracts

Keep CPU-heavy transaction calculation off serving runtime workers and avoid
reparsing public Sapling parameters unnecessarily. Baseline
`1bd810eb`; original review baseline `3ceb88f2e41038c4f6ef6959494c2f501373f1cd`.
This plan does not authorize live proofs involving real wallet keys or funded work.
Read [wallet ownership/recovery](../wallet-rotation.md),
[best-effort rules](../wallet-rotation.md#best-effort-wallet-rules),
[observations](../wallet-rotation.md#wallet-observation-policy),
[SDK calculation](../zingo-integration.md), [runtime architecture](../architecture.md),
[logging](../development.md#logging-and-diagnostics) and
[lifecycle coverage](../../tests/LIFECYCLE.md).
Preserve the serialized treasury owner, proposal/input exclusion, acceptance before
calculation, journal before disclosure, exact calculated bytes and restart recovery.
Waiter cancellation or a dropped JoinHandle cannot discard accepted financial work.
No additional proving queue, concurrency cap, whole-work deadline or replay path.

The refreshed baseline includes `main` at `a626b998`: randomized nominal reserve
ranges and saved unsigned choices, removal of configurable per-transfer ZEC caps,
and qualification's running USDC API budget. Preserve current fee and accepted
operation authority; do not restore removed ZEC budgets. Historical qualification
registries remain read-only under the version-2 authority contract.

## Current behavior and proposal

Deposit `send.rs` and shielding `refunds.rs` synchronously call backend calculation
while holding the SDK wallet write lock on the async runtime. Both now use embedded
`LazySaplingProver`, so the review's disk-load/missing-file claim is obsolete. Each
operation constructs a fresh adapter with its own OnceLock; reuse can still avoid
repeated Sapling parsing. Existing owned-transparent diagnostics must be inspected
before adding logs; healthy per-address negative results are not terminal errors.

Move synchronous calculation to a blocking execution boundary under the existing
accepted-work owner. Determine Send/lock requirements and choose the smallest
ownership transfer compatible with the SDK; do not block_on an async owner or
borrow a guard into a detached job. Hold mutation exclusion as required for SDK
correctness, while unrelated serving can progress. Retain and join accepted work,
publish exact results and clean proposal state on success/error; panic/failed joins
must preserve recoverable intent and must not cause automatic duplicate calculation.
Share immutable parsed public parameters if the SDK prover is safely reusable;
prove thread-safety from actual types, keep parsing lazy, and do not cache secrets.

## Ordered commits and gates

1. **T1 — Establish shared lazy public parameter reuse.** Inspect SDK prover trait
   bounds and both calculation callers. Share the existing embedded parser through
   a process-owned immutable provider if supported; retain absence of parsing when
   no Sapling proof is requested. Exercise two relevant calls, lazy initialization,
   concurrent first use, mixed transparent/Sapling/Orchard paths and calculation failure without reading `~/.zcash-params`.
   Update `docs/zingo-integration.md` and proving-test guidance with actual scope.
2. **T2 — Offload both deposit and shielding calculation with owned completion.**
   Introduce the smallest common blocking boundary needed by both callers, preserving
   original intent, wallet exclusion and result journaling. Carry the accepted exact
   source principal and actual network-fee authority into the blocking owner; do not
   reconstruct it from a current random draw or removed `max_funding_spend_zec`.
   Test fixed/ranged accepted quotes and changed configuration without changing
   their original bytes or calculation authority. Cover cancellation after
   acceptance, panic/error, shutdown drain, restart after interrupted publication and
   another MCP operation making progress during a deliberately held calculation.
   Assert no second calculation/disclosure and no lost input owner. Use deterministic
   synchronization and existing public synthetic proving/lifecycle fixtures. Update
   `docs/wallet-rotation.md`, `docs/architecture.md`, `docs/zingo-integration.md` and
   `tests/LIFECYCLE.md`; include focused `check-proving` and `check-lifecycle` evidence.

**G1 after T1:** independent prover-lifetime/thread-safety review of both callers
and all fixups. Exit with lazy public-only reuse and unchanged transaction semantics.
**G2 after T2:** combined ownership/concurrency gate across treasury actor,
calculation, proposal cleanup, Store publication, shutdown and recovery. Review
all commits/fixups and unchanged high-risk functions such as `prepare_with_observation`
and `shield_address`. Exit with runtime progress and no accepted-work abandonment,
duplicate I/O or financial authority changes; retain wallet locking where required.

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
