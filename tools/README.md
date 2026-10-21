# Operator and maintenance tools

These Rust utilities are explicit root-package Cargo `[[example]]` targets. Their
source lives here so `examples/` stays focused on deployment starting points.
Use the existing `scripts/zcash.sh run --example NAME -- ...` commands; build output
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
The separate `complexity/` analyzer is development tooling with its own lockfile;
see [complexity review](../tests/COMPLEXITY.md).
