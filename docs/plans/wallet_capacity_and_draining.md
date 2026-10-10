# Wallet capacity tiers and retained balances

## Objective and status

Draft implementation plan. Retain useful balances after rotation and optionally
provide a separate large-capacity Base USDC pair under an existing managed wallet
profile. Ordinary calls use ordinary capacity; expensive calls have explicitly
funded capacity without increasing every wallet's target. This document plans the
feature; it does not implement it or authorize funded execution.

Use ordinary and optional large child pools, a bounded draining set, one generic
wallet admission path, and explicit pair-maintenance transitions. Payment
eligibility and permission to allocate a replacement are separate decisions.
Positive balances are retained while space exists. One explicit retirement limit
allows bounded remainder retirement under count pressure. There is no routine
positive-balance retirement threshold or separately persisted funding-pause switch.

Funding targets remain deterministic. Amount randomization, sweeping change,
topping up old addresses, combining wallets into one payment, and an unrestricted
wallet scheduler remain outside this version. Larger wallets and longer address
use have privacy costs; neither tiers nor retained balances establish unlinkability.
High-cost POST support requires an operator-declared initial tier binding and a
sufficient initial payer; funding a large pair alone cannot route every unknown
POST transparently.

## Existing implementation and dependency boundaries

- `src/rotation/config.rs` defines funding targets/output ceilings, the API cap,
  and the compatibility-default target-based payment ceiling.
- `src/rotation/store.rs` enforces one `ACTIVE` and one `READY` wallet per pool.
  Current promotion retires the active and allocates a replacement atomically, but
  commits before returning `PayerChanged`. The new design must explicitly change
  this ordering rather than assume admission already shares that transaction.
- `src/rotation/manager.rs` holds the pool gate from reconciliation through signed
  journaling. `chain_query_inner` deletes unsigned `ADMITTED` rows under this
  ownership assumption. Serializing database calls alone does not protect a signer
  between admission and journaling.
- `src/rotation/base.rs` stores `min(confirmed_balance, latest_balance)` for
  admission. That conservative amount is not necessarily the exact balance at the
  confirmed anchor. Retirement accounting needs separately preserved evidence.
- `src/payment.rs` binds transport to a candidate before the initial request and
  permits one pre-signing GET/HEAD re-challenge on payer change. Other methods fail
  before signing. `src/discovery_relay.rs` uses `POST /curl` and shares this limit.
- Retired wallets retain historical reconciliation but cannot pay. The wallet role
  CHECK excludes `DRAINING`; migration must preserve foreign keys and encrypted
  records. Pool and funding-policy resolution currently use names in several paths.
- Deployment setup, bootstrap, supervised funding, wallet administration and
  recovery share production paths. Extend them rather than duplicate them.

Keep the pinned x402 2.0.2 signer and exact EIP-3009 accepted-payload validation.
The signer supplies cryptographic authorization and random nonces; it does not own
our wallet selection, durable liabilities, retirement or funding policy. Use one
application cap calculation at selection and authoritative store admission; SDK
validation remains a distinct protocol check. No new signer or nonce scheduler is
needed. Permit2/upto support is not added by this plan.

Production funding uses the local 1Click adapter, not the Omni Bridge SDK in
`reference_repos/Near-One`. Reference checkouts are supporting material, not proof
of the pinned application's contracts. Cargo pins and reviewed vendor patches
remain authoritative for Zingolib integration. Keep one serialized treasury owner
and existing quote, deadline, canonical settlement and recovery checks. This work
establishes no new external settlement contract and changes no submission margin.
See [wallet rotation](../wallet-rotation.md), [architecture](../architecture.md),
[network policy](../network-egress.md), and
[qualification limits](../testing.md#qualification-status).

## Configuration, routing and authority

Keep one logical named profile and existing listener/source wallet bindings. Its
large tier never becomes a global high-value wallet or causes automatic per-source
pool creation. Dynamic sources and discovery relays retain current effective
profile resolution. Configuration is operator-owned; agent arguments cannot grant
a tier, wallet, funding permission or larger payment cap.

The following semantics are settled for this version. Final TOML layout and CLI
spelling must be pinned with parser/help fixtures before implementation proceeds.

- Optional nested `large` configuration has a funding target and maximum accepted
  output. The target must exceed ordinary `funding_amount_usdc`, denoted `T`.
- Keep one parent `max_api_payment_usdc`. Do not add a separate large-tier API cap.
  Apply the existing `limit_payments_to_funding_target` setting to the selected
  wallet's current tier target: ordinary `T` or large `L`. With the default true,
  `effective_cap(tier) = min(parent_cap, configured_tier_target)`; with false it is
  the parent cap. Preserve the existing representation of an unlimited parent cap.
  Actual admission additionally requires independently verified capacity minus
  unresolved liabilities. Funding output ceilings never grant payment authority.
- Validate that an enabled large tier's effective cap permits at least one atomic
  unit above `T`. Enabling the tier alone cannot increase the parent cap.
- Retention is independently opt-in per tier, with `max_draining_wallets`, default
  eight when enabled, and `max_retirement_remainder_usdc`, default `"0.01"`
  (10,000 micro-USDC). Explicit `"0"` opts into zero automatic remainder retirement.
  Ordinary and large tiers configure these values separately. Each omitted value
  resolves to its own default; a large tier does not inherit an ordinary-tier
  override. An operator may explicitly set a larger large-tier retirement limit.
  Parse the amount as bounded nonnegative integer micro-USDC; existing positive-only
  funding parsers cannot be reused unchanged. No threshold scales with target or
  with the minimum payment amount routed to the large pair.
- Zero draining slots still applies the remainder limit. Configuration must impose
  a finite supported count ceiling chosen from the RPC/freshness measurements in
  validation. Eight is the proposed default, not evidence of acceptable latency.
- Inherit conversion-overhead, recovery-attempt and ZEC/fee safeguards. Add no
  tier-specific budget ledger or fee policy in this version.

Routing has two explicit modes:

| Current configuration | Eligible capacity by amount |
| --- | --- |
| No enabled large tier | Preserve ordinary-only legacy authority, including above-`T` payments when target limiting is false and actual ordinary capacity suffices |
| Enabled large tier, amount `<= T` | Ordinary active/ready/draining plus enabled large-tier draining wallets; never automatically use the large active/ready pair |
| Enabled large tier, amount `> T` | Large active/ready/draining only; insufficient capacity cannot churn the ordinary pair |

Exactly `T` is ordinary. Bridge minimum increases do not change `T`. Disabling or
removing the large tier restores ordinary-only routing under current ordinary caps;
it does not expose disabled large drainers. Changing configured targets changes
future routing/caps and future allocations, but never rewrites a wallet's accepted
funding target, signed authority or history. “Tier membership” means immutable
stored membership, while spending eligibility uses current enabled configuration.
A retained large wallet serving a small call still uses its large-tier cap.
The above-`T` routing threshold does not impose a minimum payment on large-tier
drainers: any otherwise eligible small payment may use one, whether its remaining
balance is above or below `T`. This does not make a large active/ready wallet
eligible for small calls or waive the existing payer-handoff rules.

Use deterministic supported-offer order: choose the first fully valid offer that
fits current routing and effective-cap policy. Do not shop across offers according
to transient balances or retry another offer after a signed attempt. Test mixed
unsupported, over-cap and cross-tier offers. Catalog prices remain estimates and
do not select tiers or authorize calls.

### Initial tier bindings for POST and other methods

Include operator-owned exact method/path initial tier bindings in this version.
Implement them in provider/source settings with explicit source overrides, resolved
through the existing selected catalog. Reject duplicate/conflicting entries,
unknown routes and large bindings without an enabled large tier. No URL patterns,
price estimates, agent-supplied overrides or new global wallet selector are needed.
The binding selects the initial tier within the already resolved profile; it never
changes owner, cap or amount routing. A large-bound route returning a small-priced
challenge fails eligibility for that pair rather than silently bypassing routing.

Default initial selection uses the ordinary active; a large binding uses the large
active. An initial binding is not a guarantee that the chosen address can afford an
unknown price. POST cannot switch to a ready/draining address after its challenge.
When the bound wallet remains sufficient and eligible, keep it even if a drainer
would otherwise be preferred. If it is unsuitable, return the actionable typed
failure without promotion, allocation, signing or replay. Repeating the same call
is not a guaranteed remedy. Expose this limitation in help and operator guidance.

Dynamic registration cannot author these bindings. This version confines bindings
to operator-configured provider/source catalogs, including the relay bootstrap
provider; dynamically imported routes keep ordinary initial selection. A future
dynamic-route policy is separate work. Dynamic fallback still uses the same
challenge-based GET/HEAD selection and generic admission. A relay binding applies
to the outer `POST /curl`, never the fetched target's method. Define and test this
at the shared PaidClient boundary. Removing a large tier also requires removing
its initial bindings; reject the conflicting configuration before mutation.

### Initial allocation and startup

Two fully funded pairs initially allocate `2*T + 2*L` USDC before accepted bridge
minimum increases: $2/$10 targets initially allocate $24. Report that initial
amount and its configured maximum, with conversion/network costs separate.
Subsequent balances retained in drainers add capital outside the pair; neither
number is lifetime allocation, current total holdings or API consumption.

Use eager bootstrap for enabled pairs. Ordinary managed serving with auto-funding
initializes both pairs before discovery; explicit bootstrap does likewise. A large
pair's failure can therefore prevent this initial startup, even if the ordinary
pair is funded. Document and test partial startup rather than imply independent
startup availability. Existing auto-fund-disabled operation retains its rules.
Inspection, init/sync and source registration never acquire funding authority.
Large bootstrap is not triggered by a provider challenge. Lazy bootstrap remains
outside this version.

## Durable membership and ownership

Map each logical profile to its existing ordinary pool and optional large child
pool through explicit unique durable membership. Preserve ordinary pool/wallet
IDs, sequence numbers, keys and authenticated encryption context. Generated names
are display values, not authority or ownership. Child IDs must not collide with
another profile or reset gross funding history.

Resolve funding policy, profile enablement, recovery and qualification through
that membership everywhere currently relying on `job.pool_name` or pool-name
lookups. Disabled children retain exact operation ownership for recovery. Keep
pair generation separate from relevant routing/configuration revision. Payments
and reconciliation on other wallets must not invalidate an otherwise eligible
selected wallet merely because another payment completed.

Each bootstrapped tier has at most one active and one secondary slot. The secondary
is either a funded `READY` wallet or an `ALLOCATED` replacement with its existing
funding job. Bootstrap's two initial jobs are the existing exception. `READY` means
funding completed, not untouched/full capacity. Draining wallets never become pair
slots again and acquire no new funding job; their historical funding records remain.
Every drainer counts against its immutable origin tier's draining limit, including
after small payments reduce a large-tier remainder below `T`. It is never converted
to an ordinary-tier wallet, moved between pools or charged to an ordinary draining
slot. Neither unused slots nor a larger retirement tolerance in another tier can
be borrowed to bypass its origin tier's bounds.

Use one shared gate per logical profile for both child pools, foreground admission,
background reconciliation, pair maintenance and retirement. Hold it across query
preparation, RPC evidence acquisition, admission, signing and signed journaling.
All paths that clear unsigned `ADMITTED` rows must take that same gate. Release it
before a provider request/response, including the unsigned re-challenge. Reacquire
and revalidate on return. No nested independently operating child gates, and no
serialization of unrelated profiles beyond the existing store/treasury ownership.
The store still rechecks role, membership, policy revision and evidence atomically;
the gate does not substitute for those checks or accepted-worker cancellation rules.

## Evidence, selection and generic admission

Separate historical authorization reconciliation from fresh admission evidence.
Retain every unresolved liability until existing canonical resolution permits its
release. Unavailable nonce observations retain exposure. Malformed evidence,
changed anchors and wrong-chain observations retain their typed safety failures.
Do not add a whole-sweep deadline or a wallet-count deadline for historical work.

Extend typed balance evidence to preserve exact confirmed balance and anchor,
latest balance and anchor, and the conservative admission amount
`min(confirmed, latest)`. Availability subtracts every unresolved reservation from
that conservative amount. Saved amounts may guide ordering, never authorize a
payment or retirement. Store complete provenance without labelling the minimum as
an exact confirmed balance.

Retirement uses the exact fresh confirmed balance. If latest evidence differs,
defer retirement of that wallet until fresh confirmed/latest evidence agrees;
do not mistake a recent debit for confirmed emptiness or omit a visible incoming
credit from the retirement decision. This wallet-local condition does not prevent
otherwise safe admission using the conservative amount. It is not an assertion
that future unexpected credits are impossible. Never require equality of the two
balance observations merely to pay.

### Bounded selection without a global best-fit requirement

Within eligible tier scope, use a deterministic draining order: saved available
amounts sufficient for the offer first, ordered ascending, then remaining drainers,
with immutable wallet-ID ties. These are ordering hints only. Verify candidates
against fresh evidence and choose the first sufficient one. Try each candidate at
most once per selection pass, then the active, then the ready wallet. A verified
usable bound payer for a non-rechallengeable method takes precedence over draining.
Re-challenge admission revalidates its selected payer without restarting the search.

Do not require fresh balances for all candidates to prove a global minimum. An
optional candidate's unavailable/stale balance makes that candidate unavailable;
a separately verifiable active/ready wallet may still pay. Preserve diagnostics
and do not turn partial observation into proof of emptiness, complete inventory
or retirement permission. Invalid shared chain evidence remains an integrity
failure, not an ignorable candidate miss.

Implement explicitly scoped balance queries/views and store updates for this path.
Current `apply_chain_view` requires every non-retired wallet in a view; do not remove
that completeness assertion globally. A partial view must name its observed wallet
set, update only those balances, retain all other liabilities and carry compatible
canonical anchors. Never combine balances across endpoints to fabricate one view.
Retain independently anchored historical resolution evidence under existing rules.
Keep bounded RPC concurrency and bound one pass by the configured live candidate
set; introduce no repeated candidate search or unrelated-progress freshness renewal.

### One admission operation

Every existing eligible active, ready or draining wallet uses the same admission
operation: validate offer and current caps, verify fresh capacity, reserve the
amount, sign, validate the payload and journal before submission. Store admission
rechecks exact identity, membership, enablement, role and relevant revision.
No role change or funding allocation is required merely because the payer is ready
or draining. Existing active liabilities cannot block an independently affordable
payment on another wallet, but still prohibit retiring/replacing that busy active.

This behavior applies regardless of whether replacement funding is blocked by
retention, lack of a permit, disabled automatic funding or another funding reason.
There is no special loss-pause admission implementation. Payment qualification
permission remains independently required. Legacy deployments retain their cap,
address-retirement and funding authority; use of an already funded ready slot is
an intentional availability extension that must be documented and tested.

## Challenge handoff and mutation ordering

Validate the complete supported offer and SDK envelope before any financial
mutation. Return an internal selected-candidate handoff binding profile, tier,
pool, wallet/address, relevant revision and request context. It is neither an
agent-supplied token nor a reservation or permission to reuse the first challenge.

For GET/HEAD requiring another payer:

1. Select the prospective identity without promotion, retirement or allocation.
2. Release the profile gate, acquire the identity-bound client and obtain the one
   permitted new unsigned challenge.
3. Reacquire the gate; validate the new offer, routing and fresh wallet evidence.
   Keep the selected identity if still eligible. Changed requirements are validated
   completely; a further payer change returns a typed failure without another loop.
4. Admit on that wallet. If a normal promotion is also permitted, commit the role
   changes, retirement event if any, replacement ownership/outbox and admission in
   one transaction. If maintenance is blocked, admit without the role changes.
5. Sign and journal on the admitted identity before the sole paid submission.

Cancellation, unusable second challenges and exhausted handoff allowance cannot
themselves request promotion, retirement or replacement. The handoff path commits
no maintenance before final offer/identity validation. A valid capacity shortfall
may instead record the separate maintenance intent defined below; it does not
allocate in response to a transport failure or payer-change error. Independent
already-authorized background work may continue. After a store mutation is
accepted, waiter cancellation does not undo it or grant permission to repeat it.
If admission commits and signing fails, retain current unsigned-lease cleanup
semantics under the shared gate; committed maintenance is not rolled back by a
missing provider result.

Provider transport, Tor isolation, cover state, signer and durable payment record
must use the same address. Direct calls, dynamic fallback and paid relay all use
this path. Possible relay submission still disables shared retries and routing
cannot reset its marker. No challenge refresh or second-wallet attempt is allowed
after signed journaling. No automatic POST replay is introduced.

## Pair maintenance and retained balances

Maintenance restores useful pair capacity under current funding authority; it is
separate from eligibility to spend existing funds. It has two replacement-producing
transitions, both requiring fresh evidence, no unresolved work on the outgoing
wallet, permitted retention/retirement, and current allocation/funding permits.

| Transition | Preconditions and durable result |
| --- | --- |
| Bootstrap `ALLOCATED` to pair roles | Existing canonical funding readiness for both initial wallets; preserve bootstrap ownership |
| Replacement `ALLOCATED` to `READY` | Existing canonical funding readiness completes the already-owned secondary slot |
| Normal promotion | Active cannot cover an authorized demand without pending liabilities; ready can. Move outgoing active to draining/retired, activate ready and create exactly one secondary allocation atomically; include admission when serving the selected ready payer |
| Repair spent secondary | Both pair wallets cannot cover the validated capacity need and have no unresolved liabilities; ready is below that need. Move ready to draining/retired and create exactly one secondary allocation without requiring it to be promotable; keep the active unchanged |
| Proven-empty active maintenance | With no active liabilities and a positive eligible ready balance, promotion may restore service without another provider call, under the same retention and funding gates |
| Existing wallet payment | Active/ready/draining role is unchanged unless an independently permitted normal promotion is committed with admission |
| Empty drainer retirement | Fresh matching confirmed/latest zero balance and no unresolved work; retain all records |
| Pressure retirement | An outgoing pair wallet or eligible drainer may retire within the single remainder limit to permit one maintenance transition |
| Manual drainer retirement | Explicit operator action described below; no replacement allocation |
| Retired to spendable | Never automatically, including migration and re-enable |

A validated capacity need must come from a fully supported, currently authorized
challenge or an outstanding maintenance need originally observed that way. Persist
only wallet/generation-bound maintenance intent and required capacity, not a payment
permit or a replayable provider request. Before later repair, revalidate current
profile/tier/caps, fresh evidence, funding policy and current permits. Disabled or
no-longer-applicable intent is inactive; it never authorizes a new API request.
Store at most one such need per secondary generation, retaining the largest still
eligible amount from concurrent valid shortfalls; clear it when satisfied or
invalidated, and never carry it into a replacement generation. This is scheduling
evidence, not a new spending budget or timer.

Record this intent under the profile gate only after a complete supported offer
and fresh evidence establish a capacity shortfall, with no sufficient eligible
wallet found. The unsigned call returns a typed capacity result and is finished.
The accepted intent can survive its waiter, but never reserves a payment or stores
provider arguments for replay. Automatic maintenance or explicit bootstrap may
later execute it under their current funding authority; auto-fund-disabled service,
read-only admin and recovery only report it. A binding/payer-change failure,
unavailable balance, unsupported offer or rejected cap cannot create this intent.
The absence of an old API waiter neither revokes an accepted maintenance intent
nor supplies missing funding authority.

For repair, the required amount must fit the configured future funding target as
well as the effective cap. Do not assume a bridge minimum increase will make a new
wallet large enough. An above-target payment allowed by the target-limit opt-out
can spend existing capacity, but cannot repeatedly allocate smaller replacements.
A sufficient existing wallet should serve the request without request-triggered
repair. Pending liabilities cannot establish depletion or cause slot replacement.
A ready wallet is not replaced merely because it was used once or fell below target.

Example: active=$1.40 and ready=$0 after independent ready-slot payments. A valid
$1.80 need under a $2 target can request secondary repair even though no spare is
promotable. If retention blocks the transition, preserve the maintenance intent.
Once space becomes available, fresh evidence and current funding authority allow
retirement of the empty ready and exactly one $2 replacement. On confirmed funding,
normal admission/promotion is possible. The old API call is never replayed.
Test the same sequence with a positive insufficient ready remainder and with
pending ready authorizations that postpone repair.

### One retirement tolerance, applied under pressure

For tiers using retention, retain positive outgoing balances while space exists.
Retire proven-empty wallets without a loss warning. When a transition needs a slot,
consider the outgoing wallet and eligible drainers in that tier; choose the smallest
fresh exact confirmed remainder within `max_retirement_remainder_usdc`, with an
immutable identity tie-break. Retirement candidate evidence can be collected for
this maintenance decision without making every payment depend on that collection.
An unknown or busy wallet is ineligible; it is never treated as the cheapest.

A pressure decision can operate on the eligible freshly observed subset. It need
not prove a global minimum across unavailable wallets. If none qualifies, retain
ownership and report the typed obstruction; missing evidence and an exceeded
monetary limit remain distinguishable. At most one positive retirement is allowed
per ordinary transition. Never retire the selected payer or a wallet with unresolved
work. With a reduced count, do not grow the existing draining set or bulk-retire it;
a transition can retire its outgoing wallet or replace one existing drainer while
keeping the count unchanged. Empty/manual retirements can reduce the excess.

If the needed retirement is above the limit, block that maintenance/allocation
transition. Existing payments and independently owned funding jobs continue.
Derive the obstruction from roles, intent and current policy; there is no durable
pause flag, reset command or approval threshold. Reevaluate through normal
reconciliation/maintenance cadence and configuration reload/restart paths. Fresh
balance evidence is required for a new transition, not a historical log entry.

The default limit is $0.01 per retirement. Positive remainders, including sub-cent
amounts, remain usable while retention space exists. Under count pressure, an
eligible remainder at or below $0.01 may retire; a $0.010001 remainder exceeds the
default limit. This keeps the live wallet count bounded without requiring tiny
remainders to reach exactly zero. It does not guarantee rotation: if every eligible
remainder exceeds the limit, the maintenance transition remains blocked.
Explicit zero tolerance can block rotation whenever only positive remainders
remain. Explain both progress tradeoffs and cumulative retirement accounting.
Operators can make ordinary authorized calls that fit the balances,
raise count/tolerance, or manually retire a drainer. No synthetic paid drain probes,
age-based retirement or implicit cumulative loss budget are introduced.

### Large-tier remainders and full draining sets

With ordinary target $2 and large target $10, a large active wallet holding $1.40
cannot cover a $5 request. When the normal promotion conditions are satisfied, the
large ready wallet takes over and the $1.40 wallet becomes a large-tier drainer.
A later eligible $0.003 ordinary API call can use that drainer under the shared
selection/admission rules. Its membership, accounting and draining-slot charge
remain in the large tier; no transfer or ordinary-tier capacity reservation occurs.
Cross-tier small-call eligibility is not a guarantee that a particular drainer
will be selected or eventually reach zero.

If the large draining set is full, consider only the outgoing large-tier wallet
and eligible large-tier drainers for retirement under the large tier's own limit.
If none qualifies, block the transition requiring another large draining slot.
Affordable existing payments, including small calls against enabled large drainers,
continue, as do independently permitted ordinary-tier maintenance and already-owned
funding jobs. Do not spill wallets into the ordinary draining set or recursively
retire ordinary wallets to make room. If both sets are full, evaluate each tier's
maintenance independently; shared treasury budgets and ownership still apply.

Keep the $0.01 default for both tiers. A remainder below the large pair's routing
threshold can still fund small calls, so that threshold does not justify automatic
retirement of larger amounts. For workloads making only expensive calls, small
remainders may never be consumed. Without sweeping or aggregation, the operator
must accept more retained wallets, explicitly tolerate larger retirement amounts,
or accept blocked replacement transitions. Moving wallets between tiers would only
relocate this constraint. Document this workload tradeoff without promising that
retention always restores progress.

### Allocation boundary and existing funding jobs

Establish replacement slot ownership and the necessary retention/retirement in the
same allocation transaction. A full draining set after that commit does not revoke
the already-owned replacement slot or require another retirement before its funding.
One outstanding secondary allocation excludes a second repair/promotion allocation.

Background funding checks the existing slot/job ownership and current funding
restrictions, monetary budgets and permits before new reservation/preparation.
It does not rerun unrelated retirement selection at every phase. Unallocated
maintenance intent waits when blocked; already allocated jobs retain ownership.
Already reserved, prepared or possibly submitted work follows existing completion
and recovery obligations. New restrictions never mean an uncertain transaction
was cancelled. Foreground and background paths share the same transition function.

### Durable accounting and manual retirement

Journal each positive retirement in the role-change transaction with wallet,
profile/tier, exact confirmed balance/anchor, latest corroborating anchor, amount,
reason, applicable policy and time. Emit a sanitized WARN after committed automatic
positive retirement; explicit manual retirement reports its accepted amount. Empty
retirement needs no loss warning. Bound repeated obstruction warnings with explicit
suppression counts while retaining every durable event.

Report the sum as **recorded balance at retirement**, with automatic/manual and
legacy/retention reasons. It is not current retired holdings, an API fee or destroyed
funds. Later credits do not rewrite the event or reactivate a wallet. Historical
unclassified retirements remain unknown. The sum never replenishes gross funding
budgets. Per-retirement tolerance does not bound cumulative stranded capital.

Add `wallet retire --config FILE --profile NAME --tier TIER --wallet-id ID --max-remainder-usdc AMOUNT`
(final shared-Clap spelling to pin with fixtures; tier is `ordinary` or `large`).
Require exclusive treasury/store
ownership while serving is stopped. Permit only a `DRAINING` wallet in the asserted
profile/tier, including a disabled one for administration, with fresh evidence and
no unresolved work. The explicit amount is the operator's maximum for this action;
reject a larger observed remainder. This permits an intentional retirement above
the automatic limit without an interactive approval workflow. It signs no payment,
transfers no funds, allocates nothing and preserves keys/history. Active/ready slot
removal is outside this manual command. Read-only status remains separate.

## Configuration lifecycle, funding and recovery

Legacy tiers that never enabled retention keep the existing retirement policy.
Persist the fact that a tier entered retained-balance management so removing TOML
cannot silently restore unlimited automatic retirement of its retained capital.
For such tiers, disabling/removing retention stops adding drainers (effective count
zero), keeps existing drainers eligible under current tier/cap policy, and applies
the current remainder limit, default $0.01, to future automatic positive
retirements. Display this resolution in offline config policy and runtime status.
Do not silently delete, freeze or retire existing drainers. Re-enabling uses the
same identities. Configuration count reductions do not bulk-clean the set.

Removing/disabling the large tier disables all its new admissions and allocations,
including its drainers, while preserving exact signed bytes, jobs, refunds and
liabilities for recovery. Re-enabling resolves the same durable child. Configuration
changes apply to future authority and allocations; accepted quote targets and past
retirement events remain immutable. Offline `config show` states runtime identity/
retention-resolution rules without opening state; runtime status gives the resolved
historical facts.

Profile rename is not an in-place operation in this version. A new configured name
creates a separate profile only under ordinary new-funding authority; the old one
remains disabled and recoverable. No suffix-based adoption or history transfer.
Treasury-wide budgets still include both histories. Document this before showing
configuration rename examples. A dedicated identity-preserving rename is deferred.

Both tiers use the same treasury, gross USDC and ZEC limits, fee checks, quote
authentication and durable recovery history. Schedule jobs through the existing
turn/backoff scheduler; do not add a second tier scheduler or promise parallel
Zcash mutation. Test independent eligible work after unsigned failures, while
preserving the wider scope justified by uncertain treasury mutation.

Check allocation restrictions and permits before any positive retirement attached
to maintenance. A denied transition cannot strand funds as a side effect. Preserve
zero-new-funding restrictions before setup and each supervised reopen. Generic
admission consumes no new funding authority; promotion/repair allocation does.
Recovery and read-only administration never bootstrap a child, fulfill maintenance
intent with a new allocation or acquire a new funding permit. Explicit bootstrap
and authorized automatic maintenance use the same eligibility implementation.
Old started qualification runs remain observation-only.

## Schema migration and restart

Version admission schema separately from encrypted instance format. Extend role
constraints and add explicit tier membership, retained-mode history and bounded
maintenance intent while preserving foreign keys, partial unique indexes and
existing encrypted records. Transactional table replacement must retain indexes,
triggers and all referencing rows; validate foreign keys before commit. Exercise
rollback and reopen after injected failures, not just a successful empty migration.

Map existing pools to ordinary membership without allocation, retention enablement,
key rewriting or historical event rewriting. Existing retirees remain retired.
Older executables must reject incompatible new state. Document backup/upgrade
requirements without suggesting downgrade via a stale financial snapshot.

Restart reconstructs membership, roles, generations, intents, funding slot ownership
and pending authorizations exactly. Accepted store work survives waiter cancellation.
No missing response grants permission to repeat financial work. Test before/after
admission, journaling, promotion, secondary repair, quote reservation, funding credit
and manual retirement commits. Backups include all new state and decryption material
under existing privacy/ownership rules; test restored readability and ownership in
isolated unfunded fixtures.

## Implementation surfaces

| Area | Required work |
| --- | --- |
| `src/rotation/config.rs`, provider/deployment config and assignment | Concrete tier/retention/route-binding schema, shared cap calculation, exact route validation, offline resolution policy |
| `src/rotation/store.rs`, `store/allocation.rs`, `store/funding.rs` | Membership/role migration, generic admission, bounded maintenance intent, two replacement transitions, atomic accounting and slot ownership |
| `src/rotation/manager.rs`, `src/rotation/error.rs` | Shared profile gate, scoped candidate selection, typed failures, selected-candidate handoff, shared maintenance entry point |
| `src/payment.rs`, `src/network.rs`, cover and relay integration | Initial binding, bound-payer preference, one GET/HEAD handoff, mutation ordering and unchanged signed-attempt isolation |
| `src/rotation/base.rs` and store evidence application | Distinct exact/conservative balances, scoped views without weakened full-view validation, retained historical reconciliation |
| `src/deployment.rs`, `src/deployment/bootstrap.rs`, `src/rotation/funding.rs`, `src/wallet_cli.rs`, shared CLI tree | Both-tier setup, member-based policy resolution, existing scheduler integration, disabled-child recovery and explicit manual retirement |
| Status/backup and qualification modules, shared live driver | Actual per-wallet capacity, membership, maintenance/accounting provenance, versioned observations and historical reader compatibility |

Status distinguishes configured targets/caps, exact observed balances, conservative
admission amounts, liabilities, freshness, roles, draining count/limit, recorded
retirement amounts, pending maintenance and owned replacement jobs. A ready wallet
can be partially spent; pair status must say so. Report disabled/retired holdings
separately and never present summed tier balances as maximum payable request size.
Ordinary logs and public reports remain sanitized and bounded.

## Documentation update checklist

Update these destinations with implementation, keeping this document marked as a
plan until the behavior is delivered. Audit statements about one pair per profile,
untouched ready wallets, active-only admission and retirement on every promotion.

| Destination | Required update and consistency check |
| --- | --- |
| `README.md` | Tier and retention opt-ins, wallet-sharing table boundaries, initial versus retained capital, high-cost POST binding and remaining limitations |
| `docs/configuration.md` | Final TOML names/defaults/ranges, independent per-tier retention overrides with $0.01 default for each, single cap formula, absent/disabled-tier routing, exact route bindings, retained-mode disable semantics, offline resolution policy and rename behavior |
| `docs/wallet-rotation.md` | Full transition table including spent-secondary repair, generic ready/draining admission, immutable origin-tier count/accounting, small-call use of large drainers, independent full-set behavior and expensive-only workload tradeoff, shared gate, evidence distinctions, single retirement tolerance and funding ownership boundary |
| `docs/wallet-cli.md` and generated `wallet --help` / `wallet retire --help` | Exact command and maximum-remainder option, exclusive ownership, drainer-only scope, no implicit funding, status terminology and backup/upgrade procedure |
| `docs/architecture.md` | Logical profile/child membership, one gate, scoped Base views, mutation ordering, maintenance intent and treasury reuse |
| `docs/agent-sources.md` | Unchanged dynamic profile sharing and principal rules; agents cannot select tiers/author route policy; dynamic fallback uses the same admission and binding behavior |
| `docs/network-egress.md` | Candidate RPC work/concurrency/count ceiling, exact/conservative anchors, handoff identity, remaining timer/budget inventory; no new whole-work deadline |
| `docs/cover-traffic.md` | Selected payer remains cover owner across tier/draining paths; one permitted re-challenge and POST limitations |
| `examples/deployments/servers-managed.toml` | Commented opt-in example with explicit parent cap, large target, separate ordinary/large retention settings, an optional explicit large-tier tolerance override, gross budget and initial allocation cost; overrides do not change documented defaults |
| `examples/deployments/agent-sources-managed.toml` | Verify shared dynamic wallet behavior remains accurate; show no automatic per-source tier creation |
| `examples/deployments/public-payment-demo.toml`, `public-swap-demo.toml`, and `docs/public-swap-demo.md` | Preserve bounded-demo opt-outs and existing authority; do not silently enable large bootstrap or retention |
| `tests/live/INTEGRATION.md`, `INTEGRATION_REFERENCE.md`, and `integration/SCENARIO_PRESETS.md` | Versioned tier/wallet/generation evidence, repair/POST/retirement assertions and exact run-authority boundaries |
| `tests/live/integration/managed-deployment.example.toml`, `rotation.example.toml`, `authorization.example.toml` | Explicit optional new scenario scope and cumulative charges; no new authority inherited from old manifests |
| `docs/testing.md` and `docs/plans/deferred/live_integration_acceptance.md` | Offline evidence limits and unchanged deferred funded lifecycle/privacy qualification; implemented tests do not imply live qualification |
| `AGENTS.md` | Reconcile active/ready lifecycle and profile guidance after implementation, preserving treasury, payment and qualification safeguards |

Verify internal links, examples and generated help against the final parser.
Documentation assertions should be checked in existing CLI/config/contract suites
where those outputs already have fixtures. Keep user-owned local deployment copies
untouched. Public Markdown excludes raw identifiers, credentials, paths and provider
prose; private JSON retains necessary typed evidence under existing bounds.

## Validation matrix and acceptance gates

Use temporary state, deterministic unfunded keys and local protocol fixtures.
Assert forbidden provider I/O, signing, retirement, allocation and preparation as
well as successful outcomes. Use barriers/fault injection for race boundaries;
harness deadlines are not runtime policy.

| Scenario | Required assertions |
| --- | --- |
| Legacy authority | Existing IDs/keys/caps/roles/budgets survive; above-`T` ordinary calls with target opt-out still work without a large tier; no migration-created funding or retired revival |
| Migration with real relationships | Populate funding jobs, attempts, resolutions, refunds and encrypted records; validate foreign keys/indexes/decryption; inject failure during table replacement, reopen and compare pre-migration state; old executable rejects new schema |
| Config/cap boundaries | Exact `T`, one atomic unit above, enabled/disabled large, parent cap, target opt-out, immutable old wallet targets and bridge minimum changes; invalid/unusable tiers and zero/overflow decimal parsing |
| Multi-offer selection | Unsupported and over-cap offers skipped deterministically, tier eligibility respected; balances do not reorder offers; exact accepted JSON reaches signer |
| Both-pair bootstrap | Expected jobs/capital, shared budgets, partial readiness and failed large startup, restart, auto-fund-disabled and zero-new-funding restrictions |
| Useful remainder | $1.40 active cannot pay $1.80, ready pays via permitted handoff/promotion; later $0.30 uses retained capacity without another allocation |
| Generic ready admission | Identical safety path with retention obstruction, denied funding permit, auto-fund disabled and active pending liabilities; no required role mutation or new funding; no cross-tier/cap bypass |
| Empty secondary repair | Active=$1.40, ready=$0, valid need=$1.80, target=$2; blocked maintenance resumes with fresh evidence after space is freed; one replacement without sufficient-spare promotion; confirmed replacement permits later service, never API replay |
| Partial/busy secondary repair | Positive insufficient ready retained/retired by policy; ready with unresolved authorization not replaced; an above-target need cannot churn smaller replacements; repeated/concurrent requests share one intent/allocation |
| Gate ownership | Pause between admission and journaling while background reconcile/retirement and another tier call contend; live `ADMITTED` row survives; cancellation/restart cleans only proven unsigned work; unrelated profiles progress |
| Initial binding and POST | Cheap POST keeps sufficient bound active despite eligible drainer; large-bound POST pays when sufficient; unbound expensive POST, small price on large binding and insufficient bound payer fail before mutation/signing/replay; invalid/duplicate/agent-authored bindings rejected |
| Handoff ordering | Correct direct/Tor/cover identity; one GET/HEAD re-challenge; changed amount/requirements, concurrent role/config changes, cancellation, transport error and unusable second challenge cause no request-triggered maintenance before admission |
| Relay and dynamic fallback | Actual `POST /curl` outer binding; inner GET is not replay permission; same profile ownership and caps; possible-submission marker never reset; no extra paid probes |
| Candidate availability | Optional drainer RPC failure/staleness does not block independently valid active admission; unknown balances never become zero; complete-view assertions remain; no mixed endpoint or invented anchors |
| Evidence divergence | Confirmed/latest differ in both directions; admission uses conservative amount, retirement waits locally; persisted exact amount matches its anchor; unrelated safe wallet still pays; unexpected later credits do not rewrite retirement events |
| Retention limits | Zero/default/max/invalid counts, reduced count without growth or bulk cleanup; omitted remainder limit resolves to 10,000 micro-USDC; sub-cent and exactly $0.01 remainders retained with room and eligible for retirement under pressure; $0.010001 exceeds default; explicit zero blocks positive retirement; unavailable/busy wallets excluded from eviction |
| Independent tier retention policy | Omitted settings default independently to eight slots/$0.01; ordinary override does not alter large defaults and large override does not alter ordinary policy; changing targets/route threshold cannot scale tolerances or move wallet membership |
| Small calls on large drainers | $2/$10 targets, $1.40 outgoing large remainder and later $0.003 eligible call; same wallet/pool, large-tier slot charge, caps and payer binding; no ordinary slot consumption, transfer or extra funding; works with ordinary draining set full and with a large drainer whose balance still exceeds `T` |
| Full large/both draining sets | Retire only within the affected tier's configured tolerance; above-limit obstruction blocks only the required transition; small payments and independently permitted ordinary maintenance/owned funding continue; no borrowed slots, cross-tier eviction, reclassification or recursive retirement; expensive-only workload stays bounded without guaranteed progress |
| Retirement accounting | No retirement with unresolved work or stale evidence; one pressure retirement per transition, atomic event/role/outbox; no loss on denied allocation; repeated small retirements visible; legacy unknowns and warning suppression |
| Owned funding jobs | Promotion fills final draining slot but its replacement remains fundable; later retention pressure cannot require a second retirement; current monetary/qualification restrictions still enforced; competing worker creates no duplicate slot |
| Maintenance resumption | Durable bounded intent, concurrent needs coalesce within one generation, fresh evidence after restart/spending/policy change, invalidated/satisfied need discarded; accepted intent survives cancelled waiter without API replay; auto-fund-disabled/recovery paths only report it; no stale intent grants API or funding authority; no mutable pause reset required |
| Manual retirement | Exclusive lock while serving stopped, exact drainer/profile identity, explicit maximum amount, disabled-tier administration, pending/active/ready rejection, cancellation and atomic event; no signing/transfer/allocation |
| Lifecycle changes | Large remove/re-enable restores same identity; ordinary-only routing restored; retention disable keeps old drainers eligible with no growth and $0.01 default remainder limit; explicit zero remains effective when configured; threshold changes and new profile name cannot adopt old state/reset budgets |
| Child policy resolution | Funding, recovery, status and qualification resolve child membership rather than display suffix/name; disabled-child uncertain operations remain recoverable; source/listener bindings retain sharing |
| Cancellation/crash recovery | Before/after admission, signed journal, promotion, repair, quote reservation, funding confirmation and retirement; accepted work retains ownership/exact bytes; no replay or orphan outbox |
| Funding limits | Combined tiers, UTC rollover, refunds, immutable accepted outputs, unsigned outages and durable attempts; zero-new-funding checked before setup and every supervised reopen |
| Scheduling | Existing turn/backoff scheduler advances independent eligible jobs after an unsigned failure; uncertain treasury mutation preserves its wider stop; no new per-tier scheduler |
| Backup/status/evidence | New schema restored in isolated fixture; partial ready capacity accurate, disabled/retired capacity excluded, recorded retirement totals distinct from current holdings; historical reports readable and old started runs observation-only |
| Performance and stalls | Measure RPC count and admission latency with default/max drainers in direct/local SOCKS fixtures; validate finite count ceiling and freshness; long historical progress, true stalls, unrelated concurrent progress and cancelled waiters preserve existing policy |

Extend `tests/rotation.rs`, `tests/managed_config.rs`, `tests/payments.rs`,
`tests/network.rs`, `tests/wallet_workflow.rs`, `tests/integration_preparation.rs`,
and adjacent Base/store/funding unit suites. Include existing store integrity and
funding-restriction fixtures for migration and authority boundaries. Shared live
wiring remains in `examples/live_integration/driver.rs`.

Before implementation, pin config/help fixtures and table-driven state transitions
covering both replacement paths, blocked maintenance and removal/re-enable. Before
shipping selection, validate scoped evidence and the shared-gate race tests. Before
enabling large tiers, validate POST/relay bindings and combined budgets. Do not mark
performance validation complete merely because a functional fixture passed: record
candidate counts, RPC counts, observed latency and freshness outcomes, and choose
the supported count ceiling from those results. Local Tor/SOCKS fixtures establish
no live privacy or settlement qualification.

Run focused default-Zcash suites for each change, then `scripts/check.sh` for the
repository's required default-build validation. Do not run no-default-feature suites
without a separate request. Validate documentation links/examples/help after final
names settle. Offline fixtures cannot qualify real settlement, funded crash recovery
or privacy. Live-funded work requires separate operator authority and explicitly
versioned tier/pool/wallet/job scope with cumulative charges. Preserve existing
deferred acceptance limits and old runs' observation-only status.

## Delivery sequence

1. Pin configuration/CLI names and fixtures, routing/cap formulas, initial bindings,
   count-validation method and both replacement transitions. Document deliberate
   legacy availability changes and high-cost POST limitations.
2. Implement membership/role migration, exact versus conservative evidence and the
   shared profile gate. Prove legacy identities/history and reconciliation ownership.
3. Implement generic admission, bounded scoped candidate selection and mutation-free
   handoff before financial transitions. Cover cheap POST and outer relay behavior.
4. Implement ordinary-tier retention, single pressure-retirement limit, durable
   accounting, normal promotion and spent-secondary repair. Prove progress after
   obstructions clear without repeated allocation or a mutable pause flag.
5. Add the large child, current-authority resolution, eager bootstrap, exact initial
   route bindings, cross-tier draining and removal/re-enable handling through the
   same admission/maintenance/funding paths.
6. Add explicit manual drainer retirement, status/backup and versioned qualification
   evidence. Complete the documentation checklist, focused matrix and repository
   validation. Keep unfinished modes disabled and qualification claims bounded.
