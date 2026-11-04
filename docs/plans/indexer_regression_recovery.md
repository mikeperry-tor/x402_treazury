# Indexer regression and chain recovery (A2/B2)

## Intent, baseline and preserved contracts

Keep a stale indexer tip from destructively resetting valid SDK observations while
retaining automatic recovery for genuine chain replacement. Baseline:
`1bd810eb`; the original review examined `3ceb88f2e41038c4f6ef6959494c2f501373f1cd`.
This is unfinished design/implementation work, not live qualification authority.

Read [SDK chain recovery](../zingo-integration.md),
[wallet reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy),
[wallet observations](../wallet-rotation.md#wallet-observation-policy),
[network policy](../network-egress.md), [vendor provenance](../../vendor/README.md),
[vendor retirement](vendor_retirement.md) and [lifecycle tests](../../tests/LIFECYCLE.md).
Preserve keys, addresses, declared birthdays, accepted inputs, original signed
bytes and journals. SDK state owns derived spendability. Rescan grants no funding,
release or submission authority; completed source work does not reopen.

The refreshed baseline includes `main` at `a626b998`: randomized nominal reserve
ranges and saved unsigned choices, removal of configurable per-transfer ZEC caps,
and qualification's running USDC API budget. Preserve current fee and accepted
operation authority; do not restore removed ZEC budgets. Historical qualification
registries remain read-only under the version-2 authority contract.

## Current gap and design decision

`vendor/pepper-sync/src/sync.rs::checked_wallet_height` clears derived data when
birthday exceeds the reported tip, or the tip regresses beyond the retained
100-block horizon. `scan_birthday` and `clear_wallet_data` preserve an earlier
effective scan floor across restart. A stale unsigned height is not evidence of a
fork. But blanket rejection of all regressions would strand genuine deep reorgs;
clamping a proven replacement to the old birthday can miss owned credits.

First establish the minimal evidence available through existing SDK block/hash
and common-ancestor recovery. Prefer retaining observations and reporting a
recoverable sync failure when a regressed endpoint cannot establish replacement.
Use existing canonical-chain checks to distinguish a stale prefix from divergent
history; never treat a second height or elapsed time as consensus proof. Keep
supported-activation rescans when missing boundaries/checkpoints or established
replacement actually require them. No new tip watermark database, quorum service,
arbitrary freshness timer or expanded retention horizon is presumed necessary.
The first design gate must settle evidence and ambiguous cases before editing.

## Ordered commits and gates

1. **I1 — Specify and reproduce stale-tip versus replacement behavior.** Inspect
   sync startup, tip discovery, retained block comparison, interrupted-range repair,
   rollback and application Ready publication. Add small deterministic SDK/application
   regressions that reproduce stale same-chain regression without production changes;
   retain genuine deep-reorg and future-birthday coverage. Document the evidence
   classification and remaining ambiguity in `docs/zingo-integration.md` and
   `tests/LIFECYCLE.md`; describe proposed behavior as proposed until I2.
2. **I2 — Implement the smallest distinction with complete recovery.** Change the
   vendored source and tracked patch/provenance/verifiers together. Cover birthday
   above tip, shallow/deep regressions, same-height divergence, pruned checkpoints,
   missing boundaries, interruption and reopen. Retain the activation floor after
   a justified clear, without permanently lowering it on a stale-only response.
   Update `docs/zingo-integration.md`, `docs/wallet-rotation.md`, relevant vendor
   documentation and synthetic lifecycle contracts in this commit.
3. **I3 — Verify application recovery and funding boundaries.** Test recovery under
   the treasury owner with pending transactions: stale endpoint failure preserves
   observations/journals and later healthy sync resumes; real replacement finds
   credits below the declared birthday and preserves exact liabilities. Assert no
   replay, premature reservation release or replacement deposit. Update owning
   guides with qualified scope; use existing SDK compatibility and synthetic-chain
   lifecycle fixtures, not live funds or a fixture deadline as authority.

**G1 after I1:** independent SDK design review selects exact replacement evidence,
ambiguity behavior and necessary preservation. Exit with an implementable contract
and tests that distinguish the two cases; unresolved evidence blocks I2.
**G2 after I2:** review all prior commits/fixups across sync, persisted ranges,
patch/provenance and application observations. Exit with no destructive stale-only
path and no lost genuine-reorg recovery. **G3 after I3:** combined integration
review, including financial consumers, full patch and unchanged recovery hotspots;
consider upstream replacement only under the existing vendor retirement process.

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
