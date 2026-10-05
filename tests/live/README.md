# Opt-in unsigned provider preflight

Use the [reusable integration runner](INTEGRATION.md) for prepared, supervised
MCP provider tests. This unsigned preflight remains a separate raw-HTTP diagnostic;
[testing](../../docs/testing.md) describes qualification boundaries.

`preflight.json` is a reviewed diagnostic request manifest covering every bundled
provider plus Exa Contents and the remaining three Google Trends routes. It is
not the funded MCP execution manifest. Requests are public read operations; POST
bodies are API reads, not job creation or account mutation. No secrets belong here.

Build before applying runtime network confinement:

```sh
scripts/zcash.sh test --example provider_preflight --example quote_near
scripts/zcash.sh build --example provider_preflight --example quote_near
```

With an independently started Tor instance and a `[network]` file pointing to it:

```sh
RUST_BACKTRACE=0 target/debug/examples/provider_preflight \
  --network-config /absolute/path/network.toml \
  --manifest tests/live/preflight.json \
  --output /absolute/path/new-private-results.jsonl
RUST_BACKTRACE=0 target/debug/examples/quote_near \
  --network-config /absolute/path/network.toml --amount-usdc 0.10
```

For the live qualification, wrap those executables with the SOCKS-only OS policy
and control-event observer described in [TOR.md](../TOR.md); merely supplying a
network config is not proof of OS confinement or observed circuit isolation.
The preflight refuses non-Tor mode, writes a new owner-only report on Unix, and
processes cases sequentially with bounded downloads and 60-second request timeouts.
An output filename cannot be reused. Every completed case is flushed to disk.
The quote example defaults to $1 and never allocates a deposit or sends funds.
`--diagnose` reports the bounded API response for the same synthetic dry request;
that mode does not validate a returned quote or authorize its use for funding.

Preflight retrieves the configured spec, builds its catalog, retrieves configured
help, and requests an unsigned challenge using a public dummy EVM transport
identity. Actual spec/help/pricing production identities remain discovery-origin
scoped. It does not sign, load wallet state, send a paid retry, call MCP, perform
startup pricing probes, test help cache behavior or qualify settlement. A raw HTTP
200 does not prove a meaningful tool result. Combined spec/catalog errors need
inspection to identify the precise substage. Request arguments are pinned in the
manifest, but this tool does not yet validate them against generated tool schemas.

Offers above 10,000 Base USDC atomic units are flagged without rounding. Unknown
assets/prices are not zero. All offers are retained as observations; this diagnostic
does not assert managed compatibility or choose an offer. Failed calls and quotes
are evidence, not authority to retry payments or raise caps. Help/spec retries for
the extra Exa/Trends diagnostic cases are explicit separate measurements, not a
claim about production process caching.

Exit success means the manifest finished, not that every provider passed. Read
per-stage results; interrupted runs retain completed lines and must be reported as
incomplete. Preserve build/config/manifest hashes and a private copy of executed
sources alongside the report. See [qualification scope](../../docs/testing.md#qualification-status)
for the coverage map. Use [INTEGRATION.md](INTEGRATION.md) for funded execution,
[provider presets](integration/PROVIDER_PRESETS.md) for the sweep, and
[scenario presets](integration/SCENARIO_PRESETS.md) for concurrency/rotation.
[Coverage selection](coverage.json) and [exclusions](coverage_exclusions.json)
account for every bundled provider; they are inputs, not current availability reports.
