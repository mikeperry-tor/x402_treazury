# Opt-in unsigned provider preflight

Use the [integration runner](INTEGRATION.md) for prepared, supervised MCP
provider runs. `provider_preflight` is a separate raw-HTTP diagnostic for
checking specs, help documents, and unsigned x402 challenges over Tor. See
[testing](../../docs/testing.md#qualification-status) for qualification
boundaries.

`preflight.json` lists public read-only diagnostic requests (including Exa
Contents and all four Google Trends routes); no secrets belong in it.

Build before applying OS network confinement:

```sh
make test CARGO_ARGS='--example provider_preflight --example quote_near'
make build CARGO_ARGS='--example provider_preflight --example quote_near'
```

Run against an already-running Tor instance with a `[network]` file:

```sh
RUST_BACKTRACE=0 target/release/examples/provider_preflight \
  --network-config /absolute/path/network.toml \
  --manifest tests/live/preflight.json \
  --output /absolute/path/new-private-results.jsonl
RUST_BACKTRACE=0 target/release/examples/quote_near \
  --network-config /absolute/path/network.toml --amount-usdc 0.10
```

- **Confinement and I/O:** Wrap binaries with the SOCKS-only OS policy and
  control observer from [TOR.md](../TOR.md) for live-Tor qualification.
  `provider_preflight` refuses direct mode, writes a new owner-only JSONL file
  (refusing existing paths), flushes after every case, and runs sequentially
  with bounded downloads and 60-second request timeouts.
- **What preflight does:** Fetches the spec, checks catalog completeness and
  overrides, fetches configured help, and sends one unsigned challenge using a
  dummy public EVM
  transport identity (never signing, loading wallet state, calling MCP, or
  testing help caching). Arguments are pinned in `preflight.json` without
  schema validation. Offers above 10,000 Base USDC atomic units are flagged
  without rounding; unknown prices stay `null`.
- **Dry NEAR quote (`quote_near`):** Defaults to $1 and never allocates a
  deposit or sends funds. `--diagnose` prints the bounded API response for the
  same dry request.
- **Interpreting results:** Exit `0` means the manifest ran to completion (read
  per-stage results for failures; interrupted runs keep completed lines and are
  incomplete). Keep build, config, and manifest hashes alongside the private
  report.
- **Related files:** [coverage.json](coverage.json) and
  [coverage_exclusions.json](coverage_exclusions.json) record the reviewed
  sweep scope; [PROVIDER_PRESETS.md](integration/PROVIDER_PRESETS.md) and
  [SCENARIO_PRESETS.md](integration/SCENARIO_PRESETS.md) document the MCP
  runner presets.
