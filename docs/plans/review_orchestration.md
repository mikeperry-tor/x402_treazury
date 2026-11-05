# Code-review implementation orchestration

## Intent, baseline and boundaries

Coordinate the review fixes through small independently reviewed changes, with
financial correctness first and independent catalog work alongside it. This
plan alone grants no implementation, live-funded qualification, upstream
submission or spending authority. Starting baseline:
`d557d82a70c6ef9bbe8a9936f696d12bf2cd2404`, including randomized funding and
qualification changes from `main` at `a626b99898d21c91552813a8b2413f48d3a1fede`.
The current implementation baseline is `b78909442d9ae78064ca42ed2c12fc2673edc6ad`,
after an unsquashed rebase onto `main` at `88cc7bebef364e9dfaaff9197596df3df438a424`.
Reconfirm the actual integration baseline before each implementation milestone.

O0 and the selected replacement-authority milestone started from
`d220eb95f766f98afe95dd8c2551778d993f5e6b`. Their replacement-authority implementation
and scoped child gates
are complete, including the publisher-consumer correction accepted below. Catalog
expansion G1 and indexer recovery G1/G2/G3 are complete. Catalog combined G2
is independently accepted through `4627301b`; execution G2 and financial GF are
accepted with the source-scoped evidence below. Registry D1/G1 and combined
D1/D2 G2 are independently accepted through `c0fa2c07`. R9 GN, GQ1, GQ2 and
combined integration are independently accepted through `b7890944`;
whole-branch GR/GI remain unfinished. Historical F1/F2 and portable-benefit SHA
references below identify preserved pre-rebase commits; their earlier runs retain
only the verified unchanged source scope, not a claimed rerun on the rebased SHA.
The completed children have yielded the maintained
[funding/source contract](../wallet-rotation.md#funding-and-source-safety) and
[SDK recovery guide](../zingo-integration.md), with
[lifecycle coverage](../../tests/LIFECYCLE.md). The fixture validation foundation
below is complete through focused continuations.
The inherited publisher-completion finding is corrected without changing settled
replacement authority. Execution acceptance is recorded at F3 below; later
material execution/authority changes reopen the affected financial concerns.

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

Windows runtime, storage, supervision and native acceptance work is explicitly
deferred by user decision. Its dedicated workflows and experiment drivers are
removed; portable Rust and strict vendor-verification improvements remain.
[macOS ACL and Unix storage concerns](portability_fixes.md) remain separate,
unfinished follow-up outside this remaining milestone. They are not marked fixed
and impose no new shared-storage or platform lifecycle gate on the financial,
catalog, registry and R9 lanes. Normal Unix financial/lifecycle checks still apply.

## Lanes and ordered integration boundaries

Rows identify integration milestones, not replacement implementation commits.
Retain each child's ordered commit IDs, coherent behavior/recovery boundaries,
acceptance evidence and documentation updates. Qualify references explicitly:
`orchestration/F1` means the milestone below; `funding/F1` and
`catalog-safety/C1` mean commits in their respective child plans.

| Milestone / lane | Child commits and default order | Integration acceptance |
| --- | --- | --- |
| O0 / coordination | Complete for the selected replacement scope: tracked baseline, preserved local changes, shared-worktree ownership, serialized builds and independent design review recorded in private evidence. | Material design findings resolved before implementation. O0 is preparation, not another application feature commit. |
| F1 / financial | Replacement authority and publisher-consumer correction complete. funding/F1 `47b30aa8` with fixups `fdb71bba`, `625d6d6b`; funding/F2 `ba368fe6` with fixup `88213cab`; publisher fixups `e9d3dcb4`, `9fd610ce`, `2ef9dcc0`. Contracts live in the [funding guide](../wallet-rotation.md#funding-and-source-safety). | G1/G2 and the scoped publisher-consumer gate accepted exact authority, archive/reconfirmation and actual publication/actor/funding/close chains. Confirmed deposit plus FAILED/REFUNDED grants no replacement; late fulfillment retains original liabilities. Historical wider GF was accepted after F2; renewed execution G2/GF is recorded at F3. |
| C1 / catalog | Catalog C1/C2 and reviewed fixups implement checked expansion and faithful schema/request projection. Contracts live in [architecture](../architecture.md#catalog-configuration-and-execution), [OpenAPI inputs](../configuration.md#openapi-inputs-and-request-bodies), [source guidance](../agent-sources.md) and [provider maintenance](../provider-updates.md). | Combined G2 independently accepted through `4627301b`: all prior commits/fixups and ingestion→selection→MCP→route→payment consumers, iterative raw-candidate closure, override parity, partial readiness/strict completeness and consumer-visible omissions reviewed. Current and carried evidence is scoped below; no live provider qualification. |
| F2 / financial | Complete: I1 `0baed039`, I2 `8fa105fa` and reviewed fixups, I3 `7e5648be`. Contracts live in [SDK recovery](../zingo-integration.md) and [wallet reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy). | G1/G2/G3 and wider GF accepted through `68a93c0b`. Stale-only tips preserve observations; future completion metadata cannot block current SDK funds. Actual disclosed originals retain exact liabilities through cancellation, restart and genuine replacement. Execution G2/GF was renewed at F3. |
| C2 / catalog | Complete: registry D1 `8b62fa47`, D2 `1477e142` and independently reviewed fixups through `c0fa2c07`. Maintained contracts live in [registration/publication guidance](../agent-sources.md#registration-and-tool-invocation) and [catalog architecture](../architecture.md#catalog-configuration-and-execution). | G1 and combined G2 accepted exact input invalidation, acquired PH1 handles/current rendering, endpoint principal/payer/help authority, one captured publication generation and persistence-first serialized mutation. Both missing-add checks still precede possibly paid acquisition. Current discovery suite: 151 passed / 3 ignored, including all four D1 and four D2 cases; production compiler (28.99s), full all-target Clippy (3m58s), compatibility 30 and source-matched complexity review passed. No quantitative performance claim. |
| F3 / financial | T1 `6a4ea635`, T2 `dbf51979` and reviewed fixups implement lazy public parameter reuse and one retained blocking calculation/publication owner. Contracts live in [SDK boundaries](../zingo-integration.md#application-sdk-boundaries), [wallet authority](../wallet-rotation.md#funding-limits-and-fees), [runtime architecture](../architecture.md) and [lifecycle tests](../../tests/LIFECYCLE.md). | T2G2/GF independently accepted: original fixed/ranged quote authority, same-runtime MCP progress and cancellation/panic/runtime-drop/restart chains reviewed. Four proving and 53 lifecycle passes retain verified unchanged source scope across rebase; actual post-rebase treasury/sync/CLI consumers add 36 passes and one ignored consensus case. Missing results never authorize replay. |
| P1 / portable benefits | Retain `4fff946f`, `574e0b38`/`5251194e` portable ErrorKind classification and unfunded native control wiring, generic fixture improvements and strict binary vendor reconstruction. | Existing scoped evidence remains valid where unchanged. Windows runtime/native acceptance is deferred, not complete; historical partial results grant no storage, recovery or confinement claim. |
| Storage / separate follow-up | [macOS ACL and Unix storage](portability_fixes.md) remain unfinished outside this milestone. Windows P2/P3/P4 runtime, storage, supervision and native gates are deferred by user decision. | No shared-filesystem framework or native GS/lifecycle gate is required before the remaining important lanes. GR must retain the confirmed macOS gap and unresolved Unix namespace risk explicitly. |
| R0 / R9 design | Complete: independent consumer/query design separated naming/client-cache work from qualification budgets and lifecycle evidence before implementation. | Ordered commits and authority-preserving acceptance agreed; no monetary cache, new cap or measured speedup presumed. |
| R1 / R9 implementation | Complete: P1 `8584f839`, N1 `00d41932` and fixup `9bed86ef`, Q1 `af467549`, Q2 `b7890944`. Maintained contracts live in [network policy](../network-egress.md) and the [qualification reference](../../tests/live/INTEGRATION_REFERENCE.md#ownership-and-authority). | GN, GQ1, GQ2 and combined R9 independently accepted. Exact raw parser domains, atomic budgets, rollback/uncertain charges, canonical transport ownership and complete transaction-consistent lifecycle output remain intact. Required history scans and export guards remain; no constant-time or live qualification claim. |
| O1 / final integration | Assemble reviewed completed milestones, fixups and maintained-guide handoff. Windows work and optional qualification backends remain deferred/excluded; focused macOS/Unix storage follow-up stays unfinished separately. | Complete GR against the tracked review before GI and cleanup, then exact cleanup/final combined review. Unfinished required financial, catalog, registry or R9 milestones keep this orchestration active. |

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
qualification registry, shared TLS fixtures and owning guides. Any future separate
storage follow-up must coordinate financial and source/cache owners. Agree any ownership transfer;
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

Historical Windows evidence includes release/help/vendor/probe successes and
both namespace API candidates at `5218fef0`, alongside failed/unfinished native
all-target checks and earlier failed experiments. Preserve those results in Git
and private evidence; hosted-image changes prevent a sole-variable causal claim.
The user has now deferred Windows work and removed its dedicated execution tools.
No complete Windows privacy, recovery, supervision or power-loss acceptance is
claimed, and no further native rerun is required for this milestone. Portable
improvements and existing guarded fallbacks remain. The separate macOS ACL gap
and Unix initial-alias concern remain unfinished, not silently resolved.
No live-funded, consensus or live Tor qualification ran. Later changes reopen
affected required checks and combined concerns.

Catalog combined G2 is independently accepted through `4627301b`: the focused
catalog stage passed 69 cases, provider catalogs 41, integration preparation 251
with eight ignored, startup 19 with one ignored, and compatibility 30. Library
evidence is an independently checked union of 1,171 passes and 21 ignored cases
(1,192 listed), assembled from source-matched stages and corrected affected cases,
not a claim that one fresh complete library run produced that union. Current full
all-target Clippy passed; the production compiler pass retains its unchanged
production-source scope. Current whole-source complexity review is accepted.

Execution G2/financial GF is independently accepted: four proving and 53 lifecycle
passes were executed before the rebase. Their verified unchanged SDK, cryptographic,
calculation and accepted-work ownership scope carries forward; current Store
reporting readers are revalidated separately below. Actual post-rebase public consumers passed
36 cases (treasury 2, treasury_sync 27, wallet_workflow 7), with one consensus case
ignored. Source review preserves PH1 hint-sharing changes as advisory and distinct
from financial evidence identity and signing authority. Prior accepted runtime
results are not relabeled as executions on the new baseline. The known macOS
compact-unwind linker warning remains recorded.

The complete offline provider preset preserves all 22 original reviewed requests,
63 cases, 23 sources and the unchanged API budget; no live qualification is inferred.
All 61 metadata-derived default targets are now accounted for: 1,896 passes and
41 ignored cases, deduplicated by target and case. Of these, 52 targets executed
at `b7890944`; nine retain independently verified source-scoped evidence. The fresh
whole library passes 1,192 cases with 21 ignored, superseding the earlier library
unions. Current production/all-target warning gates, compatibility, formatting,
static provenance and whole-source complexity review pass. The selected managed
fixed/ranged SDK reader and codec each passed once on the current implementation.
Historical four proving and 53 lifecycle passes retain only their verified unchanged
cryptographic, calculation and accepted-work ownership scope; affected Status and
lifecycle readers were revalidated through current default consumers and that SDK
case. This is assembled validation, not one successful `make check` invocation;
final GR/GI remains open. Windows runtime is deferred; macOS ACL and Unix initial
namespace concerns remain a separate unfinished follow-up, not fixed by these gates.

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
- **GR, at orchestration/O1 before GI and removal of working review material:**
  have an independent read-only subagent review the **whole accumulated branch**
  against the [original review and maintainer disposition](x402_treazury_review.md).
  Supply user intent, settled S1/S3/S7/C1 decisions, agreed scope, exact comparison
  base and candidate SHA, all implementation/integration commits and fixups, and
  validation evidence/limitations. Inspect complete relevant consumers and failure
  paths across lanes, including changes inherited from `main`; reviewing the
  latest diff or relying on child approvals is insufficient. Reconcile every one
  of the 42 finding IDs with current code, tests, documentation and its disposition.
  Explicitly assess the deferred Windows scope and separate unfinished B8/macOS
  ACL/Unix namespace follow-up; do not claim all storage issues fixed.
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
