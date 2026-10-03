# Provider contract fixtures

The 14 legacy tool catalogs were captured from the independent Python implementation
before its retirement and verified against Rust. Together with the reviewed Rust
x402-list, Exa, OneShot, StableEnrich, Agent402 and AgentUtility catalogs, the 20 fixtures pin 834 tool names, full descriptions, schemas,
methods, paths, parameter routes and help URLs. Settings snapshots cover all 24
bundled providers, including those without offline specs.
Local spec paths are relative to the repository root.

Run `cargo test --test provider_catalogs --test config` from the Rust project.
These tests need no Python, credentials or external services. Input OpenAPI
snapshots are under `tests/fixtures/`; the five curated catalogs are in `providers/`.

When changing providers, review the changed input, the generated tool contract,
and the expected output together. Do not automatically accept generated snapshots
just to make tests pass. Schema/routing edge cases also have targeted Rust tests;
these snapshots preserve the selected provider contracts, not every old behavior.

The Exa request fixture was captured from `https://api.exa.ai/openapi.json` on
2026-10-03 with the Rust `snapshot_spec` utility. It retains all 64 operations so
the exact two-operation allowlist is tested against the real excluded inventory.
Only response documentation is removed. Exa's root `oneOf` for `ids` versus `urls`
is preserved; six Locus tools likewise retain their existing `anyOf` address
alternatives after flattening. Neither change adds or rewrites HTTP parameters.

The OneShot fixture was captured from `https://win.oneshotagent.com/openapi.json`
on 2026-10-03 using `snapshot_spec`. It retains all 147 operations (omitting
response documentation) to pin exclusions as well as synchronous search. The
selected request schema is unchanged; no OneShot-specific schema repair is needed.

The StableEnrich fixture was captured from `https://stableenrich.dev/openapi.json`
on 2026-10-03 using `snapshot_spec`. All 38 operations are retained to verify the
32-operation allowlist, omitted async workflows, tag selection and request
routing. The additional help tool reads the vendor's unified `llms.txt` lazily.

The Agent402 fixture was captured from `https://agent402.tools/openapi.json` on
2026-10-03 using `snapshot_spec`. It retains all 607 operations and 24 tags while
omitting response documentation. The default golden contract contains 48 web
operations plus help. Targeted tests also exercise all 480 eligible operations,
data/crypto and LLM selections, known exclusions, prospective diagnostics, query
routing, nested gateway JSON and streaming guidance.

The AgentUtility catalog lives at `providers/agentutility/openapi.json`, captured on
2026-10-03 from the API-host OpenAPI plus documentation-host registry. The Rust
`snapshot_agentutility` utility keeps all 817 request contracts, enriches missing
tags, marks 282 aliases and adds a curated research tag. The provider allows 525
operations; its golden covers 15 research tools plus help. Tests also cover all
eligible tools, tag subsets, required-only schema alternatives and satellite image
metadata. A captured unpaid search challenge pins static signing compatibility and
managed refusal of its bazaar/builder-code extensions; no live funds were used.
