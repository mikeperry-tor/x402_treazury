# Provider compatibility, recommendations and regression policy

## Intent and status

Implementation in progress. Baseline established at `05280d66f903e38b45b521d4b973f5e58529db53`
(the plan-only commit after the original baseline). P1 audits and baseline checks
are underway; G1 must close before runtime implementation. The operator wants
provider defaults to expose supported functionality and newly arriving supported
tools, rather than silently impose a maintainer's recommended subset. Separate
recommendations from compatibility filtering, use recommendations explicitly in
`examples/deployments/privacy.toml`, and retain provider-specific tests and fixtures
only for concrete quirks or regressions. Review every bundled provider, normalize
exclusions and documentation, and establish enduring rules for future additions.
Collapse interchangeable GET/POST variants to POST in the default catalog before
collision naming, independently of recommendations. Migrate redundant method
suffixes and hardcoded references alongside the affected tool contracts.

This plan does not authorize funded calls, settlement qualification, wallet/state
changes or execution of the privacy deployment. Public documentation inspection
and ordinary offline development checks suffice for this work. Do not contact
paid endpoints to resolve catalog questions. If unsigned vendor documentation is
unavailable, distinguish unresolved evidence from a proven incompatibility.

## Baseline and owning guides

Starting revision: `6861f9687b689847065f350842d7c987eede6698`. Recheck the tree
and establish the implementation baseline before editing; preserve unrelated local
files and overrides. Initial read-only inspection found:

- 37 provider TOMLs, 19 with `include_operations`. Some restrict actual protocol
  support; others freeze compact subsets or even the entire captured inventory.
- Editorial source tag defaults also hide supported operations. Local projected
  catalogs can freeze availability independently of TOML allowlists.
- `tests/provider_catalogs.rs` freezes 971 generated tools across 26 providers;
  `tests/config.rs` freezes settings for 30. Those counts describe saved fixtures,
  not comprehensive coverage of current definitions or current vendor behavior.
- CAVEATS lacks dedicated entries for AgOSINT, Datakoot, HireScrape, NetIntel and
  TeleSINT. Its x402stock description disagrees with current exclusions. It mixes
  compatibility, recommendations, historical observations and deployment advice.
- The live-scope accounting test requires every TOML to occur in live coverage or
  exclusions, but the five providers above are absent. This is a static finding,
  not a test execution result; establish the actual focused-test baseline.
- Privacy's Claw402 source clears its inherited operation allowlist to select
  RootData. Review those actual request contracts before carrying that exception
  forward; the generic caveat about incomplete RootData schemas is insufficient.

Read and maintain [provider definitions](../../providers/README.md),
[provider caveats](../../providers/CAVEATS.md), [configuration](../configuration.md),
[catalog architecture](../architecture.md#catalog-configuration-and-execution),
[agent sources](../agent-sources.md), [testing](../testing.md),
[fixture maintenance](../../tests/fixtures/catalogs/README.md),
[development](../development.md) and
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline).
Consult the live runbook/reference before changing qualification accounting.
Method-pair review is part of this audit; no separate GET-preference workstream,
full-snapshot requirement or default operation allowlist is needed.

## Desired policy and design decisions

1. Reusable providers expose the supported catalog by default. Compactness,
   usefulness, cost estimates, overlap with other vendors and preference for a
   particular research task belong in recommendations or deployment selections.
   They are not compatibility blockers. Ordinary payment caps remain authoritative.
   The default method-twin rule below removes redundant GET variants while
   preserving supported functionality; it is separate from recommendations and
   compatibility exclusions.
2. Retain filters for identified unsupported authentication/payment mechanisms,
   unusable request contracts, unsupported output/lifecycle, fund-moving authority
   or wallet-bound state incompatible with the deployment. Explain the actual
   boundary. A diagnostic route is not intrinsically incompatible; hiding it only
   for convenience is curation. Unknown behavior is not evidence of support, but
   does not justify indefinitely freezing an otherwise supported API.
3. Prefer exclusions that describe concrete unsupported operations or families.
   Keep exact allowlists when a genuinely mixed API has only a verified payable
   island, such as Exa or OneShot, and document why exclusions cannot adequately
   express the support boundary. Do not mechanically invert every allowlist.
4. Normalize exclusion semantics: existing `exclude` is a segment-aware path
   subtree filter, not an exact METHOD/path exclusion. Add `exclude_operations`
   only where the audit demonstrates a real need to exclude one method while
   retaining another. If needed, use the existing operation syntax/validation and
   make exclusions win. Reuse paths/tags where they accurately express the rule;
   do not enumerate every currently supported operation as its complement.
5. Proposed minimal recommendation representation: provider/source
   `recommended_tools` using the existing generated-name selector syntax, plus an
   explicit source opt-in (working name `use_recommended_tools`). Final spelling
   and exact versus wildcard use are settled at the design gate. Recommendations
   alone do not select tools. Do not add a profile/workflow framework, scoring,
   a second catalog, automatic refresh service or recommendation-specific caps.
6. Proposed opt-in semantics: select recommendations from the compatible named
   inventory, then intersect ordinary source/listener filters; exclusions win.
   An omitted opt-in is false. An opted-in empty/missing or entirely stale set
   must not silently expand to all tools. Define explicit diagnostics and empty
   selection behavior using existing validation conventions. Unused stale advice
   must not make the full compatible catalog unavailable. Lists replace on
   inheritance; `[]` clears. Include help deliberately where useful.
   Resolve custom source prefixes, exact-name misses and unmatched wildcard
   diagnostics at G1; inherited advice must not silently select the wrong alias.
7. Inventory inspection must show which tools are recommended and allow inspection
   of the full compatible inventory even when a deployment opts into curation.
   `config show` reports effective metadata/opt-in and origins offline. Keep MCP
   schemas and vendor descriptions intact; recommendation markers belong in
   operator inspection, not appended prose on every tool.
8. Recommendation filtering must share existing selection machinery and must not
   change owner, payer, binding or invocation identity. No new listener setting is
   required unless source-level opt-in proves inadequate for a concrete consumer.
   Preserve independent source aliases and listener narrowing. Static provider
   recommendations do not automatically apply to agent-imported specs or raw
   calls. Existing raw-call authority is separate from catalog visibility; do not
   describe provider filters as a universal security boundary.
9. Provider-specific regression storage requires an identified quirk, historical
   failure or authored transformation. Use the smallest faithful fixture and
   meaningful assertions. Prefer synthetic shared tests for generic parsing,
   composition, filters and routing. No mandatory per-provider golden, full
   settings snapshot, exact inventory count or fixture for ordinary vendor prose.
   Full fixtures remain justified only when a specific contract needs the full
   inventory; document that need. Cheap TOML validation can cover all definitions
   without freezing their settings or contacting vendors.
10. Runtime local catalogs are production inputs, not disposable test fixtures.
    Keep necessary projections, authored schemas and payment compatibility
    evidence. Inspect every artifact's consumers before pruning; retain financial
    records and qualification authority/evidence. A new provider need not become
    a paid qualification case or require an artificial exclusion record merely
    because it exists. Qualification must still identify its actual selected
    scope, reviewed requests, caps and unobserved outcomes explicitly.

## Default POST preference and method-pair review

`prefer_post` defaults to true when absent. It is an internal provider/source
escape hatch: set false only when an actual integration needs both variants.
Do not add it to ordinary provider files, example boilerplate or advertised CLI
options. Document its exact behavior in contributor/configuration guidance so
future repairs can use it; hidden does not mean unvalidated or undocumented.
Omission inherits an explicit provider value under ordinary composition rules;
otherwise the default is true. A source may explicitly override either value.

After compatibility/path/operation/tag selection, group surviving operations by
their exact original path. Where GET and POST both survive, retain POST and drop
GET before collision counting and generated naming. Do not pair different paths,
infer aliases from names, change wire paths/methods or suppress other methods.
A surviving GET remains available when POST is absent or excluded. Existing
post-naming tool filters and recommendation selection narrow this inventory;
they do not cause GET to reappear when POST is later filtered out. To select the
GET twin deliberately, disable preference before applying name filters.

Use this default consistently in generic catalog construction, including imported
OpenAPI catalogs. Provider/source TOML can opt out; source registration does not
gain an agent-controlled preference argument. Review fixed/embedded catalogs and
pre-existing hardcoded tools/references too: apply the same policy where actual
twins exist, and migrate hardcoded method-suffixed names to the resulting canonical
names. A raw call still sends its explicitly requested method. Do not mechanically
strip `_get`/`_post` from unrelated names: distinct remaining methods, paths or
operations may still require disambiguation. The goal is to remove redundant
suffixes throughout the supported toolset, not to erase meaningful distinctions.

This is a simple structural selection rule, not a runtime equivalence detector.
The provider audit must inspect same-path pairs and preserve meaningful differences
with a narrow opt-out where necessary. Do not build automatic schema-comparison
machinery or long preferred-operation lists. If a concrete integration needs a
per-pair exception beyond the source escape hatch, resolve the smallest mechanism
at G1 rather than adding a new selector language speculatively. Vendor canonical
guidance informs that review but does not automatically override the POST default
for otherwise interchangeable pairs.

For each pair compare vendor request contracts and applicable documentation:

| Concern | Review evidence |
| --- | --- |
| Inputs | Names/types/defaults, required values, query versus JSON-body mapping, array serialization, unions and supported input sizes |
| Functionality | Actual semantics, side effects, caching behavior and any capability offered by only one variant |
| Billing/results | Documented billing differences, response status/content types, schemas, envelopes and image mappings; equal hints do not establish equal charges |
| Disclosure | Sensitive/large inputs in URLs versus request bodies; POST avoids URL arguments but does not guarantee privacy |
| Consumers | Generated names, collision siblings, overrides, response mappings, recommendations, authored/hardcoded selectors, examples and wallet/group bindings |

Use existing evidence and unsigned public documentation inspection. If equivalence
is uncertain, preserve both through the escape hatch and document the precise
uncertainty; do not discard one variant based only on a similar description.
GenuineGood's array serialization and NetIntel's canonical POST/body handling are
concrete starting points, not proof of universal vendor equivalence.

Method preference grants no retry authority. Follow the maintained
[unsigned x402 contract](../architecture.md#unsigned-x402-requests-and-post-retries):
GET and POST have the same unsigned retry eligibility, with no additional
business-side-effect gate for POST in these x402 pathways. Preserve the existing
eligible-wallet traversal and narrowly recognized structured-insufficient-funds
repetition, with every authorization potentially chargeable. Generic signed
failures, timeouts, lost responses and output-conversion failures grant no replay
for either method. No automatic POST-to-GET fallback, re-signing, method-specific
payment limits or method switch to compensate for payer selection belongs here.
GET-only startup pricing probes are a separate policy, not a reason to prefer GET.

## Ordered delivery

The identifiers distinguish three levels:

- **P1–P7 are ordered delivery phases**, not individual commits.
- **C identifiers are recommended commit boundaries within a phase.** Each table
  names a coherent change, its consumers and acceptance evidence. Use these IDs
  in commit/review descriptions so delivered SHAs can be mapped to the plan.
- **G1–G4 are wider cumulative review gates** after the specified final commits.
  They examine all preceding work and fixups, not just the latest phase or SHA.

Commit boundaries may be refined when the audit supplies concrete file ownership.
Before starting an affected change, update this plan with its ordered child IDs
(for example C3.2a/C3.2b), exact provider/files/consumers and acceptance evidence.
Keep existing IDs stable and state which final child closes the gate. A split is
justified by coherent ownership/behavior, not patch size alone. Do not split a
behavior change from required replacement/recovery paths, name migrations, focused
tests or affected permanent documentation. Every exact commit and fixup receives
independent review; a wider gate does not replace that review.

The tables are the recommended sequence, not a requirement to manufacture a
commit where there is no change. Mark conditional boundaries unnecessary with
the audit rationale. Keep sources/executables coherent and focused checks passing
at each implementation boundary. If an old snapshot obstructs a deliberate change,
replace/remove its affected coverage with justified focused tests in that same
commit; P5 prunes remaining redundancy rather than deferring required repairs.
Do not postpone documentation or replacement coverage until cleanup. Audit work
products and generated before/after inventories belong under ignored `target/`,
not in a new committed audit ledger. Durable conclusions belong in owning guides.

### P1 — Establish policy and complete the provider/consumer audit

| Commit | Coherent scope and consumers | Acceptance evidence |
| --- | --- | --- |
| C1.1 | Establish desired policy in providers/README, docs/testing and concise AGENTS rules/links; distinguish future behavior from current implementation. | Policy/link consistency; no claim that planned features already exist. |
| C1.2 | Complete individual-provider and method-pair audit; commit durable compatibility/naming findings in CAVEATS/provider guidance. | Every definition classified; method exceptions, omitted workflows and complete name/consumer migration map reviewed. |
| C1.3 | Complete test/artifact consumer audit; document retained quirk purposes and test-policy implications in owning guidance. | Every candidate artifact has a consumer/disposition; actual focused baseline failures distinguished from introduced failures. |

The two audit concerns may be investigated independently in parallel, but their
commits follow the order above and G1 waits for both complete results. Split C1.2
or C1.3 into declared coherent batches if needed; do not commit an audit ledger.

Independently divide provider compatibility and test/artifact consumer analysis.
Inspect all 37 definitions, local catalog generators and complete relevant
consumers, not only the 19 operation lists. Compare current public vendor request
and payment documentation where reachable; record source URLs, capture basis and
uncertainty. A captured fixture does not establish current upstream support.

For each provider classify every path/operation/tag/tool restriction as a concrete
compatibility boundary or recommendation; examine omitted operations and complete
research/result chains. Inspect naming anchors, duplicate methods/aliases, schema
repairs, overrides, prices/probes, response mappings, auth and lifecycle. Do not
remove `include` anchors merely because they resemble an allowlist: they also
control naming. Review privacy's selections and effective wallet bindings.
Inventory same-path GET/POST pairs across generic, embedded and hardcoded tool
definitions; apply the method-pair review above and identify narrow exceptions.
Record all existing method-suffixed names and consumers that the new default
would rename, including exact selectors in integration presets and tests.

Map fixtures/goldens/settings snapshots to their actual consumers and concrete
regression purpose, including discovery, media, payments, startup, qualification,
snapshot utilities and examples. Establish focused baseline failures separately
from introduced failures. Commit the desired policy to providers/README,
docs/testing and concise AGENTS rules/guide links, with audit conclusions in
CAVEATS. Clearly distinguish policy from runtime features not yet implemented.

**Gate G1, after C1.3 (P1 complete):** an independent design reviewer resolves the representation,
filter ordering, inspection, opt-in/empty/stale behavior and exact-exclusion need.
Confirm default-on POST preference, internal opt-out inheritance, method-pair
exceptions, generic/dynamic/embedded scope and the complete suffix migration map.
Assign every provider to one disjoint C3 group below and finalize any child commit
boundaries, affected consumers and evidence before implementation begins.
Review the complete audit scope and simplified alternatives. Exit with every
provider/artifact classified, unknowns explicit and no material design question
left for implementation to guess. Unavailable evidence may leave a narrow,
documented restriction; it must not produce a claim of successful qualification.

### P2 — Implement POST preference, recommendations and necessary exclusions

| Commit | Coherent scope and consumers | Acceptance evidence |
| --- | --- | --- |
| C2.1, conditional | Implement exact operation exclusions only if G1 establishes a need; include Config validation/composition, shared catalog matching and owning docs. | Synthetic method-specific exclusion/precedence tests; ordinary path/tag behavior preserved. If unnecessary, record that conclusion and start with C2.2. |
| C2.2 | Implement default-on POST preference and hidden opt-out across generic/dynamic/embedded paths, together with all required existing name, hardcoded reference, override and selector migrations, provider exceptions, tests and docs. | Default/opt-out/inheritance, compatibility ordering, naming, actual routing and publication tests; complete migration inventory with no broken checked-in consumer. |
| C2.3 | Implement advisory recommendations, explicit opt-in and inspection using final names; include composition, static-source/listener consumers and docs. | Focused metadata/selection/stale/empty/help/isolation tests; full inspection available; no name, payer or raw-call authority change. |

This order gives recommendation support the canonical names from C2.2. Do not
split metadata from its necessary selection/inspection semantics, or POST selection
from its required consumer migrations. If C2.2 cannot form one coherent commit,
revisit G1 and specify complete opt-in preparation boundaries before changing the
default; do not leave a broken intermediate toolset for P3/P4 to repair.

Implement the G1 design through Config, TOML composition/validation, standalone
CLI and meta-config inspection, catalog generation and deployment selection.
Implement exact operation exclusions here only if justified at G1. Preserve tool
names when only recommendation selection changes by applying it after naming.
Do not move compatibility filters across naming stages without separate evidence.
Implement default-true `prefer_post` after compatibility selection and before
collision naming. In C2.2, migrate every affected existing generic,
embedded and hardcoded name/override/selector/reference needed for a coherent
working toolset. Do not leave the required name migration until P3/P4. Document
intentional public-name changes and how operators can update exact selections;
preserve user files and do not create hidden legacy aliases.

Inspect serving/deferred publication, strict checks, routing, cache warming,
pricing probes, catalog evidence/qualification and static discovery/search/call
consumers so they agree on effective selections and cannot restore filtered tools.
Full-inventory inspection is inspection only; it must not widen serving or initiate
extra pricing probes. Keep metadata/config output free of secrets and logs on stderr.
Distinguish the full default catalog (which collapses twins) from the pre-preference
method inventory. Operator inspection with preference explicitly disabled must
make both variants inspectable, with their actual collision-resolved names.

Use small synthetic tests for default full visibility, opt-in, list replacement,
clearing, additional filters, help, stale advice, empty selection, aliases and
listener isolation. Assert recommendation changes do not rename siblings or alter
binding/payer identity. Test exact method exclusion once if introduced. Update
configuration/architecture and relevant agent-source guidance with each owning
C2 commit, rather than waiting for the phase's last commit.
Add shared synthetic tests for omitted/default-true preference, explicit false and
inheritance, same-path pairing, GET-only/POST-only/different-path operations, POST
removed by compatibility filters, other methods and naming collisions. Verify
post-naming filters do not resurrect GET, recommendation advice uses final names,
and dynamic/static publication and actual routing agree. Retain focused tests for
real provider exceptions and migrated hardcoded contracts, not full inventories.

**Gate G2, after C2.3 (P2 complete):** independently review P1/P2 plus every fixup and full
config → naming → filtering → publication → invocation/inspection call chains.
Compare CogC/CycC with the agreed baseline, including unchanged hotspots. Exit
with one coherent selection model, no hidden catalog expansion, no advisory-data
availability gate and focused passing tests. Resolve recurring findings before P3.
Verify the complete suffix migration and method selection/inspection behavior,
including unchanged financial retry semantics and dynamic-source identity.

### P3 — Normalize provider defaults and install recommendations

| Commit group | Coherent scope and consumers | Acceptance evidence |
| --- | --- | --- |
| C3.1 | Ordinary live-spec providers with straightforward compatibility exclusions; recommendation metadata, affected selectors/overrides and provider docs together. | Reviewed before/after inventories and focused generic/quirk regressions; supported new routes remain visible. |
| C3.2 | Mixed-protocol, multi-workflow and method-exception live providers requiring explicit support-boundary decisions. | Verified payable/lifecycle boundaries, justified exclusions/opt-outs, complete method/name migrations and meaningful negative tests. |
| C3.3 | Authored/projected/enriched local catalogs, their generators and companions, with all affected consumers and docs. | Runtime inputs preserved; omitted operations and transformations reviewed; focused projection/enrichment/composition regressions. |

G1 assigns exact files once, based on audit evidence; companions and shared
generator changes stay with their owning group. Each group is a recommended
commit unless it contains independently reviewable provider batches. Declare
ordered children such as C3.2a/C3.2b before edits, including shared consumers and
ownership. Complete C3.1, then C3.2, then C3.3 before P4. All required changes to
privacy or another checked-in selector caused by a batch occur in that batch;
intentional recommendation adoption remains in C4.1.

Apply the audited classification to all providers, remove editorial include/tag
defaults, normalize necessary exclusions and add useful recommendation metadata.
No forced recommendation list for a provider whose whole supported catalog is the
appropriate recommendation; define that case explicitly without enumerating it
merely to pin today's routes. Preserve required local projections and enrichment;
remove curated tags/generator assumptions only when their replacement is complete.

Produce full before/after inventories as review evidence, not permanent goldens.
Filter removal changes collision counts: inspect affected sibling names, overrides,
response mappings and every checked-in selector. Preserve established names where
possible; document unavoidable renames and update consumers in the same commit.
Do not add hidden aliases, method substitutions or alter user local files.
Recheck P2's migrated names as P3 broadens compatibility inventories, and update
any additional collision-driven renames together with their consumers. Use the
hidden preference override only for documented real method-pair differences;
do not populate true values in TOMLs or recreate the default with allowlists.
Retain/add focused tests for newly exposed boundaries and necessary exclusions;
remove obsolete suppression assertions together with the behavior they asserted.
Update each affected CAVEATS entry and provider README in its owning C3 commit.
Every declared batch includes explicit file ownership, complete consumer migrations
and exact-SHA review; gate review still examines the combined implementation.

### P4 — Consume recommendations in the privacy deployment

| Commit | Coherent scope and consumers | Acceptance evidence |
| --- | --- | --- |
| C4.1 | Adopt recommendations deliberately in privacy.toml, preserving narrower purpose selections and updating example guidance/tests. | Offline before/after inventories for all listeners, intended deltas explained, complete research chains and unchanged wallet/cap/network authority. |

Use the new opt-in in privacy.toml wherever it fits the listener's purpose; retain
explicit purpose-specific selections and additional exclusions where necessary.
Do not replace company/social subsets with a broader generic recommendation just
to demonstrate adoption. Ensure at least one suitable source actually consumes
recommendations. Review full result/dependency chains and resolve the Claw402
RootData contracts before removing its allowlist-clearing workaround.

Compare effective inventories for web/social/company/toolbox and explain intended
deltas. Preserve wallet ownership, payment caps, network/Tor settings, management
authority and purpose restrictions. Use deterministic offline catalog inputs and
focused composition/publication tests, not credentials, funding or a live serve.
Replace any fixture introduced solely for this check with synthetic contracts
unless it independently meets the regression policy. Update comments/guidance.

**Gate G3, after C4.1 (P4 complete):** independently review all P1–P4 commits/fixups and complete
provider → source alias → listener → call chains. Include local generators, naming,
overrides, pricing and privacy purpose/ownership. Verify the before/after deltas
and complexity evidence. Exit with all current providers accounted for, no
unexplained restriction/rename and no unresolved material exposure or ownership
finding. Decide whether repeated findings require a wider targeted audit.

### P5 — Normalize test policy and prune redundant artifacts

| Commit | Coherent scope and consumers | Acceptance evidence |
| --- | --- | --- |
| C5.1 | Remove universal provider-to-paid-scope coupling; minimize the offline qualification preset test's dependency on the golden case index, retaining necessary inputs until consumers change. | Qualification scope/request/authority tests pass; no provider is newly claimed qualified; no dangling fixture dependency. |
| C5.2 | Replace remaining blanket catalog/settings snapshots with justified focused/shared tests; remove their unneeded output goldens/index and update test/count guidance together. | Replacement invariants execute; generic TOML validation works without freezing settings; all consumers of removed files are migrated. |
| C5.3 | Prune residual provider input fixtures, duplicate tests and orphan utilities by named regression responsibility; maintain provenance/refresh guidance for retained inputs. | Consumer search and focused retained suites; runtime local catalogs, payment evidence and financial records preserved. |

Do not group pruning solely by vendor or file size. If C5.2/C5.3 need smaller
commits, declare children by shared test responsibility and include each fixture's
necessary test replacement, consumer migration and documentation in that child.
Earlier C2/C3 commits already own any coverage repair their behavior changes need.

Replace broad catalog/settings snapshots with focused regressions and generic
synthetic coverage identified in P1. Prune duplicated assertions and unused fixture
files only after checking all consumers; do not merely regenerate changed goldens.
Retain full inventories only with an explicit unique need. Document retained
provider-specific fixtures by failure/quirk, provenance, consumers and refresh
method in fixture maintenance; preserve complete inputs needed to reproduce bugs.

Decouple universal provider existence from paid-scope bookkeeping while retaining
qualification preset/request/authority validation. Update live coverage docs and
tests together; consult the live guides and verify scope remains explicit. Repair
known baseline inconsistencies by deleting unnecessary coupling rather than adding
boilerplate coverage rows. Preserve actual evidence and never promote untested
providers to qualified. In particular, `examples/live_integration/presets.rs`
loads `tests/fixtures/catalogs/cases.json` inside its offline test module: review
and minimize that test dependency before removing the golden case index. Retain
only justified regression inputs or use generic synthetic alternatives; this is
not a runtime preset-construction dependency and needs no production framework.
Preserve deliberately read-only directory scope where it is an actual contract.
Remove orphan helpers/generators only after consumer checks.
Run retained focused tests and verify nonzero execution, then the appropriate
combined config/catalog/discovery/media/payment/qualification consumers. Update
docs/development, docs/testing and fixture README counts/claims in the C5 commit
that changes their subject; do not defer them to C5.3 or P6.

### P6 — Complete maintained documentation and final integration

| Commit | Coherent scope and consumers | Acceptance evidence |
| --- | --- | --- |
| C6.1 | Reconcile CAVEATS/provider index across all completed batches, removing remaining duplication/stale counts and checking every current definition. | Complete index/caveat coverage and accurate evidence limits; no invented quirks or deferred documentation from earlier changes. |
| C6.2 | Reconcile cross-guide policy, remaining plan/example references and AGENTS read-first links; close remaining handoff inconsistencies. | Link/reference checks, permanent rules agree with final implementation, final required checks and source-matched complexity evidence available for G4. |

P6 is a final consistency pass. Each earlier implementation commit already
updates its owning permanent documentation. If a reconciliation finds a material
implementation defect, fix/review it against the introducing SHA and reopen the
affected gate rather than hiding it in a documentation commit.

Normalize CAVEATS in place: concise entries for every provider with a real caveat;
explicitly identify providers with no known integration-specific issue in the
provider index rather than inventing caveats. Separate restrictions/reasons,
schema/protocol quirks, source/refresh basis and evidence limitations. Keep
recommendation rationale brief; deployment-specific shopping lists belong in
example guidance, generic transport/payment policy in its owning guide. Remove
stale route counts, duplicated policy and obsolete plan references. Verify all 37
definitions and any additions during implementation are accounted for.

Finish providers/README, configuration, architecture, testing/development, fixture
maintenance, relevant live/agent-source references and privacy comments. Reconcile
other plans/references that prescribe old snapshot or allowlist defaults. Update
AGENTS's provider row/links and concise enduring rules: supported-by-default with
default POST twin collapse before naming, reasoned compatibility restrictions,
explicit recommendation opt-in and quirks-only provider-specific regression storage.
Keep detailed method review and opt-out semantics in configuration/provider guidance.

**Gate G4, after C6.2 (P6 complete):** independent final integration review of all plan commits and
fixups with surrounding consumers, not only the last diff. Verify policy claims,
all provider classifications, fixture deletions, default/new-route visibility,
explicit opt-in, privacy selections, qualification evidence and full call chains.
Analyze causes of all fixups for recurring assumptions and missing regression
coverage; simplify or widen the audit where warranted. Review complete CogC/CycC
evidence and resolve every material finding before declaring implementation done.

### P7 — Remove the completed plan

| Commit | Coherent scope and consumers | Acceptance evidence |
| --- | --- | --- |
| C7.1 | Remove this completed plan after permanent handoff verification; update links and retain only concrete unfinished work in an appropriate focused plan. | Independent exact cleanup-SHA review and final combined-branch review after removal; affected gates revalidated for any material change. |

After G4, verify permanent policy destinations and AGENTS links, carry only concrete
unfinished work into an appropriate focused plan, fix references and delete this
file. Do not retain a completed-plan archive. Independently review the exact cleanup
SHA and final combined branch after removal; any substantive cleanup change
reopens its affected gate and requires revalidation.

## Review and validation protocol

The coordinator owns integration. Use read-only independent reviewers by default;
assign distinct audit concerns, explicit file ownership for delegated edits, and
freeze source/executables during validation. Do not run competing builds.

For every exact committed SHA, give a non-author reviewer the user intent, scope,
baseline, relevant consumers and actual validation evidence. Resolve material
findings before building on the decision. Commit fixes with
`git commit --fixup=<introducing-SHA>` and independently review each exact fixup;
preserve unsquashed history through final integration. Every named gate reviews
all preceding plan commits/fixups alongside complete call chains. Later material
changes reopen affected concerns; prior approval is not an exemption.

For substantive Rust changes and architecture gates, use
`python3 scripts/complexity.py --base <implementation-baseline>` and inspect full
JSON/CSV, warnings and relevant unchanged high-risk functions under the documented
CogC/CycC discipline. Verify source matches; reuse valid unchanged evidence rather
than rerun mechanically. Evaluate removal of responsibilities and concrete simpler
representations before retaining hotspots; helper extraction is not a resolution.

Use `scripts/zcash.sh` for focused default-feature tests and Clippy, and run the
toolchain check before any plain Cargo command. Choose actual test targets after
mapping consumers; likely provider_catalogs/config plus relevant catalog,
deployment, discovery and qualification tests. Verify expected tests execute and
forbidden side effects remain absent. Run formatting and required repository
checks at final integration; broader reruns after a localized fix require new
changes or unresolved concerns. No reduced-feature run, Docker consensus test or
funded qualification is included. Check documentation links, references and diff
consistency for documentation-only commits; no Rust metrics/build is needed merely
to create or revise this plan.

## Wider work and completion criteria

Included cleanup: naming/override migrations, hidden curation in tags and local
generators, stale live-scope coupling and offline preset-test fixture dependencies,
redundant fixture documentation/counts,
redundant method variants/suffixes and privacy's allowlist-clearing escape.
These are necessary consumers of the requested policy, not separate subsystems.

Do not expand this into automatic vendor drift monitoring, a profile registry,
provider-specific execution adapters, durable async polling, SIWX support, new
payment mechanisms or broader raw-call authorization changes. If the audit reveals
one is needed to expose a blocked workflow, retain/document the precise boundary
and propose separate implementation with evidence. Consider a broader naming
redesign only if unavoidable collision churn cannot be handled locally; stop and
revise the affected design gate before undertaking it.

Completion requires the individual-provider audit, supported default visibility,
documented necessary exclusions, usable explicit recommendations adopted in privacy,
default-on POST twin collapse and complete name migration, focused justified
fixtures, maintained CAVEATS/index and linked enduring policy.
No unresolved material review finding may be deferred as cleanup.
