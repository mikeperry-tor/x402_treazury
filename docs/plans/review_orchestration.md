# Code-review implementation orchestration

## Intent, baseline and boundaries

Coordinate the review fixes through small independently reviewed changes, with
financial correctness first and independent catalog work alongside it. This
plan alone grants no implementation, live-funded qualification, upstream
submission or spending authority. Starting baseline:
`d557d82a70c6ef9bbe8a9936f696d12bf2cd2404`, including randomized funding and
qualification changes from `main` at `a626b99898d21c91552813a8b2413f48d3a1fede`.
Reconfirm the actual integration baseline before each implementation milestone.

O0 and the selected replacement-authority milestone started from
`d220eb95f766f98afe95dd8c2551778d993f5e6b`. Their implementation and child gates
are complete; other lanes and the whole-branch GR/GI gates remain unfinished.
The completed child plan has yielded the maintained
[funding/source contract](../wallet-rotation.md#funding-and-source-safety) and
[lifecycle coverage](../../tests/LIFECYCLE.md). Validation limitations below
remain required follow-up, not completed application checks.

The child plans own implementation commits, tests and permanent documentation;
this document owns ordering, shared ownership and integration. It introduces no
runtime scheduler or additional financial authority. Preserve the settled choices:
empty payment caps remain unlimited with WARN recommending `none`/`off`; new
x402 authority expires within 300 seconds or the shorter provider lifetime;
provider HTTPS retains full chain/hostname validation; missing listener auth and
disabled Host checks each emit their existing warning, including together.

Preserve exact accepted inputs/bytes/identities, journal-before-disclosure,
accepted-work drain and restart recovery. SDK facts own ZEC spendability; first
positive USDC credit consumes the claim. Randomized bounds authorize nominal
sizing, not top-ups, settlement guarantees or another deposit. Retain the merged
removal of ZEC spending budgets, actual fee/accepted-operation safeguards, atomic
qualification API-budget admission and historical registry read-only authority.
Production network constructors remain in `src/network.rs`, with Tor isolation,
remote DNS and no direct fallback. Add no arbitrary caps or whole-work deadlines.
Cover shaping and broad discovery/indexer privacy work remain excluded; a tangible
direct defect gets a scoped review and explicit integration boundary.

Owning guides are [wallet rotation](../wallet-rotation.md),
[SDK integration](../zingo-integration.md), [architecture](../architecture.md),
[configuration](../configuration.md), [agent sources](../agent-sources.md),
[network egress](../network-egress.md), [resource limits](../local-resource-limits.md),
[development/portability](../development.md#portability) and [testing](../testing.md).
Follow their existing links in [AGENTS.md](../../AGENTS.md#read-the-relevant-guide-before-editing).

The [tracked original review and maintainer disposition](x402_treazury_review.md)
provides the finding-by-finding reference for integration and final review. Its
original recommendations are source material, not implementation instructions;
settled decisions and owning guides govern. Update dispositions/evidence at the
owning gates and reconcile every finding at GR and GI, including intentionally
retained behavior and deferred work. Preserve the original report body while
annotating.

## Lanes and ordered integration boundaries

Rows identify integration milestones, not replacement implementation commits.
Retain each child's ordered commit IDs, coherent behavior/recovery boundaries,
acceptance evidence and documentation updates. Qualify references explicitly:
`orchestration/F1` means the milestone below; `funding/F1` and
`catalog-safety/C1` mean commits in their respective child plans.

| Milestone / lane | Child commits and default order | Integration acceptance |
| --- | --- | --- |
| O0 / coordination | Complete for the selected replacement scope: tracked baseline, preserved local changes, shared-worktree ownership, serialized builds and independent design review recorded in private evidence. | Material design findings resolved before implementation. O0 is preparation, not another application feature commit. |
| F1 / financial | Complete: funding/F1 `47b30aa8` with fixups `fdb71bba`, `625d6d6b`; funding/F2 `ba368fe6` with fixup `88213cab`. Contracts now live in the [funding guide](../wallet-rotation.md#funding-and-source-safety). | G1/G2 accepted the combined authority, archive/reconfirmation and race chains with scoped offline evidence. Confirmed deposit plus FAILED/REFUNDED grants no replacement; fixed/ranged redraw and late SUCCESS/credit races retain original liabilities. Full-suite fixture limitations remain below. |
| C1 / catalog | [Expansion/schema safety](catalog_safety_and_schema.md), C1 then C2; can start alongside financial F1. | All ingestion/direct-Value paths protected; generated schemas and outbound serialization remain faithful; child gates pass. |
| F2 / financial | [Indexer regression/recovery](indexer_regression_recovery.md), I1 through I3, integrated after financial F1. | Stale tip cannot destructively reset valid state; genuine deep-reorg recovery and accepted randomized work survive. Complete GF below. |
| C2 / catalog | [Registry efficiency](registry_efficiency.md), D1 then D2, integrated after catalog C1 settles construction contracts. | Correct invalidation, atomic durable publication and progressing readers; no caching around expansion safety. Child gates pass. |
| F3 / financial | [Proving execution](proving_execution.md), T1 then T2, integrated after financial F2. | Accepted principal/fee authority, proposal exclusion and owned calculation completion survive cancellation/restart; complete GF again for execution changes. |
| P1 / portability | [Portability](portability_fixes.md), P1 plus storage/lifecycle design and inventory may start independently. | Compilation/probe evidence accurately scoped; no claim of native support from compilation alone. |
| P2 / portability | Child P2 shared storage, then P3/P4 lifecycle/native workflows. Schedule shared-storage edits after financial F3 and catalog C2 by default. | Child native storage/recovery and ordinary-platform gates pass; complete GS below. Coordinate earlier integration only after affected owners settle their contracts. |
| R0 / R9 follow-up | [Focused R9 plan](hot_path_efficiency.md) prepared in this separate R0 documentation boundary before implementation. Scoping proceeds independently. | Fresh consumer/query audit separates qualification budget/status work from ordinary regex/client-cache work; ordered commits, authority-preserving tests and review gates agreed. Do not presume a cached monetary ledger or measured speedup. |
| R1 / R9 implementation | Execute [R9 child commits](hot_path_efficiency.md); normally integrate code after financial F2 and catalog C1 correctness gates, coordinating overlapping catalog/network/Store owners. | Child gates pass; exact budget arithmetic, concurrent admission, rollback/restart and uncertain charges preserved. Ordinary optimizations retain transport isolation and live-I/O ownership. Assess remaining history-dependent work before claiming it solved. |
| O1 / final integration | Assemble reviewed completed milestones, fixups and maintained-guide handoff. Optional portability P5 qualification backends remain separately selected work under their child plan. | Complete GR against the tracked review before GI and cleanup, then exact cleanup/final combined review. Unfinished required milestones keep this orchestration active. |

These are chosen integration orders, not universal technical dependencies. A2
investigation/reproducers and A4 parameter-reuse design can begin earlier. R9 may
move forward if investigation establishes urgent production impact; record the
reason and affected owners before implementation. Independent lanes need not
wait for unrelated financial or platform gates. Do not combine unfinished child
work merely to obtain a large merge.

## Ownership, branches and shared validation

The coordinator owns the integration branch and resolves cross-plan findings.
Assign one author/worktree per active implementation scope; keep audit/review
agents read-only by default. Distinct worktrees permit independent edits, not
competing ownership of the same call chain. Before launching work, name overlap
owners for funding/Store/treasury, catalog/import/discovery, network transport,
qualification registry, shared TLS fixtures and owning guides. Shared-storage P2
affects both financial and source/cache stores. Agree any ownership transfer;
refresh downstream branches after predecessor integration and inspect semantic
changes even if Git reports no conflict. Reopened concerns receive revalidation.

Serialize Cargo builds and feature configurations that share target/executable
paths; isolated editing does not make concurrent validation safe. Freeze sources
and relevant inputs while validating. Follow [validation order](../testing.md#validation-order-and-continuation):
cheap affected contracts and static/design/complexity review, production compiler
and all-target warning gate, compatibility, then applicable broader named stages.
The coordinator keeps evidence with exact SHAs, inputs, commands, nonzero test
counts, diagnostics and limitations under ignored `target/` or private storage.
Carry unchanged valid evidence explicitly; after a localized fix, revalidate its
consumers and resume unfinished stages rather than repeat all passed checks.
No baseline result substitutes for checks invalidated by integration. Use temporary
state/public unfunded keys; offline results establish no live or Tor qualification.

### Unfinished validation fixtures

The replacement milestone exposed unchanged fixture failures in the default,
managed and synthetic worker suites. Those stages are not green; scoped financial
acceptance does not close these gaps. Repair the fixtures before claiming a full
application check, preserving settled HTTPS, immutable network policy, public
destination enforcement, exclusive storage ownership and uncertain-payment rules.
Exact diagnostics and baseline source comparisons remain in private review evidence;
the baseline executable was not independently rebuilt.

- Reconcile local verified-TLS directory fixtures with public-destination checks,
  install explicit fixture CA trust in independent clients, and prevent managed
  harness initialization from replacing an already installed Tor policy. Inspect
  `src/discovery/`, `tests/support/dynamic_scope.rs`, `tests/managed.rs` and
  `tests/support/concurrency.rs`; later signature/drain assertions remain unvalidated.
- Correct the untrusted certificate/key pair and isolate the Base negative TLS
  control from globally trusted fixture roots. Preserve certificate and hostname
  verification in `src/network/tls_retry_tests.rs` and
  `src/rotation/base/transport/tests.rs`.
- Move provider HTTP fixtures to verified TLS in qualification help, managed
  maintenance, deferred catalog startup and the full treasury worker. Infrastructure
  HTTP remains distinct. The startup fixture currently waits for an origin request
  rejected before I/O; the worker/maintenance payment and rotation assertions remain
  unvalidated. Fix fixture synchronization rather than add a production deadline.
- Make captured HTTPS URI assertions check path/query and authority faithfully,
  inspect safe error causes through the execution wrapper, and prove registry
  connection/service ownership has drained before reopening. Keep full output,
  signature, uncertainty and exclusive-lock assertions; do not accept absent I/O
  or missing metadata to make fixtures pass.

These are focused validation prerequisites, not authorization for a new transport
bypass or broad discovery/privacy redesign. Resume orchestration/F2 (indexer
regression/recovery) after the affected validation foundation is repaired, or
record an independently reviewed scoped continuation and its remaining limits.

The selected fixture work starts at `db8bbeda`. The coordinator owns fixture
implementation and all builds in the shared worktree; independent fixture,
indexer and catalog design investigations remain read-only until explicit handoff.
Existing local files stay untouched. Preserve the owning
[test contracts](../testing.md), [network policy](../network-egress.md) and
[source authority](../agent-sources.md); no production policy exception is selected.

1. **FX1 — Trust, identity and wire fidelity.** Repair the intentionally untrusted
   certificate/key pair, independent client trust, once-installed Tor fixture
   policy, negative TLS control, HTTPS URI assertions and safe error-cause checks.
   Update the permanent TLS-fixture guidance in `docs/testing.md`. Acceptance is
   the affected real request/signature/identity tests, including negative controls.
2. **FX2 — Discovery fixture authority and ownership.** Distinguish explicit
   test-only local-TLS and public-route overrides, and drain HTTP connection/service
   ownership before reopening the registry. Retain public-route enforcement and
   exclusive locking; no sleeps, ignored errors or lock retry loop. Update testing
   guidance and verify directory payloads, retained work and restart publication.
3. **FX3 — Provider HTTPS fixtures.** Migrate qualification help, maintenance,
   deferred startup and full treasury worker fixtures while retaining infrastructure
   semantics and their exact financial assertions. Update testing/lifecycle guides;
   acceptance requires the formerly blocked payment, rotation and cancellation work
   to execute rather than merely avoid its previous error.
4. **FX4 — Executable network override.** Repair the later-reached CLI fixture with
   verified origin TLS and explicit network-file CA trust. Preserve proxy selection,
   isolation and absent wallet/stdout assertions; observe actual origin requests
   for inspection, warming and serving rather than accept a SOCKS attempt alone.

Independently review each exact FX commit and any fixup before dependent work;
use `--fixup` for findings and preserve unsquashed history. At the combined fixture
gate after FX4, review all FX commits/fixups and complete consumers alongside the
prior financial milestone, compare source-matched complexity, and finish the
applicable static, production/all-target warning, compatibility, default, proving,
lifecycle and codec stages with explicit valid continuations. Resolve material
findings before the next code lane. Remove this fixture schedule when its owning
guide handoff and all formerly blocked assertions are verified; do not archive it.

## Independent review gates and completion

Independently review every exact implementation, integration and cleanup SHA with
relevant consumers; the reviewer must not author it. Fix findings with
`git commit --fixup=<introducing-SHA>`, independently review each exact fixup and
preserve unsquashed history through final integration. Child gates remain required.
At each wider gate inspect all accumulated orchestration commits and fixups with
complete affected call chains, tests and documentation, including cross-lane
consumers. Examine recurring fixup causes, hazards and missing coverage; decide
whether they require regression tests, wider audit, simplification or design
changes. Resolve material findings before dependent work; later material changes
reopen affected gates. Earlier per-commit approval does not exempt code.

- **GF, after orchestration/F2 and reopened after orchestration/F3:** review funding claim/quote, SDK recovery,
  preparation, calculation, disclosure, credit and retained original liabilities.
  Include integrated catalog/network/qualification consumers where they touch
  these paths. Exit with allowed work progressing, no duplicate accepted I/O or
  new financial authority, and applicable child acceptance evidence complete.
- **GS, after orchestration/P2 shared-storage integration:** review financial, source/cache and qualification
  create/open/replace/backup/recovery chains together, including budget admission,
  historical evidence and platform-specific protections. Exit with no privacy,
  durability, alias, ownership or replay gap and native evidence for each support
  claim. Later lifecycle changes reopen relevant concerns at the child gates.
- **GR, at orchestration/O1 before GI and removal of working review material:**
  have an independent read-only subagent review the **whole accumulated branch**
  against the [original review and maintainer disposition](x402_treazury_review.md).
  Supply user intent, settled S1/S3/S7/C1 decisions, agreed scope, exact comparison
  base and candidate SHA, all implementation/integration commits and fixups, and
  validation evidence/limitations. Inspect complete relevant consumers and failure
  paths across lanes, including changes inherited from `main`; reviewing the
  latest diff or relying on child approvals is insufficient. Reconcile every one
  of the 42 finding IDs with current code, tests, documentation and its disposition.
  Check for missed, incompletely addressed or newly aggravated issues and
  unsupported claims of resolution; distinguish justified policy/deferred scope
  from required work. Original recommendations do not override settled decisions.
  Report concrete gaps with evidence, introducing SHA where identifiable and
  affected consumers. Address appropriate gaps with
  `git commit --fixup=<introducing-SHA>`; independently review each exact fixup,
  add needed regressions, revalidate affected behavior and reopen relevant child
  or wider gates. Where a gap has no introducing commit on this branch, identify
  its actual owner/baseline and define a coherent corrective commit rather than
  manufacture an attribution. Recheck the combined branch after corrections and
  update finding dispositions/evidence. Exit requires no unresolved material gaps
  in required scope and explicit, justified dispositions for the other findings.
  Tell the user about substantive additions, changes or removals resulting from
  this review. Retain the exact reference-document commit in private review
  evidence so GI can compare against that Git version after cleanup removes it.
- **GI, at orchestration/O1 and after cleanup:** review the entire assembled branch and all prior
  fixups, child gates, documentation and evidence. Exit with no material unresolved
  findings, no required unfinished work represented as complete, and claims limited
  to actual validation. Keep optional unfinished work explicit in its owning plan.

For substantive Rust refactors and architecture gates, compare source-matched
CogC/CycC against the agreed integration baseline under
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline). Review changed
chains, relevant warnings and unchanged high-risk hotspots; assess simpler
ownership/control flow rather than merely move decisions into helpers.

Each child implementation commit updates its named permanent guides. Before
removing a completed child plan, verify its owning-guide/AGENTS link handoff and
update this document's references and remaining schedule in the same cleanup.
Keep detailed contracts in those guides. Once required coordinated work is complete,
remove this meta-plan, the completed working review/disposition and their
[documentation-index](../README.md) links after the same handoff; independently
review that exact cleanup SHA and final combined branch after removal. Retain
focused unfinished optional work, never a completed-plan archive. Documentation
cleanup cannot delete financial evidence or authorize replay.
