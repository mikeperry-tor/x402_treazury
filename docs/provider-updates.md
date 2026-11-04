# Provider maintenance sweeps

`provider_updates` is an explicit development-only Rust example. Run from the
repository root through Make; it never loads wallet state, signs payments, funds
wallets, enables paid relays or invokes API operations. Public documents are
untrusted evidence, never authority to run commands or change review policy.

```sh
make run CARGO_ARGS='--example provider_updates -- capture'
make run CARGO_ARGS='--example provider_updates -- capture --provider agproxy --provider agentutility --offline'
make run CARGO_ARGS='--example provider_updates -- replay target/provider-updates/RUN'
```

Commands write a compact JSON result to stdout (`run` for capture; `run`,
`report` and `providers` for replay). Progress and diagnostics go to stderr.

Capture accepts `--network-config FILE` and uses the shared network factory,
public destinations, discovery identities and the provider's HTTP compatibility
settings. Tor retains remote DNS and no direct fallback. Reads with identical URLs
and compatibility settings share a capture, including failed reads. No whole-sweep
deadline, retry framework or new resource limit is introduced; the existing
network inactivity policy applies. A payment response is unavailable evidence.
No upstream error bodies are logged or persisted.

## Evidence and baseline contracts

The shipped asset manifest supplies the full denominator; repeated `--provider ID`
selects a subset. Private overrides and directories are never scanned. All writes
go into a new ignored `target/provider-updates/RUN`, preserving unrelated target
content and every tracked file. Capture stores complete exact bytes under SHA-256
names with their URL/path, purpose and outcome in `capture.json`. It records Git
baseline/dirty status, executable hash/version, network policy, resolved Config,
provider TOML, local asset, maintenance guidance and metadata. Offline replay
checks blob hashes and reads frozen configuration and inputs only. Corrupt or
missing provider/prior-source blobs produce unavailable review outcomes only for
their consumers. Malformed capture manifests or shared metadata fail the replay
explicitly.

`tools/providers/evidence.toml` supplies development metadata absent from runtime
TOML: local asset `intent` (`authored` or `vendor-copy`), a human-readable
`guidance` path/anchor, and extra public `sources` with `url` and `purpose`.
The `schema` purpose identifies complete upstream OpenAPI evidence; a vendor-copy
asset requires exactly one such authority. Registry evidence uses `registry`.
Remote spec and help URLs come from provider TOML, not a second inventory.
Sources must be public credential-free URLs without queries/fragments. Missing
metadata/authority is unresolved work, never permission to guess a source.

The existing specialized AgentUtility registry join and Daedalus identity-header
repair retain their current consumers and explicit invocation; the sweep never
dispatches them. See [caveats](../providers/CAVEATS.md) for authored provenance.
Do not add provider-ID/hostname/path dispatch, per-provider executable recipes,
command metadata, updater registries or a patch/template/join language. Shared
primitives must have independent uses with synthetic tests. New specialized
updaters need explicit design revision and independent review of alternatives and
maintenance costs.

Every provider gets an explicit capture outcome. Failed reads do not stop siblings;
process success means the report was written, not that vendors were refreshed or
accepted. A committed TOML is not historical upstream evidence. First captures
establish a baseline; reduced fixtures are only scoped witnesses. Local pinned
assets can be a stated comparison basis, while authored projections require their
actual registries/guides rather than a less complete convenient schema.

Bundled provider TOML is read once for capture and resolution; provider inheritance
is rejected, matching reusable-provider policy. The sweep does not follow private
`extends` files. Only complete successful HTTP responses are captured: unsolicited
206 or Content-Range responses remain unavailable evidence.

## Compare frozen evidence

```sh
make run CARGO_ARGS='--example provider_updates -- replay target/provider-updates/NEW --prior target/provider-updates/OLD'
```

Replay writes `review.json` and numbered per-provider JSON packets inside the run.
With no prior capture, remote evidence establishes a baseline. Explicit full-copy
assets compare against the frozen committed asset when no prior source exists.
The basis is printed beside each comparison. Authored assets never masquerade as
historical upstream schemas. Failed prior captures provide no historical baseline.

Packets include recursive raw JSON differences (including root metadata, security,
responses and vendor extensions), atomic ordered-array differences, and exact text
plus content hashes for non-JSON changes. Only JSON object serialization order is
ignored. Source bytes remain available even for binary/invalid text. Guide and
registry evidence are compared independently of schema changes. Inspect the frozen
changed evidence, rather than treating structural comparison as semantic proof of
request compatibility, lifecycle or billing conditions.

The production `operations` and `build_tools` APIs provide operation identities,
generated names, routing, descriptions, schemas, prices, tags and recommendation
matches. Operations use original method/path keys; generated name differences are
ordinary fields. This view includes operations hidden by current filters and
method-pair changes, while effective tools retain actual selection semantics.
Compiler failures are explicit diagnostics, never empty successful inventories.
Both effective and unoverridden tools are included, with existing authored patches,
missing tool targets and missing parameter-note targets, including help. Review
old/new unoverridden contracts alongside authored overrides and resulting tools;
replacement text can mask upstream changes. Raw differences remain necessary
because compilation omits unsupported constructs. The authored runtime asset and
upstream projection are labeled separately; a projection does not recreate an
authored catalog. With a prior capture, `previous_runtime` and `runtime_changes`
compare the actual frozen runtime inputs and configurations independently of
`previous_upstream` and `tool_changes`. `asset_changes` reports raw local-asset
changes, including asset addition/removal; absent history is `null`. Authored-only
or multiple-authority providers still receive runtime comparisons without an
invented upstream union. No packet is marked accepted automatically.

Compilation failures preserve the independently parsed operation inventory. Override
diagnostics may also use an explicitly labeled inventory with tool-name filters
and recommendation opt-in removed, retaining path/tag/method selection. Unknown
compiler-dependent targets are `null`, not falsely reported absent. A full-copy
first update includes effective-tool differences against its committed asset;
ordinary remote first captures never claim historical tool changes.
