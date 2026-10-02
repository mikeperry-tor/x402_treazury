# Provider contract fixtures

The 14 tool catalogs were captured from the independent Python implementation
before its retirement and verified against Rust. They pin 726 tool names, full
descriptions, schemas, methods, paths, parameter routes and help URLs. Settings
snapshots cover all 18 bundled providers, including those without offline specs.
Local spec paths are relative to the repository root.

Run `cargo test --test provider_catalogs --test config` from the Rust project.
These tests need no Python, credentials or external services. Input OpenAPI
snapshots are under `tests/fixtures/`; the four curated catalogs are in `providers/`.

When changing providers, review the changed input, the generated tool contract,
and the expected output together. Do not automatically accept generated snapshots
just to make tests pass. Schema/routing edge cases also have targeted Rust tests;
these snapshots preserve the selected provider contracts, not every old behavior.
