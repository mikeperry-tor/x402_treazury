# Selected request fixtures

These four snapshots hold only the selected paths, OpenAPI metadata, servers,
components, and response declarations captured on 2026-10-04 for Arkham,
botsmith, Google Trends, and x402stock.

`tests/live/runner/presets.rs` uses them exclusively for offline input and
scope validation; live deployments still fetch remote specs.

`provider_requests.json` pins the 22 paid requests from the baseline sweep
(case ID, tool, exact arguments, and reservation) independently of the preset
code. The preset test compares both files; update them together only after
explicit request and budget review.
