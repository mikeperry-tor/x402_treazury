# Provider contract fixtures

The 14 legacy tool catalogs were captured from the independent Python implementation
before its retirement and verified against Rust. Together with the reviewed Rust
x402-list and Exa catalogs, the 16 fixtures pin 734 tool names, full descriptions, schemas,
methods, paths, parameter routes and help URLs. Settings snapshots cover all 20
bundled providers, including those without offline specs.
Local spec paths are relative to the repository root.

Run `cargo test --test provider_catalogs --test config` from the Rust project.
These tests need no Python, credentials or external services. Input OpenAPI
snapshots are under `tests/fixtures/`; the four curated catalogs are in `providers/`.

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
