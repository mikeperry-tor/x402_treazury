# Improved wallet rotation

## Intent, baseline and status

Reduce the immediate timing relationship between use of a managed Base USDC
wallet and funding its replacement. Reuse retained purchasing power without
introducing an extra pair of unsigned provider requests merely to drain it.

This is an active implementation plan, not implemented behavior or authorization
for funded qualification. The current request authorizes creating, committing and
independently reviewing this plan only.

- Working branch: `main`.
- Starting baseline: `a05e4fb973a9e227eda36fa73eb4510a8d2e4263`.
- Plan: `docs/plans/improved-wallet-rotation.md`.
- Scope: managed payer selection, proactive reserve promotion, ordinary background
  reserve replenishment and their consumers, tests and maintained documentation.
- Excluded: wallet merging, gas funding, more reserve slots, top-ups, new monetary
  budgets, dependency upgrades/forks, live spending and privacy-performance claims.
- Status: planning only; independent design findings and their correction are
  recorded below. Implementation has not begun.

The user rejected making handoff from an estimated-affordable bound ACTIVE wallet
to RETAINED a routine draining strategy. Preserve that bound payer by default;
this is not a blanket no-handoff rule. Price/eligibility handoffs and the existing
recognized structured insufficient-funds traversal must still let other eligible
wallets finish a call. Avoid routinely obtaining unsigned challenges through wallets
already known to be low or empty when a healthier candidate exists. Runtime price
hints share catalog lifetime (seven days by default); use those hints to select
retained wallets before making a provider request.

Read the [planning workflow](../planning-workflow.md),
[independent review rules](../../AGENTS.md#independent-subagent-workflows),
[best-effort rules](../wallet-rotation.md#best-effort-wallet-rules),
[observation policy](../wallet-rotation.md#wallet-observation-policy),
[reconciliation policy](../wallet-rotation.md#recovery-and-reconciliation-policy),
[capacity and admission](../wallet-rotation.md#capacity-and-admission),
[retirement](../wallet-rotation.md#automatic-retirement),
[selection/isolation](../architecture.md#wallet-selection-and-network-isolation),
[reserve sizing and startup](../configuration.md#reserve-sizing-and-fragmentation),
[runtime hints](../configuration.md#pricing-metadata-and-runtime-hints),
[resource policy](../local-resource-limits.md),
[timer inventory](../network-egress.md#remaining-timers-and-budgets),
[validation order](../testing.md#validation-order-and-continuation) and
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline).

## Current behavior and evidence

- `src/payment.rs` obtains an existing inventory boundary, replaces an absent hint
  with zero and calls `ManagedPool::next_candidate`. The Store tries an affordable
  bound payer, then ACTIVE, RETAINED and READY, in creation order within each role,
  then eligible unknown/low-estimate fallbacks. After challenge discovery,
  `src/rotation/manager/selection.rs` passes the selected payer as bound and uses
  the actual offer amount. Selection performs no network I/O or role mutation.
- `src/rotation/store.rs` journals a possibly submitted authorization before
  disclosure. `store/capacity.rs::consume_reserve` atomically promotes a READY payer,
  retains the old ACTIVE and clears/increments the exact reserve claim. Ordinary
  spending from ACTIVE or RETAINED does not open another reserve slot.
- `funding/scheduler.rs::schedule_owned` fills vacancies at owner startup and
  revisits groups on `funding.poll_seconds`. `store/funding.rs` fills small and
  optional large slots through independent commits. Operation status, credit,
  source and quote lanes have separate ownership and schedules.
- The sampled nominal amount is persisted in `funding_progress.quote_target`
  before requesting a quote; archived attempts retain their terms. The first
  positive observed destination balance is saved once in the existing funding
  job's `readiness_requirement`/`readiness_capacity` fields. The original job
  `target` is not necessarily the sampled amount or actual credited balance.

## Proposed behavior

### S1 selection contract

Preserve absence of a price hint explicitly; do not overload zero to mean unknown.
A present zero hint is still a present hint. Never manufacture a price from the
per-payment cap or provider-description text.

- With a valid cached hint, prefer estimated-affordable RETAINED wallets oldest
  first, then ACTIVE, then READY. Creation sequence, not balance sorting, determines
  order within a role.
- Without a hint, prefer an eligible ACTIVE payer with a known positive estimate
  and no low-balance flag for unsigned challenge discovery. ACTIVE role alone must
  not outrank a healthy positive-estimate sibling when ACTIVE is flagged low, known
  zero or unknown. Otherwise try known-positive, non-low RETAINED oldest-first,
  then READY in existing reserve order. No ACTIVE requirement, minimum-balance
  floor or new availability gate is introduced.
  Unknown/zero/low estimates remain eligible fallbacks when no better candidate
  exists; malformed identities retain healthy-sibling fallback. A positive balance
  without a price hint is only a heuristic, not evidence it can cover the actual
  request. Do not invent a price or add a probe to certify it.
- Once the challenge is obtained, retain an eligible bound payer whose estimate
  covers the actual offer. Do not introduce a handoff merely to spend retained funds.
  If it is unsuitable, preserve existing price-based handoff and its own isolated
  unsigned challenge, method restrictions and manual-raw-header continuity.
- Missing, stale or low estimates remain advisory. Current ownership, challenge
  terms and amount authority govern signing. Each captured identity is visited at
  most once; signed uncertainty does not grant a new replay. Existing structured
  insufficient-funds traversal remains unchanged: try other existing eligible
  wallets with their own challenges while reconciliation proceeds independently,
  without funding/allocation waits. Neither this plan's privacy preference nor
  role ordering may stop that permitted traversal. Do not broaden replay authority
  to arbitrary failures or ambiguous signed outcomes.
- Keep cache identity, expiry, persistence, alias sharing and learning behavior.
  New role preferences neither renew hints nor probe providers to learn prices.

Consumers: `src/payment.rs`, `rotation/manager.rs`, `manager/selection.rs`,
`store/selection.rs`, runtime hint callers, raw calls, relay and low-level bound
payment paths. Do not accidentally impose retained-first selection on calls whose
existing credential/transport contract pins their payer.

### Stable 5%–25% threshold

Use the immutable first positive observed credit as advisory initial tranche `F`,
not the configured range, nominal quote target, current balance or sum of wallets.
This is the first observed balance, not proof of total historical receipts.

For finite current per-payment cap `M`, calculate in integer micro-USDC:

```text
U = min(M, floor(F / 4))
L = floor(U / 5)
T = one stable private pseudorandom integer in inclusive [L, U]
```

For an unlimited cap, `U = floor(F / 4)`. Thus the unrounded interval is 5%–25%
of `F` when the cap does not bind. If the cap binds, scale both ends down rather
than invert or collapse the requested interval. A $4 credit with a $1 cap gives
$0.20–$1; a $25 credit with a $1 cap also gives $0.20–$1. Integer rounding may
produce a zero or singleton interval for tiny credits; it must not create an error,
forced minimum balance, top-up or authority to spend.

Derive a private reproducible per-wallet random value with HMAC-SHA256 keyed by the
existing private Store key, over a versioned domain and unambiguously framed immutable
treasury, pool and wallet identifiers. Use a counter in that domain to produce further
blocks for rejection sampling into inclusive `[L, U]`; use integer arithmetic without
modulo bias. Do not include mutable labels, roles, estimates or process time in the
random stream. The threshold domain is distinct from keys, encryption, Tor and
transaction signing. A public address hash alone is predictable.

Promote the already locked registry `hmac` 0.13.0 to a direct dependency alongside
existing `sha2` 0.11. Its pinned `digest` 0.11 interface and SHA256 support were
checked in the local package source; no new checkout, fork, version upgrade or
vendoring is needed. Use the library primitive, not a handwritten MAC. Review the
exact framing/domain and rejection mapping with test vectors in S2. Do not change
existing identity derivations or persist a random threshold/seed. Keep secrets and
raw derived values out of logs/status. The derivation must survive reopen and avoid
process RNG redraws. Bounds use the current cap on reopen; cap changes can change
the resulting threshold but never invalidate accepted work.

Missing historical initial-credit data or an unknown estimate skips proactive
promotion only. It never excludes an otherwise authorized payment. Do not recover
that optional input by a historical sweep, recurring balance RPC or new ledger.

### Background promotion and reserve ownership

At a selected background opportunity, a used ACTIVE wallet with a known advisory
estimate at or below `T` can become RETAINED while an existing funded READY wallet
becomes ACTIVE. Reuse existing journal evidence of an authorization from the current
ACTIVE; no use counter or new balance history. This keeps unused low-credit wallets
from triggering consecutive idle promotions and new allocations.

The serialized Store transaction must recheck the current ACTIVE identity/use,
estimate/threshold inputs, pool enablement and exact eligible READY claim/generation.
Prefer the small READY slot before the optional large slot, matching current reserve
order. Perform at most the single current-ACTIVE transition for that group opportunity;
do not recursively promote its replacement in the same pass. No READY means no
proactive transition. No RPC, signing or source I/O belongs in the role transaction.

Both roles remain eligible for accepted calls. Background demotion cannot invalidate
a captured payer, abandon authorizations or duplicate a READY generation consumed
concurrently by journal admission. A stale competing transition is a no-op with
current state re-evaluated at a later normal opportunity, not a financial retry.
Share the atomic role/claim mechanics with first-use promotion while distinguishing
their call-site contracts: the current helper explicitly requires a journal
transaction and must not simply be invoked outside it unchanged.

Preserve immediate first-use promotion of a READY payer when requests outpace
background preparation. New funding from either promotion route waits for an
independent sweep; payment admission never allocates, waits for funding or changes
the sweep's timing. Current retirement/source protections still apply when a wallet
becomes RETAINED. Partial/late credit, refunds and zero balance never reopen a
wallet's initial funding claim.

### Independent refill opportunities

Replace ordinary serving's eager startup refill and unconditional periodic vacancy
fill together. Keep one retained funding owner and remove its allocation cursor.
For each configured eligible group, independently draw a uniform integer duration
from inclusive **30–120 seconds** and retain one volatile monotonic next-due instant.
The nominal mean interval is **75 seconds**, about **48 opportunities per group per
hour** under regular operation. These are the user-selected policy endpoints; the
earlier proposed `funding.reserve_sweep_mean_seconds` setting is superseded and must
not be added. No new tuning settings are required for this implementation.

This explicitly replaces the prior prohibition on per-group due times: permit one
in-memory scheduling instant per group, but no per-wallet/per-vacancy delay record,
persisted timer, use counter or replenishment permit. Use fresh private randomness
for each interval, independent of public addresses, threshold derivation, payments,
balances, vacancy age and other groups. Draw the initial interval at owner startup
and continue the schedule even when the group has no vacancy. Payment/capacity wakes
and completion activity cannot reset, advance or redraw a group's scheduled instant.

When groups are due, process each due group at most once without the scheduler's
idle sleep between groups, while preserving progress of accepted-work lanes. Set
all currently due groups' next instants from one captured monotonic scheduling
instant plus their independent fresh durations, before any Store or source I/O.
After a delayed wake, do not replay missed intervals
or run catch-up sweeps. Group count and unrelated completions must not define the
interval. Store/group load can still delay actual execution; no additional queue,
sticky funding right or new I/O limit follows from a due instant.

Only due groups evaluate proactive promotion and fill their current vacancies.
A due opportunity may promote and then refill the resulting vacancy; bridge
execution remains asynchronous. Existing `funding.poll_seconds` remains the cadence
for its existing operation observations, not a grid for randomized replenishment.

Use one shared timer armed for the earliest group due instant or existing required
observation/retry due time, plus completion, new-work and shutdown notifications.
Do not introduce a timer task per group or retain a one-second wake solely to check
whether refill work is due. Existing inventory lanes also reread exhausted inventory
on `poll_seconds`; changing refill intervals alone does not remove those wakeups.
Within S2, trace new-work publication/notification and due-time consumers before
replacing idle wake paths. Preserve required observation cadence and robust discovery
of newly committed work; no lost wake or indefinitely sleeping owner is acceptable.
Use the smallest change that eliminates redundant wakeups, not a new scheduler or
notification framework. Record any remaining necessary polling and do not claim
process/power savings from opportunity counts alone.

Cold serving waits for each group's first 30–120-second opportunity rather than
filling every group synchronously. Restart redraws independent initial intervals;
no scheduled instant is persisted. Consequently, repeated restarts, suspension or
owner unavailability can extend a wait beyond 120 seconds. Simultaneously overdue
groups after suspension can still run close together. The bound describes nominal
opportunity intervals under a continuously running, schedulable owner, not a funding
completion or wall-clock guarantee. No emergency refill on exhaustion: that would
silently change the chosen availability/privacy tradeoff.

The opportunity authorizes ordinary vacancy allocation only. Once a job is allocated,
existing quote/source work, retries, credit reads, status checks, recovery and ZEC
discovery proceed under their original authorities/schedules; do not wait for another
refill opportunity for accepted work. Persisted allocated jobs resume after restart
without requiring another interval. Signed expiries and provider quote deadlines do
not change.
New funding remains governed by `auto_fund`, enabled slots, current restrictions,
actual supported treasury funds and existing accepted liabilities. Neither sweep
interval nor the threshold is another financial authorization or ZEC budget.

Keep small/large vacancy commits independent and preserve completed-uncredited
replacement/source-closure rules. One failed or blocked group/slot must not prevent
other selected groups or existing observation lanes from progressing. A busy Store
or source owner does not reset another operation's backoff or permit overlapping I/O.
Do not turn a missed opportunity into a sticky replenishment permit or a generic queue.

Explicit `wallet bootstrap` retains its captured-claim completeness and prompt
authorized allocation behavior, without proactive promotion of captured claims.
One-pass `tick`, offline sync, inspection and separately authorized qualification
must use explicit existing-operation/explicit-allocation semantics, not accidentally
inherit random ordinary-serving delay or acquire additional allocations. Trace these
consumers before changing the shared scheduler API. `auto_fund=false` continues
accepted-work observation and forbids new funding; omit proactive background
rotation when the ordinary automatic-funding capability is disabled.

## Hazards and preserved boundaries

- Timing is softened, not hidden. Low volume, matching funding sizes, request
  content, shared pricing hints and bridge/network observations can still correlate
  wallets. Draining retained wallets keeps their old histories active longer.
- Incorrect hints can still produce the existing necessary unsigned handoff. The
  change must add no handoff solely because another wallet has a preferred role.
- Retained-first selection cannot consume fragments smaller than future prices.
  Expensive-only demand, poor estimates or failed authorizations can leave retained
  wallets behind. Do not claim bounded retained inventory or add a wallet-count cap.
- Nominal opportunities are bounded to 30–120 seconds apart, but restart, suspension,
  scheduler delay, unavailable capital and slow swaps can extend actual waiting.
  Cold starts, bursts, single payments that jump below the threshold and no READY
  inventory can leave paid calls unavailable. Listeners/catalogs and already-funded
  payers remain usable; report actual unavailability, not a delivery guarantee.
- One group with simultaneous small/large vacancies may still fund them close
  together. Existing source serialization/provider latency also shape execution
  times; independent opportunity draws do not establish independent on-chain times.
- Independent intervals avoid intentional group synchronization and the existing
  five-second lottery grid, but duration granularity, restart/suspension and source
  serialization can still expose timing structure. Deterministic oldest-first
  retained reuse provides no anonymity-set guarantee.
- Estimates can be stale, concurrently replaced or debited for unsettled signatures.
  Promotion is advisory inventory policy; it must not become a balance certificate,
  a second exposure deduction or a gate on a permitted signature.
- Journal before disclosure; preserve exact accepted bytes, identities, liabilities,
  claim generations and accepted-work ownership through cancellation/restart/reorgs.
  No new source scans, deposits, top-ups, budget enforcement or replay authority.
- Keep production transport construction in `src/network.rs` and preserve payer-bound
  transports, public destinations and Tor remote DNS/no-direct-fallback.
- Preserve stable state formats, CLI JSON contracts, generated pool names, existing
  token/key domains and environment names. No schema migration or new durable state
  is needed; volatile per-group due instants are explicitly permitted. The policy
  must work equivalently on Linux and macOS.
- Follow existing logging severity/clarity policy. Routine sweep scheduling and
  scans are not INFO/WARN events. Explain material delays and capability failure
  safely without logging credentials, upstream prose or secret derivation inputs.

## Ordered commits, evidence and review gates

| ID | Coherent commit scope and consumers | Acceptance evidence and documentation |
| --- | --- | --- |
| P0 | Commit this plan on the baseline branch; independent design review D0 before implementation. | Exact plan SHA, baseline/source checks, links and diff formatting; record findings and resolve material design choices, including bounded intervals, wake paths and keyed derivation. No Cargo checks for plan-only prose. |
| S1 | Preserve optional hint input; use oldest-affordable RETAINED first for hinted calls and healthy ACTIVE-first for no-hint discovery; preserve bound-payer stability and necessary handoff/fallback contracts. | Focused selection, hint, handoff, manual-header and payment consumer tests; retained-first hint coverage, absent versus zero hint, stale/high/low hints, malformed/unknown balances, no ACTIVE, known-low/zero ACTIVE with healthy siblings, shared expiry/restart behavior and no role-only unsigned pair for affordable bound ACTIVE. Prove actual-price and structured insufficient-funds handoffs still succeed through eligible alternatives and retain earlier liabilities. Update wallet selection/best-effort guidance, architecture, configuration reserve-sizing/hint text and affected tests alongside code. Exact-SHA review covers full caller chain. |
| S2 | Implement stable thresholds, transactional proactive promotion and independent 30–120-second ordinary-serving opportunities together; remove eager/unconditional refill only with its replacement. Consolidate redundant wake paths while keeping explicit/bootstrap/qualification behavior and accepted-work scheduling distinct. | Focused threshold/role/scheduler/idle-wake and startup/recovery/financial tests below; complete complexity review and final combined gate G1. Update maintained wallet/capacity/observation/reconciliation text, startup and bootstrap guides, both cadence inventories, affected configuration examples/contracts and lifecycle/startup test guides alongside code. |
| C0 | After implementation review and subsequent user rounds, remove this completed plan and repair links under the planning workflow. | All enduring rules live in owning guides; unfinished scope has concrete next actions/acceptance. Independently review exact cleanup SHA and combined branch; preserve plan ancestry, commits and unsquashed fixups. No push/merge implied. |

**D0 independent design gate:** a fresh read-only reviewer inspects the exact P0 SHA
and baseline consumers, first records an assessment independently of author rationale,
then reconciles decisions/findings. Specifically challenge linkability (including
double-unsigned patterns), thresholds and initial-credit facts, idle funding chains,
multi-group timing, startup/qualification behavior, concurrent selection/admission,
retirement, restart/cancellation and starvation/availability. Verify pinned dependency
contracts where relevant, investigate simpler representations and question new state,
enforcement and configuration. Resolve material findings and finalize currently
proposed tuning/derivation before S2. A plan-only approval does not certify runtime.

Independently review every exact S1/S2 SHA and relevant consumers; authors do not
review their own work. Identify any uncommitted review by base SHA and exact content,
including new files, and review its resulting committed SHA. Correct findings using
`git commit --fixup=<introducing-SHA>` and independently review each exact fixup.
Keep unsquashed history and concise finding/cause/disposition evidence here.

**G1 final architecture/integration gate, after S2:** examine S1+S2 and every fixup,
all affected call chains and maintained docs, prior design findings and acceptance
evidence. Complete CogC/CycC review against the starting baseline using the pinned
analyzer, including new/worsening and unchanged high-risk selection/journal/scheduler
hotspots; assess complete JSON/CSV and simplifications, not just score thresholds.
Record source-matched artifacts under ignored `target/`. Resolve material findings
and compelling simplifications, then repeat affected review/validation. Per-commit
approval is not this combined gate; one independent review can cover both scopes
when it actually examines both. Reopen affected gates after material later changes.

### Validation order and concrete scenarios

For this planning turn, verify prose against source, local links and `git diff --check`
only. For implementation, use pinned Make entrypoints and the affected-behavior order
from the testing guide: design and cheap contract checks; applicable complexity
analysis and independent disposition; production compiler/all-target warning gate
`make check-clippy`; selected compatibility checks before broad runtime suites;
focused affected consumers before any selected broader stage. Do not automatically
select full `make check`, dependency provenance, proving or reproducibility merely
because this plan exists. If explicitly requested later, complete those named stages
unchanged. Record exact commands, source/profile/features, diagnostics, exit status
and nonzero test counts; reuse evidence only while inputs remain valid.

Use deterministic injected randomness, paused time and barriers in existing fixture
families; do not add real-time waits or statistical/privacy benchmarks. Select socket
permissions before localhost payment/startup fixtures, keep socket-free unit tests
ordinary, and freeze sources/builds during validation. Never load real secrets or
perform funded qualification.

- Selection: oldest affordable retained wins a valid hint; larger insufficient
  fragments do not exclude other wallets; absent hint chooses ACTIVE; zero hint is
  distinct; healthy siblings take priority over known-low/zero/unknown ACTIVE and
  remain usable without ACTIVE; low/unknown candidates are not made ineligible.
  Actual-price discovery
  preserves an affordable bound ACTIVE and adds no role-only unsigned request;
  genuine price/eligibility handoffs keep transport/challenge isolation. Exercise
  raw-header pinning, relay and hint expiry/reopen. A recognized signed insufficient-
  funds response must still reach another eligible wallet and succeed when its
  provider attempt succeeds, preserving every earlier potentially chargeable
  authorization. Price corrections and balance races must retain allowed handoffs;
  arbitrary failures must not gain replay authority.
- Threshold: private reproducibility across reopen; different wallet/domain inputs;
  unbiased inclusive mapping and integer boundaries; finite, zero, unlimited and
  changed caps; tiny/partial initial credit; nominal amount differing from actual;
  missing historical data; unchanged balances and unused wallets cannot cause idle
  promotion chains. No added RPC or monetary authority.
- Ownership: overlap proactive promotion, two journal admissions and estimate
  publication with barriers; exactly one claim-generation consumption, still-valid
  old-payer authorizations, no duplicated allocation, and cancellation/restart between
  role commit and refill. READY first-use fallback remains atomic. Preserve source
  obligations and retirement fences for the newly retained wallet.
- Schedule: independently injected interval streams include both 30/120-second
  endpoints; no group is checked/refilled before its due time. Identical injected
  streams yield identical group due instants under idle and completion-heavy runs;
  payment/capacity wakes neither redraw intervals nor refill early. No synchronized
  eager restart fill or catch-up burst; restart redraws without duplicating accepted
  work, while suspension processes due groups once. Accepted allocated/prepared/
  submitted work and funded siblings progress independently of future refill times.
  Small-slot success survives large-slot failure; no group is starved by scheduling
  order or another operation's backoff. Verify the earliest-due shared wait,
  cancellation/completion delivery, producer wakeup between inventory inspection and
  sleeping, new committed work while idle, and absence of redundant one-second
  refill checks. Retain and identify any necessary inventory/observation polling;
  do not infer actual process wake counts from 48 nominal opportunities per hour.
- Boundaries: cold serving binds listeners before funding and reports empty-pool
  unavailability; funded READY can serve without ACTIVE; explicit bootstrap captures
  claims without proactive churn; finite sync and qualification do not gain random
  delay or new allocation authority; disabled auto-funding still recovers accepted
  work. Assert forbidden allocation, top-up, duplicate I/O, signature replay,
  unsolicited balance polling and whole-work deadlines remain absent.

## Documentation handoff and review record

Enduring policy belongs in `docs/wallet-rotation.md` (selection, capacity, observation
and best-effort rules), `docs/architecture.md` (selection/isolation and journal
ownership), `docs/configuration.md` (hint/no-hint behavior, sweep intervals, fragmentation
and startup), and `docs/wallet-cli.md` where explicit bootstrap differs. Keep
`docs/local-resource-limits.md` and `docs/network-egress.md` cadence inventories in
agreement. Update relevant `tests/STARTUP.md` / `tests/LIFECYCLE.md` coverage guidance.
Existing AGENTS.md links already reach these owning guides; amend those links only
if implementation introduces or renames an owning section. Add no duplicate policy
checklist. Retain this plan until implementation, combined review and user rounds
finish; planning completion alone is not grounds for deleting it.

- D0 initial independent review examined exact P0
  `a9a3f55170a3921c5490cd9e874184362ad6e01c` against the starting baseline. The
  reviewer recorded an initial source assessment before consulting the plan and
  traced selection/handoff, claims/credits, source/retirement ownership, startup,
  bootstrap and qualification. No runtime or quantitative privacy evidence claimed.
- Finding D0-F1 (material): preserving the existing allocation cursor made each
  group's lottery frequency depend on group count and accepted-work completions.
  The old scheduler handles one group per outer iteration and sleeps one second
  when idle; probability `P / S` therefore did not imply the proposed per-group
  mean. Cause: reusing scheduler structure without tracing its full outer loop.
  Resolution in the P0 fixup: remove the cursor, capture one whole-group draw round
  per existing monotonic boundary, skip missed boundaries and add idle/busy
  multi-group deterministic coverage. The independent reviewer accepted this
  correction at design level; review its exact committed contents and combined
  plan before handing off D0 as complete.
- D0-F2 (unfinished choice in P0): concrete threshold primitive. Resolved to library
  HMAC-SHA256 using the already locked `hmac` 0.13.0 and existing `sha2` 0.11,
  with framed immutable identifiers, a versioned domain and counter/rejection
  mapping. The independent reviewer agreed with this choice; exact implementation
  and test vectors remain part of S2. No custom crypto, checkout or fork.
- The reviewer found no additional material issue in the examined hint/bound-payer,
  actual initial credit, tiny/unlimited caps, used-wallet guard, concurrent role
  ownership, source/retirement, restart/cancellation or explicit-mode contracts.
  This is focused plan review, not repository-wide assurance. Retained residuals,
  shared poll phase, cold availability and random tails remain documented limits.
- Exact fixup and combined plan at
  `b23be9a1be154e70d21eb22e5e3f051ca67eaf35` passed independent review with no
  additional material findings; D0-F1/F2 were resolved for that planning snapshot.
- Subsequent user clarification: avoiding extra handoffs is a default-path privacy
  preference, not a prohibition. Preserve actual-price/eligibility handoffs and
  recognized insufficient-funds traversal. Clarify that no-hint ACTIVE preference
  cannot choose known-zero/low/unknown ACTIVE ahead of a healthy positive sibling;
  do not use the old absent-hint-as-zero shortcut or introduce a minimum-balance
  gate. Add explicit consumer acceptance cases above. Review the exact clarifying
  fixup and resulting combined plan before final handoff.
- Plan validation: local Markdown targets/anchors resolved; diff formatting passed.
  No empirical tuning or runtime validation has been performed. S1, S2, G1 and C0
  are not started. No code or financial authority changed. Exact corrected-SHA and
  combined review results must accompany the planning handoff.
- Exact second fixup and combined plan at
  `beec557316d73d3a46509bfbcf69a7aebaeb84ff` passed independent D0 review with no
  material findings, including the necessary-handoff clarification.
- User-directed schedule revision supersedes the 180-second-mean geometric lottery,
  its unbounded random tail, shared poll-grid draw rounds and proposed mean setting.
  Use uniform 30–120-second intervals, mean 75 seconds, independently per group.
  Allow one volatile due instant per group and a shared earliest-due wait; no
  per-vacancy or durable timer state. Preserve the D0-F1 lesson that group count and
  unrelated activity must not control scheduling, with interval-based tests replacing
  lottery-round tests. Restart redraws and nominal versus actual bounds are explicit.
  Independent option review favored uniform renewal for simplicity and identified
  the existing one-second/exhausted-inventory wake paths; this revision names their
  consumers and focused no-lost-wake validation. Review its exact commit and combined
  plan before treating the revised scheduling design gate as complete.
