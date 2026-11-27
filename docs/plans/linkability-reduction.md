# Linkability reduction for managed swaps and reserve sizing

## Objective, scope, and baseline

Reduce two distinct on-chain and explorer linkability vectors in managed Zcash
funding swaps without introducing new spending caps, unsolicited network
probes, or parallel financial ledgers:

1. **Rotation linkability:** Replace the single `30-120` second refill
   opportunity timer with a volatile, vacancy-triggered two-tier delay system:
   a long randomized **lazy** tier (`24-48` hours by default, optionally
   configurable) when the group still has spendable runway, and a short
   **urgent** tier (`30-120` seconds by default, optionally configurable) when
   no funded `READY` reserve remains and the sole `ACTIVE` wallet is missing,
   low, or hit structured insufficient funds.
2. **Usage linkability:** Reduce default `reserves.small` wallet sizes toward
   the live route minimum so wallets rotate more often per dollar spent, using
   a `[floor, min(2 * floor, max_funding_amount_usdc)]` default range where
   `floor` starts from the wallet's finite `max_api_payment_usdc` (or
   `min_funding_amount_usdc` when unlimited), learns proven `BridgeMinimum`
   values passively in memory with a 24-hour monotonic TTL, never silently
   raises the `$3.00` default ceiling, and falls back to
   `max_funding_amount_usdc` with a `WARN` when minimum quoting fails with a
   generic HTTP 400 below the ceiling.
3. **Quote parity and solver coverage:** Send 1Click's documented
   `quoteWaitingTimeMs = 3000` on every newly requested swap quote (both
   managed funding and direct swaps) to collect broader solver bids, improve
   small-swap fill rates and minimum discovery, and match the canonical NEAR
   SDK value instead of standing out with unusual timing settings.

**Current status:** Active plan committed with operator-confirmed parameters;
next step is the Gate 0 pre-implementation independent design review. No
implementation commits exist yet.

**Starting baseline:** Branch `linkability-improvements-plan`, commit `03b45e4c`.

**Current behavior vs. proposed behavior:**

- **Quote auction window:** Currently neither `src/rotation/near.rs` nor
  `src/direct_swap/route.rs` sends `quoteWaitingTimeMs`, so 1Click defaults to
  `0` ms (fastest single response). Proposed: send `quoteWaitingTimeMs: 3000`
  on every new quote request, keep `quoteWaitingTimeMs` optional (absent or
  `3000`) when validating saved pre-upgrade quotes, and treat it as
  non-binding metadata on `/v0/status`.
- **Refill scheduling:** Currently `Opportunities` in
  `src/rotation/funding/scheduler.rs` runs a single per-group volatile uniform
  `30..=120` second continuous timer (mean 75 seconds) that invokes both
  `promote_low_active` and `fill_funding_vacancies`, and `Store.funding_changes`
  is only awaited when deferred bootstrap groups exist. Proposed: decouple
  local `promote_low_active` role transitions from network swap allocation,
  gate `fill_funding_vacancies` with a volatile, vacancy-triggered two-tier
  timer (`24-48` hours lazy, `30-120` seconds urgent, both optionally
  configurable under `[funding]`), wake the scheduler unconditionally via
  `Store.funding_changes` in `Mode::Serving`, and latch an in-memory
  per-group `(pool_id -> active_wallet_id)` `active_insufficient_funds` map on
  `Store` so Base RPC balance refreshes cannot race-clear the urgency trigger
  and reactive `ACTIVE` -> `READY` failovers stay lazy.
- **Small reserve sizing and route minimum:** Currently default
  `reserves.small` resolves at deserialization to a fixed `$2.00-$3.00` range
  (`DEFAULT_SMALL_MINIMUM_USDC` and `DEFAULT_SMALL_MAXIMUM_USDC` in
  `src/rotation/config.rs`), independent of `max_api_payment_usdc`, and
  `configure_reserves` persists `$2.00` into `reserve_claims.target`. During
  quoting, `src/rotation/funding.rs` already catches `BridgeMinimum` on the
  dry `EXACT_OUTPUT` preview and redraws within `[floor, maximum_output]` for
  that single operation only. Proposed: leave auto-sized default
  `reserves.small` unset in `ReserveConfig` (serializing as `{}`), represent
  it as `SizingPolicy::AutoSmall { seed, maximum }`, persist
  `(seed, small_ceiling)` into `reserve_claims` (no SQLite schema migration),
  draw directly from `[floor, min(2 * floor, small_ceiling)]` using a single
  24-hour monotonic in-memory `CachedRouteMinimum` slot for proven
  `BridgeMinimum` values (`chosen < N <= ceiling`), fall back once to
  `max_funding_amount_usdc` (`$3.00` default) with a `WARN` on generic HTTP
  400 below the ceiling (`AutoSmall` only), and emit startup `INFO` and `WARN`
  diagnostics once per resolved managed group in `initialize_wallets_mode`
  (`src/deployment.rs`).

**Preserved invariants:**

- ZEC remains Ironwood only; managed funding still derives spendability and
  reconciliation from blockchain and SDK facts with no parallel monetary
  ledger or new ZEC spending budgets.
- Journal-before-disclosure, exact accepted inputs, existing SQLite schema
  version (v23) and state formats, persisted quote compatibility, `auto_v1_`
  pool names, Tor token domains, and `X402_*` environment contracts remain
  intact.
- All outbound HTTP requests remain centralized in `src/network.rs` with
  per-wallet Tor circuit isolation (`IsolationId::evm`).
- Idle treasuries and deferred `[wallet_bootstrap]` groups do not generate
  unsolicited startup traffic to 1Click.
- Untrusted 1Click 400 error text can never raise swap size past
  `max_funding_amount_usdc` (`$3.00` default for small reserves, or the
  operator's explicit configured ceiling).
- `wallet bootstrap`, `Mode::Once`, and `Mode::Explicit` continue to allocate
  promptly without waiting on lazy serving delays.

**Owning guides and destinations for enduring rules:**

- [docs/wallet-rotation.md](../wallet-rotation.md): behavioral rules and
  privacy rationale for two-tier vacancy-triggered refill delays (including
  low small-swap Base USDC volume on NEAR Intents), early rotation decoupling,
  `quoteWaitingTimeMs = 3000`, default `[floor, 2 * floor]` sizing, passive
  minimum learning, limitations, fallbacks, 24-hour TTL reset, and hard
  ceiling.
- [docs/configuration.md](../configuration.md): reserve defaults, optional
  `[funding]` lazy and urgent timer settings, `max_api_payment_usdc` floor and
  `reserves.large` interaction, startup `INFO` and `WARN` diagnostics, and
  ceiling enforcement.
- [docs/network-egress.md](../network-egress.md): updated
  [timer and budget inventory](../network-egress.md#remaining-timers-and-budgets)
  covering both lazy and urgent refill tiers and the 3,000 ms quote wait.
- [docs/local-resource-limits.md](../local-resource-limits.md): single-slot
  volatile `CachedRouteMinimum` storage and 24-hour monotonic TTL.
- [docs/development.md](../development.md): update
  [diagnostic context](../development.md#diagnostic-context) and
  [log severity](../development.md#log-severity) to document the startup
  cap-exceeds-ceiling `WARN` and explicitly record the `AutoSmall`
  generic-400 fallback `WARN` as an intentional unexpected-loss-of-functionality
  `WARN` (emitted at most once per operation, safe fields only, documenting
  why it is permitted alongside the fallback).
- [README.md](../../README.md): concise operator guidance explaining the
  volatile `24-48` hour lazy background refill behavior and advising operators
  not to run `x402_treazury` on a short restart loop.
- [README_ADVANCED.md](../../README_ADVANCED.md),
  [examples/README.md](../../examples/README.md),
  [docs/architecture.md](../architecture.md),
  [docs/public-swap-demo.md](../public-swap-demo.md),
  [docs/usage/configuration.md](../usage/configuration.md),
  `examples/deployments/*.toml`, and `tests/live/deployments/*.toml`: remove
  stale copies of `30-120` second single-tier timing and static `$2-3` small
  reserve defaults.
- [AGENTS.md](../../AGENTS.md): verify existing guide links cover all touched
  areas.

## Agent handoff protocol

An incoming agent resuming this plan must first run these read-only checks to
reconcile the working tree and Git history with the checkpoint table:

```sh
git status --short
git rev-parse --abbrev-ref HEAD
git log --oneline --decorate 03b45e4c..HEAD
```

Skip any row in `### Execution checkpoints` already marked `Done` whose
recorded commit SHA and validation evidence match the checkout.

**Standard validation commands** (run from repository root via `Makefile`):

```sh
# Formatting and fast lint
make fmt-check
make clippy

# Targeted suites (confirm nonzero test counts in output)
make test CARGO_ARGS="--lib" TEST_ARGS="rotation::near::"
make test CARGO_ARGS="--lib" TEST_ARGS="direct_swap::route::"
make test CARGO_ARGS="--lib" TEST_ARGS="rotation::funding::"
make test CARGO_ARGS="--lib" TEST_ARGS="rotation::store::early_rotation"
make test CARGO_ARGS="--lib" TEST_ARGS="rotation::config::"
make test CARGO_ARGS="--lib" TEST_ARGS="wallet_cli::preview::"
make test CARGO_ARGS="--test managed"
make test CARGO_ARGS="--test managed_config"
make test CARGO_ARGS="--test public_demo_profiles"

# Compiler warning gate (all targets including zcash-regtest)
make check-clippy

# Documentation check (links, anchors, whitespace)
git diff --check

# Full qualification before final integration gate
make check
```

Note: localhost socket tests (including `rotation::funding::redraw_tests`,
`rotation::near::tests`, and `--test managed`) require socket-capable
execution when the default sandbox blocks localhost binding. Run feature
configurations sequentially.

### Execution checkpoints

| Phase or item | Target | Commit or ref | Validation | Review | Status |
| --- | --- | --- | --- | --- | --- |
| Plan finalization | Commit finalized `docs/plans/linkability-reduction.md` | This commit | `git diff --check` clean | N/A (Gate 0 next) | Done |
| Gate 0: Pre-implementation design review | Fresh independent design review of finalized plan | Pending | N/A | Pending | Pending |
| LR-1: `quoteWaitingTimeMs = 3000` for all 1Click swaps | `src/rotation/near.rs`, `src/direct_swap/route.rs`, tests, owning docs | Pending | Pending | Pending | Pending |
| LR-2: Two-tier refill scheduling (lazy vs. urgent) | `src/rotation/config.rs`, `src/rotation/funding/scheduler.rs`, `src/rotation/store/`, `src/rotation/funding.rs`, tests, owning docs | Pending | Pending | Pending | Pending |
| LR-3: Default `[floor, 2 * floor]` small reserves and passive minimum learning | `src/rotation/config.rs`, `src/rotation/assignment.rs`, `src/rotation/store/`, `src/rotation/funding.rs`, `src/deployment.rs`, `src/wallet_cli/preview.rs`, tests, owning docs | Pending | Pending | Pending | Pending |
| LR-4: Operator docs, `README.md`, and example alignment | `README.md`, `README_ADVANCED.md`, `examples/README.md`, `docs/*.md`, `docs/usage/configuration.md`, `examples/deployments/*.toml`, `tests/live/deployments/*.toml` | Pending | Pending | Pending | Pending |
| Gate 1: Combined integration, complexity, and doc review | Net diff `03b45e4c..HEAD`, all fixups, CogC and CycC, writing style | Pending | Pending (`make check-clippy`, `make check`) | Pending | Pending |
| Whole-changeset loop and completed-plan cleanup | Clean combined review, remove plan file, review cleanup SHA | Pending | Pending | Pending | Pending |

#### Open findings and next action

- Run the Gate 0 pre-implementation independent design review in a fresh
  subagent against this committed plan before starting `LR-1`.

## Design and rationale

### 1. `quoteWaitingTimeMs = 3000` for all 1Click swaps (`LR-1`)

**Behavior:**

- Define a shared constant `QUOTE_WAITING_TIME_MS: u64 = 3000` for 1Click
  `/v0/quote` payloads.
- Include `"quoteWaitingTimeMs": 3000` in every newly constructed quote
  request in both `src/rotation/near.rs` (free function `request_on`, called
  by `NearClient::request`, covering both dry `EXACT_OUTPUT` sizing previews
  and executable `EXACT_INPUT` quotes) and `src/direct_swap/route.rs`
  (`request_on`, covering direct send and receive previews and executable
  quotes).
- **Pre-upgrade persisted quote compatibility:** Keep `quoteWaitingTimeMs`
  optional (absent or integer-equivalent `3000`) in
  `validate_quote_structure` (`src/rotation/near.rs`) and `validate_request`
  (`src/direct_swap/route.rs`) when validating saved requests, rejecting any
  other value. Pre-upgrade quotes saved in SQLite without `quoteWaitingTimeMs`
  continue to pass `validate_saved_quote` at `prepare` time.
- **Echo validation across `/v0/quote` and `/v0/status`:** In both
  `check_echo` implementations (`src/rotation/near.rs` and
  `src/direct_swap/route.rs`, adding an `accepting_quote` parameter to
  `near::check_echo` to match `route::check_echo`), validate
  integer-equivalent `3000` on `/v0/quote` when sent in the request, and treat
  `quoteWaitingTimeMs` as non-binding metadata on `/v0/status` so upstream
  status normalization, omission, or float formatting (`3000.0`) never breaks
  status polling on already-funded swaps.
- Keep `tests/fixtures/near/public-quote.json` untouched as the authentic
  legacy omitted-field regression (stripping `quoteWaitingTimeMs` from the
  synthetic request in
  `captured_public_quote_accepts_normalization_but_preserves_policy`), and
  update `src/direct_swap/route.rs:514` so the test echo helper defaults
  `quoteWaitingTimeMs` to `0` only when absent from `request`.

**Rationale to document in owning guides:**

- Omitting `quoteWaitingTimeMs` defaults 1Click to `0` ms, which returns only
  the fastest solver response. Waiting `3000` ms matches the NEAR Intents
  Message Bus auction window and canonical 1Click SDK examples.
- Collecting bids from slower solvers improves the chance of finding a
  market maker willing to quote smaller swap sizes and improves pricing,
  while blending into standard 1Click SDK traffic (`3000` ms) rather than
  standing out with an unusual wait duration in echoed `quoteRequest` fields.
- Adding 3 seconds per quote call (~6 seconds across dry preview and
  executable quote) stays far below `[network].read_timeout_seconds` (default
  60 seconds) and does not meaningfully delay background funding.

### 2. Two-tier refill scheduling (`LR-2`)

**Motivation to document in owning guides:**

- Small ZEC-to-Base-USDC swap volume on NEAR Intents is low, so a `30-120`
  second refill timer creates a tight temporal link between spending down an
  `ACTIVE` wallet on Base and broadcasting a replacement swap. An observer
  correlating Base spend activity with Zcash or NEAR Intents Explorer
  timestamps can link the outgoing API wallet to the incoming funding swap.
- Separating local wallet rotation from network swap initiation and stretching
  non-urgent refills across a `24-48` hour random window breaks both sides of
  that link: the pre-funded `READY` wallet ages before its first spend, and
  after promotion the replacement swap waits `24-48` hours as long as the
  group still has spendable runway.

**State, signaling, and timer rules:**

- **Decouple local early rotation from network refill (read-only pre-check):**
  - `promote_low_active` only flips local SQLite roles (`READY` -> `ACTIVE`,
    old `ACTIVE` -> `RETAINED`) and performs no network I/O, but it acquires
    an `IMMEDIATE` SQLite write transaction.
  - Share a read-only `active_wallet_health` helper in
    `src/rotation/store/early_rotation.rs` (using the same per-wallet HMAC
    threshold as `promote_low_active`) so the scheduler inspects all admitted
    groups in a single read-only SQLite call on `Store.funding_changes` wakes,
    and invokes `promote_low_active` only on groups that actually have a
    funded `READY` reserve and a qualifying low `ACTIVE` wallet.
- **Urgent vs. lazy predicate for a group with a reserve vacancy:**
  - Share a read-only `is_replaceable_blocked_job` helper in
    `src/rotation/store/reserve_replacement.rs` combining `blocked_job` with
    `replacement_allowed(&tx, &job)` so an irreplaceable completed job never
    arms a spurious vacancy loop. A group has a vacancy when any enabled
    `reserve_claims` slot has `wallet_id IS NULL` OR holds a replaceable
    `blocked_job`.
  - **Lazy tier:** The group has at least one funded `READY` reserve, OR it
    has a funded `ACTIVE` wallet that has not crossed its `promote_low_active`
    threshold and does not match the group's latched
    `active_insufficient_funds` wallet ID.
  - **Urgent tier:** The group has **no** funded `READY` reserve AND any of:
    1. Its sole `ACTIVE` wallet is at or below its `promote_low_active`
       threshold (the primary urgent trigger; do not require `used == true` on
       the urgency check if an `ACTIVE` wallet's observed balance is already
       at or below threshold, since a newly funded wallet starts well above
       `credit / 4`),
    2. It has no funded `ACTIVE` wallet at all (initial non-deferred startup
       or first-use `[wallet_bootstrap]` activation), or
    3. A payment attempt hit structured insufficient funds on that exact sole
       `ACTIVE` wallet (covering a single spend or burst that jumps from above
       the 25% early-rotation threshold straight into insufficient funds).
- **Reliable wakeups and wallet-scoped insufficient-funds latch (reusing
  `funding_changes`):**
  - Reuse `Store.funding_changes` (`Arc<tokio::sync::Notify>`, which stores a
    permit via `notify_one()` so notifications are never dropped while the
    scheduler is busy) rather than per-pool `capacity_changes`
    (`notify_waiters()`).
  - Track `admitted_groups: BTreeSet<String>` separately from armed vacancy
    timers in `Opportunities` so disarming a vacancy-free `[wallet_bootstrap]`
    group does not leave it in `deferred_groups` indefinitely.
  - Await `funding_changes` in a single `select!` branch in `Mode::Serving`
    whenever `!stopping` (even when `deferred_groups` is empty), running both
    deferred-group discovery (when `!deferred_groups.is_empty()`) and
    admitted-group health, promotion, and vacancy reconciliation from that one
    wake. If reconciliation hits `SQLITE_BUSY` after consuming a permit,
    schedule `allocation_retry` so no urgent transition is dropped.
  - Call `self.funding_changes.notify_one()` from `journal_authorization`,
    `low_balance`, `finish_balance` (only when the updated wallet's current
    role is `ACTIVE`, avoiding wakes from `RETAINED`, `RETIRED`, or
    zero-balance `ALLOCATING` refreshes), `promote_low_active`, and from
    `record_funding_status` and `reconcile_funding_phase` when a funding job
    closes without initial credit (waking an idle scheduler when a
    `blocked_job` becomes replaceable).
  - Maintain an in-memory `active_insufficient_funds: BTreeMap<String, String>`
    (`pool_id -> active_wallet_id`) on `Store`. Record `(pool_id, wallet_id)`
    inside `low_balance` ONLY when `wallet_id`'s current role is `ACTIVE`
    (never for `RETAINED` wallets). In the urgency check, treat the latch as
    active only when
    `active_insufficient_funds.get(pool_id) == Some(current_active_wallet_id)`.
    Clear `pool_id` whenever `journal_authorization` or `promote_low_active`
    promotes a new `ACTIVE` wallet (preventing reactive `ACTIVE` -> `READY`
    failovers from falsely arming the urgent timer), as well as when
    `record_initial_credit` funds a new reserve or `fill_funding_vacancies`
    allocates a replacement. This closes both the `finish_balance` Base RPC
    clear race and reactive-failover false urgency.
- **Vacancy-triggered volatile timer and configuration:**
  - Defaults:
    - **Urgent tier:** uniform `30..=120` seconds (integer seconds, mean 75s).
    - **Lazy tier:** uniform `86400..=172800` seconds (`24..=48` hours,
      integer seconds, mean 36 hours).
  - Expose optional `[funding]` settings validated in
    `FundingConfig::validate_swap()` (so direct-swap-only configs with
    `[funding]` also reject invalid delay fields) as
    `1 <= min_seconds <= max_seconds <= 2_592_000` (30-day upper bound, well
    within `u32::MAX`, paired with `checked_add` on `Instant + Duration` so
    deadline math cannot panic; the two tiers are validated independently so
    tests and operators can narrow both windows cleanly):
    - `urgent_refill_min_seconds` (default `30`),
      `urgent_refill_max_seconds` (default `120`),
    - `lazy_refill_min_seconds` (default `86400`),
      `lazy_refill_max_seconds` (default `172800`).
  - Pass configured refill delays through a `FundingWorker::with_refill_delays`
    builder method (matching `with_bootstrap_policy`) to avoid churning
    unrelated test constructors.
  - **Vacancy-triggered arming:** Keep a group's entry in `Opportunities`
    disarmed when the group has no vacancy (including no replaceable
    `blocked_job`). When a vacancy appears, arm the group at
    `now + draw_lazy()` or `now + draw_urgent()` according to its current
    urgency. When `funding_changes` wakes the scheduler and a group waiting on
    a lazy deadline transitions to **urgent**, clamp its deadline to
    `min(existing_deadline, now + draw_urgent())`. Payments never reset or
    extend an already-sooner deadline. Disarm the group once its vacancies are
    filled.
  - Keep `Opportunities` **volatile (in-memory only)** with no SQLite schema
    addition. Document clearly in `README.md` and `docs/wallet-rotation.md`
    that restarting `x402_treazury` redraws the `24-48` hour lazy window, so
    operators should not run `x402_treazury` on a short restart loop (if
    `ACTIVE` runs low during frequent restarts, the urgent tier still funds a
    replacement wallet, but background pre-aging is reduced).
  - `wallet bootstrap`, `Mode::Once`, and `Mode::Explicit` continue to bypass
    serving opportunity delays.

### 3. Default `[floor, 2 * floor]` small reserves and passive minimum learning (`LR-3`)

**Motivation to document in owning guides:**

- Funding small wallets with a large static default concentrates many API
  calls onto a single Base address, increasing cross-provider and
  cross-session usage linkability.
- Sizing default small wallets close to the live route minimum with a
  proportional `[floor, min(2 * floor, max_funding_amount_usdc)]` random band
  rotates addresses more frequently per dollar spent while preserving amount
  randomization so funding swaps do not cluster at one repeated dollar value.
- Clamping the default small floor to at least the wallet's finite
  `max_api_payment_usdc` ensures a newly funded small wallet targets at least
  the largest single API call permitted on that wallet, even when the bridge
  accepts smaller swaps.

**Sizing model and operator config interactions:**

- Distinguish **auto-sized default `reserves.small`** (when the operator omits
  `[wallets.<name>.reserves.small]` or leaves it empty) from **explicitly
  configured reserves** (`funding_amount_usdc`, or explicit
  `min_funding_amount_usdc` and `max_funding_amount_usdc`) and from
  `reserves.large`.
- **Serde representation and roundtrip compatibility:**
  - Keep all three `Option<String>` fields of `ReserveConfig` as `None` for
    auto-sized default `reserves.small` (serializing `reserves.small` as
    `{}`). This preserves exact JSON and TOML roundtrips
    (`tests/managed_config.rs:690`, `tests/assignment.rs:266`) and satisfies
    `profile["reserves"]["small"].is_object()`
    (`src/qualification/funding/registry_state.rs:258`), without a
    `#[serde(skip)]` flag that would silently drop default status across
    serialization.
  - Update `ReserveConfig::resolved(SizingSlot::Small)` and
    `ReservesConfig::validate()` to accept `(None, None, None)` for `small`
    while continuing to reject empty `large` or partial bounds.
  - Represent auto-sized small reserves at policy evaluation time with
    `SizingPolicy::AutoSmall { seed: u64, maximum: u64 }`. Keep cap-free
    `ReservesConfig::initial_inventory_minimum_atomic()` working for existing
    callers (`tests/managed_config.rs:477`) and add cap-aware
    `small_policy(cap)` and `initial_inventory_minimum_atomic_for_cap(cap)`
    helpers for `WalletConfig`, `assignment.rs`, `preview.rs`, and
    `store/capacity.rs`.
- **Hard ceiling:** For auto-sized default `reserves.small`, the ceiling
  remains `DEFAULT_SMALL_MAXIMUM_USDC` (`$3.00`). `x402_treazury` never
  silently raises `$3.00` based on `max_api_payment_usdc` or an untrusted
  1Click 400 error.
- **Explicit reserve bounds and startup diagnostics:**
  - Explicitly configured `reserves.small` and all `reserves.large` settings
    remain authoritative and do not run automatic minimum learning or
    generic-400 ceiling fallback.
  - Expose pure inspection helpers on `WalletConfig` and `ReservesConfig`, and
    emit startup diagnostics once per resolved managed wallet group inside
    `initialize_wallets_mode` in `src/deployment.rs`
    (`target: "x402_treazury::startup"`, covering both
    `ManagedSetup::Treasury` and `ManagedSetup::StoreOnly` while keeping
    `WalletConfig::validate()`, `show_config`, `load`, `load_for_inspection`,
    and `wallet preview` silent):
    - Startup **`INFO`** whenever `reserves.small` bounds are explicitly
      configured, explaining that explicit small-reserve bounds disable
      automatic minimum sizing and may increase usage linkability.
    - Startup **`WARN`** whenever a wallet's finite `max_api_payment_usdc`
      exceeds that wallet group's largest configured reserve ceiling, computed
      in `U256` as `U256::from(small_max.max(large_max.unwrap_or(0)))`.
- **Base floor (`seed`) for auto-sized default `reserves.small`:**
  - Compute `seed` in `U256` (`cap.min(U256::from(small_ceiling)).to::<u64>()`)
    so large finite decimals clamp cleanly to `small_ceiling` instead of
    overflowing `u64::try_from`:
    - When `max_api_payment_usdc` is finite and positive AND either
      `reserves.large` is disabled OR
      `max_api_payment_usdc <= U256::from(small_ceiling)`:
      `seed = cap.min(U256::from(small_ceiling)).to::<u64>()`, with **no
      artificial lower safety minimum** below `max_api_payment_usdc`.
    - When `max_api_payment_usdc` is `"none"`, `"off"`, `"0"`, or exceeds
      `U256::from(small_ceiling)` while `reserves.large` is enabled:
      `seed = DEFAULT_SMALL_MINIMUM_USDC` (`$2.00`).
- **Database authority alignment (zero schema migration):**
  - Pass `limit: Option<U256>` (already available in `ensure_capacity_group`
    and `configure_capacity_group` in `src/rotation/store/capacity.rs`) into
    `configure_reserves` so auto-sized default `reserves.small` persists
    `(seed, small_ceiling)` into `(reserve_claims.target,
    reserve_claims.maximum_output)`.
  - Schema version stays at v23; `quote_term_authority`,
    `save_funding_quote_for_operation`, and `accepted_quote_suits_current_claim`
    accept sub-`$2.00` targets cleanly and automatically refresh unprepared
    quotes if `max_api_payment_usdc` changes across restarts.
  - Define the `show_config` contract: `shown["wallets"][name]["reserves"]["small"]`
    serializes as `{}` for auto-sized groups while `wallet_summary`
    (`src/rotation/assignment.rs`) and `wallet preview`
    (`src/wallet_cli/preview.rs`) report the effective `(seed, small_ceiling)`
    offline without making 1Click calls. Update assertions in
    `tests/managed_config.rs` and `tests/public_demo_profiles.rs`.

**Passive minimum learning inside the existing redraw loop, limitations, and
safe fallbacks:**

- **Single-loop passive learning (no separate probe phase, no `200 OK`
  caching, no wire disclosure of `max_api_payment_usdc`):**
  - Managed funding in `Backend` (`src/rotation/funding.rs`) uses one fixed
    route (`near::ZEC` -> `near::USDC`) and one immutable
    `self.funding.confidentiality` per `Backend`. Store a single O(1) volatile
    slot `Arc<Mutex<Option<CachedRouteMinimum>>>` (holding
    `(minimum_atomic: u64, observed_at: tokio::time::Instant)`) on `Backend`,
    shared with `Observations`, with a **24-hour monotonic TTL** (documented
    in `docs/local-resource-limits.md`; no keyed map or eviction code, and no
    dependency on `near.assets()` during `quote_prerequisite`). Provide a
    constructor or default initializer so test callers building `Backend`
    (`src/deployment/bootstrap_actor_tests.rs`,
    `src/treasury/lifecycle_tests/{worker,discovery}.rs`,
    `tests/support/funding_backend.rs`) stay clean.
  - Scope both cache reads and writes and generic-400 ceiling fallback strictly to
    `SizingPolicy::AutoSmall`. Store **only proven `BridgeMinimum(N)` values**
    where `chosen < N <= ceiling`. Never cache `200 OK` responses (which are
    upper bounds and would let higher-cap groups poison lower-cap groups),
    never cache `N > ceiling` (so `FundingCondition::AmountLimit` backoff
    retries observe upstream fee recovery), and never cache `N <= chosen`
    (preserving `near_bridge_minimum_not_increasing`).
  - Change `src/rotation/near.rs:428` from untyped `anyhow::bail!("near_http_400")`
    to `return Err(HttpStatus(400).into())` (preserving `"near_http_400"`
    `Display` and `near_diagnostic`) so `quote_with_clock` detects generic
    HTTP 400 structurally.
  - On each `Allocated` job's `quote_prerequisite` (`src/rotation/funding.rs`),
    for `SizingPolicy::AutoSmall { seed, maximum }`:
    - Read the single `CachedRouteMinimum` slot (clearing it if expired).
    - Compute `floor = cached.unwrap_or(0).max(seed).min(maximum)` and
      `upper = min(floor.saturating_mul(2), maximum)`.
    - Draw `preferred` uniformly from `[floor, upper]` via
      `policy.draw_from_floor(floor)` and pass it into
      `select_quote_terms_for_attempt` (which preserves an existing
      `quote_target` across transient failures and restarts; if a previously
      saved `quote_target` is below the current cached `floor`, either raise
      it via `reselect_quote_terms` or let `quote_with_clock` redraw once on
      `BridgeMinimum`).
  - In `quote_with_clock`, send the dry `EXACT_OUTPUT` preview at `chosen`
    with `quoteWaitingTimeMs = 3000`:
    1. **200 OK:** Proceed directly to `executable_quote` (one dry preview,
       no extra 3-second wait, no disclosure of `max_api_payment_usdc`, and
       full restart compatibility).
    2. **HTTP 400 with `BridgeMinimum(N)`:** Enforce `N > chosen`
       (`near_bridge_minimum_not_increasing`). If `N > ceiling`, fail with
       `FundingCondition::AmountLimit` without caching `N`. If
       `N <= ceiling`, update `CachedRouteMinimum` with `(N, now)` (for
       `AutoSmall`), redraw via `request.sizing.draw_from_floor(floor, ...)`,
       persist via `reselect_quote_terms`, and continue the loop.
    3. **HTTP 400 generic failure (`HttpStatus(400)`, no numeric
       `BridgeMinimum`, e.g., solver refusal or changed 1Click error prose)
       for `SizingPolicy::AutoSmall` when `chosen < ceiling`:** Emit one
       `WARN` per operation (naming the wallet, job, and fallback ceiling with
       safe structured fields and no raw upstream 400 prose), persist
       `ceiling` (`max_funding_amount_usdc`) via `reselect_quote_terms`, and
       retry the dry preview once at `ceiling`. When `chosen == ceiling`, or
       for explicit `SizingPolicy::Fixed` and `SizingPolicy::Range`, do not
       retry at `ceiling` or emit the fallback `WARN`.
    4. **5xx, timeout, or transport failure:** Do not cache a minimum or fall
       back to `ceiling`; let normal quote retry backoff handle the transient
       outage.
- **Documented limitations of `bridge_minimum()` (`src/rotation/near.rs`):**
  1. **Undocumented error prose:** `bridge_minimum()` parses the exact literal
     prefix `"Amount is too low for bridge, try at least "` followed by ASCII
     digits. If 1Click changes phrasing or formatting, parsing returns `None`
     and the generic-400 fallback to `max_funding_amount_usdc` (with `WARN`)
     keeps funding working safely.
  2. **Swap-type dependent units:** On `EXACT_OUTPUT` the digits represent
     destination Base USDC atomic units; on `EXACT_INPUT` they represent
     origin zatoshi. Minimum learning and caching must ONLY consume
     `BridgeMinimum` from `EXACT_OUTPUT` Base USDC dry previews, never from
     `EXACT_INPUT` quotes.
  3. **Bridge floor vs. solver liquidity floor:** `BridgeMinimum` reports the
     Omni (OMFT) bridge fee floor, not whether a market maker will fill that
     size. Combining `quoteWaitingTimeMs = 3000` with the generic-400 fallback
     to `max_funding_amount_usdc` (with `WARN`) handles solver illiquidity
     cleanly.
  4. **Slippage semantics:** `[floor, upper]` governs the nominal
     `EXACT_OUTPUT` target range; `funding.slippage_bps` (default 500 bps)
     remains applied on the executable `EXACT_INPUT` quote.
- **24-hour monotonic TTL:** Using `tokio::time::Instant` prevents NTP wall
  clock jumps from freezing cached entries and allows deterministic
  `#[tokio::test(start_paused = true)]` expiry coverage. Expiry and process
  restart reset the cache so temporary fee spikes do not permanently ratchet
  wallet sizes upward or collapse `[floor, 2 * floor]`.

## Ordered commit boundaries

### `LR-1`: Send `quoteWaitingTimeMs = 3000` on all 1Click quote requests

- **Scope:**
  - `src/rotation/near.rs`: add `QUOTE_WAITING_TIME_MS = 3000` to free
    function `request_on`; keep `quoteWaitingTimeMs` optional (absent or
    `3000`) in `validate_quote_structure` for legacy persisted quotes; add
    `accepting_quote` to `check_echo`, validate integer-equivalent `3000` on
    `/v0/quote`, and treat it as non-binding metadata on `/v0/status`.
  - `src/direct_swap/route.rs`: add `quoteWaitingTimeMs = 3000` to
    `request_on`; keep `quoteWaitingTimeMs` optional (absent or `3000`) in
    `validate_request`; apply the same `/v0/quote` and `/v0/status` echo
    rules in `check_echo`.
  - Keep `tests/fixtures/near/public-quote.json` untouched as the legacy
    omitted-field regression; update unit and fixture tests
    (`src/rotation/near.rs`, `src/direct_swap/route.rs`,
    `src/rotation/funding/redraw_tests.rs`,
    `src/treasury/lifecycle_tests/`).
  - Document the 3,000 ms quote wait and its rationale in
    `docs/wallet-rotation.md` and `docs/network-egress.md`.
- **Acceptance evidence:**
  - Tests prove both managed funding (`sizing_preview` and `executable_quote`)
    and direct swap quotes send `quoteWaitingTimeMs: 3000`, reject mismatched
    echoes on new `/v0/quote` responses, tolerate status metadata variations
    on `/v0/status`, and still validate persisted legacy quotes without
    `quoteWaitingTimeMs`.
  - Exact-SHA independent review passes.

### `LR-2`: Two-tier refill scheduling (lazy vs. urgent)

- **Scope:**
  - `src/rotation/config.rs`: add optional `[funding]` fields
    `urgent_refill_min_seconds` (default `30`), `urgent_refill_max_seconds`
    (default `120`), `lazy_refill_min_seconds` (default `86400`), and
    `lazy_refill_max_seconds` (default `172800`), validated in
    `FundingConfig::validate_swap()` (`1 <= min_seconds <= max_seconds <=
    2_592_000`) with `checked_add`.
  - `src/rotation/store.rs`, `src/rotation/store/balances.rs`,
    `src/rotation/store/early_rotation.rs`, `src/rotation/store/funding.rs`,
    and `src/rotation/store/reserve_replacement.rs`: emit
    `funding_changes.notify_one()` from `journal_authorization`,
    `low_balance`, `finish_balance` (`ACTIVE` role only), `promote_low_active`,
    and `record_funding_status` / `reconcile_funding_phase` (when closing
    without initial credit); maintain the in-memory per-group
    `active_insufficient_funds: BTreeMap<String, String>`
    (`pool_id -> active_wallet_id`) latch for `ACTIVE` wallets only, cleared
    on `ACTIVE` promotion (`journal_authorization` and `promote_low_active`),
    `record_initial_credit`, and `fill_funding_vacancies`; share read-only
    `is_replaceable_blocked_job` and `active_wallet_health` helpers so the
    scheduler inspects admitted groups in one read-only SQLite call and runs
    `promote_low_active` only when needed.
  - `src/rotation/funding.rs` and `src/rotation/funding/scheduler.rs`: add
    `FundingWorker::with_refill_delays(...)`; track
    `admitted_groups: BTreeSet<String>` separately from armed vacancy timers;
    await `funding_changes` in one `select!` branch in `Mode::Serving`
    whenever `!stopping` (driving deferred discovery plus admitted-group
    promotion and vacancy reconciliation, with `allocation_retry` on
    `SQLITE_BUSY`); replace the continuous single-tier `Opportunities` timer
    with vacancy-triggered volatile lazy and urgent tiers; clamp lazy
    deadlines to `min(existing_due, now + draw_urgent())` on lazy-to-urgent
    transitions without extending sooner deadlines.
  - Keep `Mode::Bootstrap`, `Mode::Once`, and `Mode::Explicit` prompt.
  - Update tests (`src/rotation/config.rs`,
    `src/rotation/funding/opportunity_tests.rs`,
    `src/rotation/funding/bootstrap_opportunity_tests.rs`,
    `src/rotation/store/early_rotation.rs`, `tests/managed.rs`) covering
    vacancy-triggered arming and disarming (including replaceable vs.
    irreplaceable `blocked_job`s), `deferred_groups` admission tracking after
    disarming, `SQLITE_BUSY` retry, lazy delay with `READY` present, lazy
    delay with healthy sole `ACTIVE`, reactive `ACTIVE` -> `READY` failover
    staying on the lazy timer (not poisoned by demoted `ACTIVE`'s
    insufficient-funds event), lazy-to-urgent transition on
    `journal_authorization` crossing `promote_low_active`, `ACTIVE`
    insufficient-funds latch surviving `finish_balance`, `RETAINED`
    insufficient funds not triggering urgency, custom `[funding]` ranges and
    `validate_swap()` bounds, no extension of sooner deadlines, and volatile
    restart redraw.
  - Document two-tier vacancy-triggered scheduling, urgency triggers, volatile
    restart behavior, and rotation-linkability rationale in
    `docs/wallet-rotation.md`, `docs/configuration.md`, and
    `docs/network-egress.md`.
- **Acceptance evidence:**
  - Deterministic unit and scheduler tests verify all lazy and urgent state
    transitions and confirm `promote_low_active` performs no network I/O.
  - Exact-SHA independent review passes.

### `LR-3`: Default `[floor, 2 * floor]` small reserves and passive minimum learning

- **Scope:**
  - `src/rotation/config.rs`: keep default `reserves.small` unset (`{}` in
    JSON and TOML) so serde roundtrips (`tests/managed_config.rs:690`,
    `tests/assignment.rs:266`) and `profile["reserves"]["small"].is_object()`
    (`src/qualification/funding/registry_state.rs:258`) stay intact; add
    `SizingPolicy::AutoSmall { seed, maximum }` with `U256`-clamped `seed`;
    add cap-aware `small_policy(cap)` and
    `initial_inventory_minimum_atomic_for_cap(cap)` while preserving cap-free
    `initial_inventory_minimum_atomic()`; add pure helpers for explicit
    small-reserve detection and `U256` cap-vs-ceiling comparison.
  - `src/rotation/near.rs`: return typed `HttpStatus(400)` instead of untyped
    `anyhow::bail!("near_http_400")` at line 428.
  - `src/deployment.rs`: emit startup `INFO` when `reserves.small` is
    explicitly configured and startup `WARN` when finite
    `max_api_payment_usdc` exceeds the wallet group's largest configured
    reserve ceiling once per resolved managed wallet group inside
    `initialize_wallets_mode` (`target: "x402_treazury::startup"`, silent for
    read-only inspection commands); keep `show_config` accurate.
  - `src/rotation/store/capacity.rs` and `src/rotation/store/funding.rs`: pass
    `limit: Option<U256>` into `configure_reserves` so auto-sized default
    `reserves.small` persists `(seed, small_ceiling)` into `reserve_claims`
    with zero SQLite schema migration; keep explicit reserve bounds
    authoritative.
  - `src/rotation/funding.rs`: add a single-slot volatile
    `Arc<Mutex<Option<CachedRouteMinimum>>>` (24-hour `tokio::time::Instant`
    TTL) on `Backend` shared with `Observations` (keeping `Backend`
    construction clean across `src/deployment/bootstrap_actor_tests.rs`,
    `src/treasury/lifecycle_tests/{worker,discovery}.rs`, and
    `tests/support/funding_backend.rs`); integrate `SizingPolicy::AutoSmall`
    into `quote_prerequisite` and `quote_with_clock` so proven
    `BridgeMinimum(N)` (`chosen < N <= ceiling`) updates the slot and redraws
    in `[N.max(seed), min(2 * N.max(seed), ceiling)]`, generic
    `HttpStatus(400)` for `AutoSmall` when `chosen < ceiling` emits at most
    one `WARN` (safe fields only) and retries once at `ceiling`, `N > ceiling`
    fails with `FundingCondition::AmountLimit` without caching, explicit
    `Fixed` and `Range` reserves never trigger generic-400 ceiling fallback,
    and `EXACT_INPUT` zatoshi rejections remain isolated from the cache.
  - `src/rotation/assignment.rs` and `src/wallet_cli/preview.rs`: report
    effective `(seed, small_ceiling)` offline without making 1Click calls.
  - Update and add tests (`src/rotation/config.rs`,
    `src/rotation/funding/redraw_tests.rs`, `src/rotation/store/capacity.rs`,
    `src/wallet_cli/preview/tests.rs`, `tests/support/funding_backend.rs`,
    `tests/managed.rs`, `tests/managed_config.rs`,
    `tests/public_demo_profiles.rs`) covering:
    - Probed minimum below `$2.00` producing `[floor, 2 * floor]` within
      `$3.00`,
    - `max_api_payment_usdc` acting as `seed` (including `U256` clamping for
      huge finite decimals, with no artificial lower floor) and learning
      upward when `BridgeMinimum` is higher,
    - `200 OK` on a `$2.00` or `$1.00` group never poisoning a `$0.10` or
      `$0.50` group's cache,
    - Crash and restart between `select_quote_terms_for_attempt` and quote
      save preserving selected terms cleanly,
    - JSON and TOML serde roundtrips preserving auto-sized vs. explicit
      `reserves.small`,
    - `max_api_payment_usdc` unlimited (`"none"` or `"off"`) or `"0"`
      starting from `$2.00`,
    - `reserves.large` enabled with `max_api_payment_usdc > $3.00` keeping
      `reserves.small` within `$3.00` without false warnings,
    - Startup `WARN` when `max_api_payment_usdc` exceeds the largest reserve
      ceiling and startup `INFO` when `reserves.small` is explicitly
      configured (without firing during read-only inspection commands),
    - Generic non-bridge `HttpStatus(400)` fallback to
      `max_funding_amount_usdc` with one `WARN` for `AutoSmall` when
      `chosen < ceiling`, no fallback retry or `WARN` when
      `chosen == ceiling`, and no fallback jump on explicit `Fixed` or `Range`
      reserves,
    - `EXACT_INPUT` zatoshi rejection never poisoning the Base USDC cache,
    - 24-hour monotonic TTL expiry (`start_paused = true`) resetting a spiked
      floor back down,
    - Hard `$3.00` ceiling rejecting an excessive `BridgeMinimum` with
      `FundingCondition::AmountLimit` without caching the rejected floor.
  - Document default `[floor, 2 * floor]` sizing, usage-linkability rationale,
    limitations, fallbacks, 24-hour monotonic TTL reset, ceiling enforcement,
    and the new `WARN` entries (explicitly reconciling `AutoSmall` generic-400
    fallback `WARN` with `log-severity`) in `docs/wallet-rotation.md`,
    `docs/configuration.md`, `docs/local-resource-limits.md`, and
    `docs/development.md`.
- **Acceptance evidence:**
  - All targeted Rust tests pass with nonzero counts, `make check-clippy`
    passes cleanly, and exact-SHA independent review passes.

### `LR-4`: Operator documentation, `README.md`, and example alignment

- **Scope:**
  - Update `README.md` with concise operator guidance describing the volatile
    `24-48` hour lazy background refill behavior and advising operators not to
    run `x402_treazury` on a short restart loop, linking to
    `docs/wallet-rotation.md`.
  - Remove stale copies of `30-120` second single-tier timing and static
    `$2-3` small reserve defaults across `README_ADVANCED.md`,
    `examples/README.md`, `docs/architecture.md`, `docs/public-swap-demo.md`,
    `docs/usage/configuration.md`, `examples/deployments/managed.toml`,
    `examples/deployments/research_desk.toml`,
    `examples/deployments/comprehensive.toml`, and
    `tests/live/deployments/*.toml`.
  - Verify `AGENTS.md` guide links, single ownership across guides (no
    duplicated reference tables), valid local links and heading anchors, and
    compliance with `AGENTS.md` writing-style rules.
- **Acceptance evidence:**
  - All local links and heading anchors verified, `git diff --check` clean,
    and exact-SHA independent documentation review passes.

## Review and integration gates

- **Gate 0 (Pre-implementation design review):** Fresh independent subagent
  reviews this finalized plan against pinned code (`src/rotation/`,
  `src/direct_swap/`, `src/deployment.rs`, `src/wallet_cli/`, owning docs) to
  challenge scope, necessity, state compatibility, privacy assumptions, and
  simpler alternatives before `LR-1` begins.
- **Per-commit and fixup reviews:** Every commit (`LR-1` through `LR-4`) and
  every `git commit --fixup=<SHA>` receives an independent exact-SHA review
  in a fresh subagent conversation. History remains unsquashed through user
  review.
- **Gate 1 (Combined integration, complexity, and documentation gate):**
  - Full validation: `make fmt-check`, `make check-clippy`, and `make check`.
  - Complexity review per `tests/COMPLEXITY.md#review-discipline` on all
    changed Rust modules (`src/rotation/funding/scheduler.rs`,
    `src/rotation/funding.rs`, `src/rotation/near.rs`,
    `src/rotation/config.rs`, `src/rotation/store/`).
  - Fresh independent combined review of `git diff 03b45e4c..HEAD`, all prior
    findings and fixups, full call chains, failure scenarios (concurrent
    payments during lazy-to-urgent transition, insufficient-funds jump from
    above 25%, `finish_balance` race against `low_balance`, 1Click 400 prose
    drift, restart during quote selection, 24-hour TTL rollover), and
    documentation writing style.
- **Whole-changeset loop and completed-plan cleanup:** Repeat combined review
  until clean, complete user review rounds, delete
  `docs/plans/linkability-reduction.md` per `docs/planning-workflow.md`,
  verify `git diff --check`, and run a fresh independent review of the
  cleanup commit and final plan-free diff.
