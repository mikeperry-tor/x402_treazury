# Replacement funding authority (S4)

## Intent, baseline and preserved contracts

Prevent a provider's FAILED/REFUNDED report from authorizing another Zcash deposit
for a still-potentially-fulfilled initial funding claim. This plan starts from
`1bd810eb`; the review originally examined `3ceb88f2e41038c4f6ef6959494c2f501373f1cd`.
Planning grants no implementation or spending authority.

Read [wallet ownership/reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy),
[best-effort rules](../wallet-rotation.md#best-effort-wallet-rules),
[observation policy](../wallet-rotation.md#wallet-observation-policy),
[SDK recovery](../zingo-integration.md), [architecture](../architecture.md) and
[financial tests](../../tests/LIFECYCLE.md). Preserve journal-before-disclosure,
original inputs/bytes/identities, retained accepted work and existing fee/recovery
authority. SDK state owns spendability; no replacement monetary ledger or new cap.
The first positive USDC observation consumes the claim, even if partial. Neither
refund attribution nor a target floor is a prerequisite for ordinary shielding.

The refreshed baseline includes `main` at `a626b998`: randomized nominal reserve
ranges and saved unsigned choices, removal of configurable per-transfer ZEC caps,
and qualification's running USDC API budget. Preserve current fee and accepted
operation authority; do not restore removed ZEC budgets. Historical qualification
registries remain read-only under the version-2 authority contract.

## Current gap and proposed behavior

`src/rotation/store/funding.rs::funding_retry_allowed` accepts provider `failed`
or `refunded` flags. `reauthorize_closed_funding_for_operation` also requires
`source_attempt_closed`, but that includes resolved CONFIRMED source work: chain
confirmation establishes delivery to the swap, not absence of destination liability.
`src/treasury/recovery.rs::reauthorize_recovery` consumes this decision. Thus the
original report overstates status alone, but identifies a real authority gap.
The merged test `negative_terminal_confirmed_source_can_reauthorize_under_current_claim`
still permits this replacement and now checks that selected quote terms reset.
A fresh random draw changes sizing, not the old swap's delivery liability.

Remove provider-negative-only retry authority. Preserve established paths for
proven non-disclosure and authenticated finite source expiry after completed SDK
coverage at the configured depth, under the current unconsumed claim and funding
policy. Keep exact old-request observation where unresolved; late bound SUCCESS
consumes the same claim and blocks a replacement not yet disclosed. A refund or
source confirmation alone proves neither non-delivery nor a new claim. Reconcile
actual SDK funds normally; reconciliation cannot remove a still-live external
settlement liability. Do not require another indexer opinion or proof of returned
coin provenance. If additional bridge evidence is proposed, define its exact
contract and obtain independent design review before granting authority.

## Ordered commits and gates

1. **F1 — Correct the shared retry authority with its recovery consumers.** Change
   the Store classifier and atomic reauthorization/preparation checks together;
   inspect current and archived operations, source closure, automatic funding and
   operator recovery. Preserve stable state formats and idempotent archive behavior.
   Update `docs/wallet-rotation.md` and affected `tests/LIFECYCLE.md` contracts.
   Acceptance: disclosed confirmed deposit plus FAILED/REFUNDED never creates a
   new operation/calculation/disclosure; proven undisclosed and finite-expired
   attempts can progress under existing authority; consumed claims never reopen.
   Cover stale operation bindings, malformed advisory status, restart and cancellation.
   Exercise fixed and ranged policies: preserve immutable allocation intent, distinguish
   current nominal bounds from saved `quote_target`/`quote_ceiling` and accepted
   output, and permit a fresh draw only after independent replacement authority.
   Test rejected quote/floor redraws never close a disclosed liability or charge a
   signed recovery attempt; do not impose a credited-output floor or top-up.
2. **F2 — Exercise replacement/old-operation races across the full owner chain.**
   Add focused Store/treasury lifecycle regressions for late old SUCCESS before
   preparation and before disclosure, credit during replacement, and two already
   disclosed liabilities. Preserve independent observation/drain and original bytes;
   assert no duplicate I/O or erased journals. Include another group's progress.
   Cover credit below the nominal minimum, favorable credit above the maximum,
   changed configuration and late old SUCCESS during a replacement redraw.
   Retain qualification job-count/API-budget boundaries without a ZEC budget.
   Update `docs/wallet-rotation.md` and lifecycle guide with any clarified contract.

**G1 after F1:** independent authority review of Store -> treasury recovery ->
funding scheduler -> source preparation/disclosure and every archive consumer.
Exit: no status-only authority, no new top-up or replay path, and allowed recovery
still progresses. **G2 after F2:** combined architecture/integration review of all
commits/fixups and race evidence, including retained prior liabilities and guide
handoff. Resolve material findings before cleanup.

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
