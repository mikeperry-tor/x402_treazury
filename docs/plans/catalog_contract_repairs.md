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
Pending: P1, validation, exact-SHA/combined G1, and P2 cleanup/review.
