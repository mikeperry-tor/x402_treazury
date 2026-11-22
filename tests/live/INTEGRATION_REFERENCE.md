# Live integration reference

Start with the [provider walkthrough](INTEGRATION.md). This reference specifies
case selection, content and cache assertions, fee and lifecycle evidence,
child supervision, owned Tor confinement, and offline re-auditing.

- [Scenario presets](integration/SCENARIO_PRESETS.md): single-wallet smoke,
  shared and separate-wallet concurrency, rotation and refill, two-round
  lifecycle, treasury-only bootstrap, unsigned discovery, and reliability.
- [Provider preset](integration/PROVIDER_PRESETS.md): 63 explicit cases for the
  23-provider baseline (22 paid requests, 40 unsigned help fetch and hit calls
  across 20 help tools, and 1 unsigned directory call).

## Preset arguments and reviewed request bodies

Preset arguments match the exposed MCP tool schemas (for example, Arkham uses
`body = { address = "..." }` for open extra properties, and AgentFund uses
`body = { days = 1 }` so an optional body stays distinct from no body). Offline
tests compare routed methods, URLs, and JSON bodies against
`tests/live/fixtures/provider_requests.json`.

## Case selection

`select-cases` produces a narrower manifest from a reviewed provider-sweep,
unsigned, or reliability manifest offline (no catalogs, wallet keys, registry
access, or network calls). Use a new run ID and an existing owner-only output
directory:

```sh
target/release/examples/live_integration select-cases \
  --manifest private/reviewed.toml --run-id selected_providers \
  --source socialfetch --source brazilayer --exclude-flagged \
  --output private/selected.toml
```

- `--source` and `--case` form an inclusive union (omitting both starts from
  all cases); `--exclude-source`, `--exclude-case`, `--exclude-reliability-tag`,
  and `--exclude-flagged` remove matches. Names are exact, case-sensitive IDs
  and tags (unknown names fail; excluded tags must appear on at least one input
  source).
- Rejects empty output, broken dependencies, and lifecycle or concurrency
  manifests. Writes resolved absolute paths without overwriting files or
  resetting registry reservations, and prints a JSON summary of omitted cases,
  hashes, and selected reservation total.
- Filters **cases only**: unselected sources still load at startup and all
  declared wallet pools keep their configuration. To omit a startup source,
  author a separate deployment removing both the source and its listener
  references, then run `plan`.

## Provider content assertions

Content checks evaluate provider semantics separately from MCP transport and
Base settlement:

```toml
[[cases.checks]]
kind = "json_exists"
pointer = "/results"

[[cases.checks]]
kind = "json_equals"
pointer = "/degraded"
value = false
```

- Also supports `text_contains` with nonempty `text`. JSON checks require one
  unambiguous JSON body (deduplicating identical copies across structured
  content and text blocks); multiple distinct JSON bodies or unsupported
  content yield `manual_review_required`.
- Any JSON object containing boolean `isError: true` anywhere in the parsed
  tree is a semantic failure even when outer MCP succeeded (`error` fields have
  no automatic failure rule).
- Provider-sweep and reliability cases require declared content checks; failed
  or manual required checks block dependent phases while independent phases
  continue.
- Bounds: at most 32 checks per case (4,096 serialized bytes each), 1,000 MCP
  blocks, 100,000 JSON nodes, and 64 nesting levels. Exceeding a bound marks
  the case unassessed with the limit category rather than clipping.

## Offers and fees

Single-run reports emit `fee_observations` recording challenge offers, admitted
Base USDC amounts, and verified canonical debits, each with a strict `> 10000`
atomic ($0.01) flag:

- Challenge capture records up to 2 observations per case (initial challenge
  plus an allowed unsigned payer-change retry), up to 65,536 encoded header
  bytes and 128 offers per observation. Reaching a bound logs a warning and
  records explicit report evidence rather than a partial list.
- Retains only amounts, fixed scheme and asset classes, and a header hash (no
  prose, extensions, or payee addresses). Headerless legacy v1 body offers
  stay unobserved.
- Version 2 admission evidence uses `signed_admission_pending` (local signing
  and validation happened before the separate financial transaction committed);
  case charges survive failed or uncertain commits without authorizing replay.

## Help cache assertions

Unsigned help cases may declare `help_cache = "fetch"`, `"hit"`, or `"shared"`,
observing the production `OnceCell` initializer directly (`fetch` ran the
initializer, `hit` found a ready cache, `shared` waited on another caller).
Failed downloads remain uncached; restarting the application starts a fresh
cache. Reports record retrieval status, HTTP code, cache path, and byte count
without URLs or document text.

```toml
[[cases]]
id = "docs_first"
server = "b"
source = "socialfetch"
tool = "socialfetch_help"
arguments = {}
reserve_usdc = "0"
reviewed_read_only = true
unsigned = true
help_cache = "fetch"

[[cases]]
id = "docs_cached"
server = "b"
source = "socialfetch"
tool = "socialfetch_help"
arguments = {}
reserve_usdc = "0"
reviewed_read_only = true
unsigned = true
help_cache = "hit"

[[phases]]
id = "documentation"
window = "main"
cases = ["docs_first", "docs_cached"]
pools = []
required = true
[phases.scenario]
kind = "unsigned"
```

## Pricing and retained observations

Catalog preparation, startup, and warming make no endpoint pricing probes.
Current manifests reject the retired `pricing_checks` field (the observation
reader still tolerates it in archived manifests alongside historical
`application_pricing` events without rewriting saved evidence).

## Report output

```sh
target/release/examples/live_integration report --state-dir /private/state \
  --run RUN_ID --format markdown --output /private/new-report.md
```

- Plan JSON, report JSON, and nested `summary` use version 2, reporting the
  running API authorization budget (no obsolete ZEC budget projections).
- JSON (default) contains private evidence; Markdown uses numbered cases and
  assertions in JSON order, leads with a numbered provider-stage table, and
  omits IDs, addresses, transaction hashes, paths, credentials, and provider
  prose.
- Reports distinguish selected-run reservations, registry-wide reservations,
  and verified Base debits; missing amounts stay `null`, invalid proofs
  suppress totals, oversized reports fail without truncation, and existing
  output files are never overwritten.
- The network field classifies runs as direct, external proxy-only, or
  owned-control-audit-required (use `audit-tor` for authenticated Tor control
  evidence).

## Execution and existing-pool adoption

`tests/live/live_integration.rs` plans offline, freezes complete catalogs
(128 MiB capture cap; 16 MiB evidence cap), and supervises the pinned
`x402_treazury` executable.

- **Settlement observation:** MCP results leave settlement `PENDING`. The
  runner observes canonical Base `USED` or `EXPIRED_UNUSED` outcomes after
  batches and shutdown; `NOT_SIGNED` requires completed application execution
  and drained Store ownership. Managed signed responses also journal bounded
  seller receipt classifications (requiring Base and matching payer) without
  storing signatures or vendor error prose.
- **No run reuse:** A started or reserved run can never launch again (`report
  --run RUN_ID --eligibility` inspects windows without launching). To execute
  untouched cases after a stopped run:
  1. Inspect the report and canonical accounting; never copy reserved,
     dispatching, uncertain, or completed cases.
  2. Author a fresh manifest with new run and case IDs on the same treasury,
     registry, and configured pools (`start.mode = "funded_pools"`, zero new
     funding jobs unless separately reviewed).
  3. Prepare and run normally (prior charges remain on the original run).
- **Startup safety:** Rejects unresolved payments, unknown jobs, lost baseline
  history, and pending source work owned by another run. Known unprepared
  `ALLOCATED` or `QUOTED` refills (and positively closed unpublished intents)
  may remain paused under their original run permit during a zero-funding run
  or outside selected pools in a positive-funding run without transferring
  their permit.

## Reliability windows

The `reliability` scenario pairs identical cases across reviewed absolute UTC
windows (separate case IDs and reservations), verifying dispatch timestamps,
minimum spacing, and later-day boundaries. Overall `run_seconds` allows `1` to
`604800` seconds (7 days); call, phase, and cleanup bounds cap at 1 day. No
daemon schedules future windows automatically; for cross-day sampling, run
separate manifests against the same cumulative registry.

## Funding and pool adoption

- `funded_pools` with zero new-job authority can serve from existing `ACTIVE`
  wallets even without a `READY` standby when an earlier refill paused before
  preparation (still rejecting prepared or pending transactions and unresolved
  payments).
- Positive-funding runs require `funding.auto_fund = true`, registry job
  authority, and `--allow-funding`; selected pools require ready active and
  standby wallets at fresh startup (or bootstrap in `treasury_only` mode).
  Zero-job authority installs `--qualification-no-new-funding` regardless of
  `--allow-funding`.

## Canonical payment evidence

`BaseRpc::verify_transfer` in `src/rotation/base/receipt.rs` independently
verifies confirmed receipts against the admitted payer, payee, amount, and
nonce, requiring both USDC `Transfer` and
[AuthorizationUsed](https://eips.ethereum.org/EIPS/eip-3009#event) logs with
canonical block-hash rechecks (checking one queued receipt per pool per
reconciliation pass outside the payer lock). Consumed authorizations without
debit proof and paid cases that never pay cannot qualify a run.

```sh
make build CARGO_ARGS='--example live_integration'
target/release/examples/live_integration plan --manifest /private/path/run.toml
make test CARGO_ARGS='--test integration_preparation'
```

## Planning and concurrency assertions

`plan` uses `Deployment::show_config` offline (no catalogs, keys, wallet state,
or Tor). Concurrency assertions require overlapping MCP and application
intervals in one session, plus reviewed wallet bindings, distinct payers or
nonces, canonical debit proofs, admission-time ownership and balance evidence
(captured in the same Store command as admission while the pool gate is held:
ownership version 1 with payer role and reserve claim plus advisory balance
schema 3 `estimate_atomic`, while retaining `remaining == balance - exposure`
validation for legacy schema-2 records), and admission-to-response and
signed-request timing (reporting each overlap level separately).

## Rotation and lifecycle assertions

Managed admission and receipt events record lifecycle schema 2 (independent of
status JSON 6, bootstrap JSON 3, and DB schema 23): stable group generation,
independent reserve-slot generations, `ACTIVE`, `READY`, `RETAINED`, and
`RETIRED` roles, complete funding-job and source-operation lists in original
order, and SHA-256 digests of exact encrypted prepared records (no keys, signed
bytes, quotes, or upstream prose).

- **Natural `READY` consumption:** Stops depletion when a payment journal
  consumes the captured `READY` reserve slot (recording exact slot and
  generation), marking unused suffix cases `SKIPPED_TARGET_REACHED`.
- **Two-round lifecycle and restart:** Round 1 depletes `test_pool` ->
  consumes `READY` -> observes queued refill -> gracefully restarts
  `x402_treazury` while keeping Tor running (verifying exact
  `prepared_record_sha256` continuity across pre-restart, drained, and
  post-restart snapshots) -> dispatches service calls from both pools while
  refill is pending -> verifies independent Base credit and Zcash confirmation.
  Round 2 repeats without restarting.
- **Refill observation window:** Bounded by the manifest's remaining phase,
  run, and window deadlines (never lowering balances, forcing promotion, or
  artificially delaying funding).

## Registry authorization and preparation

```sh
target/release/examples/live_integration authorize-registry \
  --state-dir /private/treasury/state --authorization /private/auth.toml
```

- Stores the authorization ledger in `<state-dir>/live-integration/registry.sqlite`
  (owner-only, WAL mode), recording exact baseline jobs, wallets, pools,
  `cumulative_test_budget_usdc`, `cumulative_new_jobs` (paired with each
  manifest's `limits.new_funding_jobs`), and run ceilings offline.
- `prepare` pins binary build identity and SHA-256, fetches and hashes full
  catalogs (up to 128 MiB), validates tools and arguments, reloads the frozen
  catalogs to verify exact parity, and writes an immutable `prepared` record.
  Failed preparations never overwrite prior records.

## Wallet setup and readiness

Use production wallet commands (`wallet init`, `wallet addresses`,
`wallet sync`, `wallet backup`, `wallet status`) before authorization.

- **Starting modes:** `start.mode` supports `funded_pools` (verifies existing
  pools have ready `ACTIVE` wallets and, for positive-funding selected pools,
  `READY` wallets with no unresolved payments or conflicting source work) and
  `treasury_only` (starts from empty pools and waits for production bootstrap
  jobs). Unsigned runs use `start.mode = "funded_pools"` with `pools = []`,
  `total_test_budget_usdc = "0"`, and `unsigned = true` on every case,
  launching under `--qualification-unsigned` without funded EVM wallets.
- **Preflight checks:** Verifies DB schema 23, treasury UUID, wallet key
  readability (when funded), and clean baseline correspondence in one
  read-only pass.

## Ownership and authority

- **Registry owns qualification authority:** The runner never bypasses
  application caps, creates replacement payments, or modifies treasury state
  directly. Every supervised `x402_treazury` process receives an explicit
  qualification hook enforcing per-case, per-run, and cumulative registry
  budgets atomically before payment admission commits.
- **Child supervision and teardown:** First `SIGINT` or `SIGTERM` cancels
  runner waits while letting the supervised `x402_treazury` process drain
  accepted work before closing. Forced exit preserves reservations and
  journals.
- **Accounting boundaries:** Every API authorization charges
  `total_test_budget_usdc` and `cumulative_test_budget_usdc` before disclosure.
  Wallet funding is governed separately by reviewed job counts, reserve sizes,
  spendable Ironwood ZEC, and SDK fees. Under the Ironwood-only policy,
  transparent refund shielding is unsupported (any legacy shielding entries in
  archived snapshots remain read-only historical records).

## Owned-Tor qualification

Configure owned Tor inside the manifest's `[network]` table with
`tor_mode = "owned"`, `confinement = "macos_sandbox"`, and
`require_isolation_evidence = true`:

- Preparation and execution each launch a dedicated `tor` process with
  dedicated persistent state (owner-only data directory, cookie-authenticated
  loopback control port, `IsolateSOCKSAuth`), subscribing to stream and circuit
  events before spawning `x402_treazury`.
- On macOS, `x402_treazury` runs inside a temporary `sandbox-exec` profile
  permitting only loopback MCP inbound listeners and outbound connections to
  the dedicated Tor SOCKS port, verified with unconfined positive and confined
  negative IPv4/IPv6 TCP and UDP DNS `EgressProbe` checks before and after Tor
  shutdown. Run standalone confinement verification with:

  ```sh
  target/release/examples/live_integration qualify-confinement \
    --evidence-dir /private/new-confinement-dir \
    --socks-port SOCKS_PORT --mcp-port MCP_PORT
  ```

- **Keyless outage phase:** Configure a standalone keyless run
  (`--qualification-unsigned`) with a `tor_outage` phase (`kind = "tor_outage"`
  naming `warm_case`, `cached_case`, and `uncached_case`). The runner stops the
  dedicated Tor process after `warm_case` and verifies `cached_case` serves
  from cache while `uncached_case` fails within timeouts without direct
  fallback and negative probes stay denied. Never hand off a funded process
  into a keyless outage run. Generate a local macOS live-Tor fixture by running
  the ignored `prepare_real_tor_qualification_fixture` test in
  `tests/integration_preparation.rs` with `TREAZURY_M4_FIXTURE_DIR` set to a
  new absolute directory.

## Unsigned cover experiment

Optional keyless cover experiments run in an independent unsigned process
against a verified same-origin range resource, recording `TREAZURY_COVER_REPORT`
statistics (`episodes`, range counts, body bytes, padding bytes, peak streams)
without spending funds or mixing with funded provider qualification:

```sh
target/release/examples/live_integration cover-fixture \
  --directory /private/cover-dir --binary /private/x402_treazury \
  --tor-binary /path/to/tor --resource httpbin --profile ranges \
  --distribution log-normal --concurrency 1 --samples 4 \
  --socks-port SOCKS_PORT --mcp-port MCP_PORT
```

- Supported options include `--resource httpbin` or `rfc`, `--profile none`,
  `ranges`, `padding`, or `combined`, `--distribution uniform`, `exponential`,
  `weibull`, `log-normal` (default), or `weighted-discrete`,
  `--concurrency 1..3`, `--samples 1..16`, and manifest `post_batch_wait_ms`.
- Run offline distribution matrix coverage locally with:

  ```sh
  make test CARGO_ARGS='--lib cover::matrix_tests::cover_distribution_matrix' TEST_ARGS='--ignored --exact'
  ```

See [cover traffic](../../docs/cover-traffic.md) for the cover policy.

## Offline Tor re-audit

Re-audit retained Tor control logs offline without starting Tor or authorizing
paid calls:

```sh
target/release/examples/live_integration audit-tor \
  --state-dir /private/treasury/state --run RUN_ID \
  --output /private/new-audit.json
```

- `audit-tor` re-checks stream and circuit separation, remote hostnames,
  expected SOCKS credentials, confinement probe results, and required target
  coverage from retained control logs, printing sanitized verification output
  without mutating financial evidence.
- Requires `control-completion.json` for a complete audit; missing completion
  markers leave the audit as `observed_streams_only` and exit `3`.

### Owned-Tor receipt recovery

To recover missing Base debit proofs for pending authorizations after a run,
invoke `reconcile-receipts` (which launches a dedicated owned Tor process and a
confined keyless read-only RPC observer; 900-second deadline, 16 MiB output
cap, exiting `3` if receipts stay pending, never signing or replaying any
payment):

```sh
target/release/examples/live_integration reconcile-receipts \
  --state-dir /private/treasury/state --run RUN_ID \
  --evidence-dir /private/new-receipt-dir
```

## Shielded-refund evidence and qualification

Before authorizing any live Zcash scan or controlled refund experiment:

1. Inspect retained private run reports, `wallet status --json`, and offline
   `wallet notes --config FILE` to establish existing Ironwood notes, pending
   source work, and historical swap bindings.
2. Verify any target refund receiver is an Orchard-receiver-only Unified
   Address derived under `TransparentAddressDiscovery::disabled()`.
3. Confirm dry quote format acceptance separately with
   `make run CARGO_ARGS='--offline --example quote_near -- --diagnose --refund-address ADDRESS'`
   (add `--network-config /path/network.toml` to route over Tor) using a public
   test address; dry quotes check format only, never live refund delivery.
4. Any live funded refund test requires separate operator approval, bounded
   funding job count, explicit registry authorization, and post-refund
   verification via confirmed Ironwood SDK receipt at the exact saved refund
   receiver (never relying on NEAR `REFUNDED` status alone and never using
   transparent shielding). See
   [refund policy](../../docs/wallet-rotation.md#refunds-state-and-recovery).
