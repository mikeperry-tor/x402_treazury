# Build, development and operator tools

This directory contains build helpers, development tooling, operator diagnostics
and provider maintenance utilities.

The diagnostics and provider utilities below are explicit root-package Cargo
`[[example]]` targets. Their source lives here so `examples/` stays focused on
deployment starting points.
Use the existing `make run CARGO_ARGS='--example NAME -- ...'` commands; build output
still lives in `target/{profile}/examples/`. Ordinary application builds do not
build these targets. The [live runner](../tests/live/INTEGRATION.md) follows the
same arrangement with source beside its qualification inputs and runbooks.

| Target | Source and purpose |
| --- | --- |
| `snapshot_spec` | `providers/snapshot_spec.rs`: fetch or reduce public OpenAPI inputs for maintained catalogs. |
| `snapshot_agentutility` | `providers/snapshot_agentutility.rs`: maintain AgentUtility's authored catalog transformation. |
| `diagnose_credit` | `diagnostics/diagnose_credit.rs`: read-only canonical Base credit checks; see the [wallet guide](../docs/wallet-cli.md). |
| `quote_near` | `diagnostics/quote_near.rs`: credential-free dry quotes using public test identities; see [preflight](../tests/live/README.md). |

Networked utilities use the shared network factory and accept `--network-config`.
`snapshot_agentutility` consumes reviewed local files only. Utilities never change
ordinary serving's authority. Maintenance and qualification sources
remain in the network and complexity inventories and outside the application
coverage denominator. Relocation establishes no coverage or complexity improvement.

`protoc/` is an independently locked build helper that locates the bundled Protocol
Buffers compiler before application compilation. Ordinary Make build, check, test
and run commands use it, as does release qualification; see the
[build entrypoints](../scripts/README.md) and
[reproducible builds](../docs/reproducible-builds.md).

`complexity/` is an independently locked development analyzer invoked by
`scripts/complexity.py` and `make check-static`; see
[complexity review](../tests/COMPLEXITY.md). Neither helper is a root-package Cargo
example target.
