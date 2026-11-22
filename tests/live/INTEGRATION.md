# Live integration: provider walkthrough

Use `live_integration` to prepare reviewed MCP calls, run them through the
production `x402_treazury` executable, and report provider behavior and
verified Base debits. This is opt-in mainnet work requiring explicit operator
funding and review. See the [technical reference](INTEGRATION_REFERENCE.md) for
evidence schemas, assertions, and recovery contracts.

Start with the [23-provider preset](integration/PROVIDER_PRESETS.md): 22 paid
calls, 1 unsigned directory read, and 40 help fetch and cache-hit calls.
Automated lifecycle and restart qualification and combined cover and payment
qualification remain
[deferred](../../docs/plans/deferred/live_integration_acceptance.md).

## 1. Build and review inputs

Pin `[network]` directly inside the qualification deployment file before
preparation (`--network-config` is rejected during qualification snapshots and
supervised serving). The runner lives in `tests/live/live_integration.rs` and
`tests/live/runner/` as the root `live_integration` Cargo example (its shared
unit tests run once through `tests/integration_preparation.rs`). See
[tools](../../tools/README.md).

Build the application and runner together so their build identities match:

```sh
make build CARGO_ARGS='--bin x402_treazury --example live_integration'
```

Copy both executables (regular file copies, not hard links) into an owner-only
directory for this run, set the manifest's `binary` to that private
`x402_treazury` copy, and invoke that private `live_integration` copy for all
commands below so unrelated Cargo builds cannot overwrite `target/` mid-run.

Copy `integration/providers.example.toml`,
`integration/authorization.example.toml`, and its referenced deployment to
private paths and fill in every placeholder (treasury, run, and authorization
IDs, absolute paths, Tor binary, listener ports, and finite UTC windows). The
baseline sets a **$4.40 running API authorization budget across 22 paid cases**
with zero new funding jobs and `auto_fund = false`, keeping all declared pools
configured.

Plan offline without fetching catalogs or opening wallet keys:

```sh
target/release/examples/live_integration plan --manifest /private/run.toml
```

Use optional `select-cases` to create a narrower manifest (see
[case selection](INTEGRATION_REFERENCE.md#case-selection)); it filters cases
only, not startup sources.

## 2. Establish a ready wallet and budget

Initialize, fund, sync, and back up the treasury with the production wallet
CLI (`wallet init`, `addresses`, `sync`, `backup`; see
[wallet setup](INTEGRATION_REFERENCE.md#wallet-setup-and-readiness)).
Standalone Tor wallet commands require Tor's SOCKS listener to be running. Keep
seeds, keys, and backups private.

With serving stopped, authorize the cumulative ceilings against the
treasury-local registry (offline, no signing or network traffic):

```sh
target/release/examples/live_integration authorize-registry \
  --state-dir /private/treasury/state --authorization /private/auth.toml
```

- `total_test_budget_usdc` (per run) and `cumulative_test_budget_usdc` (across
  runs) charge each potentially payable API authorization atomically before
  admission commits; wallet funding is bounded separately by reviewed job
  counts, reserve sizes, and spendable ZEC.
- Existing charges and liabilities cannot be erased by reauthorizing. See
  [authorization details](INTEGRATION_REFERENCE.md#registry-authorization-and-preparation).

## 3. Prepare complete catalogs

```sh
target/release/examples/live_integration prepare \
  --state-dir /private/treasury/state --manifest /private/run.toml
```

Preparation launches an owned Tor process under macOS `sandbox-exec`, freezes
complete catalogs, and validates every case (execution later launches a fresh
owned Tor process reusing the same persistent state, with separate control
evidence; see [owned-Tor qualification](INTEGRATION_REFERENCE.md#owned-tor-qualification)).

- Preparation continues loading sibling catalogs after a failure and saves
  stage diagnostics while refusing to publish a partial runnable inventory.
  Kronos and RegimeShift use local snapshots in the baseline template.
- If a source fails preparation, leave its evidence intact and create a new
  run ID with an updated deployment (removing both the source and its listener
  references while keeping all wallet pools intact, then rerunning `plan`).

## 4. Run once, then inspect

```sh
target/release/examples/live_integration run \
  --state-dir /private/treasury/state --run RUN_ID
target/release/examples/live_integration report \
  --state-dir /private/treasury/state --run RUN_ID --output /private/new-report.json
target/release/examples/live_integration report \
  --state-dir /private/treasury/state --run RUN_ID --format markdown \
  --output /private/new-report.md
```

- **Exit codes:** `0` = qualified success, `2` = completed failures, `3` =
  incomplete work, `4` = safety, configuration, or process-evidence error.
- **Execution safety:** Each batch reserves atomically before dispatch.
  Independent provider phases continue after normal provider failures, while
  unknown payment outcomes or missing safety evidence stop dependent work and
  retain charges. Let graceful drain finish and never replay a reserved,
  uncertain, or completed case.
- **Payment verification:** Canonical `EXPIRED_UNUSED` and `NOT_SIGNED` stop
  observation with a warning (an MCP success without verified debit cannot pass
  as paid). When a managed call traverses multiple wallets after a structured
  insufficient-funds `402`, every signed attempt consumes the case permit and
  retains its own receipt and debit proof.
- **Reports:** JSON retains private details; Markdown is sanitized. Neither
  overwrites existing files; omit `--run` to report all runs. Add
  `--eligibility` to inspect windows and run readiness without launching
  processes. A started run can never launch again (`status` and `resume` are
  not runner commands); see
  [existing-pool adoption](INTEGRATION_REFERENCE.md#execution-and-existing-pool-adoption).

## 5. Assess provider-workflow coverage

Evaluate catalog loading, help caching, pricing, MCP responses, body checks,
and canonical debits separately using the
[qualification status](../../docs/testing.md#qualification-status) and
[preset assertions](integration/PROVIDER_PRESETS.md). Flag offers above $0.01
(missing prices stay `null`, never `0`). AgentUtility requires manual semantic
review if it returns a body.

For an unsigned run, keep a `[treasury]` ID and state path matching the
registry with a nonexistent `key_file`, unset indexer environment references,
no managed wallets or funding, and a zero-cap static placeholder whose key
variable is unset.

## Other scenarios and diagnostics

- [Scenario presets](integration/SCENARIO_PRESETS.md): smoke, concurrency,
  rotation, lifecycle, and treasury-only runs.
- [Technical reference](INTEGRATION_REFERENCE.md): assertions, limits,
  accounting, and child supervision.
- [Shielded-refund evidence](INTEGRATION_REFERENCE.md#shielded-refund-evidence-and-qualification):
  inspect retained evidence and `wallet notes` before authorizing any scan or
  controlled refund experiment.
- [Offline Tor re-audit and receipt recovery](INTEGRATION_REFERENCE.md#offline-tor-re-audit):
  re-verify Tor logs or recover missing Base debit proofs without paid calls.
- [Keyless outage](INTEGRATION_REFERENCE.md#owned-tor-qualification) and
  [unsigned cover experiment](INTEGRATION_REFERENCE.md#unsigned-cover-experiment):
  standalone keyless runs.
- [Unsigned HTTP preflight](README.md): raw-HTTP diagnostic when catalogs block
  MCP preparation.
