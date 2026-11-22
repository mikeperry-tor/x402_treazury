# Build, development, and operator tools

This directory holds build helpers, development analyzers, operator
diagnostics, and provider maintenance utilities so `examples/` stays focused on
deployment templates.

## Root-package Cargo examples

Invoke these explicit `[[example]]` targets through
`make run CARGO_ARGS='--example NAME -- ...'` (binaries build under
`target/{profile}/examples/`, skipped by `make build`):

| Target | Source and purpose |
| --- | --- |
| `provider_updates` | `providers/provider_updates.rs`: public evidence capture and frozen offline replay; see [maintenance sweeps](../docs/provider-updates.md). |
| `snapshot_spec` | `providers/snapshot_spec.rs`: fetch or reduce public OpenAPI inputs for maintained catalogs. |
| `snapshot_agentutility` | `providers/snapshot_agentutility.rs`: maintain AgentUtility's authored catalog join from local files. |
| `diagnose_credit` | `diagnostics/diagnose_credit.rs`: read-only canonical Base credit checks; see [wallet CLI](../docs/wallet-cli.md). |
| `quote_near` | `diagnostics/quote_near.rs`: credential-free dry NEAR quotes using public test identities; see [live preflight](../tests/live/README.md). |

- `quote_near --diagnose --refund-address ADDRESS` checks a public mainnet
  Zcash refund address format using `dry=true` with no credentials, wallet
  state, or funds (use public test identities only).
- Networked utilities use `src/network.rs` and accept `--network-config`.
  `snapshot_agentutility` reads local files only.
- These sources stay in the network and complexity inventories and outside the
  application coverage denominator. The
  [live qualification runner](../tests/live/INTEGRATION.md) follows the same
  example-target layout under `tests/live/`.

## Provider maintenance data and scripts

- **`providers/evidence.toml`:** Declares local asset intent (`authored` or
  `vendor-copy`), `providers/CAVEATS.md` anchors, and extra public evidence
  sources for `provider_updates`.
- **`providers/snapshot_daedalus.py`:** Offline Python helper that materializes
  `AgentIdentity` `X-Agent-Id` header parameters from a local Daedalus OpenAPI
  JSON snapshot (`python3 tools/providers/snapshot_daedalus.py SOURCE OUTPUT`).

## Workspace and build helpers

- **`protoc/`:** Independently locked build helper that locates the bundled
  Protocol Buffers compiler for Make builds, tests, checks, and release
  qualification; see [development tools](../scripts/README.md) and
  [reproducible builds](../docs/reproducible-builds.md).
- **`complexity/`:** Workspace package (`treazury-complexity`) sharing the root
  lockfile and build cache, invoked by `scripts/complexity.py` and
  `make check-static`; see [COMPLEXITY.md](../tests/COMPLEXITY.md).
