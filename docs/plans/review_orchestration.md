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
`d220eb95f766f98afe95dd8c2551778d993f5e6b`. Their replacement-authority implementation
and scoped child gates
are complete, including the publisher-consumer correction accepted below. Catalog
expansion G1 and indexer recovery G1/G2/G3 are complete. Financial GF is accepted
through `68a93c0b` and reopens after F3; remaining child work and whole-branch
GR/GI are unfinished. The completed children have yielded the maintained
[funding/source contract](../wallet-rotation.md#funding-and-source-safety) and
[SDK recovery guide](../zingo-integration.md), with
[lifecycle coverage](../../tests/LIFECYCLE.md). The fixture validation foundation
below is complete through focused continuations.
The inherited publisher-completion finding is corrected without changing settled
replacement authority. Execution changes in financial F3 require renewed GF.

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
| F1 / financial | Replacement authority and publisher-consumer correction complete. funding/F1 `47b30aa8` with fixups `fdb71bba`, `625d6d6b`; funding/F2 `ba368fe6` with fixup `88213cab`; publisher fixups `e9d3dcb4`, `9fd610ce`, `2ef9dcc0`. Contracts live in the [funding guide](../wallet-rotation.md#funding-and-source-safety). | G1/G2 and the scoped publisher-consumer gate accepted exact authority, archive/reconfirmation and actual publication/actor/funding/close chains. Confirmed deposit plus FAILED/REFUNDED grants no replacement; late fulfillment retains original liabilities. Wider GF is accepted after F2 and reopens after F3. |
| C1 / catalog | [Expansion/schema safety](catalog_safety_and_schema.md): C1 and its G1 accepted through `a50d4f7a`; C2 remains next in this child. | All ingestion/direct-Value paths protected. C2 must preserve faithful generated schemas/outbound serialization and reopen affected expansion concerns at combined G2. |
| F2 / financial | Complete: I1 `0baed039`, I2 `8fa105fa` and reviewed fixups, I3 `7e5648be`. Contracts live in [SDK recovery](../zingo-integration.md) and [wallet reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy). | G1/G2/G3 and wider GF accepted through `68a93c0b`. Stale-only tips preserve observations; future completion metadata cannot block current SDK funds. Actual disclosed originals retain exact liabilities through cancellation, restart and genuine replacement. F3 reopens GF. |
| C2 / catalog | [Registry efficiency](registry_efficiency.md), D1 then D2, integrated after catalog C1 settles construction contracts. | Correct invalidation, atomic durable publication and progressing readers; no caching around expansion safety. Child gates pass. |
| F3 / financial | [Proving execution](proving_execution.md), T1 then T2, integrated after financial F2. | Accepted principal/fee authority, proposal exclusion and owned calculation completion survive cancellation/restart; complete GF again for execution changes. |
| P1 / portability | [Portability](portability_fixes.md): `4fff946f` plus `574e0b38`/`5251194e` implement portable classification and native control wiring; shared-storage/lifecycle design proceeds independently. | macOS six controls pass; Windows release build/CLI smoke tests pass at `7126f716`. Native vendor preflight passes at `5218fef0` after `68a93c0b` and `cec39403`; application build and later test gates remain pending. Namespace buffer-only control still failed before flush; absolute-path control `3e97d27b` passes both API candidates at `5218fef0`, including restricted-token/collision/identity/byte/flush checks. This is compatibility evidence, not P2 privacy, recovery or power-loss acceptance. Native P1/Linux/storage gates remain open. |
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

### Validation foundation and publisher correction

The fixture prerequisite is complete through `1ca4f524`, with maintained contracts
in [verified TLS fixtures](../testing.md#verified-tls-fixtures),
[network ownership](../network-egress.md) and
[lifecycle coverage](../../tests/LIFECYCLE.md). Independent C1 G1, indexer G1/G2/G3
and financial GF accept scoped assembled evidence through `68a93c0b`: all 58
default/compatibility targets account for 1,802 passes and 41 ignored cases;
proving passes four, synthetic lifecycle 45 and the codec one. The lifecycle
ledger counts each current case once, combining 39 unchanged passes with four
corrected cases and two new disclosed-original scenarios. Earlier failed runs
remain failed. This is an assembled check, not one successful `make check`.

Production/full all-target warning gates and format pass. Source-matched
complexity covers 447 files without exclusions; paired SDK analysis and unchanged
financial hotspots have explicit simplification review. Evidence carries only
where source, feature and input domains remain valid. One changed SDK metadata
unit also passed in an exact-source standalone cohort using existing locked
packages and a scoped `proptest` `std` dev feature; this does not claim
full upstream/default-feature testing. The macOS test-link compact-unwind warning
remains recorded. Generated ledgers and source/command provenance remain private.

The inherited `560b31796` publisher defect is corrected by `e9d3dcb4`, with
independently reviewed fixups `9fd610ce` and `2ef9dcc0`. Durable acknowledgment is
separate from the current pending-disclosure view: a late-fulfilled calculation
can remain RESOLVED without a false publication retry. Exact raw bytes, snapshots,
inputs and existing fulfillment proofs remain; positive credit alone is distinct.
Actual publisher, actor, funding and close regressions cover the full boundary.
The correction grants no closure, spending, replay, input-release or replacement
authority. Its scoped consumer gate is accepted; T2 design may use this boundary,
while T2 integration still follows financial F2.


Indexer recovery additionally exposed a real future shard-completion metadata
bug and fixture assumptions about retained anchors, status balances and snapshot
persistence. Reviewed fixups preserve tree data while repairing scheduling
metadata; regressions now exercise actual selected block identities, SDK
spendability and persisted restart state. Independent recurrence audits found no
additional material gaps. G3 and financial GF accepted full SDK/Store/publisher/
funding/qualification call chains; execution changes in F3 reopen that review.

Windows release build and CLI smoke tests passed at `7126f716`; later Checks
failed at vendor patch reconstruction before Rust test gates. Native vendor
preflight now passes at `5218fef0` after the reviewed invocation/binary-output
corrections; the application build and later tests remain pending. At that SHA,
the absolute-path namespace experiment passes both directory-flush candidates,
including ordinary restricted-token controls, no-replace collision preservation,
identical file identities and exact public bytes. Earlier failures remain failed,
and the hosted image changed, so no sole-variable cause is inferred. This permits
P2 to use the simpler Win32 candidate in its design; it does not establish complete
storage privacy, recovery or power-loss durability. Linux preparation is not native test acceptance.
No live-funded, consensus or live Tor qualification ran. Later changes reopen
affected checks and combined concerns.

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
  these paths, including durable publication completion versus pending disclosure
  projections and all prior fixup causes. Exit with allowed work progressing, no duplicate accepted I/O or
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
