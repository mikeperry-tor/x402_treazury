# Deployment examples

Start with [deployments/managed.toml](deployments/managed.toml): one authenticated
MCP listener, one managed wallet pool and the Curl HTTP Request provider's pinned
local catalog. The [README quickstart](../README.md#quickstart-managed-wallet)
walks through building, offline inspection, treasury setup and serving.

[deployments/comprehensive.toml](deployments/comprehensive.toml) combines curated
research sources, four listeners, two named pools, paid discovery relay and a
persistent endpoint for agent-added APIs. Its relationship map explains sharing.
It retains the `state/privacy` treasury, registry and operational identities of
the original privacy template; renaming the template does not migrate state.

Copy a deployment beside its tracked template as `NAME.local.toml` before
customizing it. Relative paths resolve from the file declaring them. Existing
local configurations remain operator-owned; keep using their original state and
identities unless deliberately changing them.

The starter uses direct networking. The comprehensive example requires a running
Tor Browser SOCKS listener at `127.0.0.1:9150`; adjust its `[network]` for your own
Tor daemon. [network/tor.toml](network/tor.toml) supplies the same kind of standalone
policy for commands using `--provider` and `--network-config`. Deployments use
their own `[network]`. Tor resolves destination names remotely and has no direct
fallback; separate circuits do not hide identifying request content.

Managed serving binds listeners before funding. Initialize the treasury explicitly
and fund its shielded receive address; available tools remain accessible during
funding or discovery delays. Reserve targets are funding preferences, payment
caps authorize individual signatures, and actual spendable ZEC limits ordinary
funding. See [wallet administration](../docs/wallet-cli.md),
[configuration](../docs/configuration.md) and
[agent source management](../docs/agent-sources.md).

Static-wallet deployments remain supported in the same executable; see the
[configuration reference](../docs/configuration.md). Operational Rust utilities
live in [tools/](../tools/README.md), and qualification deployments and runner
instructions live in [tests/live/](../tests/live/INTEGRATION.md).
