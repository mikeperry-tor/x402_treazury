# Code-review implementation orchestration

## Intent, baseline and boundaries

Coordinate the review fixes through small independently reviewed changes, with
financial correctness first and independent catalog work alongside it. This
request authorizes this document and its commit, not implementation, live-funded
qualification, upstream submission or spending. Starting baseline:
`d557d82a70c6ef9bbe8a9936f696d12bf2cd2404`, including randomized funding and
qualification changes from `main` at `a626b99898d21c91552813a8b2413f48d3a1fede`.
Reconfirm the actual integration baseline before each implementation milestone.

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
owning gates and reconcile every finding at GI, including intentionally retained
behavior and deferred work. Preserve the original report body while annotating.

## Lanes and ordered integration boundaries

Rows identify integration milestones, not replacement implementation commits.
Retain each child's ordered commit IDs, coherent behavior/recovery boundaries,
acceptance evidence and documentation updates. Qualify references explicitly:
`orchestration/F1` means the milestone below; `funding/F1` and
`catalog-safety/C1` mean commits in their respective child plans.

| Milestone / lane | Child commits and default order | Integration acceptance |
| --- | --- | --- |
| O0 / coordination | Capture clean tracked baseline, local changes, assigned worktrees/file owners and existing validation provenance. Independently challenge the selected child design before implementation. | Material design findings resolved; selected scope and overlap owners explicit. O0 is preparation, not another application feature commit. |
| F1 / financial | [Replacement authority](funding_replacement_authority.md), F1 then F2. Highest financial priority. | Child gates pass; confirmed deposit plus FAILED/REFUNDED grants no replacement; fixed/ranged redraw and late SUCCESS/credit races retain original liabilities. |
| C1 / catalog | [Expansion/schema safety](catalog_safety_and_schema.md), C1 then C2; can start alongside financial F1. | All ingestion/direct-Value paths protected; generated schemas and outbound serialization remain faithful; child gates pass. |
| F2 / financial | [Indexer regression/recovery](indexer_regression_recovery.md), I1 through I3, integrated after financial F1. | Stale tip cannot destructively reset valid state; genuine deep-reorg recovery and accepted randomized work survive. Complete GF below. |
| C2 / catalog | [Registry efficiency](registry_efficiency.md), D1 then D2, integrated after catalog C1 settles construction contracts. | Correct invalidation, atomic durable publication and progressing readers; no caching around expansion safety. Child gates pass. |
| F3 / financial | [Proving execution](proving_execution.md), T1 then T2, integrated after financial F2. | Accepted principal/fee authority, proposal exclusion and owned calculation completion survive cancellation/restart; complete GF again for execution changes. |
| P1 / portability | [Portability](portability_fixes.md), P1 plus storage/lifecycle design and inventory may start independently. | Compilation/probe evidence accurately scoped; no claim of native support from compilation alone. |
| P2 / portability | Child P2 shared storage, then P3/P4 lifecycle/native workflows. Schedule shared-storage edits after financial F3 and catalog C2 by default. | Child native storage/recovery and ordinary-platform gates pass; complete GS below. Coordinate earlier integration only after affected owners settle their contracts. |
| R0 / R9 follow-up | Separate documentation commit creating a focused R9 plan before R9 implementation. Scoping can proceed independently. | Fresh consumer/query audit separates qualification budget/status work from ordinary regex/client-cache work; ordered commits, authority-preserving tests and review gates agreed. Do not presume a cached monetary ledger or measured speedup. |
| R1 / R9 implementation | Execute the resulting child commit map; normally integrate code after financial F2 and catalog C1 correctness gates, coordinating overlapping catalog/network/Store owners. | Child gates pass; exact budget arithmetic, concurrent admission, rollback/restart and uncertain charges preserved. Ordinary optimizations retain transport isolation and live-I/O ownership. Assess remaining history-dependent work before claiming it solved. |
| O1 / final integration | Assemble reviewed completed milestones, fixups and maintained-guide handoff. Optional portability P5 qualification backends remain separately selected work under their child plan. | Complete GI below, including exact cleanup review. Unfinished required milestones keep this orchestration active. |

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
