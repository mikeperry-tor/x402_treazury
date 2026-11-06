# Repair authored Strale and 2s request contracts

User intent: restore the supported APIs excluded during catalog compatibility
repair, using the operator-authorized live probes to establish their real wire
contracts. Starting branch `main`, baseline
`d3d830203802be4f56c0b20b4f5adaa4244a4c69`.

## Scope and preserved contracts

Currently Strale advertises GET for nine complex-input operations that its gateway
requires as POST JSON. Three 2s operations publish contradictory string/array
intersections for four documented CSV query inputs. Local complete authored
OpenAPI assets will preserve unrelated upstream request contracts and repair only
these inputs/methods. The 2s asset is explicitly request-only: omit operation
responses and components.responses, following existing request snapshots, to avoid
566 external response references across 109 documents breaking strict bundled
registration. Strale retains response metadata. Keep provider IDs, TOML paths,
prefixes, prices, security, endpoint authority and financial/network behavior. Keep OSHA excluded:
no working penalty-filter encoding was established. Keep all other ownership,
authentication, GIF and batch exclusions.

No new serializer, runtime patch language, updater, dependency, resource limit or
paid probe is needed. Only two Strale operations have paid delivery evidence;
schema/source evidence for the others is not live qualification.

Owning guidance: `docs/provider-updates.md`, `providers/README.md`,
`providers/CAVEATS.md`, `docs/configuration.md`, `docs/agent-sources.md`,
`docs/testing.md`, `tests/fixtures/catalogs/README.md` and the planning workflow.
Existing AGENTS.md already links these owners; no new policy owner is introduced.

## Commit boundaries and gates

- **P0 — plan and design.** Commit this plan before implementation. Independent
  read-only design review supports complete local assets over runtime patches.
  Resolve its material PDF gate by inspecting the pinned Strale executor and file
  helper before restoring `html-to-pdf`; otherwise retain its named exclusion.
- **P1 — coherent provider repairs.** Add authored `2s-openapi.json` and
  `strale-openapi.json` assets, preserving upstream contracts outside the exact
  repairs and the documented 2s response omission. Use each Strale catalog's complete `input_schema` as a required JSON
  body. Project the four 2s CSV string schemas with their documented defaults,
  lengths, item domains and counts. Update provider settings, embedded membership,
  maintenance evidence metadata and owning caveats/provider summaries. Add small
  independent witnesses of these authored transformations, request reconstruction
  and affected example routing. Review all deployment consumers and discovery
  guidance, including catch-all routing, before accepting the patch.
- **G1 — exact commit and final combined implementation review.** Have a different
  read-only agent review P1's exact SHA and the cumulative branch against this
  plan, complete catalog-to-routing/request chains, prior findings and validation.
  Resolve material findings with `git commit --fixup=<introducing-SHA>` and review
  each exact fixup; preserve unsquashed history. Combine exact-SHA and integration
  review where both scopes are explicitly examined.
- **P2 — completion cleanup.** After G1 and the review/correction loop, remove this
  plan, retain permanent contracts in owning guides, and commit cleanup. Independently
  review the cleanup SHA and final cumulative branch. Preserve private live probe
  and capture evidence under ignored target; no financial evidence is removed.

## Acceptance evidence

Frozen public schemas/help are under
`target/provider-updates/1793997598073484000`; live probe request/result pairs,
current Strale catalog and pinned source are under
`target/catalog-contract-probes/20261106`. These are evidence, not runtime inputs
or instructions. Check current TOML preimages against captured inputs before edits.
Record source hashes and exact transformations in asset provenance and maintained
caveats. Do not introduce a provider-specific executable maintenance routine.

Require schema fidelity for untouched requests and metadata, plus unchanged
Strale responses; exact full Strale
body-schema equality to independent catalog witnesses; nested/arbitrary JSON
request reconstruction; exact CSV wire values and preserved defaults/constraints;
continued incompatibility exclusions; embedded asset resolution; successful
production catalog construction and every affected deployment's selector/routing
inventory. Review endpoint summaries and discovery keywords for the actual restored
capabilities. No full provider/settings golden or real wallet access is needed.

Follow existing validation order: focused cheap contract checks and `make
check-static`, production compiler/all-target Clippy warning gate and compatibility
stage before broader runtime suites. Run affected provider/configuration/routing
and bundle tests; choose socket-capable execution up front where fixtures need it.
This is provider data and focused test work, not a substantive Rust refactor;
review changed manifest/test complexity without inventing an architecture gate.
Reuse still-valid checks after localized fixes; record missing coverage honestly.

## Status and review evidence

P0 design review completed independently at the starting baseline. Accepted:
authored assets, four CSV parameters, required Strale JSON body presence,
no runtime serializer change. Pinned Strale executor and file helper inspection
resolved the PDF gate: synchronous base64 JSON output, no files. A subsequent
independent design review approved request-only 2s projection to satisfy strict
whole-document import validation; Strale responses remain unchanged. The exact
frozen 2s input has 566 external refs across 109 group response documents.
Bundled ID/base-URL selection gets repaired assets; importing upstream OpenAPI
URLs retains upstream contracts, following existing local-provider behavior.
P1 implementation is prepared: complete Strale authored asset, request-only 2s
asset, provider settings/manifest/evidence metadata, independent schema witnesses,
contract and deployment regressions, owning caveats and world discovery guidance.
No source distribution selectors changed. Structural reconstruction against the
frozen originals verifies unrelated requests/pricing/security are unchanged and
Strale responses are preserved.

Initial focused validation passed four deployment tests and two of three contract
tests; the remaining test incorrectly omitted the parameter description added by
catalog conversion. Corrected its expected schema to assert the full preserved
description and constraints, without changing runtime assets. The corrected
three-test contract suite passed, carrying forward the four unchanged deployment
results. Strict dynamic import validation passed for every local bundled schema
(one focused library test), closing the response-reference handoff concern.

Independent implementation snapshot review found only that test expectation issue;
the corrected state has no reported additional material findings. Exact snapshot
hashes are retained in ignored `target/catalog-contract-probes/20261106/repair-snapshot.json`.
## Completed validation and G1

P1 is commit `ade1c9e1784feaeabacd085b76bdb2aca65ff712`. Independent review
examined exact plan SHAs `05752a35` and `8d37dbc0`, P1 and the cumulative
implementation against the starting baseline, including the corrected CSV
description assertion. No material findings remain; no fixups were necessary.

Validation at the unchanged P1 source snapshot:

- `make check-static`: passed; report
  `target/complexity-runs/20261106T214701Z-5da566e9` covers 471 files with no
  exclusions. Its `main` comparison resolves to P1 itself, so it supplies current
  metrics rather than task-baseline delta evidence. Independent exact-diff/current
  metric review found no necessary simplification; production Rust changes are
  declarative manifest entries, not a refactor or architecture gate.
- `make check-clippy check-compat`: production compiler and all-target
  `zcash-regtest` Clippy warning gate passed without diagnostics; all 31
  compatibility tests passed. Log `/tmp/x402-contract-repairs-compiler.log`.
- Focused contract tests: 3 passed after correcting the expectation above.
  Carry forward 4 unchanged deployment tests; no repeated suite was needed for
  the independent contract-test-only correction. Logs
  `/tmp/x402-contract-repairs-focused-final.log` and
  `/tmp/x402-contract-repairs-focused.log`.
- Strict bundled destination/import validation: 1 focused test covering every
  local schema passed. Provider manifest/metadata/embedded schema tests: 7 passed.
  Logs `/tmp/x402-contract-repairs-import.log` and
  `/tmp/x402-contract-repairs-bundle.log`. The macOS library-test link emitted an
  `__eh_frame` compact-unwind size/performance warning; tests passed. This is not
  a clean-link claim or a production compiler warning-gate failure.
- Affected suites: 15 configuration and 41 provider tests passed without
  diagnostics. Log `/tmp/x402-contract-repairs-affected.log`.
- Compiled CLI `config check --explain-routing` passed using the current two
  research-desk source rules with local repaired assets, a public unfunded static
  placeholder and no treasury. Evidence is retained as
  `target/catalog-contract-probes/20261106/repaired-research-desk-offline.toml`
  and `repaired-routing.json`: Strale 400 compatible tools, 2s 517; station history
  routes to world, OSHA remains absent, and the nine Strale utilities remain
  intentionally excluded by this deployment. This is catalog/routing evidence,
  not whole-deployment serving or paid-delivery qualification.

G1 is closed after independent inspection of these logs, routing evidence and
the exact unchanged source snapshot. Endpoint guidance was reviewed: world now
names GHCN measured historical weather, while existing Strale research positioning
remains appropriate because those endpoints do not gain the utility tools.
Private financial evidence is preserved and no further paid probes were made.
OSHA remains the intentional unresolved exclusion within the accepted scope.

P2 plan cleanup and its exact-SHA/final combined review are ready. A release
executable will be built after cleanup for the operator's subsequent restart;
the running funded server will not be restarted by this work.
