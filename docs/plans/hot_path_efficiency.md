# Hot-path work and qualification history (R9)

## Intent, baseline and boundaries

Remove demonstrated repeated pure work, global client-cache lock construction and
redundant qualification history passes while preserving exact authority, complete
financial evidence and active-I/O ownership. The user selected remaining work in
[orchestration](review_orchestration.md), including its separate R0 planning commit
before R9 implementation. This plan itself grants no live-funded qualification,
upstream submission or new spending authority.

**Integration baseline: 74dc4dc0d381039bf081441b528f5be0e19bd573.** The fresh audit
examined `55440132` after replacement-authority work and the fixture repairs; refresh
the actual branch state, local files, owners and validation provenance before R0
and every implementation boundary. Preserved settled decisions and financial
contracts come from [AGENTS.md](../../AGENTS.md#financial-and-execution-boundaries),
[wallet recovery](../wallet-rotation.md#recovery-and-reconciliation-policy),
[best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[observation policy](../wallet-rotation.md#wallet-observation-policy),
[network policy](../network-egress.md), [catalog architecture](../architecture.md),
[qualification testing](../testing.md) and the
[qualification reference](../../tests/live/INTEGRATION_REFERENCE.md).

Preserve journal-before-disclosure; exact accepted inputs, signed bytes, original
identities and liabilities; first-positive-credit claim consumption; SDK-owned
ZEC spendability; no ordinary/qualification ZEC spending budget, top-up or credited
output floor. Qualification's per-case, run and cumulative registry API budget
remain distinct authority. Every potentially chargeable authorization stays charged
through failed/uncertain outcomes, missing receipts, cancellation and restart.
Historical version-1 qualification registries stay read-only; version-2 execution
authority is never reconstructed from historical ZEC-budget values.

Add no mutable monetary/hash cache, parallel ledger, pruning, new cap, readiness
queue, whole-work deadline or transport bypass. All production HTTP/gRPC
constructors stay in `src/network.rs`; preserve public-destination enforcement,
full TLS verification, immutable context/trust, identity-bound scopes, Tor remote
DNS and no direct fallback. Catalog names, display pricing, state/evidence formats,
`auto_v1_` names and `X402_*` contracts remain stable. Quantitative speedup or
constant-time admission claims require separate evidence; none is presumed here.

## Current implementation and selected approach

- `catalog::slug` and `money` compile constant regexes repeatedly. Reuse identical
  regexes through standard-library initialization; preserve Unicode/lowercase,
  accepted Value kinds, naming and display formatting instead of inventing a parser.
- `NetworkContext::http_transport` holds its shared cache mutex through client
  construction. Build an immutable miss candidate outside the lock and recheck the
  complete key before publication; return the canonical winner rather than expose
  or overwrite it with a losing candidate. Keys include runtime, identity, origin,
  selection, public policy, HTTP/TLS permissions and inactivity timeout. Retain
  expected-Arc promotion and eviction's cache-ownership-only semantics.
- HTTP construction is network-lazy, but its optional private scope observer can
  persist associations and fail. Its contract includes constructed unused
  candidates. Preserve pre-I/O observation/failure behavior and accurate loser
  observations; do not describe construction as side-effect-free or create a new
  scheduling framework solely to coalesce harmless object construction.
- `PaymentContext::record` scans run payments for its case, tentatively inserts an
  admission, then `budget::check` separately scans run and all-registry payments.
  Prefer one transaction-local pass over existing canonical events to derive exact
  case/run/registry totals under the same IMMEDIATE write transaction. Strict JSON
  validation, per-event u64 validation plus checked U256/u64 arithmetic, current authority and complete rollback
  remain. This removes overlapping work without a durable total or release rule.
- Qualified admission reads keyed payment evidence, then attaches a lifecycle
  projection; post-response observation builds another lifecycle through full
  status. Current paths traverse wallets, funding/source history and encrypted raw
  record hashes. Replace demonstrated redundant status construction/N+1 reads with
  a consistent, complete row projection. Preserve all existing output fields and
  reader contracts; hashing streamed source rows once does not require a hash cache.

The original report's claim that every payment fails after 10,000 prior payments
is not current acceptance evidence. The full `qualification_state` export retains
separate 10,000-row guards used by baseline/permit/preparation/restart consumers;
keyed payment admission has no such guard. Do not remove, tighten or relocate these
limits into ordinary serving. Full evidence/report output and an exactly derived
cumulative budget still have inherent O(history) work. This plan removes identified
redundancy, not the cost of publishing or validating required complete history.

## Q2 projection contract

Preserve lifecycle schema2 and its complete collection domains, ordering and nulls:

| Projection | Canonical source and preserved behavior |
| --- | --- |
| Root identity | SQLite schema23 and instance version1/id; never infer treasury identity from a caller. |
| Pools | Complete current PoolStatus, including every address, reserve claim, ownership/readiness predicate, retirement accounting and funding degradation. Reuse existing checked authority helpers. |
| Funding jobs | All jobs in row order, including completed and unrelated work: original id, pool/wallet/recipient/target, current operation and decoded phase. Health still contributes to pool degradation. |
| Source operations | All treasury operations ordered by id: submission with active expiry precedence, attempts, decoded original facts and SHA256 of exact outgoing.raw. Missing raw is an error, never an omitted operation. |

Keyed admission uses its existing financial write transaction and persisted-only
pool view; direct/post-response/maintenance lifecycle preserves current runtime
balance evidence enrichment from the same serialized Store command. A later
response observation must remain fresh, rather than reuse admission's earlier view.
Full readonly qualification export keeps its existing single transaction, complete
histories and row guards. It is not replaced by the narrower lifecycle projection.

Admission lifecycle failure remains optional and cannot invalidate otherwise valid
original keyed evidence. Post-response lifecycle failure retains the receipt and
uncertain charge with its existing unavailable sentinel; receipt persistence failure
still propagates. Maintenance failure cannot become successful balance evidence.
Required full-export/permit failures remain required. Preserve every reader in
consumed-reserve, refill, capacity, restart, registry accounting and permit validation.
Only diagnostics never exported in lifecycle may cease to be constructed there;
ordinary Status and full export retain their complete contracts.

## Ownership and coordination

The coordinator owns integration and validation. Assign one author/worktree per
active scope; reviewers remain source read-only. Name overlap owners for
`catalog.rs`, `network.rs`, financial Store/qualification, live-runner registry and
owning guides before editing. Default integration follows orchestration: financial
F2 and catalog C1 correctness gates precede R9 code; earlier pure work requires an
explicit coordinated reason. Refresh after predecessor integration and inspect
semantic changes even without Git conflicts. A new unresolved ownership, financial
or projection design concern blocks dependent edits.

Serialize Cargo builds and feature configurations, freeze source/executable/inputs
for validation, preserve unrelated local files and never inspect real secrets.
Private evidence belongs under ignored `target/` or private storage. Existing
fixture-stage results may carry only where their exact relevant inputs remain
valid; production R9 changes require affected compiler/runtime revalidation.

## Ordered commits and acceptance

**R0 — Standalone plan commit.** Commit this plan, its orchestration/index references
and actual baseline before implementation. Independent non-author design/exact-SHA
review challenges checks, alternative simpler representations, scope boundaries
and complete consumers. Exit with material findings resolved and the Q2 output
contract explicit enough to implement; a desired optimization is not authority to
weaken evidence.

1. **r9/P1 — Reuse constant catalog regexes.** Use standard-library immutable lazy
   initialization for the exact current expressions. Inspect default-prefix,
   generated suffix, price and provider metadata consumers. Run affected naming,
   pricing/schema and catalog tests; add only missing meaningful boundaries, not
   provider inventories or tests mirroring initialization. Update owning catalog
   documentation if an enduring contract is clarified. No new dependency.
2. **r9/N1 — Release cache lock during HTTP construction.** Keep URL/public guards
   before acquisition, construct outside the lock, recheck/touch an existing winner
   or insert once under current eviction policy. Include candidate/promotion,
   private observer and all factory consumers. Deterministically hold two synchronous miss constructors past lookup before
   testing concurrent same-key winner identity; independent identity/runtime/origin/transport/public-policy keys;
   retained active transports through eviction; stale promotion refusal; and
   observer failure without outbound I/O. Observed losing constructed scopes do not
   become successful routing or financial authority. Update network ownership/cache
   guidance in `docs/network-egress.md` and relevant testing contracts. Complete GN.
3. **r9/Q1 — Derive exact admission budgets in one pass.** Change admission,
   budget helpers and their report/authorization consumers coherently. Current
   per-case/window/pool/session/attempt checks and unique-attempt enforcement stay
   under the write transaction. Qualification charge commits before financial
   dispatch; a later financial commit failure cannot erase that charge or grant
   replay. Preserve `used(Some(run))` scope/error semantics for report callers.
   Test exact case/run/registry boundaries, concurrent admissions in one run and
   distinct runs, duplicate attempts, malformed/missing/non-string/oversized and
   overflowing amounts, refused rollback without financial journal/disclosure,
   reopen with failed/uncertain charges and historical-v1 refusal. Use existing
   PaymentContext/budget/PaidClient/registry fixtures and deterministic failure
   injection, never real funds or time delays. Update `docs/testing.md` and
   `tests/live/INTEGRATION_REFERENCE.md` in this commit. Complete GQ1.
4. **r9/Q2 — Read complete lifecycle evidence without redundant status work.**
   First map every current field to its canonical SQL/cache source and readers in
   lifecycle/refill/capacity/restart, registry application/accounting and permit
   validation. Use the caller's consistent Store transaction/serialized snapshot,
   not a new connection or independently refreshed monetary view. Stream SQL rows
   into the complete existing projection and hash each required immutable record
   once per observation; no mutable hash cache. Remove only demonstrated duplicate
   construction/queries. Retain original identities, all required histories,
   timing/correlation, output completeness and historical reader compatibility.
   Preserve optional lifecycle-observation failure behavior: missing evidence cannot
   erase receipts, invalidate accepted charge ownership or authorize repetition.
   Test exact output parity on faithful small states, admission/post-response/refill
   timing, capacity and source continuity, cancellation/restart, unrelated malformed
   diagnostic scope and full snapshot/report guards. Revalidate ordinary reporting
   consumers if shared read helpers change. Update `docs/wallet-rotation.md`,
   `docs/testing.md`, `tests/LIFECYCLE.md` and the qualification reference as affected.
   Complete GQ2 before integration claims.

SQL projections/indexing are available only after demonstrating their exact
contract. SQLite SUM/CAST can overflow or coerce invalid values; JSON filters can
silently omit malformed admissions. A narrow JSON projection must not relax strict
validation of accepted history, skip charges, change scope or grant authority.
Use current indexes unless a query-plan/consumer audit establishes a missing one;
no second monetary index/ledger framework is presumed. If Q2 requires narrower or
versioned evidence rather than complete current output, reopen independent design
before implementation and update this plan; do not silently drop published history.

## Independent gates, validation and completion

Independently review every exact R0/implementation/integration/cleanup SHA and all
relevant consumers; reviewer must not author it. Address findings with
`git commit --fixup=<introducing-SHA>`, independently review every fixup and keep
history unsquashed through final integration. Give reviewers user intent, scope,
exact baseline/candidate, preserved invariants and actual nonzero test evidence.

- **GN after r9/N1:** inspect all earlier R9 commits/fixups alongside complete
  factory/key/candidate/promotion/eviction, active I/O, trust/Tor and qualification
  observer chains. Exit with correct winner ownership, no identity-policy leakage,
  unobserved disclosure or duplicated active network I/O, and no unnecessary
  singleflight/state machinery.
- **GQ1 after r9/Q1:** inspect all earlier R9/funding commits and fixups, financial
  Store journal -> qualification transaction -> current expiry check -> disclosure,
  concurrent registry writers, report/authorization and v1/v2 authority consumers.
  Exit with exact atomic case/run/registry budgets, refused rollback, retained
  uncertain charges and no new money authority/cache/ledger.
- **GQ2 after r9/Q2:** inspect every prior plan commit/fixup with complete
  lifecycle/status/receipt/refill/permit/restart/report call chains and predecessor
  financial/catalog changes. Exit with complete compatible output, consistent
  observations, no erased accepted evidence and demonstrably removed redundant work.
- **R9 final integration gate:** review the entire assembled R9 work plus all
  fixups and its relevant predecessors, not only the latest diff. Reassess recurring
  assumptions, fixup causes, hazards and coverage gaps; decide whether wider audit,
  regressions, simplification or design change is required. Explicitly assess
  remaining inherent history-dependent work and avoid unsupported performance or
  full-resolution claims. Resolve material findings before cleanup; later material
  changes reopen affected gates.

Use source-matched CogC/CycC against the agreed baseline for substantive Rust/query
refactors and architecture gates under
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline). Inspect warnings,
changed complete chains and unchanged admission/recovery/network hotspots. Compare
actual deletion of duplicate work and invalid states; helper extraction or passing
tests alone does not justify current organization.

Follow [validation order](../testing.md#validation-order-and-continuation): affected
cheap contracts and applicable independent design/complexity review, production
compiler and full all-target Clippy warning gate, compatibility, then applicable
named default/proving/lifecycle/codec stages. Choose socket-capable execution up
front for local listeners. Use pinned Make commands, public unfunded keys and
frozen inputs. Record exact SHAs, commands, features/profile, counts, diagnostics,
limits and why carried evidence remains valid. Resume unfinished stages after
focused fixes; never call an excluded/failed stage green. Offline fixtures establish no live-funded, consensus or live Tor qualification.
Native behavior claims require the corresponding platform to execute the relevant
checks; source inspection or cross-compilation alone does not establish them.

Permanent rules live in the named owning guides with each implementation commit.
[AGENTS.md](../../AGENTS.md#read-the-relevant-guide-before-editing) already links
network, wallet, development/testing and lifecycle guidance; add an owning-guide
link there only if a new enduring area requires one. Before removing this completed
plan, verify those links/guide handoffs, update orchestration/R9 disposition and
remove its documentation-index reference in the same cleanup. Independently review
both exact cleanup SHA and final combined branch after removal. Retain concrete
unfinished work, never a completed-plan archive. Cleanup cannot delete financial
evidence or grant replay authority.
