# Live integration: provider walkthrough

Use the reusable runner to prepare reviewed MCP calls, execute them through the
production application and report provider behavior and verified payments. This
is opt-in mainnet work; the commands below are a workflow, not spending authority.
For assertions, evidence schemas, lifecycle and diagnostic details, see the
[technical reference](INTEGRATION_REFERENCE.md).

Start with the
[23-provider preset](integration/PROVIDER_PRESETS.md): 22 paid requests, one unsigned
directory read and 40 help fetch/hit calls. Provider failures are valid observations.
Full automated lifecycle/restart and combined cover/payment qualification are
[deferred](../../docs/plans/deferred/live_integration_acceptance.md).

## 1. Build and review inputs

Build the application and runner together; preparation pins their build identity.
Default builds include Zcash. Build traffic is separate from runtime Tor policy.

```sh
scripts/zcash.sh build --bin treazury --example live_integration
```

Before preparation, copy both executables into an owner-only directory for this
experiment (ordinary copies, not hard links). Set the manifest's `binary` to that
private `treazury` copy and invoke the private `live_integration` copy for all
commands below. Keep them unchanged through reporting. Cargo may replace files
in `target/debug` during unrelated tests; rebuilding that path during a pinned
live run breaks artifact continuity even if the running process never restarts.
The command examples show build output paths only for brevity.

Copy `integration/providers.example.toml`, `integration/authorization.example.toml`
and its referenced deployment to private local paths. Replace every placeholder:
treasury/run/authorization IDs, state and evidence directories, executable/provider
paths, installed Tor binary, listener ports and finite UTC windows. Use absolute
paths when moving templates and owner-only evidence directories. Review exact tools,
arguments, response assertions and source/listener/wallet assignments.

The baseline reserves **$4.40 across 22 paid attempts**, not a fee forecast. It
permits zero new funding jobs/source exposure and disables automatic funding and
startup pricing. Retain all declared pools, including those not used by the sweep.
Extra routes/providers, funding and repeated calls need separate reviewed bounds.

Plan offline without catalogs, wallet keys, balances or requests:

```sh
target/debug/examples/live_integration plan --manifest /private/run.toml
```

Optional `select-cases` produces a new reviewed manifest; see
[selection rules](INTEGRATION_REFERENCE.md#case-selection). It filters calls only,
not startup sources. New IDs never reset cumulative charges or old liabilities.

## 2. Establish a usable wallet and budget

Use production `treazury wallet init`, `addresses`, `sync` and `backup` commands;
see [wallet setup](INTEGRATION_REFERENCE.md#wallet-setup-and-readiness). Standalone
Tor wallet commands require an already-running configured SOCKS listener. A failed
sync is an error, not a zero balance. Keep seeds, keys and backups private.

Existing-wallet tests require fresh observations and adequate active balances.
Saved balances are not a fresh funded starting-point check. Arrange
any necessary funding separately under reviewed source/job limits; do not enable funding or force rotation merely to make
the provider sweep run. The preset does not substitute a static EVM private key.

With serving stopped, authorize the reviewed cumulative ceilings against the
existing treasury-local registry. This records a baseline without signing or
network requests. Existing charges and liabilities cannot be erased by reauthorizing.

```sh
target/debug/examples/live_integration authorize-registry \
  --state-dir /private/treasury/state --authorization /private/auth.toml
```

See [authorization details](INTEGRATION_REFERENCE.md#registry-authorization-and-preparation)
for existing liabilities and positive-funding scenarios. No plan or report grants
permission to spend.

## 3. Prepare complete catalogs

```sh
target/debug/examples/live_integration prepare \
  --state-dir /private/treasury/state --manifest /private/run.toml
```

Owned Tor is selected in the manifest. Preparation owns an installed Tor process
with dedicated persistent state and supported macOS confinement, then freezes full
catalogs and validates cases. Execution later starts a separate owned process using
the same persistent state. Each has separate control evidence. External SOCKS alone
cannot qualify isolation. See [Tor details](INTEGRATION_REFERENCE.md#owned-tor-qualification).

Preparation continues independent catalog loads after a failure, saves typed stage
observations, and refuses to publish a partial runnable inventory. If a source blocks
preparation, retain its failure and review a new smaller deployment/manifest without
changing the required pool/wallet scope. Explicit fixture substitution must preserve
the original failure and fixture origin/hash. Kronos and RegimeShift currently use
local snapshots; local/frozen loading does not establish live catalog availability.
When removing a startup source after case selection, remove its listener entries
and pricing assertions too, then run `plan` again before preparation. Keep every
wallet profile and record all omitted cases. Failed preparation evidence is never
overwritten; use a new run ID for corrected inputs.

Changing executable, configuration or catalog inputs requires a new run; existing
pins and reservations are not overwritten.

## 4. Run once, then inspect

```sh
target/debug/examples/live_integration run \
  --state-dir /private/treasury/state --run RUN_ID
target/debug/examples/live_integration report \
  --state-dir /private/treasury/state --run RUN_ID --output /private/new-report.json
target/debug/examples/live_integration report \
  --state-dir /private/treasury/state --run RUN_ID --format markdown \
  --output /private/new-report.md
```

Independent provider phases can continue after ordinary failures. Unknown payment
outcomes and missing safety evidence stop affected/dependent work and retain
exposure. Allow deliberate draining to finish; never restart to replay a reserved,
uncertain or completed case. Run exits are 0 for qualified success, 2 for completed
failure, 3 for incomplete work and 4 for safety/configuration/process-evidence errors.
Report success only means evidence was read and rendered.
Canonical `EXPIRED_UNUSED` or `NOT_SIGNED` outcomes stop payment observation with
an explicit warning; a successful MCP response still cannot qualify as paid without
a verified debit. Pending authorizations and consumed authorizations without debit
proof retain bounded observation. None of these outcomes permits replay.

JSON contains private identifiers and detailed observations; Markdown is sanitized.
Existing output files cannot be overwritten. For all runs, omit `--run`. Optional
eligibility inspection adds case-window observations and the unstarted-run gate:

```sh
target/debug/examples/live_integration report \
  --state-dir /private/treasury/state --run RUN_ID --eligibility
```

This cannot launch processes or authorize execution. A started/reserved run cannot
run again, even if untouched cases still have open windows. See
[existing-pool adoption](INTEGRATION_REFERENCE.md#execution-and-existing-pool-adoption)
for explicitly reviewed new runs. `status` and `resume` are no longer runner commands.

## 5. Assess provider-workflow coverage

Use the [qualification scope](../../docs/testing.md#qualification-status) and
[preset assertions](integration/PROVIDER_PRESETS.md). Keep catalog, help/cache,
pricing, MCP result, body semantics and canonical debit proof separate. Flag sampled
offers above $0.01; missing prices/proofs are not zero. AgentUtility may still require
manual semantic review. Preserve blocked/unattempted stages and every failure.

The baseline disables pricing. Use a separate small eligible-provider configuration
with [pricing assertions](INTEGRATION_REFERENCE.md#startup-pricing-assertions),
preferably keyless/unsigned. Keep normal production discovery policy; do not force
extra probes or paid requests. Observe cover failures/interference separately;
successful shaping is not a gate for this provider-workflow acceptance.
Preparation still needs a `[treasury]` ID/state reference matching the existing
registry, even for this unsigned run. Use a nonexistent `key_file` and unset
indexer/submission environment references, no managed wallets or funding, and a
zero-cap static placeholder with an unset private-key reference. Production unsigned
serving never opens these keys or starts treasury workers.

Record whether the runner and its reports cover the original workflow without
bespoke scripts. Retain exact evidence links and manual gaps. Private financial evidence remains immutable; saved observations are not fresh balances.

## Other scenarios and diagnostics

- [Scenario presets](integration/SCENARIO_PRESETS.md): smoke, shared/separate-wallet
  concurrency, rotation, lifecycle and treasury-only runs.
- [Reference](INTEGRATION_REFERENCE.md): content/cache/pricing assertions, evidence
  limits, accounting and production supervision.
- [Tor re-audit and receipt recovery](INTEGRATION_REFERENCE.md#offline-tor-re-audit):
  inspect retained evidence without authorizing new paid calls.
- [Keyless outage](INTEGRATION_REFERENCE.md#owned-tor-qualification) and
  [unsigned cover experiment](INTEGRATION_REFERENCE.md#unsigned-cover-experiment):
  independent tests, never a funded-to-keyless process handoff.
- [Unsigned HTTP preflight](README.md): useful when catalogs prevent MCP preparation.
  The [qualification scope](../../docs/testing.md#qualification-status) documents preserved evidence and regression
  coverage; all funded execution uses this runner.
