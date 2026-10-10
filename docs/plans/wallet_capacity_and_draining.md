# Wallet capacity tiers and retained balances

## Objective and status

Draft implementation plan. Retain useful balances after rotation and optionally
provide a separate large-capacity Base USDC wallet pair under an existing managed
wallet profile. Ordinary calls continue using small wallets; expensive calls have
explicitly funded capacity without increasing every wallet's target. This plan
does not implement the feature or authorize funded execution.

The proposed architecture reuses double buffering for each capacity tier and adds
a bounded set of draining wallets. It avoids an unrestricted wallet scheduler.
Funding amounts remain deterministic in this phase. Amount randomization, sweeping
change, topping up old addresses, and combining wallets into one payment are out
of scope. Larger wallets and longer address use have privacy costs; neither tiers
nor retained balances establish unlinkability.

## Existing implementation

- `src/rotation/config.rs` defines the managed funding target, maximum accepted
  funding output, API cap, and historical target-based payment ceiling.
- `src/rotation/store.rs` has one `ACTIVE` and one `READY` wallet per pool, enforced
  by partial unique indexes. Promotion retires the previous active, activates the
  spare, and allocates its replacement in one durable transaction. Admission also
  compares the expected wallet and pool generation before reserving a payment.
- `src/rotation/manager.rs` validates challenges, obtains canonical balance and
  nonce evidence, admits under a pool gate, and journals signed authorization
  before submission. Pending liabilities cannot trigger promotion.
- `src/payment.rs` selects a payer before the initial HTTP request. The transport
  is bound to that address. A pre-signing payer change permits at most one new
  unsigned GET/HEAD challenge; other methods receive a payer-change error.
- `src/rotation/store.rs` currently excludes retired balances from admission while
  retaining their historical authorization reconciliation. Its role CHECK constraint
  excludes `DRAINING`; this is a schema change, not merely another selection query.
- `src/deployment.rs`, `src/deployment/bootstrap.rs`, `src/rotation/funding.rs`, and
  `src/wallet_cli.rs` share managed setup, bootstrap, supervision and recovery paths.

Existing financial and network contracts remain authoritative. See
[wallet rotation](../wallet-rotation.md), [architecture](../architecture.md),
[network policy](../network-egress.md), and
[qualification limits](../testing.md#qualification-status).

## Proposed configuration and authority

Keep one logical named profile and its existing listener/source bindings. Add an
optional large tier belonging only to that profile. It must not become a global
high-value wallet shared across otherwise isolated profiles, or cause automatic
per-source pool creation. Dynamic sources and paid discovery relays use the same
effective profile resolution as today.

Define `T` as the ordinary profile's configured `funding_amount_usdc`. Interpret
"above the minimum of the original pair" as `payment > T`, not the lesser current
balance, the bridge's current route minimum, or a discovered provider estimate.
Exactly `T` stays in the ordinary tier. A bridge minimum adjustment must not silently
move the routing threshold. Profile changes may change the explicit threshold for
future admissions, but never rewrite existing wallet targets or signed authority.

Proposed configuration concepts, with final TOML names to settle before coding:

- An optional nested large tier with its own funding target, maximum accepted
  output, and explicit per-payment ceiling. Require its target to exceed `T` and
  its effective payment range to include at least one amount above `T`.
- The parent `max_api_payment_usdc` remains an overall ceiling. Effective large-tier
  authority is the intersection of parent and tier caps, the tier target policy,
  and actual confirmed capacity minus liabilities. Enabling a tier alone cannot
  raise the parent cap. Without a tier, preserve existing target-limit behavior,
  including `limit_payments_to_funding_target=false`.
- Define target-limit behavior per selected tier: the legacy ordinary target must
  not accidentally reject every large payment, nor may enabling the large tier
  silently remove all target limits. Validate contradictory/unusable settings.
- Retained-balance mode is independently opt-in, with an explicit maximum draining
  wallet count per tier. This preserves old deployments' address-lifetime policy.
  A configured zero retains existing immediate retirement.
- Inherit existing conversion-overhead and ZEC/fee safeguards initially. Any later
  tier override must be explicit and must preserve treasury-wide ceilings.

For ordinary target `T` and large target `L`, two fully funded pairs allocate
`2*T + 2*L` USDC before accepted bridge-minimum increases. For example, $2 and $10
targets allocate $24, rather than the current $4. This is capital allocation, not
API consumption or total ZEC cost. Report both planned allocation and the maximum
under configured output ceilings, with conversion and network costs separate.

Use eager bootstrap for an enabled large pair in the first implementation. It
matches the existing ready-pair contract and avoids challenge-triggered funding.
Ordinary managed serving with auto-funding initializes both enabled pairs before
discovery; explicit bootstrap does likewise. Existing funded service can continue
under existing auto-fund-disabled rules. Config inspection, wallet init/sync and
source registration never acquire funding authority. Lazy large-pair bootstrap is
a separate possible extension, not an implicit fallback.

## Pool structure and durable identities

Prefer a logical profile mapped to ordinary and optional large child pools, each
reusing the existing pair lifecycle, rather than making every pair operation
understand multiple active/ready slots. A coordinator chooses the eligible pool
and draining wallet before admission. Prove this approach against the migration
and identity requirements before committing to the representation.

Preserve the existing ordinary pool ID, wallet IDs, sequence numbers, encrypted
keys and authenticated encryption context. Add explicit durable profile-to-tier
membership with uniqueness constraints; do not infer authority from generated
display-name suffixes. A large child must not collide with another configured
profile, inherit an unrelated pool, or reset gross funding history. Existing
qualification references to the ordinary pool must still resolve exactly.

Each tier retains one active, one ready, and at most the existing permitted
replacement work. Draining wallets have no funding outbox and cannot become active
or ready again. Keep pair generation separate from any routing/configuration
revision needed to validate candidate selection. Ordinary payment completion must
not invalidate unrelated candidate handles merely because a different wallet paid.

## Selection and admission

Validate the complete supported offer and payment authority before selection can
mutate roles, allocate replacements, reserve funds, or sign. Startup pricing and
catalog descriptions remain estimates and never authorize or route a payment on
their own. Multiple offers must retain deterministic supported-offer selection.

For an amount above `T`, select only the large tier; absence, disablement or
insufficient capacity returns a typed result without churning the ordinary pair.
For an amount at or below `T`, use ordinary capacity with the draining exception
below. Do not use the large active/ready pair as an automatic small-payment fallback.

Recommended selection order:

1. Consider eligible draining wallets within the logical profile. For a small call,
   allow both ordinary and large draining balances; for a large call, allow only
   large draining balances. This lets a $1 remainder from a large wallet be useful.
   Enforce the current caps and original tier eligibility; a draining wallet's
   balance is never an independent grant of authority.
2. Choose the smallest sufficient available draining balance, with a deterministic
   tie-break on immutable wallet identity. Availability means a fresh confirmed
   balance minus every unresolved reservation, never the last saved balance.
3. Otherwise use the selected tier's active wallet if sufficient.
4. Otherwise apply the existing pending-liability rule before considering a
   promotion. Insufficiency explained by unresolved active liabilities does not
   permit promotion or replacement allocation. Choosing another already-draining
   wallet with independent capacity does not release those liabilities.
5. Promote only a verified sufficient spare in that tier. Commit the previous
   active's new role, generation, replacement allocation/outbox and payment
   admission under the same existing ownership and transaction guarantees.

Selection of an already-spendable draining wallet must not itself trigger promotion
or refill. All selection and admission paths revalidate enabled profile/tier,
wallet membership, role, caps, revision and evidence inside the serialized store.
Use an initial coordinator gate per logical profile if needed for correctness;
release it before awaiting the signed provider response. Avoid locking two tier
gates in inconsistent order or serializing unrelated profiles.

No wallet aggregation, Base transfers between these addresses, or replay under a
different payer is part of this design.

## Challenge and network identity binding

This is a prerequisite, not a transport detail to resolve after state changes.
The initial request often has no known price. It cannot reliably choose the large
tier or a fitting remainder before receiving a challenge.

Replace the current assumption that re-running `candidate()` always discovers
the desired new active with an explicit internal selected-candidate handoff. It
must bind logical profile, tier/pool, wallet/address, relevant generation/revision,
and request context. It is not agent-supplied authority, a balance reservation,
or permission to reuse a previous challenge. It must survive the single allowed
GET/HEAD re-challenge without starting again at the ordinary active wallet.

Acquire the new wallet's identity-bound client, obtain a fresh unsigned challenge,
validate its price and requirements again, and admit against fresh evidence. If
the challenge changes tier, the selected wallet is no longer suitable, or concurrent
work invalidates the candidate, return a typed result after the existing bounded
re-challenge allowance. Do not introduce a payer-selection retry loop.

For POST and other methods outside the current GET/HEAD rule, preserve the
actionable pre-signing payer-change failure. Document that transparent routing of
an unknown expensive POST is not solved by funding another pair. Optional future
operator-declared route-to-tier hints would need separate validation and still
would not authorize a stale challenge or automatic POST replay.

The exact same selected identity must govern provider transport, Tor isolation,
cover state, signer and durable payment record. Validate direct calls, dynamic
fallback, and paid discovery relay behavior. A possible relay submission still
disables shared relay retries as today; routing cannot reset its submission marker.
Once signed/journaled, no challenge refresh or second-wallet attempt is permitted.

## Draining and retirement transitions

| Transition | Required condition | Durable effect |
| --- | --- | --- |
| `ALLOCATED` to `READY` | Existing canonical funding readiness | Existing credit/funding journal behavior |
| `READY` to `ACTIVE` | Bootstrap or permitted promotion | Tier generation and replacement ownership remain atomic |
| `ACTIVE` to `DRAINING` | Promotion with a positive confirmed remainder and retention enabled | Preserve key, balance provenance and every liability; no refill for the old wallet |
| `ACTIVE` to `RETIRED` | Existing retirement mode, or proven empty wallet with no liabilities | Retain all historical records |
| `DRAINING` to `DRAINING` | New eligible payment or reconciliation | Reserve and resolve on the same wallet |
| `DRAINING` to `RETIRED` | Fresh canonical zero balance and no unresolved liabilities, or explicit operator retirement | Stop new admissions, retain funds/keys and recovery obligations |
| `RETIRED` to spendable | Never automatically | Historical retirees are not reactivated by migration |

Do not equate unavailable balances, busy reservations, seller receipts, or missed
RPC responses with emptiness. A nonzero remainder is not automatically dust; future
APIs may accept it. Explicit retirement of nonzero balances must report that they
remain owned but unavailable for new payments. It neither deletes funds nor spends
them. A disabled profile/tier blocks new admissions while reconciliation continues.

The draining count protects bounded admission RPC work and retained live candidate
state. Recommended initial cap behavior: stop a promotion that would exceed it,
while continuing eligible payments and reconciliation. Return a specific capacity
reason; do not silently evict a funded wallet or discard an active operation.
Operators can drain, raise the bound explicitly, or explicitly retire a remainder.
This can reduce availability when no suitable small calls arrive and must be shown
in status/help. Decide the numeric default before implementation; test it as a
resource bound, not as an elapsed-work deadline.

Do not introduce age-based automatic retirement, fresh whole-sweep deadlines,
reconciliation wallet-count deadlines, or timers renewed by unrelated progress.
Retain canonical historical reconciliation and acquire separate fresh admission
evidence as today. Update the network timer/budget inventory for changed count or
byte bounds and identify their owning layer.

## Funding and recovery

Both tiers use the same treasury, serialized budget reservation, gross USDC limits,
ZEC limits, fee checks, quote authentication and recovery history. Accepted targets
are immutable; retries cannot resize a quote-bound wallet. No tier-specific budget
ledger may reset or evade treasury-wide history on configuration changes.

Replacement funding is caused by an authorized pair transition, never by an
ordinary switch among draining wallets. A temporarily unaffordable request,
unsupported scheme, over-cap offer, unsigned outage or cancelled waiter cannot
allocate repeated replacements. Schedule ordinary and large funding jobs fairly
under existing treasury ownership; one tier's unsigned outage should not prevent
independent eligible work, while uncertain treasury mutation retains its existing
wider safety scope.

Preserve zero-new-funding restrictions before setup and every supervised reopen.
Already-spendable draining selection can be read/admit-only, but promotion still
includes replacement allocation and therefore retains its current funding gate.
Separating promotion from allocation would be a distinct policy change with its
own design and tests. Recovery/admin commands never bootstrap an absent large
pair, allocate a replacement, or acquire a new funding permit.

Removing or disabling a tier prevents new starts but leaves exact signed bytes,
funding/refund jobs, keys, historical budgets and liabilities recoverable. Re-enable
only through the same durable identity and current authority. Renaming must not
adopt unrelated state or reset recovery attempts. Config changes to targets apply
to future allocations; they do not refill previously allocated wallets.

## Schema migration and restart

Version the store migration, extend role constraints, and preserve foreign keys,
partial unique indexes and encrypted records. SQLite table replacement, if needed
for the role CHECK, must be transactional and checked with foreign-key validation.
Keep admission schema versioning distinct from the encrypted instance format.

Map every existing pool to its ordinary tier without allocating any wallet or
rewriting historical financial events. Existing retired wallets remain retired.
Migration alone must not enable retention or large-tier spending. Older executables
must reject a newer incompatible schema; document backup/upgrade expectations
rather than promising downgrade by restoring a stale financial snapshot.

Restart must reconstruct child membership, active/ready/draining roles, generation,
funding outboxes and pending authorization ownership exactly. Test crashes before
and after durable promotion, admission, signed journaling, quote reservation and
funding confirmation. Accepted store work survives waiter cancellation. No missing
response grants permission to repeat financial work.

## Implementation surfaces

| Area | Required work |
| --- | --- |
| `src/rotation/config.rs`, deployment config and assignment | Parse/validate tier and retention settings; preserve sharing boundaries and cap semantics; offline resolved configuration |
| `src/rotation/store.rs`, `store/allocation.rs`, `store/funding.rs` | Membership migration, role constraints, selection/admission, atomic transitions, bounded draining capacity, funding ownership |
| `src/rotation/manager.rs`, `src/rotation/error.rs` | Profile coordinator, typed selection outcomes, freshness and cap checks, candidate handoff |
| `src/payment.rs`, `src/network.rs`, cover integration | One bounded re-challenge with the actual selected payer; unchanged signed-attempt and isolation rules |
| `src/rotation/base.rs` and transport | Query eligible wallet balances while reconciling all historical liabilities; preserve coherent anchors and bounded concurrency |
| `src/deployment.rs`, `src/deployment/bootstrap.rs`, `src/rotation/funding.rs`, `src/wallet_cli.rs` | Both-tier startup/bootstrap, worker scheduling, configuration removal, read-only admin behavior |
| Store status/backup/recovery and CLI output | Tier/role attribution, recoverable state, available versus reserved balances and funding limitations |
| `src/rotation/store/qualification.rs`, qualification modules, shared live driver | Exact tier/pool/wallet attribution, compatible evidence/report readers and versioned new observations |
| README, wallet/config guides, examples, architecture and network inventory | Explain allocation cost, routing threshold, POST limits, retirement tradeoffs and unchanged qualification limits |

Status should distinguish configured targets/caps, observed confirmed balances,
outstanding liabilities, freshness, pair readiness, draining count/cap and why a
promotion or payment cannot proceed. Never sum all tier balances and present the
sum as the maximum payable request. Retired and disabled capacity must not appear
as active spendable capacity. Keep ordinary logs sanitized and bounded.

## Validation matrix

Use temporary state, deterministic unfunded keys and local protocol fixtures.
Assert forbidden I/O and mutations as well as expected results.

| Scenario | Required assertions |
| --- | --- |
| Legacy config and migration | Same IDs, keys, caps, roles, budgets and effective authority; no extra wallets, funding or retired-wallet revival |
| Config boundaries | Disabled/absent tier, invalid ranges, exact `T`, one atomic unit above `T`, parent/tier caps, target-limit opt-out, route minimum changes |
| Both-pair bootstrap | Exactly the expected jobs/allocations, shared budgets, partial readiness, restart and auto-fund/qualification restrictions |
| Useful ordinary remainder | $1.40 active cannot pay $1.80; spare does; $0.30 later uses the retained wallet without a new replacement |
| Large routing | Above-`T` uses large tier only; small calls do not deplete its active/ready pair; a permitted large-tier remainder can serve small calls |
| Unknown or changing price | Candidate handoff reaches correct transport; one GET/HEAD re-challenge only; changed amount/requirements revalidated; POST never automatically replayed |
| Concurrent admissions | No double reservation, duplicate replacement or cap bypass; stable identity under parallel tier/draining calls; shared listeners observe the same state |
| Pending liability | No false depletion/promotion, no release from seller response or unavailable nonce; other eligible draining capacity remains independently accountable |
| Retention bound | Boundary and overflow, tiny positive remainder, explicit retirement, zero-balance canonical retirement; no hidden eviction or allocation |
| Stale evidence and RPC failure | Preserve useful historical work; reject stale admission; no mixed anchors, fabricated zero or timer renewed by unrelated progress |
| Cancellation and restart | Surviving accepted store work, exact signed bytes and liabilities, one outbox, no orphan ownership or replay after lost response |
| Funding budget/recovery | Combined tier limits, rollover, refunds, accepted quote targets, unsigned outages, durable attempts and zero-new-funding all unchanged |
| Configuration removal/re-enable | No new starts in disabled tier; all journals recover; no authority/budget reset or cross-profile wallet adoption |
| Direct/Tor/cover and relay | Payer transport equals signer identity; fresh challenge uses new identity; relay retry-disable semantics preserved; no extra paid probes |
| Saved/live evidence | Tier/payer/generation provenance, historical report compatibility, true retired capacity excluded, no inferred readiness or fee proof |
| Slow reconciliation | Progress beyond prior test durations with many historical wallets, true stalls, concurrent unrelated progress and cancelled waiters; no new runtime deadline |

Extend focused suites in `tests/rotation.rs`, `tests/managed_config.rs`,
`tests/payments.rs`, `tests/network.rs`, `tests/wallet_workflow.rs`, and
`tests/integration_preparation.rs`, plus adjacent unit tests where appropriate.
The shared live driver remains in `examples/live_integration/driver.rs`; do not
duplicate CLI/module wiring. Qualification manifests must explicitly represent
enabled tiers and retain cumulative source/job/API charges. Old started runs remain
observation-only and cannot gain authority through a migrated reader.

Run focused default-Zcash tests after each change, then the repository's required
default-build validation before completion. Do not run no-default-feature suites
without a separate request. Offline fixtures do not qualify real settlement,
funded crash recovery, or privacy. Any live-funded validation requires separate
operator authority; preserve existing deferred acceptance limits.

## Delivery sequence and decisions

1. Settle configuration names, tier ceiling semantics, draining count/default and
   explicit nonzero retirement behavior. Confirm the stable threshold and eager
   bootstrap interpretation. Keep these decisions visible in user documentation.
2. Implement durable tier membership and role migration with legacy/restart tests.
   Prove old pool identities and financial histories remain unchanged.
3. Implement selected-candidate handoff and transport binding, including changed
   challenges and non-GET limitations. Establish this before advertising routing.
4. Implement bounded draining selection and atomic promotion/retirement, initially
   for the ordinary tier, with concurrency, uncertainty and cancellation coverage.
5. Add the optional large pair, authority validation, bootstrap/funding integration,
   cross-tier draining selection, and configuration lifecycle handling.
6. Update status, admin/backup, qualification evidence and maintained guides. Run
   the focused matrix and final repository checks. Keep unfinished modes disabled.

Recommended first version: fixed targets, two independent pairs within each
existing profile, deterministic selection from bounded draining balances, and no
automatic retirement of positive balances. The principal remaining tradeoff is
availability when retained-wallet capacity fills; explicit blocking is predictable
and preserves funds, whereas automatic positive-balance retirement would require
a separately documented operator policy.
