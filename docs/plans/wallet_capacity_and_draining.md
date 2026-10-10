# One active wallet, sized reserves and retained balances

## Objective and status

Draft implementation plan; no feature or funded execution is authorized by this
file. Replace the active/ready pair with one active wallet, a small reserve slot,
an optional large reserve slot, and retained wallets. One named managed profile
owns the group. Every wallet uses the same group per-payment cap. Reserve sizes
control replenishment, not spending authority or wallet classification.

Separate reserve sizes also serve an address-reuse objective for users with many cheap
calls and occasional expensive calls. Small-funded wallets can rotate more frequently
through cheap-call traffic without requiring every wallet to hold enough for a rare
expensive call. A uniformly large funding size could keep many more cheap calls on the
same payer address. Preserve both sizing options in delivery; they are not merely a
capital-allocation optimization. This is an opportunity to reduce linkability, not a
privacy guarantee: after a large reserve is consumed, sufficient-bound-payer stability
can keep cheap calls on that large-funded active wallet when hints or smaller usable
observations are missing; smallest-sufficient hint selection can instead use a smaller
wallet before binding. Reusing retained capacity can still link calls to older addresses. Funding and network observations can also correlate activity. Do not
claim strict cheap/expensive separation or introduce tier-specific payment paths.

The initial active allocation is small-funded. While it is unfinished, a ready reserve
may supply the unsigned default identity without changing roles or creating a vacancy.
Consuming a reserve through a journaled payment makes it active, retains the previous
active when present, and leaves a vacancy. Existing funding
machinery fills that vacancy under current authority and actual spendable treasury capacity.
Payment admission does not depend on retirement or replacement permission; it can
still wait for bounded resource access and fresh evidence. Retained funds remain
usable until explicitly permitted retirement. Fragmented balances can increase capital
requirements, but wallet count never obstructs replacement funding.

Favor a single new configuration and runtime model. The operator will update local
configuration and perform the stopped-service migration. Reject the replaced options
enumerated in the configuration section;
do not preserve target-based payment caps, legacy retirement-only execution, tier
membership, or separate small/large payment paths. Preserve all previously spendable
wallets, keys, balances, liabilities, accepted jobs and financial history. Configuration
migration never authorizes changing accepted transactions or deleting wallet state.

## Settled design and exclusions

- A group has at most one active pointer and named reserve slots `small` and optional `large`.
  A reserve slot is vacant, allocated with one owned funding job, or ready with one
  canonically funded wallet. A ready wallet has not journaled an application payment
  since entering that slot. Discarded private signatures do not consume it. Its actual
  balance still requires verification. The initial default identity may use a ready reserve when no funded active exists; that selection
  alone never consumes its slot. The active pointer may be absent during initial funding.
- All other funded, non-retired group wallets are retained and eligible to pay.
  Retained wallets never receive top-ups or automatically return to reserve slots.
  Payment from a retained wallet does not change the active pointer.
- Small/large labels belong only to slots. After a reserve wallet leaves its slot,
  its old slot size does not constrain selection, payment, retirement or retention.
  Original funding target, accepted quote and actual credit remain immutable history.
- One group cap applies to every payment. Each selected wallet must independently
  cover the payment after all unresolved liabilities. Summed balances cannot fund
  one authorization. Large-funded active/retained wallets may pay small calls.
- There is no live-wallet count ceiling. Bound acquired work, queues and caches rather
  than imposing another funding gate. One explicit retirement tolerance permits ordinary
  reconciliation to stop routine balance polling and payment selection for eligible old
  wallets in enabled groups independently of automatic funding. Retirement never enables allocation or
  supplies spending capacity. Retention is part of the new model, not an opt-in.
- Consuming a reserve creates the refill obligation. There are no provider-price
  demand records, promotion/repair amounts, pair-depletion predicates, pending-request
  queue, durable funding-pause flag or price-driven secondary repair.
- Keep existing group bindings, one treasury owner, the funding scheduler, quote and
  accounting journals, and the payment admission implementation. Add no tier scheduler,
  operation-to-tier configuration, nonce scheduler or parallel financial ledger.
- Migration serves one known operator/developer and one inspected source state. A
  one-time developer utility preserves keys and financial records; exceptional recovery
  is incident-specific custom work. No migration framework, abandonment command or
  evidence-assessment command is part of delivery. See the migration scope decision below.
- The manual retirement CLI is deferred; automatic retirement and its accounting remain
  in scope.
- Automatic clean partial detachment of incomplete externally credited allocations is
  deferred; preserve their claims and history across disablement.
- Sweeping, combining wallets, topping up old addresses, randomized targets and
  identity-preserving profile rename remain outside this version. Provider catalog
  curation is independent. Retention and shared groups do not establish unlinkability.

## Existing implementation and dependency boundaries

`src/rotation/store.rs` currently enforces one ACTIVE and one READY per pool, and
admission can retire active, promote ready and allocate a replacement before returning
`PayerChanged`. Replace that coupling; do not adapt it with another special-case payer.
`src/rotation/manager.rs` currently holds a pool gate through RPC reconciliation,
admission, signing and journaling. Move unsigned RPC collection outside the gate while
preserving owned local signing and one atomic admission/journal commit.

Keep the unsigned-lease ownership fix independently deliverable. Both `chain_query` and
`payment_chain_query` reach `chain_query_inner`, which deletes ADMITTED rows.
`FundingBackend::credit` calls that mutable path without the payment gate. Serializing
store calls alone does not protect the interval between admission and journaling.
Make query construction read-only and give abandoned unsigned-lease cleanup explicit
ownership. Cover actual funding credit, reconciliation, cancellation and restart.
This prerequisite needs no reserves, hints or new financial schema. It protects the
current two-step path until the atomic admission/journal path below replaces it. Ship it
first if the legacy path will remain deployed during that work. Keep cleanup within the
existing owner and stopped-service recovery boundaries; add no lease service, cleanup
scheduler or new schema. If atomic admission ships together with the read-only query fix,
test the original race at that boundary without building an intermediate cleanup subsystem.
The new runtime retains no unsigned-lease cleanup machinery.

`src/rotation/base.rs` currently retains only min(confirmed, latest). Preserve the
exact observations separately for retirement and provenance; the old minimum cannot
be relabelled as either exact value. Current complete-view assertions remain valid
for complete views; add explicitly scoped evidence rather than weakening them globally.

`src/payment.rs` binds identity before the unsigned request and currently permits one
GET/HEAD payer-change re-challenge. The new unsigned handoff applies to every supported
method. Keep x402 2.0.2 and exact accepted-payload validation. The SDK signs EIP-3009
with random nonces; it does not enforce our durable exposure, retirement or
funding policy. Selection's cap filter and authoritative store admission share one
calculation; SDK validation remains a separate protocol check.

Funding uses the local 1Click adapter in `src/rotation/near.rs`, not the Omni Bridge
SDK in `reference_repos/Near-One`. Cargo pins and reviewed vendor patches govern
Zingolib. Keep proposal ownership, treasury sync/readiness, canonical settlement,
refunds, source reservations, fee safeguards and submission margins with their existing
owners. Remove aggregate funding-budget policy as specified below; preserve its financial
history, not its spending ceilings. This feature establishes no new external settlement contract.

See [wallet rotation](../wallet-rotation.md), [architecture](../architecture.md),
[network policy](../network-egress.md) and [qualification limits](../testing.md#qualification-status).

## Normative contracts and implementation scope

Each contract has one authoritative location. Lifecycle prose explains its use;
validation rows specify fixtures and assertions; implementation and documentation rows
assign work. Those later sections must reference these contracts rather than define
additional transitions or authority.

| Contract | Authoritative location |
| --- | --- |
| Configuration, funding capital and sizing | [Configuration and authority](#configuration-and-authority) |
| Operation lifetime, shared gate and signing ownership | [State and ownership](#state-and-ownership) and [atomic admission/journaling](#atomic-admission-and-signed-journaling) |
| Foreground acquisition and admission recovery allowances | [Admission recovery transitions](#admission-recovery-transitions) |
| Destination-credit threshold and full/partial disposition | [Applicable destination requirement](#applicable-destination-requirement) |
| Payer selection, hints and request attempts | [Initial selection](#initial-selection-price-hints-and-admission) and [unsigned handoff](#unsigned-handoff-for-every-supported-method) |
| Selection/isolation decision and considered alternatives | [Selection decision and alternatives](#selection-decision-and-alternatives) |
| Evidence acceptance, failure scope and retirement eligibility | [Evidence and resource contract](#evidence-and-resource-contract); its failure table owns failure scope and its observation table owns ordering/timing cases |
| Canonical compatibility and persisted support | [Linear checkpoint verification](#linear-checkpoint-verification) and [durable evidence provenance](#durable-evidence-provenance) |
| Wallet roles and slot claims | [Reserve consumption and refill](#reserve-consumption-and-refill), with [disabled allocations](#disabled-allocations-and-partial-credits) preconditions |
| Financial completion, source accounting and provider outcomes | [Completion predicates](#financial-completion-and-source-accounting-predicates) and [phase dispatch](#funding-phase-dispatch) |
| Retirement cadence and allocation order | [Retirement](#retirement-during-reconciliation) and [vacancy allocation](#independent-vacancy-allocation) |
| Startup and configuration boundaries | [Startup](#startup-configuration-changes-and-recovery) and [current funding policy](#funding-terms-and-current-policy) |
| Safe closure versus authority for another attempt/fill | [Same-job recovery](#disabled-slots-and-same-job-recovery) |

Keep the lease prerequisite independently deliverable. Extend existing owners and guarded
transitions before introducing another lifecycle abstraction. Smallest-sufficient runtime hint selection
and sufficient-bound-payer stability are part of delivery under the selection contract.

## Configuration and authority

Use the existing logical profile name and listener/source binding rules. Dynamic
sources and discovery relays resolve the same effective group as ordinary tools.
Agent arguments and imported metadata cannot select a wallet, reserve, cap or funding
permission. Static-wallet configuration is outside this managed-profile redesign.

The proposed managed shape is:

```toml
[wallets.research]
mode = "zcash_rotation"
max_api_payment_usdc = "10.00" # Required explicit group per-payment authority.
max_retirement_remainder_usdc = "0" # Default: automatically retire only empty wallets.
max_conversion_overhead_percent = 5
max_attempts = 3
# max_funding_spend_zec = "0.02" # Existing optional group safeguard.

[wallets.research.reserves.small]
funding_amount_usdc = "2.00"
# max_funding_amount_usdc = "2.00" # Omitted means no increase above target.

[wallets.research.reserves.large]
funding_amount_usdc = "10.00"
# max_funding_amount_usdc = "10.00"
```

Pin this shape with parser/help fixtures before implementation. `small` is required;
`large` is optional and, when present, has a target at least as large as small. Equal
targets are valid. Payment selection uses available capacity, not configured targets;
its tie rules are defined in the selection contract. Vacancy allocation remains
small-first, and unsigned startup fallback retains its target-based order. These names describe reserve sizing, not
payment classes. The initial active uses the small slot's current target/output ceiling. There is no
independent active funding-size option. Slot output ceilings constrain accepted
funding quotes and never increase payment authority.

Require an explicit `max_api_payment_usdc` for managed groups, preserving the existing
explicit unlimited representation. Remove `limit_payments_to_funding_target` and
reject it. Reject the old top-level funding target/output ceiling and new-plan tier/
retention spellings rather than interpreting aliases. The operator chooses the cap
when updating configuration; report that the old target-based ceiling is gone. Do
not silently infer a larger cap from reserve sizes. Existing static cap semantics stay
unchanged. The rejected existing fields are managed-profile top-level
`funding_amount_usdc`, `max_funding_amount_usdc`, `limit_payments_to_funding_target`,
and the three aggregate-budget fields listed below. Reject unimplemented tier/retention
aliases; the exact new fields shown here are the supported syntax. Keep unrelated
compatibility behavior, including the currently ignored/warned `wait_seconds`, outside
this parser change. Nested reserve funding fields remain supported.

There is no `max_live_wallets` option or count-based allocation gate. Reject that
unimplemented spelling rather than retaining an ignored setting. Fresh inventory has
two wallets without large and three with large; retained inventory may grow over time.
Bound runtime memory and outstanding I/O with indexed pagination, bounded caches, queued
callers and concurrency under the evidence/resource contract. Do not load or refresh all
wallets eagerly, promise a fixed refresh interval for every retained wallet, or substitute
an implicit wallet-count limit. Actual spendable treasury capacity and existing funding
authority govern replenishment. Inventory counts are descriptive only.

`max_retirement_remainder_usdc` defaults to "0"; positive automatic retirement requires
an explicit configured tolerance. Parse
bounded nonnegative integer micro-USDC; positive-only funding parsers are unsuitable.
No automatic tolerance scales with price, target, wallet age or original slot.
There is no cumulative retirement-loss allowance; report cumulative recorded amounts.
A finite per-retirement tolerance does not bound lifetime stranded capital.
The tolerance authorizes automatic retirement of eligible retained wallets during
ordinary reconciliation of enabled groups, independently of `auto_fund`, inventory size,
vacancies or available treasury funds. The explicit tolerance supplies retirement authority;
funding permission supplies no additional retirement gate. Qualification restrictions
remain authoritative. Retirement stops routine balance polling
and payment selection for that wallet, reducing ongoing observation work. Positive
tolerance explicitly trades its remaining purchasing power for that reduction; sub-cent
balances are not inherently unusable. No measured accumulation rate is assumed.
Retired funds remain at their addresses with keys/history kept; retirement performs no
transfer, never releases financial obligations and never enables a refill. Zero remains
the default. Disabling automatic retirement cannot block otherwise authorized funding.

The funding-capital limit is the Zcash treasury's actual available spendable funds,
not a configured cumulative or daily allocation allowance. Remove
`daily_funding_limit_usdc`, `total_funding_limit_usdc` and
`daily_treasury_spend_limit_zec`, including the requirement to configure at least one
aggregate budget. Reject removed fields with an actionable migration error; do not
silently keep enforcing them, ignore configured limits, or replace them with another
aggregate ceiling. Update the maintained documentation and tracked examples on delivery.
Adding spendable ZEC restores funding capacity without increasing a configured budget;
UTC rollover and historical gross allocations do not create or remove funding authority.

Available funds means funds usable by the production treasury owner after confirmation,
existing reservations/liabilities and transaction fees, not the displayed total ZEC
balance. Reuse its read-only readiness/preparation and atomic source-reservation checks;
add no balance estimator or allocation-time budget gate. Keep explicit auto-funding and
qualification permissions, group API caps, per-transfer `max_funding_spend_zec`, reserve
output ceilings, fee/overhead safeguards and recovery limits. These govern individual
operations or authorized runs, not a lifetime/daily funding allowance. Preserve old
accounting and exact reservations even though aggregate budget enforcement is removed.
Both slots and the initial active draw from the same treasury.
Initial requested USDC is `2*S + L`, or `2*S` without large, before accepted bridge
minimum increases. With $2/$10 targets it is $14. For comparison, two independently
configured active/ready pairs at $2 and $10 would request $24; a single existing $2
pair requests $4. The $24 comparison is not the baseline for every current deployment. Report
configured maximum output separately from target, actual credit and conversion costs.

A cap above every target is valid: existing verified capacity might cover it after
an accepted larger credit. It does not cause larger replacement requests. Targets
above the cap are also valid capacity choices. Do not introduce rejection gates for
these arrangements or promise that configured funding can serve every allowed price.

### Recommended reserve sizing

Size the large reserve to at least twice the largest expected payment; prefer four
times that payment when prices vary and reducing stranded balances matters. Equivalently,
recommend payments at most 50% of the large funding target, preferably 25%. Larger
reserves can also keep more calls on one address after consumption; weigh that address
reuse against the capacity benefit rather than treating larger as always better. These are
sizing recommendations, not new configuration fields, parser requirements, warnings on
individual calls or admission restrictions. The explicit group cap and the selected
wallet's verified available capacity remain authoritative, even above these ratios.

For a wallet initially funded to target `L`, repeated identical settled payments `p`
leave `L - floor(L/p)*p` when the next payment no longer fits. Smaller prices do not
reduce that remainder monotonically: a $10 wallet leaves zero after two $5 calls,
$2 after two $4 calls, and $3.32 after two $3.34 calls. Outside the recommended sizing,
a $6 call leaves $4; that stress case illustrates why larger reserves are recommended.

| Maximum expected payment / large target | Repeated identical payments: stranded fraction | Mixed prices: remainder insufficient for the next payment |
| --- | --- | --- |
| 50% | Less than 33⅓% | Less than 50% |
| 33⅓% (exactly one third) | Less than 25% | Less than 33⅓% |
| 25% | Less than 20% | Less than 25% |
| 10% | Less than 9.1% | Less than 10% |

These bounds assume settled payments, no other liabilities or unexpected credits, and
replacement because the next payment exceeds the remaining balance. They describe one
wallet's remainder, not total deployment holdings or capital allocated ahead of demand.
Pending authorizations can cause earlier reserve consumption. Use actual starting credit
when evaluating measured results; accepted bridge minimum increases can differ from targets.
For predictable $6 calls, $12 and $24 large targets fit two and four calls respectively.
Larger targets deploy more capital per wallet and require sufficient spendable treasury
funds for the accepted ZEC input and fee.
No positive ratio eliminates fragmentation for every workload; retained use by smaller
calls remains valuable. Additional fills may consume the remaining spendable treasury
capital even while older wallets collectively hold substantial USDC. Wallet count alone
never stops an authorized fill.

Choose small sizing with expected cheap-call address reuse in mind, alongside large
sizing for expensive-call capacity. Do not present a uniform large target as an equivalent
privacy choice. The selector still permits cross-size use: these settings create rotation
opportunities, not separate identities for price classes or a bound on linkage. Preserve
that distinction in examples and in any later utilization or privacy claims.

For a mixed-price example within the preferred 25% threshold, a $10 large wallet can
serve three settled pairs of $2.25 then $0.50, spending $8.25 and leaving $1.75. The
next $2.25 call requires another wallet. Without usable hints, the fallback selector keeps a
sufficient active payer, so subsequent $0.50 calls also use that new active wallet even
though the retained $1.75 could cover them. Recommended sizing reduces the remainder;
it does not guarantee that smaller calls drain retained wallets. With a usable $0.50
runtime hint and fresh saved observations, initial selection chooses the smallest
sufficient available capacity across all eligible roles before any provider request.
The retained $1.75 wins when no other sufficient wallet has less available capacity. The actual challenge and fresh admission evidence still
govern payment. Cold, expired or inaccurate hints can leave the fallback limitation;
retained capacity is also usable when the bound payer cannot cover the actual challenge.
Keep payer stability after binding and test both learned draining and this fallback case.

## State and ownership

Reuse each existing profile's ordinary pool/group identity. Add reserve-slot records,
not child pools. Preserve wallet IDs, addresses, key encryption context, sequences,
network identity domains and exact operation correlations. Group names resolve
configuration; explicit stored ownership governs recovery. Profile rename is not
adoption of the old group's funds or jobs.

| Object | Meaning and invalidation scope |
| --- | --- |
| Group active pointer | Funded active wallet, initially supplied by a small allocation; may be absent during initialization. Unsigned fallback identity is derived from eligible funded wallets without consuming reserves; pointer changes do not invalidate captured payment identities. |
| Reserve slot | Named funding destination with current configuration and a monotonically advancing vacancy/claim generation |
| Slot generation | Owns one vacancy and at most one allocation/job; unrelated payments cannot advance it |
| Wallet ID and group ownership | Stable identity for keys, liabilities, transport and history; never a funding-size classification |
| Funding job/operation ID | Owns exact accepted terms and recovery history independently of present slot or active status |
| Operation revision | Source-local identity for invocation/hints; unrelated catalog publication is not financial invalidation |
| Wallet evidence sequence | Publication order, not an unconditional rejection gate |
| Canonical checkpoint | Compact verified confirmed history per wallet/asset/network; independent of balance freshness and minimum balance height; no shared group checkpoint |
| Debit watermark | Greatest compatible applied USED-resolution anchor requiring a balance at or beyond it; EXPIRED_UNUSED proofs retain separate exact canonical records without raising this balance floor |

Derive serving/inventory readiness, treasury-capacity waiting, retirement enablement
and funding-policy obstruction from current configuration, claims, jobs and evidence.
Do not persist parallel readiness flags or a second lifecycle ledger. Persist ownership,
accepted financial facts, required history checkpoints and explicit integrity blocks.

Use one group gate for local signing and atomic admission/journaling, role/slot
transitions, applying evidence, retirement commits and separate allocation commits.
Query construction is read-only. Accepted store-worker jobs survive waiter cancellation.
Never hold the gate across provider HTTP, Base RPC acquisition, re-challenge or funding
completion waits. Per-wallet checkpoints add no per-wallet lock hierarchy. Shared endpoint invalidation
and its authoritative checks use the serialized store owner across groups, as specified
in the [evidence contract](#durable-evidence-provenance).

Enforce one gate per store/group through the shared financial runtime's group factory.
`ManagedPool::new` currently creates its own mutex; restrict construction so two handles
for the same group cannot create independent gates. Serving, funding-credit publication,
background history, startup/warm and supported library callers obtain the same group
owner. Test independently requested handles, not only clones of one handle.

### Atomic admission and signed journaling

The managed path uses a local `PrivateKeySigner`. The pinned SDK constructs and signs
EIP-3009 payloads locally; signing itself neither transmits an authorization nor reserves
funds. Keep that boundary explicit. Remote signers or a signer that publishes payloads
would require a separate ownership review and are outside this design.

Parse and validate the challenge once, preserving the exact selected requirements and
the existing extension/description handling. Use the pinned SDK's
`X402SchemeClient::accept` and `PaymentCandidate::sign()` for the selected local signer,
including any permitted unpublished replacement. Keep exact accepted-payload validation.
Do not synthesize another HTTP response to re-sign or duplicate EIP-712/nonce construction.

The supported exact EIP-3009 requirements supply `maxTimeoutSeconds`; the SDK derives
`validBefore` from signing time plus that interval. This is the signed authorization's
lifetime, not an absolute deadline measured from challenge receipt. Preserve actual
caller/qualification expiry at its existing owner. Opaque challenge annotations grant
no authority and create no new deadline. A future absolute challenge deadline requires
an explicitly supported protocol field; add no generalized deadline subsystem here.

After acquiring unsigned evidence, perform these steps under the existing group owner:

1. Acquire the group gate, select the payer from completed evidence and provisionally
   validate its authority/capacity. If a different identity needs an unsigned handoff,
   return it without signing or mutation and release the gate before provider I/O.
2. Obtain usable same-payer evidence under the [admission recovery transitions](#admission-recovery-transitions),
   releasing the gate for RPC and rechecking all predicates after reacquiring it.
   Then read the exact owned wallet key without creating an unsigned reservation. Keep the
   role unchanged. Sign locally and validate the exact accepted payload. The owner
   retains the signed bytes privately; do not log, return, persist for dispatch or expose
   them to transport before the transaction below is acknowledged.
3. In one store transaction, authoritatively recheck current wallet/group eligibility,
   cap, exact signed terms and validity, compatible fresh evidence and all liabilities.
   Insert the `POSSIBLY_SUBMITTED` journal, add exposure once, assign its reconciliation
   sequence and persist supporting evidence for audit, without advancing the financial
   checkpoint merely for adding a fully reserved liability. If this
   payer occupies a ready reserve, consume it and change roles in the same transaction.
   Reuse the selection arithmetic, but only this transaction grants admission.
4. Dispatch only after successful commit acknowledgement. Apply the
   [admission recovery transitions](#admission-recovery-transitions) to a definite rejection;
   an absent acknowledgement is never proof of rollback. Unknown commit outcomes
   prohibit retry, replacement signing and dispatch and preserve any committed liability.

The safety boundary is unpublished bytes plus proven rollback, not a count of local
cryptographic operations. Use only discard-and-regenerate for the bounded retry path;
do not add retained-signature reuse, a signature cache or selectable signing policy.
After a successful commit or
an uncertain outcome, preserve the existing no-replacement/no-replay contract.

The gate remains held from final pre-signing validation through commit acknowledgement.
Same-wallet checkpoint, debit-watermark, liability and role mutations therefore cannot
race that interval when all writers obey the ownership contract. Keep the authoritative
transaction predicates: an unexpected state mismatch fails without dispatch or retry and
reports an ownership/integrity failure for investigation. Do not turn it into an ordinary
refresh path. Compatibility races during RPC outside the gate retain the pre-signing
refresh rule. Time can advance while signing or waiting for the store, so evidence and
signed-authority expiry checks remain necessary. Verify this exclusion through every
writer before removing any existing recovery branch during implementation; abnormal
owner failure still follows the conservative lost-acknowledgement contract.
Endpoint-wide invalidation is serialized by the shared store owner, independently of
group gates; its authoritative commit check can reject evidence even while this group
gate remains held. That rejection follows the evidence failure contract, not expiry recovery.

The new runtime creates no `ADMITTED` rows, unsigned payment leases, cleanup queue or
lease-specific group fence. Cancellation/signing failure before transaction acceptance
has no durable payment effect. After acceptance, the existing store owner finishes the
whole transaction or rolls it back. A lost acknowledgement or owner failure prevents
transmission and preserves any committed exposure/vacancy; absence of a reply is not
proof of rollback. Conditional transaction predicates and store serialization protect
against a late accepted job even if an abnormal owner exit drops the group guard.
Subsequent admissions see either the complete prior commit or its absence. Do not split
admission, journaling, evidence or role changes into separately accepted jobs.

Keep an accepted owner through signing, commit acknowledgement and permitted dispatch.
Reuse existing accepted-work tracking for MCP calls, startup relay discovery, catalog
warming and supported direct library callers. Register ownership before a commit can be
enqueued; close new operation admission and drain accepted jobs before releasing the
store or financial prerequisites. Listener tracking alone does not cover startup/warm.
Do not add a nested whole-call supervisor where the caller already supplies that ownership.
Normal completion retains the gate through the commit result, then releases it before HTTP.

Distinguish response delivery from operation ownership. A disconnected response waiter
or MCP response cancellation does not cancel accepted work. Inventory actual supported
operation-owner cancellation sources; add no user-facing cancellation API for this feature.

| Event | Operation and transmission behavior |
| --- | --- |
| Response disconnect or MCP response cancellation | Accepted work continues under its owner, including permitted payment/result processing; lost delivery grants no retry. |
| Explicit operation-owner cancellation before dispatch commitment | Suppress dispatch. Discard unpublished bytes if no commit was accepted; otherwise drain the accepted transaction and retain any committed liability. |
| Operation-owner cancellation after dispatch commitment | Treat transmission as possibly started and preserve existing completion/reconciliation ownership. |
| Graceful serving shutdown | Close admission of new operations, then drain accepted work while financial workers remain available. |
| Owner failure or lost commit acknowledgement | No dispatch or replacement signature; drain accepted store work, preserve a committed journal and its atomic role changes. No unsigned-lease cleanup or speculative group fence. |
| Forced process exit | SQLite transaction recovery retains either the whole signed journal/role transition or none of it; preserve committed liabilities and funding claims. |

Only where explicit operation-owner cancellation is supported, serialize it with the
commitment to dispatch through one small operation-local transition. A token check then
`http.execute` is not atomic. Apply this rule to original unsigned requests, handoffs and
signed submission at those entry points. Keep the existing submission marker monotonic;
mark possible submission no later than enqueueing the signed journal, preserving the
relay's conservative no-paid-retry boundary even if acknowledgement is lost. Do not add
dispatch wrappers or cancellation states to already-owned MCP calls solely for tests.

Test signing failure, failed predicates, queued commit cancellation, panic, lost completion
and process exit on both sides of the atomic transaction. Assert no signed bytes escape
before acknowledgement and no partial exposure/role/evidence update survives rollback.
If shipped separately, the narrow lease fix protects the old `ADMITTED` path until replacement.
Conversion handles any legacy unsigned rows only under exclusive stopped-service
ownership, preserving signed rows; the new runtime has no ongoing unsigned cleanup path.

Validate the selected wallet's current eligible role, group and authority rather
than insisting it still has the role seen by initial selection. A wallet moved from
ready to active, or active to retained, can still pay with fresh sufficient capacity.
Retirement or group disablement makes it ineligible. The selected address must remain
unchanged throughout its challenge/signature/transport context.

### Admission recovery transitions

This is the authoritative foreground acquisition/recovery table. One existing operation
owner carries the state through selection, handoff, signing and commit; add no nested
retry owner. Candidate traversal and the single unsigned handoff retain their selection
contracts. Track the invocation's one evidence-race refresh separately from its one
recovery commit: the former bounds repeated RPC acquisition caused by admission races,
the latter repeated local signing/store work after expiry. Neither limits spending or
renews caller/qualification authority. Handoff resets neither allowance. Do not layer
another counter, whole-call deadline or retry loop around them.

| Operation state or result | Permitted transition and allowance |
| --- | --- |
| First validated unsigned challenge | Use completed usable evidence or conduct the one captured candidate pass under the selection contract. This ordinary acquisition does not consume the evidence-race refresh. Successful gated selection ends the pass. |
| Second validated unsigned challenge after the one handoff | Keep the selected payer. Use compatible fresh evidence or acquire one complete fixed-payer observation, including when the unsigned response outlasted prior evidence. This ordinary acquisition does not consume or reset the evidence-race refresh; it starts no candidate search. |
| After ordinary acquisition, evidence expiry, a concurrent USED debit-watermark advance or missing checkpoint coverage is the only obstruction | First use compatible evidence already available. Otherwise release the gate and consume the invocation's one evidence-race refresh: obtain a complete balance for expiry/debit, or an anchor-only compatibility recheck for missing coverage, preserving original balance lifetime. Reacquire the gate and recheck all predicates. This may happen before signing or after the definite expiry rollback below. |
| Evidence-race refresh already consumed, or its acquisition fails | Independently usable compatible evidence may still be used; do not acquire again for the race. Without usable evidence, fail before signed dispatch. A failed acquisition alone does not invalidate an independently usable observation. |
| Initial commit definitely rolled back solely for balance-evidence expiry, private-signature expiry, or both | If current authority and usable evidence permit, consume the sole recovery commit for the same payer, amount and exact accepted requirements. Any needed evidence acquisition must fit the remaining evidence-race allowance. Prove the old bytes never escaped, discard them even if still valid, and sign a replacement with the SDK's fresh nonce and validity. Recheck every predicate in the new atomic transaction. Local replacement neither consumes nor replenishes the RPC allowance. |
| Other definite commit rejection, rejected recovery commit, expired caller/qualification authority, insufficient capacity, integrity failure or lost eligibility | No recovery commit or extra acquisition from this table; fail without dispatch. Pre-selection contention may continue the original candidate pass under its existing rules, not restart it. |
| Commit acknowledged successful | Release the group gate and permit the one signed submission under operation ownership. No replacement or replay. |
| Commit outcome unknown or acknowledgement lost | No dispatch, replacement signing or retry. Drain accepted store work and preserve any committed liability/role transition. |

A definite rollback means no journal, exposure, supporting evidence or role change from
that transaction committed. Private-signature expiry concerns only the unpublished
signature's validity, never caller authority. Acquisition and compatibility verification
use existing shared RPC bounds and finite configured-endpoint fallback; fallback is part
of that acquisition, not another admission-recovery allowance, and restarts verification
as a unit. No result permits provider renegotiation beyond the one unsigned handoff,
funding from a challenge, or reservation against a pre-debit balance after USED release.
The [verifier design](#verifier-implementation-design-and-fixtures) supplies compatibility
checks; this table alone owns foreground recovery allowances.

## Initial selection, price hints and admission

Without a usable runtime hint, bind the unsigned request to active, or to the
read-only startup fallback identity when active is absent. An existing
funded eligible wallet may obtain a free response or challenge during a Base outage;
fresh balance RPC is not a prerequisite for the original unsigned request. After
migration, a default identity is resolved from owned funded state as specified
below; absence of any funded identity is typed unavailability, not permission to
allocate on a provider challenge.

Use one smallest-sufficient ordering across eligible active, retained and ready wallets.
Available capacity is the saved verified conservative balance minus all current unresolved
liabilities, using the same arithmetic as admission. Exclude expired, unknown or otherwise
unusable observations. Rank sufficient wallets by available capacity ascending; only for
equal capacity prefer active, then retained, then ready. Break remaining retained ties by
wallet ID and ready ties by small-before-large slot order, then wallet ID. Slot targets,
original funding size and total balance before liabilities do not determine payment rank.

With a usable runtime hint, apply that ordering to already available evidence before
binding the unsigned request. If none appears sufficient, use the default identity.
Hints and initial selection grant no admission authority, reserve no funds and change
no roles. Reuse this ordering for challenge-time replacement selection below; bound-payer
stability is its only payment-selection exception.

Initial hint selection reads only already available runtime prices and balance evidence;
it starts no pricing probe or foreground balance query and never waits for maintenance.
Keep eligible wallet observations available through the existing bounded balance owner.
After the actual challenge, consult all completed usable candidate evidence before
selecting a replacement, without waiting for an unfinished smaller-wallet query. A wrong
hint is not itself an agent-visible error: ordinary challenge-time selection may use the existing one
unsigned handoff when the bound wallet cannot pay and another candidate can.

For the actual valid unsigned challenge, use one bounded candidate-acquisition pass.
The [admission recovery table](#admission-recovery-transitions) defines fixed-payer
acquisition after handoff and the evidence-race allowance; neither starts another candidate search.

Capture an immutable wallet-sequence upper boundary at pass start and traverse it by
stable keyset pages. New allocations join later invocations/maintenance, not this pass.
Recheck current eligibility and liabilities before use. Page size and shared concurrency
bound acquired work, not total pass cost: an unsuccessful pass may inspect the entire
captured inventory. Do not describe that as fixed latency or add a wallet-count ceiling
or whole-call deadline. Stop only when a sufficient candidate is selected under the group
gate. A provisional candidate losing capacity before that selection does not end the pass:
continue its unvisited candidates and still-owned in-flight reads within the original
boundary. Do not restart traversal or reacquire evaluated candidates to seek a favorable result.

Within that captured boundary, when usable observations are missing, give the bound payer and configured ready reserves
prompt acquisition opportunities alongside retained traversal. These few identities must
not sit behind the retained keyset cursor. Deduplicate them against each other, traversal
and already-owned reads. Use the existing foreground owner and shared RPC bounds; reserve
acquisition must not depend on an earlier payer or reserve query completing when permits
are available. Retained traversal still receives execution opportunities. Acquisition order
does not change the selection ordering among completed observations or guarantee progress
when all applicable permits are occupied.

Distinguish no sufficient wallet in completed observations from incomplete/unavailable
acquisition and from a completed captured-inventory pass with no individually sufficient
wallet. A negative pass is complete only when each eligible captured candidate was evaluated
with usable evidence at its evaluation time; failed, cancelled, unvisited or already-stale
observations leave it incomplete. Do not require every result to remain fresh until the
end of a long pass or restart traversal to achieve a simultaneous inventory snapshot.
Results describe that pass, not present global capacity or a promise about later arrivals.
An incomplete pass must not report global insufficient funds. If a future resource
policy stops acquisition early, report incomplete acquisition and justify its distinct
resource boundary rather than disguising it as a spending or inventory limit.

1. Validate supported exact Base USDC offers for this resource and select the lowest
   amount within the group cap, breaking equal-amount ties by challenge order. Preserve
   that offer's exact accepted terms; never combine fields from different offers.
   This balance-independent choice avoids rejecting an affordable later offer merely
   because an earlier one costs more. It needs no extra probes or offer-search loop.
2. Consider already complete, still-fresh verified observations, and acquire missing
   candidate evidence under shared RPC limits. Keep the bound payer when its completed
   evidence is sufficient. Otherwise apply the shared smallest-sufficient ordering to
   completed eligible candidates using the actual selected offer amount. Best fit is
   opportunistic among usable completed observations, not a proof about every wallet.
   Do not wait for an unfinished bound-payer query when another completed candidate can pay.
3. Proceed to gated selection as soon as a sufficient candidate completes; do not wait
   for an unfinished smaller-wallet or bound-payer query. Keep the pass owned until that
   selection succeeds or acquisition finishes.
4. Under the group gate, select again from the already completed compatible observations
   using current roles, cap and liabilities. Keep the sufficient bound payer; otherwise
   apply the shared smallest-sufficient ordering across all completed eligible candidates.
   Recompute available capacities under the gate, including liabilities added since
   initial selection. The bound-payer exception applies at this final selection too;
   a sufficient bound payer never hands off solely for better utilization or address reuse.
   A candidate losing capacity to a concurrent reservation must not hide another completed payable
   candidate. If the bound payer wins, retain gate ownership through local signing and the
   atomic admission/journal transaction specified above; selection creates no reservation.
   Otherwise return its selected replacement identity for the one unsigned handoff, without a
   reservation or role change, and release the gate before provider I/O. After handoff,
   admission considers only that identity; intervening contention may then fail before signed submission.
   If contention leaves no completed sufficient candidate before handoff/signing, release
   the gate and continue the same captured pass as above. Successful gated selection
   ends candidate acquisition: cancel unused caller-owned reads without waiting for EOF,
   retaining independently owned reconciliation. Exhaustion or unavailable evidence uses
   the pass-result distinctions above and creates no demand record, role change, retirement
   or job. This continuation grants no post-handoff search or additional provider request.

Smallest-sufficient initial hint selection can drain a retained remainder or select a
small ready reserve even when a larger active/retained wallet could pay. Selection alone
creates no vacancy; only a journaled reserve payment consumes it and permits ordinary
refill. Equal-capacity role ties avoid consuming a reserve when an equally suitable
active or retained wallet exists. After a challenge, keeping a verified sufficient bound
payer avoids handoffs solely to improve utilization or address reuse. Cold or changing
prices can still require one affordability handoff. No utilization-only handoff, extra
pricing discovery or additional negotiation attempt is introduced.

For example, with $8 active, $2 small ready and $0.40 retained, a usable $0.10 hint
selects the retained remainder while it remains sufficient and smallest. Once that
remainder cannot pay, the small reserve wins over the $8 active. Its first journaled
payment makes it active and retains the $8 wallet. A later $6 call may use that retained
wallet without displacing the small active. Use actual capacity after liabilities in
this example and in implementation; a large-funded wallet with little available capacity
can still be the smallest fit. This is a capacity heuristic, not a privacy classification.

Choosing smaller reserves can increase refill activity while larger balances remain
usable. Preserve treasury, auto-funding and qualification checks for
refill; never add funding authority to selection or require refill permission to pay.
Separate this tradeoff from correctness and make no universal privacy or utilization claim.

The [reserve-sizing guidance](#recommended-reserve-sizing) explains the mixed-price
limitation with an example inside the preferred 25% threshold. Smaller calls relieve
fragmentation only when selection actually uses retained capacity; preserve that limit
in user-facing descriptions of cold, expired or inaccurate hint behavior.

The authoritative store operation checks the group cap and the selected wallet's
verified conservative balance minus all current unresolved liabilities, then reserves
and journals the already validated signature atomically. Share cap/capacity
arithmetic with selection, retaining the transactional check as the authority boundary.
SDK checks validate the protocol and cannot replace exposure accounting or exact
accepted-payload validation. Pending liabilities on other wallets cannot block
independently affordable admission. There is no raw-depletion prerequisite for using
a reserve: temporary reservations may cause earlier reserve consumption and refill.
Available spendable treasury funds and existing funding authority bound that policy; documentation must
state the potential for earlier funding while older liabilities remain unresolved.

### Runtime hints and trusted scope

Deliver one small bounded runtime hint cache with smallest-sufficient initial selection.
Learn only from normal invocation traffic. Correct hints let the chosen wallet pay with
the ordinary one unsigned request and one signed submission. No dedicated discovery,
refresh request, price preflight, extra handoff or retry is permitted for draining.
Do not add a user-facing toggle. Missing or unusable hints fall back to the default identity and the
ordinary bounded challenge-time selector; they never prevent a free response or grant
payment authority. Test cold fallback independently as well as actual learned selection.

Construct a private operation key at the runtime binding/invocation boundary, using
`BoundTool` source identity/revision, normalized method, exact OpenAPI path template,
effective group and applicable transport/network scope. Pass it through the routed
request context to `PaidClient`. `ToolSpec` remains a reusable catalog definition;
do not serialize deployment ownership into provider contracts. Static sources need
an explicit internal source identity too. Direct and fallback invocation share keys.
The relay's operation is its outer POST /curl; target-dependent prices are estimates.
Prices for different arguments of one operation may differ too. A route-level hint is
the last observed estimate, not a guaranteed quote. Do not add argument fingerprints,
provider-specific prediction or a second price-discovery system to improve accuracy.

Preserve the existing immutable-snapshot invocation contract. Source removal stops
new selection through published views; an already accepted invocation retains its
captured binding. It must still satisfy current wallet/payment authority. Revision
invalidation keeps old hints from new bindings and does not add a global catalog
recheck that revokes unrelated or already accepted calls. Late hints from old bindings
cannot overwrite a new revision's entries.

Keep one bounded in-memory runtime-observation cache alongside, but distinct from,
startup pricing in `src/pricing.rs`. Populate only from the actual selected, fully
validated unsigned 402 offer. Admission success is unnecessary. A second challenge
can update the amount, including downward. Malformed, unsupported or over-cap offers
teach nothing usable. Order updates by challenge receipt sequence, not admission
completion. Keep fixed-size scope identifiers, amount, receipt time and sequence;
store no bodies, arguments, URLs, credentials, challenges or payer histories.

Choose a documented finite initial TTL and entry limit, then exercise them in
focused resource and workload fixtures. Derive the memory bound from the fixed-size
entry representation and bounded container overhead; add no separate mutable byte counter.
Broad workload tuning is not a release gate;
hints grant no authority. Measure TTL
monotonically from receipt; reads, free responses and payment completion do not renew
it. Eviction/restart returns to default-identity selection and cannot cancel financial work.
No persistent price ledger, extra probe, parameter prediction or new agent option is
needed. Startup pricing stays one-shot display discovery with its existing HTTP cache.
Do not trigger or refresh startup pricing to populate runtime hints. Cache expiry or
eviction changes initial selection only; it never causes network traffic by itself.

Preserve the existing maximum of two unsigned requests and one signed submission.
An inaccurate hint may use the one affordability handoff that an active-first request
would not have needed; it creates no further allowance. Price changes or contention
after that handoff can still fail before signed submission. Do not claim identical failure probability
to active-first selection or guaranteed draining. Prefer existing fresh balance evidence
minus all current liabilities, and test incorrect-hint recovery within this fixed budget.

### Selection decision and alternatives

Decision: retain runtime price hints with wallet-scoped HTTP pooling, cover ownership
and Tor isolation. Prefer reuse of pooled connections, cover capability/episode state
and Tor circuits within the configured isolation boundary over choosing the exact
smallest fit on every cold or inaccurately predicted call. Reuse is an opportunity,
not a guarantee of circuit lifetime or unlinkability. Preserve the shared
[smallest-sufficient ordering](#initial-selection-price-hints-and-admission) before
binding and sufficient-bound-payer stability afterward. Actual challenge terms and
fresh financial evidence remain authoritative for admission; hints never authorize payment.

Late payer selection from the ordinary 402 response is technically feasible. Standard
v2 exact EIP-3009 requirements do not require selecting the signer before that response;
the pinned x402 2.0.2 client parses requirements and then selects/signs. The current
prebinding/re-challenge requirement belongs to this application's network ownership
contract, not to a universal x402 rule. `src/payment.rs` binds the unsigned request's
client and cover owner to the selected EVM address; `src/network.rs` uses that identity
for connection pooling and SOCKS isolation. Do not describe the alternatives as
protocol-impossible, or silently change those ownership boundaries in a future selector fix.

| Alternative | Benefit and request behavior | Decision and reason |
| --- | --- | --- |
| Selected: smallest-sufficient hints before binding, actual-price admission afterward | Normally one unsigned request and one signed submission; preserves wallet-scoped pooling, cover state and isolated Tor reuse. No dedicated price requests. | Keep. Missing/stale/inaccurate hints may produce a less suitable wallet or use the one affordability handoff. Accept that tradeoff without additional prediction machinery. |
| Choose a different signer after the challenge on the original wallet's pooled connection | Exact-price selection with the same two ordinary requests. | Reject. A signature from wallet B on wallet A's reused connection/cover identity can associate B with A's prior activity and violates the wallet-isolation contract. |
| Fresh invocation identity for both unsigned and signed requests; choose payer after the challenge | Exact-price selection with one unsigned request and one signed submission. Could remove runtime hints and affordability re-challenge logic while keeping one transport context for the negotiation. | Not selected. Loses cross-call wallet connection/circuit reuse and changes cover-owner lifetime, cache behavior and qualification evidence. The current bounded HTTP cache would churn; fresh cover owners would consume registry capacity and repeat capability qualification unless explicitly redesigned. Financial journal safeguards would still be required. |
| Fresh unsigned identity, then the selected wallet's transport for the signed request | Exact-price selection with no additional API request; retains pooling for signed wallet traffic. | Not selected. Adds an unsigned transport/cover lifecycle and switches network context between challenge and submission. Core payment fields do not establish provider session or transport portability. A rejection after possible signed submission cannot authorize a fallback payment or replay. |
| Hints plus a utilization-only handoff whenever the actual price reveals a smaller wallet | Improves fit after binding while keeping wallet-specific transports. | Reject. Adds unsigned provider requests merely to improve fit, increasing pressure on providers that rate-limit unsigned traffic. Keep the single affordability handoff only. |
| Extra pricing probes, preflights or hint-refresh requests | Could improve prediction before binding. | Reject. Adds rate-limited traffic and a discovery/refresh lifecycle. Learn runtime hints from ordinary invocation traffic only. |
| Skip the ordinary unsigned request using a cached estimate or previously observed challenge | Could reduce unsigned traffic below the ordinary one-per-invocation flow. | A display estimate is insufficient to sign. Reusable exact requirements would need a concrete provider contract covering terms, request/argument scope and validity; an old observed challenge alone supplies no such contract. This is separate future work, not a fallback here. It also removes the chance to discover a free response. No signed rejection permits automatic replacement or replay. |

Keep three caches/flows distinct. Catalog/help discovery retains its existing caching
and coalescing. Startup pricing remains one-shot and cached, with HTTP-supported disk
freshness for derived display estimates; payment challenges are not persisted as reusable
authority. Runtime hints learn from actual invocation challenges and only guide selection.
The current invocation path still makes an ordinary unsigned request even when a display
estimate is cached. Late selection would use that same response, not inherently add a
pricing request or invalidate discovery caches, and could remove the affordability
re-challenge. Its rejection here rests on transport/cover reuse and lifecycle cost.
The selected design also creates new identities over time; its reuse objective depends
on the HTTP and cover retention fixes in the [resource contract](#evidence-and-resource-contract).
Reclamation may require cover requalification and does not guarantee indefinite state retention.
Conversely, a fresh-identity design must account for repeated cover qualification rather
than claiming unchanged total unsigned traffic from API request counts alone.

Retain the small/large reserve design's mixed-cost address-reuse rationale from the
[objective](#objective-and-status) and [sizing guidance](#recommended-reserve-sizing).
A uniform large funding size is not an equivalent privacy choice; the selected heuristic
still permits cross-size use and does not guarantee price-class separation.

This is a deliberate design choice, not an unimplemented requirement to add another
selection mode. Revisit it only with an explicit change in priorities, a concrete provider
reuse contract, or evidence that warrants the transport/cover tradeoff. Such a revision
must assess request counts, pooling, owned cleanup, cover qualification and Tor isolation
evidence together; it cannot introduce signed replay. Transfer this decision and its
alternatives to maintained documentation on delivery as specified below.

### Unsigned handoff for every supported method

Assume the provider gates protected execution before issuing its unpaid challenge,
as the existing unsigned-then-paid POST flow already requires. This is not general
POST idempotency. A known violating provider needs an explicit compatibility decision.
The [x402 HTTP transport](https://github.com/x402-foundation/x402/blob/main/specs/transports-v2/http.md)
and [reference middleware](https://github.com/x402-foundation/x402/blob/main/typescript/packages/http/express/src/index.ts)
explain why submitted settlement failure cannot prove that application work was absent.
These moving links are explanatory only. Pin the middleware fixture and transport
contract to reviewed commits before implementation rather than importing future upstream
semantics. Count protected handler executions, not just HTTP requests. No general POST
idempotency subsystem is required.

A fully validated unsigned 402 with a clear submission marker permits one handoff
before signing. Bind the selected wallet/group/request context, rebuild identity-bound
HTTP and cover, and repeat the same method, destination, business headers and body
bytes once. Fail before I/O if the body cannot be reproduced. Preserve the production
no-automatic-redirect policy: a redirect response never dispatches to its Location.
Preserve public-destination restrictions, remote DNS and Tor isolation. Wallet transport and
cover metadata follow the new selected address.

A free second response succeeds without signing or consuming a reserve. Otherwise
validate the second challenge independently, update its hint, and admit only on that
selected wallet. A price increase beyond its capacity fails; a decrease needs no
wallet-class exception. There is no second candidate search or handoff. After the second
challenge, follow the fixed-payer acquisition row in the
[admission recovery table](#admission-recovery-transitions), including expiry during a
slow unsigned response. No new provider negotiation occurs.

After signed journaling there is no handoff, re-signing or replay, including seller
402, insufficient_funds, settlement failure or output-conversion failure. The relay
submission marker stays monotonic across negotiation; it cannot acquire a second paid
target attempt. Maximum: two unsigned requests and one signed submission.

## Evidence and resource contract

Separate historical authorization reconciliation from current balance evidence.
Unavailable nonce observations retain liabilities; they do not prevent spending
other independently verified capacity. Never release exposure solely on seller
responses, elapsed time or a missing result. Keep canonical used/expired-unused proofs.

Preserve complete per-wallet observations: exact confirmed/latest amounts and
height/hash anchors, conservative minimum, original block-derived validity, network/
endpoint scope and observation order. Admission uses the minimum; retirement uses the
maximum. Endpoint fallback restarts a complete scoped query, never mixes fragments.
Acquisition failures and contradictions use the distinct scopes in the failure table;
authenticated fallback never conceals demonstrated integrity failures. Partial observations never
satisfy a full-inventory qualification assertion.

Existing reconciliation owners publish independently completed wallet observations
without waiting for a slow nonce or unrelated wallet. Schedule history and balances
independently with separate failure/backoff status under the existing lifecycle.
History backoff up to 300 seconds must not suppress the balance owner's ordinary
five-second scheduling cadence or independent publication. Preserve default 120-second
evidence freshness. The cadence is not a promise to refresh every retained wallet every
five seconds. No new periodic refresh owner is needed. Retain useful historical progress
and prevent duplicate concurrent sweeps.

Enumerate retained wallets in indexed, bounded pages through the existing balance owner,
with runtime traversal progress and bounded in-flight work. Give active/reserve refreshes
and retained traversal execution opportunities; a growing retained population must not
materialize an unbounded task queue or postpone every retained wallet behind new arrivals.
Use a captured traversal boundary and stable wallet sequence, without a durable cursor
or a new wallet class. Retired wallets leave routine balance traversal. Independently
actionable financial obligations remain scheduled regardless of wallet role.
Keep only a bounded cache of complete observations/certificates; eviction removes an
optimization, never eligibility, keys, liabilities or funding authority. Initial hint
selection uses cached usable evidence. Foreground candidate acquisition traverses owned
wallets in bounded pages, starts no full-inventory fan-out and stops when a completed
sufficient candidate can be selected under the selection contract. A missing cache entry
alone cannot permanently exclude a wallet. At large inventory sizes some balances will
be stale or absent; reacquire needed evidence without weakening freshness. No fixed
all-wallet refresh or global best-fit guarantee is made.

RPC concurrency does not bound unresolved authorization history.
Read history in bounded, resumable batches rather than materializing every pending row.
Use stable keyset traversal and a captured pass boundary; arrivals join the next pass.
Assign an immutable, non-reused monotonic reconciliation sequence when an attempt is
journaled as POSSIBLY_SUBMITTED, atomically with that transition. Capture the committed
sequence upper bound at pass start and enumerate unresolved entries by sequence through
that bound, with an index supporting group/state/sequence lookup. UUIDs remain exact
attempt identities, never insertion-order cursors. A payment committed after
pass start belongs to the next pass even if its local signing began before the boundary.
Keep the cursor and captured upper bound in the runtime owner only. Reuse a suitable
existing immutable insertion key where available; otherwise add the sequence to the journal
and initialize legacy rows deterministically without claiming historical creation order.
New assignments must exceed existing values across reopen. This key is enumeration metadata,
not a durable traversal checkpoint, work queue or financial ledger.
Within the existing history owner, acquire nonce evidence through a bounded rolling set
of futures, and verify/commit completed proofs independently. Do not await an entire page
before starting later entries or committing completed proofs. A continuously progressing
response may never time out; one such response must not block later nonces while other
history permits are free. Each proof needs its own completed canonical verification;
partial RPC results cannot release exposure.

Advancing the runtime cursor never resolves a skipped, unavailable or in-flight
authorization. Persist each verified resolution with the store transition below. On restart,
start a new pass over unresolved rows; completed resolutions remain durable and idempotent,
and every unfinished row remains discoverable regardless of the lost cursor. A live owner
excludes its bounded in-flight IDs from redispatch across a pass boundary. At most one query
per authorization is active under that owner. Advance enumeration without waiting for a
slow member. Add no persisted cursor/pass state or cursor migration/crash-recovery protocol.
Durable traversal progress is deferred unless measured restart starvation or repeated work
justifies it; liability preservation does not depend on that optimization.
Unavailable entries retain liabilities and are revisited under the existing backoff.
If all history permits are occupied, further history progress is not guaranteed until a
permit returns; keep the separate balance/foreground progress contract explicit.
Use the existing history owner with ordinary database pagination, streamed results and
controlled RPC concurrency. Batch size determines how much work is acquired at once;
finishing a batch continues enumeration. Add no internal byte-accounting subsystem,
history-size rejection gate, payment-count cap or whole-sweep deadline. Preserve every
liability regardless of history size or reconciliation progress.

Admission must account for all unresolved liabilities without repeatedly loading or
scanning the entire unresolved journal while holding the group gate/store worker. Prefer
a checked per-wallet exposure total maintained transactionally with the existing journal:
atomic admission/journaling adds once, and canonical resolution subtracts once.
Failed admission and discarded private signatures change no exposure. Duplicate transitions change
nothing. This is a derived aggregate, not spending authority, a memory quota or another
financial ledger. Use exact existing amount arithmetic with checked overflow/underflow;
never sum decimal text through SQLite floating-point coercion. Initialize it from the
journal during the supported conversion, and preserve it atomically across reopen.
An equivalent implementation may avoid the aggregate if it establishes the same efficient
admission behavior; do not require a new accounting abstraction for its own sake.
Audit treasury readiness/preparation helpers too: `require_spend_ready` currently calls
full `status()`. Use targeted indexed reads for operational predicates and keep reporting
separate. Test large histories, aggregate/journal equivalence where used, and unrelated
store progress; internal collection size alone must never reject an affordable payment.

At atomic admission/journal commit, under the gate:

- Check current wallet eligibility, group cap, original evidence lifetime, canonical
  compatibility and all current unresolved liabilities, including new reservations.
- Require the balance confirmed anchor at or beyond the wallet's debit watermark.
  Releasing a USED authorization cannot make a pre-debit balance spendable. A canonically
  proven EXPIRED_UNUSED authorization caused no debit and does not raise that floor.
  Its exact expiry/unused proof must still be compatible with the accepted canonical
  history; elapsed time alone never releases exposure.
- Use a complete verified observation. A newer complete compatible observation may
  replace the captured one as a unit with its own original lifetime. Do not splice
  amounts/anchors or extend freshness because work finished.
  Later completion alone cannot supersede observations at newer canonical anchors;
  reject anchor regression and preserve the ordering of the underlying evidence.
- A lower observed balance does not itself reject payment: the replacement observation
  can still cover amount plus liabilities. Superseded evidence cannot override a known
  lower balance. If the newer evidence is unavailable, incompatible or insufficient,
  do not repeatedly reacquire until a favorable value appears. An unsuccessful refresh
  alone is not newer balance evidence and does not invalidate a still-fresh compatible
  complete observation; actual incompatibility or insufficient capacity fails unsigned.
- Benign same-wallet publication does not invalidate evidence already covered by that
  wallet's accepted checkpoint lineage. Uncovered same-wallet results use the bounded
  compatibility check below. Another wallet's publication, checkpoint advancement or
  resolution never changes this wallet's compatibility, refresh allowance or generation. Known conflicting canonical anchors
  retain their typed failure scope. Late history may add exact resolutions without
  overwriting newer balances.

### Linear checkpoint verification

Persist one confirmed financial checkpoint per wallet/asset/network and a separate USED
debit watermark. Advance the checkpoint atomically only with a verified liability-release
or accounting fact, preserving continuity with prior accepted financial history. Readiness,
retirement, ordinary balances and fully reserved admissions do not advance it. Another
wallet's progress creates no compatibility dependency or refresh requirement.

Admission requires a complete, unexpired observation canonically compatible with that
wallet's checkpoint and a confirmed balance anchor at or beyond its USED debit watermark.
EXPIRED_UNUSED retains its exact canonical proof without raising that balance floor.
A common older checkpoint or height order alone does not prove two newer anchors compatible.
Older compatible resolutions may commit without regressing the checkpoint or replacing
newer balances. Checking headers never renews balance freshness.

Reject conflicting anchors/amounts before treating results as superseded. Apply the
[evidence failure scopes](#evidence-and-resource-contract), including scoped durable repair
when released liabilities/accounting are affected. Invalidated captures and late results
cannot grant authority; re-verification must establish compatibility under current scope.
Reopen discards runtime verification credentials while retaining exact financial support.

One private verifier shared by admission and maintenance establishes these facts under the
configured trusted-RPC model; this is not independent consensus or irreversible finality.
The existing owners retain acquisition, serialization and commit responsibilities. No
second certificate service, query registry, lifecycle owner or dependency ledger is needed.
Bounded compatibility rechecks use the existing evidence acquisition rules.

The [verifier implementation design and fixtures](#verifier-implementation-design-and-fixtures)
provides an illustrative compatibility algorithm and cache strategy.
Those representation and optimization choices are not additional financial acceptance
predicates. Equivalent implementations must establish the same compatibility, freshness,
liability and failure-scope invariants and pass the corresponding fixtures.

### Durable evidence provenance

Persist the supporting value fields in the existing financial records, not just an
observation ID pointing into an evictable cache. A balance-dependent record retains wallet,
asset/network and confirmation scope, a sanitized endpoint identity, exact confirmed/latest
height/hash/amount tuples, block-derived validity, and the verified checkpoint used at
commit. A resolution retains the exact attempt/payer/nonce/outcome, proof block/time and
verified checkpoint. Share the bounded encoding across payment journals, credit records,
retirement records and Base accounting; no second financial/dependency ledger is needed.
The checkpoint itself is persisted atomically with the fact that advances it.

Use indexed lookups of these records by wallet/asset/network/block hash to determine
whether conflicting evidence supported a liability release/accounting disposition,
only admission with intact exposure, or a readiness/retirement observation. Return a
bounded matching/conflicting record, not all historical observations. Keep indexes for both
confirmed and latest support where relevant. Compare complete amounts at the same hash;
never choose a favorable value. Supporting evidence becomes financial history only in
the atomic admission/journal transaction; before then a failed payment has no durable
reservation or financial fact to clean up.
After cache eviction or reopen, distinguish replaceable observations, admission-only support
with intact liabilities, readiness/retirement observations, and liability-release/accounting
dispositions. Use exact journal state and support; no general dependency graph or lifetime scan is required. If the records cannot
establish that the conflict leaves all liability-release/accounting support intact, preserve
the failure table's scoped block instead of guessing.

On reopen, clear all runtime certificates and begin a new namespace. Load compact
per-wallet checkpoints and exact debit watermarks as needed, then verify each wallet
against its own records before granting it fresh financial authority. Do not load or
verify the full retained inventory as a startup or unrelated-payment prerequisite.
Historical supporting records remain readable but do not become
fresh observations. No lifetime scan or reconstruction of an observation graph is required.

Foreground acquisition and recovery follow the single
[admission recovery transition table](#admission-recovery-transitions). Compatibility
rechecking uses step 4 of the [verifier design](#verifier-implementation-design-and-fixtures);
this evidence section adds no refresh or commit allowance. Canonical compatibility
requires verifier-established checkpoint compatibility, not height or endpoint identity
alone. Checking headers never renews balance freshness.

Evidence scope includes confirmation depth. With a fixed depth, confirmed height is
latest height minus that depth, so ordinary complete observations move both heights
together. A depth/configuration change invalidates reusable evidence and requires a
complete refresh in the new scope; do not merge differently scoped observations or
build a general partial-order evidence engine. Known corruption and confirmed-anchor
conflicts use the explicit failure scopes below.

| Evidence result | Failure and recovery scope |
| --- | --- |
| Read timeout, unavailable endpoint or cancelled unsigned query | Failed acquisition only; retain any independently usable complete observation and unresolved liabilities. Ordinary transport fallback/backoff remains bounded. |
| Expired observation, depth/scope mismatch or invalid fixed-depth tuple | Reject that observation for admission/retirement; ordinary complete verification restores evidence. No group disablement. |
| Latest-tip replacement with compatible confirmed history | Invalidate the affected observation; ordinary verification may recover. No operator stop for an unconfirmed replacement alone. |
| Malformed new RPC response or TLS handshake failure without contradictory accepted facts or demonstrated compromise of earlier evidence | Reject and report that endpoint's acquisition, then permit bounded fallback to another configured endpoint with a complete fresh verification. Do not advance evidence generations or discard earlier independently verified complete observations merely because no usable new evidence arrived. Preserve ordinary freshness and compatibility requirements; never bypass TLS or combine endpoint fragments. |
| Contradictory amounts at one hash or a demonstrated integrity/trust failure affecting accepted evidence | Fail the affected verification, identify and invalidate the implicated facts/captures, and apply financial repair rules where released liabilities/accounting depend on them. Widen wallet/endpoint scope only to demonstrated impact; do not conceal the conflict through fallback. |
| Confirmed anchor changes hash, affecting only replaceable observations or admission evidence with full liabilities still reserved | Invalidate those observations and captured results. Ordinary complete verification can recover after proving preserved financial checkpoints unchanged and accounting for all current liabilities. Independently intact resolution support governs any already resolved authorization; its former admission balance adds no repair obligation. Preserve signatures, journals and role changes; no rollback, replay or permanent repair block solely for that admission anchor. |
| Confirmed replacement affects an applied liability-release/accounting anchor, or intact support for those facts cannot be established under the provenance contract | Preserve records and block the affected scope with `chain_repair_required`; no automatic financial rollback. Scope shared impact to actual dependencies. A transport outage alone is failed acquisition, not evidence of a fork. |
| Contradiction confined to historical readiness/retirement support, with liability-release/accounting support intact | Preserve the historical observation and report its discrepancy. Fresh complete verification can restore payment eligibility with all liabilities included. A disputed funding outcome remains review-required for that operation; it grants no new funding, closure or replay. Do not reactivate retirees or impose a permanent wallet spending block solely for that observation. |
| Wrong chain or verified endpoint/network identity mismatch | Reject the endpoint for that configured scope; do not publish its evidence. A wider stop requires demonstrated impact on other accepted evidence. |
| Insufficient current capacity or lost wallet eligibility | Fail that admission/retirement predicate, not unrelated wallets or maintenance. Final pre-handoff selection may use another completed candidate. |

Represent these results with fixed typed categories used by publication, admission,
retirement and diagnostics. Protocol integrity failures remain failures even when a
cached amount would be favorable. Resource sharing alone does not widen their scope.

Use the existing finite configured-endpoint traversal and shared RPC bounds for these
acquisition fallbacks; add no retry loop, endpoint discovery or TLS override. A TLS
handshake failure contributes no accepted evidence. A malformed response is eligible
only when it establishes no contradiction of accepted facts; classify demonstrated
wrong-chain, conflicting-amount/anchor or compromised-evidence cases first under their
own rows. Local request-construction errors cannot be repaired by endpoint fallback.
Keep sanitized failure diagnostics even if a fallback succeeds. Anchor-only fallback
still requires complete acquisition on the new endpoint under the verifier contract.

Invalidation must reach captured observations and late query results, not just cache
entries. Wallet-specific invalidation remains under the existing group owner and changes
only that wallet's generation. Endpoint/network trust invalidation spans every implicated
group using that configured scope; separate group gates cannot serialize it.

Keep endpoint generations and fixed reasons in the existing shared store owner's runtime
state. Serialize their advancement and the authoritative generation checks in that owner,
including checks inside financial transaction jobs. A caller-side generation check before
enqueueing is insufficient. Query capture/publication obtains applicable generations
through that same owner; no caller may independently reset them. Invalidation ordered
before an evidence commit rejects its old generation, including for a caller already
holding another group's gate. A commit ordered before invalidation retains its financial
records and receives the failure table's scoped invalidation/repair treatment; invalidation
never rolls it back or authorizes replay. Reopen starts a new runtime namespace and
preserves any durable financial repair blocks. No extra service, deployment-wide financial
mutex, all-group lock acquisition or separate persistent invalidation ledger is needed.

Each balance and historical query captures applicable wallet/endpoint generations;
publication, admission, authorization-resolution commits, credit completion and retirement
reject invalidated evidence at their authoritative boundary. Failed acquisitions alone,
ordinary publication and checkpoint advancement do not advance generations. Demonstrated
endpoint/network trust failures affect only their implicated scope; wallet-local conflicts
do not quarantine an endpoint or other groups merely because they share it. Do not create
a group-wide evidence generation or propagate a wallet failure through role relationships.
Apply the failure table to facts already accepted as well as captured or late results.

Retain useful completed historical proofs after replacement of ordinary balance evidence,
but never accept an old certificate merely because its block was once called confirmed.
A query can have observed a newer tip than the query detecting a replacement. When the
proof's endpoint trust generation is unchanged, its immutable facts remain intact and
no scoped financial repair block applies, permit one bounded recertification of its exact proof anchor and current
financial checkpoint on the original endpoint. Capture current generations before the
check and revalidate them at commit. Preserve the original nonce result, proof block/time
and operation binding; only successful canonical verification creates a new certificate.
The number of intervening replaceable-balance invalidations is not an eligibility gate.
No event-count exception, last-invalidation-reason shortcut or invalidation-history registry
is needed. A changed proof anchor, contradictory proof, endpoint trust invalidation or
financial conflict cannot use this path. If another invalidation races the recheck, defer
to ordinary history work without releasing liability or looping. Old balance captures
remain invalid, and recertification never renews balance freshness.

Apply a historical resolution under the same group gate in one store transaction:
validate its captured process namespace and applicable wallet/endpoint invalidation generation, exact
attempt/wallet identity and verifier-established canonical compatibility; then record
the resolution once and change only that attempt's unresolved state. Advance the debit
watermark only for a USED result at a greater compatible anchor. A late older compatible
proof may resolve its exact attempt without regressing the watermark or replacing balances.
An EXPIRED_UNUSED proof retains its own canonical anchor and does not change the balance
floor. Apply [linear checkpoint verification](#linear-checkpoint-verification) to its
exact proof anchor and the accepted financial checkpoint. It need not enumerate retained
balance anchors; each balance carries its own certificate and is checked when used.
Rejecting incompatible proofs preserves exposure and requires no payment retry.
Duplicate equivalent proofs are idempotent; conflicting proofs use the integrity failure
scope, never last-writer-wins. A rejected stale-generation proof changes no liability,
resolution record or watermark. Reacquire evidence through ordinary history work rather
than replaying the stale result, except for the explicitly verified recertification
above. Ordinary observation recovery cannot revive an earlier query's certificate.

Apply the failure table to invalidation and recovery; identify affected accepted facts
before choosing a scope. Where it permits ordinary recovery, a later acquisition must
verify a complete observation and the compact financial checkpoint through an authenticated
configured endpoint, preserving liabilities and history. Missing support cannot establish
that a conflict affects only replaceable observations or fully reserved admissions.
The configured endpoints define trust and change only on supervised reopen; do not add
endpoints, weaken TLS, majority-vote amounts or retry within a failed verification to
conceal a contradiction. A later success supplies no retroactive authority and cannot clear
a financial repair block. An operation's review obligation blocks wallet spending only
when exposure or required canonical accounting cannot be established.

Readiness (`D`) records that capacity was observed, not an irrevocable promise that those
funds remain. A contradiction confined to its support leaves the record readable but
unusable as completion/closure proof until the exact operation is re-established through
ordinary verification or reviewed recovery. The failure table governs payment eligibility
and retirement discrepancies; disputed readiness never authorizes another funding attempt.

Report `evidence_repair_required` for amount contradictions and `chain_repair_required`
for canonical conflicts only at the financial scope that requires repair. Retain exact
records and sanitized supporting evidence.

Do not add an evidence-assessment or fork-repair mode to `wallet recover` in this feature.
That command retains its existing funding-recovery purpose. Exceptional repair is deferred:
the operator/developer may inspect the preserved state and commission a narrowly scoped
one-off tool for the actual incident. No assessment schema, repair command, historical
reconstruction framework or speculative repair fixture is a delivery requirement.
Diagnostics must distinguish ordinary observation refresh from a durable blocked scope
that needs custom investigation; do not suggest repeated `wallet recover` calls can fix it.

The failure table above exclusively defines conflict, replacement and transport-failure
handling. These additional cases define ordering and acquisition behavior:

| Observation case | Required result |
| --- | --- |
| Same confirmed/latest anchors and amounts, later publication | Equivalent evidence; no sequence-equality gate and no freshness renewal |
| Incoming anchors both at or beyond saved anchors, with canonical compatibility verified | Replace as one complete observation with its original lifetime, regardless of whether the amount rises or falls |
| Late completion with both anchors older than or equal to saved anchors | Discard the superseded observation; do not fail a payment using the usable saved one |
| One anchor advances while the other regresses | Reject an invalid fixed-depth tuple or mismatched evidence scope; acquire a complete observation in the current scope through ordinary verification; no splicing or in-call retry loop |
| USED resolution for the same wallet commits after balance acquisition | Recheck the debit watermark under the gate; use compatible available evidence or the invocation's one evidence-race refresh, then fail before signed submission if compatibility still cannot be established |
| EXPIRED_UNUSED resolution for the same wallet commits after balance acquisition | Apply linear checkpoint verification, using the existing anchor-only recheck if necessary; no debit-watermark increase or balance-age renewal |
| Evidence expires during second unsigned response | After its challenge, acquire ordinary fresh evidence for the fixed payer; no extra candidate search, handoff or signature |
| Evidence expires between acquisition and admission | Apply the [admission recovery transitions](#admission-recovery-transitions). |
| Unpublished signature expires before atomic admission commits | Apply the [admission recovery transitions](#admission-recovery-transitions); an earlier RPC refresh does not prevent local replacement using still-usable evidence. |
| Another wallet advances its checkpoint or resolves an authorization | No change to this wallet's evidence, generation, refresh allowance or eligibility; retain ordinary group role/liability checks. |

Check conflicts before treating observations as superseded. A query that began before a
new resolution cannot assert compatibility solely from its height; its certificate must
satisfy [linear checkpoint verification](#linear-checkpoint-verification). A compatible
EXPIRED_UNUSED proof needs no newer balance solely because its proof height advanced.
Apply the [admission recovery table](#admission-recovery-transitions) when its preconditions hold. Do not reacquire repeatedly
on harmless publications or globally invalidate other wallets. Before coding, pin concrete
anchor/amount fixtures for this table, including fallback with non-monotonic anchors.

Retirement has its own commit predicate. Require complete fresh evidence, exact group
ownership and no unresolved payment authorization or pending destination funding
obligation on that wallet. Preserve unrelated historical source accounting; it is not
an excuse to erase it or a blanket veto on every wallet action. Use one shared
destination-obligation predicate for retirement, derived from exact
operation facts rather than `funding_jobs.COMPLETE` or the current wallet role. The [funding phase dispatch](#funding-phase-dispatch) table maps the existing journal
phases to these exact facts; retirement uses the following ownership predicate:

| Outstanding work on the exact destination | Retirement consequence, in addition to the ordinary role/evidence/payment checks |
| --- | --- |
| Preparation intent uncertain, prepared bytes, or submitted swap with unresolved destination outcome | Block retirement while the operation can still fund the destination or that possibility remains unresolved. Preserve exact recovery obligations even if an external credit already made the wallet usable. |
| Destination readiness recorded, destination outcome closed under the existing completion/recovery contracts, only source confirmation/accounting outstanding | Source work alone does not block retirement. Keep it scheduled and preserve all source reservations/history; retirement neither completes that work nor releases its liabilities. |
| Destination readiness recorded but provider outcome remains unknown or conflicting | Readiness alone cannot exclude a further owed destination credit. Block retirement until the destination obligation is resolved; the wallet remains usable for otherwise affordable payments. |
| Failed-swap refund processing or unresolved destination outcome | Apply the same destination-obligation predicate; failed-swap closure requires reviewed disposition in this release. If the exact destination obligation is closed, remaining treasury refund/shielding/accounting work alone is not a veto. A refund phase label or unverified seller assertion does not establish closure or release source exposure. |
| Successful swap with returned excess input | Apply ordinary successful destination closure. Correlated surplus-return accounting, including a late return or separately authorized shielding, does not reopen destination obligations or veto retirement. Ambiguous attribution blocks only dependent accounting unless it undermines the established destination outcome. |
| Full external satisfaction with an unsigned quote and proven absence of preparation effects | Atomically prevent further funding for that allocation and close/archive the unsigned operation through the existing guarded path. After that proof, the historical quote alone does not block retirement. No replacement, new quote, abandonment state or retry authority is created. |

Reuse the normative completion/recovery facts below to establish destination closure.
For success, correlated validated SUCCESS plus recorded canonical destination readiness
closes that side, absent disputed support or a conflicting outcome, even while source accounting continues.
Introduce no additional receipt requirement or retirement-specific settlement protocol.
An allocated or ready wallet is still ineligible by role. Disabling its slot alone does
not close a funding obligation or release its claim. Unexpected future external credits
remain outside the observed retirement bound.

Known newer balance evidence must supersede the retirement capture as a complete
verified observation too. A known increase beyond the accepted maximum prevents
retirement; a superseded small capture cannot retire a known larger balance. A later
complete compatible lower observation may qualify under its own freshness and bound.
Reevaluate retirement's remainder predicate against the newest compatible complete
observation; admission separately reevaluates amount plus liabilities. A lower balance
can satisfy either predicate and is not itself an invalidation. Both exact observations
must be zero for empty retirement. Max(confirmed, latest) otherwise
bounds observed remainder, not possible future unexpected credits.

Bound active unsigned RPCs, owned queries and queued callers before spawning work.
Start with shared deployment-wide concurrency and bounded existing foreground,
reconciliation and funding owners. Acquire network permits outside financial gates;
retries/fallbacks use the same limit. Cancellation prevents queued dispatch and reaps
caller-owned work without cancelling independent history or funding ownership.

Measure bidirectional progress before adding coalescing or finer scheduling. Fair
queues cannot free slots occupied by continuously progressing responses. State which
owner bounds leave independent work execution opportunities and which saturation
cases cannot guarantee progress. Keep connection and read-inactivity guards distinct
from evidence freshness. Add no whole-call, whole-sweep or wallet-count deadline.

Apply bounded retention to the existing network and cover owners as well. The HTTP
factory currently clears all clients at 256 entries, and wallet-scoped Base RPC clients
share that cache with provider clients. Replace whole-cache clearing with bounded
recency-based eviction of individual entries. Acquired client handles retain their
connections and active work. Interleaved retained-wallet traversal must not flush the
whole cache or systematically discard a frequently reused provider client. Preserve
identity/origin/transport cache keys; add no inventory limit or cache-maintenance timer.

The cover registry currently retains every identity/origin owner for the runtime and
rejects new owners at `max_owners` (default 1024). Make inactive state reclaimable within
that existing bound. Reclamation requires completed session ownership and cleanup,
including callbacks that still access the registry; zero call/stream counters alone
do not prove completion. Active or draining owners remain attached. Reusing a retained
entry preserves its capability/fallback state. Completed capability results, including
negative results, live only as long as the retained cache entry. This deliberately changes
the current runtime-long `Capability::Unavailable` behavior: eviction forgets the completed
failure and used fallback, and recreation starts ordinary bounded qualification. Repeated
calls or episodes on a retained failed entry do not retry qualification. Recreated entries
may add unsigned cover traffic, charged to the existing process request/byte limits;
eviction never resets those limits or an active session's limits and cleanup ownership.
Do not evict an entry merely to retry a failed qualification or obtain another fallback.
Use ordinary bounded recency-based reclamation of fully inactive entries, without a
negative-cache tombstone registry or a new suppression timer.

Eviction cannot restart live qualification or cancel financial work. If every entry is
still owned, retain the existing optional-cover unavailability behavior until cleanup
permits reclamation. Neither retirement nor larger resource limits are required to admit
later cover owners. Use the existing registry/session lifecycle, without a second owner
registry, periodic collector or wallet-count funding gate. Document finite negative-cache
retention and qualification after eviction rather than promising indefinite failure memory.

Wallets, completed funding jobs and treasury operations accumulate over the application's
lifetime. Maintenance must query indexed, bounded batches of actionable operations
instead of loading full `Store::status()` and searching historical vectors on every tick.
Keep reporting separate from scheduling; paginate acquired rows, stream results and preserve all
history. Reuse existing field/input bounds rather than tracking internal object bytes.
Test both many retained wallets and long lifetime history with few live wallets, not
just many pending nonces. Retirement is optional workload reduction; boundedness must
hold with zero tolerance and no eligible retirement candidates. This batching contract
applies to application database work; the pinned wallet's in-memory return-discovery
traversal has the explicit limitation in [source accounting](#financial-completion-and-source-accounting-predicates).

The current `FundingWorker::tick` serially awaits completed-job source reconciliation,
recovery and the next funding step. Permit bounded independent 1Click quote/status and
Base credit work within the existing funding owner. Keep at most one mutating step per
operation, but allow independent status/credit observations for that same operation while
source reconciliation occupies the treasury actor. Bound and deduplicate reads by exact
operation and observation kind; coalesce with the existing balance owner where it already
owns that read. Results carry immutable operation/request bindings and commit through the
serialized store owner against current facts. A read result cannot regress a phase, revive
an archived attempt, authorize another send or cancel accepted treasury work. Independent
ready-to-run reads must progress under supported resource bounds while a treasury command
is occupied. Quote acquisition currently first calls treasury.refund_address; only the
unsigned HTTP portion after that owned prerequisite is eligible for this independence.
Do not promise a new quote can bypass an occupied treasury actor or derive a refund
address outside its owner. Test both a prerequisite-complete quote and one still waiting.

Independent reads also need independent scheduling within this owner. The current
job-wide `next_poll`, `error_streak` and `last_error` cannot gate all concurrently
actionable observation kinds. Track due time, failure streak and in-flight ownership by
exact operation and observation kind, reusing the balance owner's scheduling where
applicable. A Base success cannot reset 1Click failure backoff; a status failure cannot
postpone independent source or credit observation. Derive job health for reporting from
those outcomes rather than letting the last callback overwrite scheduling authority.
Preserve finite retries, ordinary backoff and the existing swap health/polling floor;
these scheduling records grant no financial retry or phase-transition authority.
Use bounded runtime scheduling metadata, not another durable financial ledger. Reopen
reconstructs missing work from committed facts under ordinary startup/backoff rules and
never resets durable recovery attempts or replays accepted transactions. Validate reverse
completion orders, repeated failures, cancellation and reopen through the actual worker.

Make this ownership change concrete: current `FundingBackend` methods borrow `&mut self`.
Use bounded read futures owning immutable request snapshots and cloned/shared Base/1Click
adapters, with results returned to the existing funding owner for serialized transitions.
Keep treasury commands under the treasury actor and apply the read/mutation distinction above.
A semaphore around the existing serial `&mut self` worker does not provide independence;
a mutex around the entire backend would preserve that blockage. Avoid a second scheduler.

The treasury actor also serializes commands: source reconciliation calls wallet sync
before transaction lookup. Spare RPC permits therefore cannot guarantee another job's
preparation, submission or recovery will progress during that command. Preserve this
ownership boundary and explicitly report the wait. Exercise the actual worker and
actor together; do not promise all-job progress or move wallet sync/mutation outside
the actor. A future need to extract a proven read-only acquisition is a separate
review, not a general scheduling redesign in this feature. No duplicate sweeps,
tier scheduler or whole-job deadlines are needed.

Separately, the store's `one_outgoing` index and `require_spend_ready` permit only one
unresolved prepared source transaction across the treasury. This remains true while
the actor is idle, until canonical confirmation or proven-safe expiry resolves that
outgoing record. Retain this ownership restriction in this delivery. Independent slot
claims and unsigned reads do not imply parallel source preparations or submissions;
an unresolved source transaction can delay every group's refill despite other spendable
funds. Report this wait separately from actor occupancy and treasury insufficiency.
Zingolib's spent-note exclusions, single stored proposal and process-local advisory
locks do not replace durable snapshot/preparation and uncertain-submission ownership.
Relaxing the singleton requires separate work proving those invariants across multiple
outgoing transactions, cancellation and restart; it is not aggregate-budget removal.

## Reserve consumption and refill

| Event | Atomic state change and authority |
| --- | --- |
| Active or retained payment journals | Record authorization on exact payer; roles/slots unchanged |
| Ready reserve payment journals | Record authorization, retain old active when present, make payer active, advance consumed slot generation and persist vacancy; no allocation or retirement |
| Active reconstructed from retained inventory | Only when no eligible active exists, choose an eligible retained wallet by sequence then wallet ID under the group gate. Set active without a payment, slot mutation or allocation. |
| Unsigned default identity selected from ready reserve | When no funded active exists, bind the request to an eligible ready reserve without any role, claim, generation or job mutation. Only its journaled payment consumes the reserve. |
| Fill vacant configured slot | Under current funding authority, uniquely claim generation and create one wallet/job/outbox at current slot target; retained inventory size is irrelevant |
| Canonical replacement credit | Shared destination-readiness transaction records exact verified capacity and assigns the role from current claim/group state; unfinished operation accounting remains scheduled; repeated credit cannot restore a consumed wallet to ready |
| Remove reserve configuration | Stop new fills; detach existing ready wallet into retained; preserve allocated job ownership and completion as described below |
| Reconciliation retirement | Under the configured remainder tolerance, retire eligible retained wallets within tolerance using fresh evidence; each retirement is atomic, stops routine balance polling/payment selection and creates no wallet/job; zero tolerance requires dual-zero |
| Unsuccessful or disabled allocation | Preserve the exact wallet/job/claim and history. Configuration disablement pauses new funding; treasury insufficiency uses ordinary backoff. Re-enablement reuses that allocation under current authority without a release/reclaim cycle or retry reset |
| Partial external credit on an enabled clean allocation | Keep the same wallet/job/claim and continue its original first funding operation under current authority. Sub-target credit alone causes no detachment, successor or quote refresh. |
| Partial external credit on an allocation disabled by slot/group configuration | Preserve the same wallet/job/claim and holdings; continue permitted observations and safe recovery. No readiness, detachment, successor or new funding. |

This is the normative role/claim transition table. Lifecycle sections below specify
its preconditions and recovery, rather than defining alternate transitions.
Reserve consumption requires payment authority only; it can proceed
with auto-funding disabled or a funding permit denied. Preserve the old active's exact
liabilities when retaining it, including at observed zero. Vacancy fills independently
require current auto-funding/qualification authority and unique claims. Actual source
capacity is checked by the existing treasury preparation/reservation owner.

Replace the duplicate completion/role logic in `Store::record_credit` and
`apply_chain_view` with one transactional destination-readiness primitive. Both the actual
funding worker and background reconciliation call it with complete verified evidence
and the exact wallet/job correlation. It validates accepted credit, current claim
generation and group state, records destination readiness once, and chooses ready, reconstructed
active or retained according to the table and startup rules. Disabled groups remain
unavailable to pay; disabled slots in enabled groups yield retained capacity. Repeated
or late completion never overwrites newer balances, reattaches a consumed slot or
displaces an active payer. Source/refund accounting remains independently owned.
Evidence acquisition stays outside the transaction. Test both callers in both arrival
orders, including stale completion after payment, slot disablement and reconstruction.

### Applicable destination requirement

Use one shared destination-requirement function for readiness,
phase dispatch and captured-bootstrap outcomes. Resolve it from committed facts under
the store transaction before archiving a quote or changing claims:

| Current destination facts | Applicable requirement |
| --- | --- |
| Readiness already recorded (`D`) | Preserve its exact original requirement/correlation; later spending or policy changes cannot undo that historical fact. |
| Outstanding accepted operation, including an unsigned accepted quote | Its exact accepted output and destination, including accepted bridge-minimum increases. Current configuration and original allocation size cannot replace it. |
| No outstanding accepted operation | The allocation's recorded original target and destination. A proposed/rejected quote or new configuration alone does not change this requirement; archived operations cannot satisfy a different outstanding attempt. |

Given fresh compatible conservative capacity, record full readiness only when that
requirement is met. Sub-target credits remain attached to the same allocation whether
enabled or disabled. Snapshot the requirement, operation correlation and observed capacity
in the resulting readiness record. A late quote acceptance must recheck readiness and
claim ownership in its existing conditional commit. If acceptance commits first, credit
uses its accepted requirement; if credit commits first, stale acceptance is rejected.
These are explicit serialized outcomes, never a mixture of pre/post-acceptance terms.

For example, original allocation $10, accepted clean $2 quote, disabled slot and verified
$3 credit records full readiness under $2, then guarded external satisfaction. Conversely,
original $2 with an accepted $10 output and $3 credit remains attached even while disabled.
Without an accepted operation, the original recorded target applies. External credit
invents no quote or settlement history. Sufficient balance cannot close outstanding
swap/source/refund accounting. Historical operation evidence stays correlated with its
original operation and cannot resolve a different outstanding attempt. Keep destination
readiness distinct from those obligations; add no transfer-receipt prerequisite to
independently verified payment capacity. Define these correlations in the shared primitive
before replacing either caller.

Current `complete_job` marks `funding_jobs` COMPLETE; `next_funding_job` excludes it,
and the completed-job path primarily reconciles the source transaction. Do not reuse
that terminal transition as the meaning of destination readiness. Preserve the existing
operation phase and its scheduling until the completion predicates below hold for the
original source transaction, swap and any failed-swap recovery. Successful excess-return
accounting remains independently owned even after funding completion. Derive wallet readiness from the committed credit
record/role; keep operation progress in the existing funding journal. Do not add a second
settlement ledger. The following predicates govern financial completion; the role/claim
table above governs wallet transitions. Completion never restores a consumed slot or
modifies a later generation. Source/status/refund work continues for the exact original
operation independently of the wallet's role.

### Financial completion and source-accounting predicates

Destination readiness is an immutable once-recorded observation for the exact allocation and,
when present, its accepted operation. Spending below the original credit requirement does
not revoke it. Its support must be undisputed to satisfy financial completion or destination
closure; the evidence failure contract governs historical contradictions. The shared
primitive returns that recorded fact on repeat credit calls;
it does not demand that a consumed wallet rebuild its original balance. Current payment
capacity still requires ordinary fresh evidence. Keep scheduling the original operation
until one of these outcomes is established through its existing journal:

| Outcome | Required facts and transition |
| --- | --- |
| Successful funding operation | Canonically confirmed source transaction and gross expenditure accounting, a validated 1Click SUCCESS response correlated to the exact accepted quote/deposit/recipient, and recorded canonical destination readiness for that operation. No conflicting failed-swap outcome or disputed support. Record funding completion once, even if the wallet has since spent or changed role. Ordinary excess-input returns are compatible with success and follow the independent accounting rule below. |
| Excess input returned from a successful operation | Preserve SUCCESS and destination readiness. Canonically confirmed, exactly correlated returned outputs update the existing source-return accounting idempotently, before or after funding completion. No REFUND_PENDING transition, destination reopening, manual failed-swap disposition or new funding attempt. Shielding remains separately operator-requested. |
| Failed-swap refund or ambiguous returned funds | Preserve reviewed failed-swap disposition and scope attribution uncertainty to the dependent accounting. Observe and record canonical source/refund facts, but returned funds alone cannot close a failed swap, release its destination obligation or authorize another attempt. A return observed before provider outcome is known proves neither success nor failure; continue ordinary outcome observation without classifying it as a terminal refund. |
| Expired or proven unprepared operation | Canonical-expiry or guarded-unprepared closure archives the exact operation and closes only its proven-safe reservations, independently of permission to retry. Destination readiness suppresses new quotes/preparation for that allocation; already accepted work follows the phase-dispatch table. Later work uses the separate same-job authorization below. |
| Destination funded externally before quote acceptance | Atomically record readiness and prevent acceptance/preparation on that allocation. There is no accepted swap to complete. Preserve any unsigned attempt history without inventing settlement or a source charge. |
| Missing or conflicting completion evidence | Keep the original operation unresolved or review-required under existing typed rules, independently of wallet usability. No fresh quote, duplicate deposit or favorable-status overwrite. |

The success predicate combines independent facts: 1Click reports fulfillment, Base proves
usable destination capacity, and Zcash proves source accounting. An external credit plus
SUCCESS does not prove which Base transfer fulfilled the swap. Report those facts with
that attribution limit; do not label the balance observation a canonical swap-transfer
receipt. The [phase-dispatch contract](#funding-phase-dispatch) specifies persisted provider
correlation and its limits. It adds no destination-transfer receipt gate to admission.

The adapter requests `EXACT_OUTPUT`. The [1Click swap-type contract](https://docs.near-intents.org/integration/distribution-channels/1click-api/swap-types)
describes an input-side slippage buffer and return of unused input. Therefore a successful
destination transfer and a partial source return are compatible. Verify and pin the
applicable ZEC-route fixtures during implementation; the general contract supplies no
exact return amount, fee or arrival deadline for this route. Do not reduce the accepted
deposit to `minAmountIn` or infer a receivable from the quoted buffer.

Reuse `src/rotation/store/refunds.rs` and treasury return observation, not a second ledger
or funding phase. Persist returned outpoints with exact original-operation ownership;
record each once after canonical confirmation and source reconciliation. Preserve gross
source expenditure and report confirmed returns separately, with any derived net cost.
Retain existing principal attribution bounds: returns cannot erase source fees, credit
another operation or count returned principal again when shielding it. A reported or
expected return is not spendable capacity; actual treasury readiness remains authoritative.
Missing or ambiguous original correlations require scoped investigation, never association
with whichever operation currently occupies the slot or a guess from amount alone.

Do not reuse current refund lookup/scanning unchanged. `Store::refund_bindings`
currently enumerates current funding-progress/address bindings; implement exact immutable
original-operation/address lookup across applicable archived recovery history as well.
Missing or conflicting ownership remains scoped uncertainty. Replace `reconcile_refunds`'
full-status dependency with targeted accounting reads and replace repeated linear
binding/operation searches with keyed matching. Revalidate known credited outpoints by
transaction ID and output index through the treasury owner, including spent/shielded
outputs. Keep exact original-operation ownership and confirmation checks.

Carry that failure scope through treasury synchronization. Currently `SyncSession` propagates
every `reconcile_refunds` error before publishing fresh readiness, and reconciliation stops
at the first collection or record failure. Separate typed attribution/accounting uncertainty
from failures that undermine canonical wallet state, input ownership or spendability.
Preserve and report an affected operation's accounting obligation while processing independent
returns and publishing otherwise valid sync/readiness. An ambiguous historical binding alone
cannot block another group's funding. Wider blocking requires evidence affecting the shared
treasury state or dependent reservations; storage failures and contradictions affecting shared
financial state must not be downgraded to harmless attribution errors. Use existing operation records
and diagnostics, with no second repair ledger. Accounting uncertainty grants no release,
guessed credit, shielding or retry authority.

Discovering unknown late returns still requires inspecting wallet outputs. The pinned
reference and vendored Zingolib expose `wallet_transactions` as an in-memory `HashMap`,
not a paginated transaction feed. For this delivery retain one linear discovery pass
under the existing treasury owner, using keyed matching and avoiding a second full
history copy. Document its history-dependent work and actor occupancy; bounded SQLite
batches do not make this scan incremental or bound the dependency's resident history.
This limitation does not permit skipping late outputs or weakening canonical revalidation.
A dependency-level incremental discovery API/index is separate work: specify how it
finds unknown outputs and handles reorgs before promising bounded incremental traversal.
Do not add a persistent scan cursor or another return ledger to this feature.

Test late returns after COMPLETE, reserve consumption, destination retirement, restart and
applicable operation archival, including ambiguous bindings and duplicate outpoints. Exercise
large wallet transaction/return histories through the actual treasury sync path, not just
store fixtures. Address-correlated canonical receipt proves attribution under the recorded
binding, not the sender's identity or a provider-authenticated refund receipt.

Funding completion does not wait for a speculative surplus or a proof that no later
return will arrive. Keep exact return bindings and ordinary treasury observation after
COMPLETE, reserve consumption, destination retirement and restart. A later canonical
return updates accounting without reopening the funding job. Do not require an extra
1Click status request, new timer, zero-refund assertion or manual review merely because
the return is partial relative to the gross deposit. Canonical contradictions and genuinely
conflicting provider outcomes retain their existing scoped recovery requirements.

### Funding phase dispatch

This table is the normative dispatch mapping for the existing `FundingPhase` journal.
`D` means the exact allocation/operation's destination-readiness record exists; `S`
means the original source transaction is canonically confirmed and accounted; `P`
means a persisted correlated 1Click outcome. These letters are shorthand for existing
facts, not new flags, phase machines or a second ledger. A current wallet balance below
the historical credit requirement cannot clear `D`. Slot ownership controls new funding;
accepted operation identity controls outstanding work after role/claim changes.
Returned-input observations update their existing accounting independently of this
dispatcher; an observed return never synthesizes `P = REFUNDED` or changes the funding
phase by itself. The completion contract determines whether failed-swap recovery applies.

Evaluate terminal/conflicting facts first. Disputed readiness stays recorded as `D` but
cannot prove completion/closure: use the evidence failure contract before this table,
never a no-D funding branch. Then apply the shared destination requirement:
preserve existing `D` and record newly sufficient full readiness. Sub-target credits
leave allocation ownership unchanged. Select the next action from committed facts.
Closure alone never authorizes a job or its slot to dispatch new funding. Treat
missing/mismatched operation correlations as an integrity failure, not an ALLOCATED
fallback. Commit actions conditionally on exact operation ID and expected journal state.
New quote acceptance and preparation additionally require the current claim generation.
A late completion for accepted work need not match a consumed slot's later generation;
it may update only its own operation and the readiness primitive's allowed wallet facts.

| Journal phase and facts | Next permitted action and transition |
| --- | --- |
| Sub-target external credit on an enabled allocation | Continue ordinary phase dispatch on the same job and claim. Do not detach, resize, refresh its quote or allocate a successor because of that credit. |
| Sub-target external credit on a configuration-disabled allocation | Preserve the same wallet/job/claim and incomplete readiness. Continue permitted observation and safe recovery; no new fill or detachment. |
| `ALLOCATED`, enabled, no `D` | With normal funding authority and current terms, obtain one unsigned quote and conditionally persist `QUOTED`. A late quote cannot commit if readiness won the race. |
| `ALLOCATED`, disabled, no `D` | Preserve the wallet/job/claim. No new quote acceptance/preparation while disabled; re-enablement resumes that same job under ordinary authority. Independently permitted observations continue. |
| `ALLOCATED`, `D`, no accepted operation | Close the destination job as externally satisfied: `COMPLETE` with that outcome, not swap success. No new quote, proposal, source reservation or send. |
| `QUOTED`, no `D` | Existing read-only readiness and quote-validity checks, then guarded refresh or an atomic `PREPARING` intent under current authority. Treasury insufficiency retains the claim and uses ordinary backoff/recovery according to the actual preparation boundary. |
| `QUOTED`, `D` | If the guarded unprepared path proves no preparation effects, archive/close the exact quote and mark the job externally satisfied. Otherwise preserve it and use recovery; no quote refresh followed by funding of the already-ready destination. |
| `PREPARING`, either value of `D` | Drain the accepted preparation owner. Persisted exact prepared bytes lead to `PREPARED`; a typed proven-unstarted result permits the existing guarded return to `QUOTED`, where `D` selects closure or normal preparation. Uncertain preparation goes to `RECOVERY_REQUIRED`; missing bytes alone never permit a second proposal. |
| `PREPARED`, original source attempts zero, either value of `D` | The original exact-byte first submission remains eligible only under its existing current authority, safeguards and validity margin. `D` does not cancel already accepted prepared work or authorize new bytes. Denied authority/expired window retains ownership and uses ordinary observation/canonical-expiry recovery. Successful or possibly-started submission proceeds to `DEPOSIT_PENDING`; no second send from this dispatcher. |
| `PREPARED` with a recorded attempt, or `DEPOSIT_PENDING` | Reconcile the exact source operation; never submit again. Independently collect missing status/readiness for this same operation under the evidence/resource ownership contract. Source confirmation records `S`; derive the resulting phase from all committed facts rather than overwriting concurrently recorded success/refund progress with `SWAPPING`. |
| `SWAPPING` or `VERIFYING_CREDIT`, missing `P`/nonterminal `P` | Poll the original quote's status; unavailable/unknown status preserves ownership and backoff. Correlated SUCCESS records `P` and permits `VERIFYING_CREDIT`; FAILED/REFUNDED enters `REFUND_PENDING`; incomplete deposit or contradictory terminal outcomes use `RECOVERY_REQUIRED`. An already recorded `D` changes none of these outcomes. |
| `SWAPPING` or `VERIFYING_CREDIT`, `P = SUCCESS`, no `D` | Acquire and commit missing canonical destination readiness. Continue missing source reconciliation. Complete only when the success predicate holds; never infer credit from provider status. |
| `D` and `P = SUCCESS`, missing `S` | Run source reconciliation for the missing obligation; independent returned-output observation may continue. Do not re-demand the original destination balance, requery a recorded success merely to reconstruct it, or prepare another deposit. With `S` and no conflicting outcome/disputed support, commit `COMPLETE` once. Compatible excess returns do not delay that transition. |
| `REFUND_PENDING`, either value of `D` | Continue source/refund observation and applicable status polling; preserve `refund_requires_review`. Do not automatically shield, close the job/destination obligation, release uncertain reservations or start fresh funding. A later SUCCESS cannot re-enter the success path. Otherwise usable wallets may pay under independent admission checks. |
| `RECOVERY_REQUIRED`, either value of `D` | Existing typed recovery determines what can be observed or safely archived. Do not suppress independently permitted source/status/refund reads solely because of this phase. After proven unprepared/canonical-expiry closure: with `D`, complete the satisfied job without a new attempt; without `D`, the existing eligible same-job reset may return to `ALLOCATED`, preserving attempts and current claim/authority rules. Uncertainty/conflicts remain review-required. |
| `COMPLETE` | No new quote, preparation, submission or ordinary swap-status polling. This phase is written only when the terminal predicates hold, with an explicit success/external/recovered outcome as applicable. Independent treasury observation/accounting of successful excess returns remains permitted, including late outputs. An unresolved original source transaction, destination outcome or failed-swap recovery contradicts completion and is an integrity/migration error; compatible later return accounting does not. |

Persist correlated provider facts in the existing operation journal. Distinguish the
locally assigned operation ID and immutable accepted-quote reference from provider-observed
fields: status, echoed deposit address and validated request echo. The current adapter
checks `quoteResponse.quote.depositAddress` against the accepted deposit, and `check_echo`
checks every original request field (including recipient, assets, amount and routing),
normalizes deadline timestamps and validates allowed provider-added routing/fee fields.
Reuse those checks and pin their exact field behavior in fixtures. Persist bounded
correlation support using the existing protected quote/request record and its exact local
binding, plus observation order/time and sanitized status; do not duplicate raw provider
responses or retain upstream prose/credentials. A local operation ID is not a provider-
authenticated identifier. These checks do not validate every returned quote field or a
destination-transfer receipt. Test mismatched deposit/request/recipient, missing required
fields and the distinction between local association and provider evidence.
Apply the following local compatibility rules to observations of the original submitted
operation, including while source reconciliation is still pending. Status publication and
source completion derive phase from their combined committed facts; neither overwrites
the other from its captured phase. The table grants no source-reservation release or
external settlement proof.

| Saved provider fact | Incoming correlated observation | Result |
| --- | --- | --- |
| None or pending/unknown | SUCCESS, FAILED or REFUNDED | Record the fact and enter the existing success or refund path. |
| Any recorded fact | Equivalent repeated fact | Idempotent; no duplicate completion, charge or release. |
| SUCCESS, FAILED or REFUNDED | Pending/unknown or a transport failure | Retain the recorded fact; no downgrade, invented reversal or new funding authority. |
| FAILED | REFUNDED | Compatible refund progress. Retain the failure fact and record reported refund completion; canonical refund/shielding accounting is still required. |
| REFUNDED | FAILED | Retain the stronger reported refund fact and failure history; no regression or repair solely for this ordering. |
| SUCCESS | Correlated returned excess input, with SUCCESS unchanged | Preserve success; apply canonical returned-output accounting independently. A partial returned amount or nonzero returned-amount metadata is not a REFUNDED swap status. |
| SUCCESS | FAILED or REFUNDED swap status | Preserve both facts and require review; do not select the favorable outcome or relabel a conflicting terminal status as an ordinary surplus return. |
| FAILED or REFUNDED | SUCCESS | Preserve both facts and require review; never re-enter success from the refund path. |
| Any | INCOMPLETE_DEPOSIT | Preserve prior facts and use the existing review-required disposition; it grants no refill or replay. |
| Any | Wrong operation/deposit/request/recipient binding | Reject as a correlation failure; never apply it to this operation or erase accepted evidence. |

Stop ordinary status polling after correlated SUCCESS or REFUNDED; continue missing
canonical observations and preserve any review requirement. FAILED keeps ordinary bounded
status polling/backoff so a later REFUNDED can be observed. This reports provider progress,
not permission to close a refund. Apply the compatibility table to late responses.

Current refund synchronization records confirmed returned outputs independently of the
swap phase; preserve that separation for successful excess returns. For failed swaps,
`wallet recover` retains `refund_requires_review`. Shielding is an explicit operator
command that prepares a separate transaction. This delivery adds no automatic failed-swap
refund closure or shielding. Reviewed failed-swap disposition must identify the original operation, confirmed source,
exact returned outpoints/amounts, outstanding destination obligations, and any separately
prepared shielding operation/fees and its canonical outcome. Partial failed-swap returns or missing
correlations remain unresolved; neither returned principal nor REFUNDED proves all these
facts. A future automated closure feature needs its own predicates and authority review.


The `QUOTED -> PREPARING` intent is the preparation ownership boundary. Serialize it with
readiness publication: if `D` commits first, no new intent is accepted; if intent commits
first, the already accepted owner must drain and report exact effects. Check this boundary
at the actual treasury-owner entry point, not merely in a scheduler snapshot. Never let
credit erase an accepted command or infer that an unacknowledged preparation did nothing.
The first submission of already prepared bytes follows the same existing submission
capability and authority checks regardless of the destination's current payment role.

Select actionable operations by these missing facts in bounded indexed queries, including
operations whose wallets are now ready, active, retained or retired. Apply the
[evidence/resource ownership contract](#evidence-and-resource-contract) for independent
reads and serialized mutations, with the shared destination-readiness primitive. Derive
each next action from committed facts; do not cache a second work-state flag. Tests must
assert the next actual worker/backend call and forbidden calls for each row, not merely
the displayed phase or eventual completion.

Audit every consumer of `funding_jobs.COMPLETE`. Remove the aggregate USDC allocation
check in `src/rotation/store/allocation.rs` and daily treasury-spend enforcement rather
than adapting their carryover predicates to destination readiness. Remove configuration,
summary, help and test requirements for those ceilings. Do not preserve a lifetime-history
scan on the admission/preparation path merely to enforce a removed policy.

Keep the existing source journal and atomic reservations for actual ZEC input and fees.
Quote persistence creates no spending reservation. Read-only treasury readiness precedes
proposal mutation and reservation; concurrent jobs compete for actual available source
funds at the existing serialized owner. A failed read or sync is not evidence of insufficient
funds, and absence of a result is not proof of a clean preparation boundary.

Apply the [completion predicates](#financial-completion-and-source-accounting-predicates)
and [closure/reauthorization contract](#disabled-slots-and-same-job-recovery) to source
reservations. Historical gross allocations remain accounting under the configuration
contract; qualification permits and cumulative run charges remain authoritative.

Audit `ALLOCATED`/`COMPLETE` consumers in source submission, recovery and status against
[phase dispatch](#funding-phase-dispatch). A consumed slot can have a new replacement
while its previous operation still reconciles, so actionable queries must follow exact
operations independently of current roles/claims. Apply the shared
[retirement eligibility](#evidence-and-resource-contract) predicate to remaining destination
obligations.

The [role/claim table](#reserve-consumption-and-refill) owns consumption and vacancy
transitions; the [allocation contract](#independent-vacancy-allocation) owns new fills.
Payments can consume the other reserve while one replacement is allocated. Keep vacancy
claims unique across callers/restart and preserve disabled allocations under the
[disabled-allocation contract](#disabled-allocations-and-partial-credits). There is no demand
copied between generations or post-failure provider call/repair acquisition.

### Retirement during reconciliation

During ordinary reconciliation of an enabled group, independently of `auto_fund`, evaluate
eligible retained wallets as complete fresh observations become available. Retire those
whose max(confirmed, latest) remainder is within the explicit tolerance. Use bounded
existing reconciliation work; no inventory threshold, sorted retirement queue, global
minimum search, extra scan or dedicated timer is needed. Failure to retire a wallet
cannot obstruct allocation or payment from other eligible wallets.

Each retirement transaction, under the group gate, rechecks the retained
role, ownership, current tolerance, evidence and absence of relevant unresolved work,
then records retirement and changes the role atomically. No network I/O or allocation
occurs in this transaction. Concurrent admission cannot reserve funds between the
eligibility check and retirement. Check the initialized retirement tolerance and group
enablement; shutdown does not carry selected candidates into a reopened runtime as
retirement authority. Reopening with auto-funding disabled still permits eligible
retirement under the current tolerance and qualification authority. Never retire
active/reserve wallets or wallets with
unresolved authorizations or destination-funding obligations. A zero tolerance
requires both observed balances to be zero. The manual retirement CLI is deferred.

Reconciliation, rather than a seller receipt or a one-time settlement event, owns this
pass. It handles canonical settlement, expired-unused authorizations, wallets becoming
retained after settlement, and restart. A wallet with other outstanding authorizations
remains ineligible. The pass uses the existing cadence and evidence publication; add no
new timer or settlement callback. It may run without a vacant slot or when funding
treasury funds/permits obstruct funding, provided qualification permits the retirement
mutation. `auto_fund=false` pauses replenishment, not retirement. Removed/disabled groups
receive no automatic retirement.

Retirement excludes the wallet from payment selection and routine balance refresh.
Keep keys and immutable accounting, and continue separately owned outstanding source
accounting. Unexpected later external credits do not reactivate it or cause periodic
retired-wallet scans. Explain that the recorded remainder is an observation, not a
promise about future credits or a deletion of assets.

The configured tolerance is permission to give up automatic use of that remainder to
reduce ongoing observation work. Keep zero as the default; positive sub-cent tolerances
are explicit operator choices, not conclusions inferred from current provider prices.
No replacement-permission preflight, retirement/allocation exchange, age/price heuristic
or cumulative-loss cap is needed. Focused eligibility, race and polling-exclusion tests
suffice. Measure accumulation and observation load before making performance claims or
changing the default tolerance.

### Independent vacancy allocation

Allocation validates the current configured vacancy and funding authority atomically
with its unique claim and wallet/job creation. Existing retained wallets, their balances
and retirement eligibility never gate allocation. Reuse an existing incomplete allocation
for that slot rather than creating another. Bound acquired work through the existing
funding owner; source spendability and reservations remain authoritative at preparation.

When both slots are vacant and eligible, allocate small before large. Initial active
allocation still comes first for a new group. Use this fixed order without affordability
estimates, dynamic priorities or fallback scheduling. Small's claim does not exclude
large's claim or require small funding to finish before large becomes actionable. With
no wallet-count ceiling, small-first cannot consume a final allocation slot; no new
large-first policy or demand scheduler is needed to resolve that former obstruction.

Treasury insufficiency preserves the allocated wallet/job/claim and ordinary backoff or
recovery at the actual preparation boundary. Do not resize, release or replace that job
merely to try another amount. Small-first does not guarantee affordability or override
an existing job's ownership. Additional treasury funds permit ordinary work
to resume under existing readiness/recovery rules.

Other independently eligible jobs retain ordinary scheduling. Allocation order does not
promise source-reservation priority: when two already allocated jobs compete for funds,
actual quote input, proposal fees and serialized treasury reservations govern preparation.
Do not introduce an allocation-time price estimate, budget check or alternative source
reservation path. Preserve exact claims, transactions and recovery history throughout.

A wallet that is still in a ready slot has never paid through this application.
Managed Base keys are exclusively spent through this journaled path. Consequently
ordinary payment activity cannot produce a partially spent ready wallet needing repair.
Canonical funding must verify the accepted credit before marking ready. Unexpected
external depletion or inconsistent ownership is reported explicitly; it does not
silently authorize a repair job. Legacy ready-wallet migration is handled below.

## Startup, configuration changes and recovery

New groups eagerly initialize a small-funded active plus configured reserves under
normal funding authority. Serving readiness requires one owned eligible funded default
payer, including the read-only reserve fallback while the active pointer is absent.
Inventory readiness describes current desired occupancy: an eligible funded active
and a ready wallet in every currently configured reserve slot. Consuming a reserve makes
inventory incomplete until its replacement is ready. Neither readiness fact implies a
fresh balance sufficient for an arbitrary payment. Historical completion of initial jobs
and completion of explicit bootstrap's captured work are separate facts, not inventory
readiness flags. A failed or slow reserve does not block serving from funded capacity.
Report partial inventory honestly and preserve every accepted job. Initializing a wallet,
inspecting configuration/catalogs or registering a source never initiates this funding.

Apply the same serving-readiness rule to new, existing and migrated groups. Keep an
existing eligible funded active pointer. The initial active allocation retains its
original assignment until credit: if no active exists then, make it active; if an actual
reserve payment already established an active, its later credit becomes retained. The same rule applies if eligible retained inventory
reconstructed an active during recovery.
The two initial small jobs need no claim transfer or completion-order rebinding.

When active is absent, an existing eligible retained wallet may reconstruct the active
pointer without allocation, choosing earliest sequence then wallet ID. Otherwise resolve
the unsigned default identity from eligible ready reserves by configured target,
small-before-large, then wallet ID. This is read-only selection, not an active role or a
second persisted pointer. A ready reserve suffices for serving readiness and free calls
while the initial active job finishes. An outstanding unfunded allocation is not eligible.

Choosing a reserve for an unsigned request never vacates it, changes a generation or
creates refill work. If the initial active becomes ready during that request, preserve
the captured request identity; authoritative admission checks its current eligible role.
A free response leaves inventory unchanged. A journaled payment from the selected reserve
performs ordinary atomic consumption, retaining any active that now exists. Only that
real payment creates its vacancy/refill obligation. Unknown commit outcomes retain any
committed transition as specified by atomic admission; they do not authorize replay.

Without application payments, every completion order, including large-first, must reach
the configured inventory with exactly the original initial
jobs and targets. No extra allocation, quote or claim rebinding is needed. If a reserve
actually pays before initial funding finishes, subsequent refill is ordinary additional
work under current funding authority; status distinguishes that work from the
initial target. Preserve original jobs, accepted terms, bytes and recovery ownership
through restart. Never return a consumed or retained wallet to a reserve to repair
inventory, nor displace an established active merely because another job finishes.

Startup consumes the funding owner's existing actionable-job queries and typed dispositions;
it does not implement another funding state machine or forecast quote affordability. For
each required group, an eligible funded payer permits serving. Otherwise continue accepted,
actionable or temporarily waiting work. Return a typed startup failure only when the same
funding queries account for every relevant initial allocation/reserve path with a concrete
operator obstruction and none can currently proceed. Include enabled vacancies under the
ordinary allocation rules; an empty job query alone does not establish impossibility.
Recheck committed state before returning and report the affected jobs and fixed reasons
(funding disabled/denied, attempts exhausted, invalid policy, repair or verified insufficiency).

Insufficiency for one quote is a disposition for that job, not a group-wide forecast. A
blocked large allocation must not stop an affordable or still-actionable small allocation,
in either completion order. An outstanding source reservation is a wait for its owned
resolution, not proof of exhaustion. Once all relevant paths have concrete obstructions
and no accepted operation is still awaiting an outcome that could resolve them, report them
without waiting for a hypothetical treasury deposit. An unsigned quote alone is not such a wait.
Close new admission and drain accepted work safely before returning; this does not
abandon signed/prepared work or grant another attempt on restart. Keep observation and
canonical recovery obligations durable for the next owner.

A transient RPC/status failure, slow progress, or an accepted operation awaiting canonical
outcome remains a wait under those same dispositions and ordinary backoff, without a
whole-startup deadline. Report waiting versus operator obstruction separately from inventory
readiness.

Current `serve` calls `Deployment::bootstrap`, whose supervisor stops and drains workers
when its completion monitor returns. Merely changing that monitor's predicate would still
make serving wait for an unrelated accepted funding step to drain. Instead, initialize
one owned financial runtime for serving and retain its store, treasury/funding workers,
managed groups and signing ownership through readiness, catalog discovery and listener
startup. Reaching serving readiness is a notification, not worker shutdown. Transfer that
same owner into the running deployment; do not close/reopen it or start duplicate workers
at the boundary. Explicit `wallet bootstrap` retains its finite command supervisor while
sharing allocation, credit and readiness transitions. No legacy startup mode is needed.

Serving never waits for reserve refill, retirement or unrelated captured maintenance.
Adding a large slot creates a background vacancy, not a new startup barrier. If catalog
loading or listener startup fails after readiness, the startup owner closes new admission
and drains accepted financial/signing work before teardown. Preserve the same ownership
on caller cancellation. Expose serving and current inventory readiness separately.

Explicit `wallet bootstrap` uses shared transitions but captures the initial jobs,
current vacant slot generations and existing replacement jobs at command start.
Wait for those owned allocations to reach canonical credit, or return typed incomplete
for an operator-policy obstruction, including disabled funding, denied authority,
exhausted attempts or invalid current funding parameters. Report the incomplete captured
slot/job and fixed reason; do not allocate around it. Retained inventory size is never
an obstruction. Drain already accepted work before returning. Do not chase subsequent consumption
generations.
There is no special both-empty pair repair. Cancellation preserves accepted work;
rerunning finds the same jobs rather than creating replacements. auto_fund and normal
funding authority remain required, as in the existing command.

Configuration is captured at initialization in the current deployment model. Changes to
targets, enabled slots, caps and auto-funding policy take effect on a supervised reopen;
do not add hot reload, configuration generations or mutable-policy dispatch machinery
solely for this feature. Test those changes across restart. Commit-time checks remain
required for genuinely changing state: balances, roles, claims, source reservations, operation
admission and qualification permits. Revalidate configuration during I/O only if a future
supported runtime actually allows it to change.

Target/output-ceiling changes apply at the next proven-clean quote boundary, including
existing allocations that have no outstanding financial work. Preserve original
allocation history and immutable accepted quote/transaction terms. Already quoted work
uses the guarded refresh or recovery row below; uncertain work must pass ordinary
recovery before becoming clean; configuration loading alone never rewrites an accepted
operation. A previously funded ready wallet need not match the new target to remain
payable; report actual capacity. Never top it up or retire it solely because
configuration changed.

### Funding terms and current policy

Keep original allocation intent, immutable accepted operation terms and current funding
permission separate. `Backend::policy` currently reads an output ceiling from current
configuration while `FundingJob` stores its target. At a proven-clean quote boundary,
select the current enabled slot's target and ceiling together under the serialized owner.
Preserve original allocation intent in audit history and record the pair selected for the
quote attempt. Do not maintain an independently frozen next-quote policy that requires
operator recovery merely because configuration changed. An initial active-designated job
uses current small configuration even if another funded wallet has since become active.
Add no successor-job lineage or second budget ledger.

Use three distinct meanings in existing records rather than overloading `target`:
original allocation intent (immutable history), the target/ceiling pair chosen for the
current unsigned quote attempt, and the exact accepted operation terms (immutable from
quote acceptance, including any accepted bridge-minimum increase). Persist the quote
atomically through the existing owner; reserve spending only at the existing preparation
boundary defined in the source-accounting predicates above. Rejected unsigned quotes grant no
accepted-operation authority. Recovery archives accepted terms rather than
rewriting them. A configuration change affects only a later proven-clean quote boundary;
late destination credit and source/refund reconciliation always use their original
accepted operation. Do not infer historical ceilings from current configuration.

| Job state | Terms and permitted action |
| --- | --- |
| Disabled allocation | Preserve the same wallet/job/claim; no new quote acceptance, reservation or preparation until configuration is re-enabled and current funding authority permits it. |
| Allocated, no outstanding accepted operation or preparation | With a current owned claim, at the next quote boundary, prove no preparation intent, outgoing operation or outstanding source reservation would be overwritten; atomically record the current enabled slot's target/ceiling pair on the same job before quote I/O. No explicit recovery command is required solely to adopt configuration. |
| Quoted, no outstanding preparation intent, outgoing bytes or source-reservation record | Reuse `refresh_unprepared_quote` when current parameters make the quote unsuitable or its ordinary validity margin requires refresh. Archive the immutable quote, assign a new operation ID on the same job/wallet/claim and retain the durable refresh marker. No transaction recovery attempt is consumed. New work still requires current authority. |
| Quoted with preparation effects or an uncertain boundary | Guarded refresh is unavailable. Ordinary recovery must prove safe cleanup and preserve cumulative recovery history before a new quote; configuration changes do not release reservations. |
| Preparing or preparation uncertain | No policy substitution, fresh quote or replacement preparation. Existing typed recovery must prove the next transition safe; absent bytes/results do not prove no work occurred. |
| Prepared, submitted or destination outcome unresolved | Preserve exact terms, bytes and obligations. Canonical completion, expiry or authorized refund recovery governs; configuration changes grant no new send or quote authority. |
| Canonically expired and eligible for recovery | Archive/release only through existing canonical recovery. Once clean, the next ordinary quote uses current slot policy on the same job; preserve cumulative attempts. |
| Complete | No request update or top-up. Preserve actual credit and history; later reserve consumption creates ordinary new work. |
| Slot/group disabled, or safeguards unavailable | No new quote. Continue independently permitted observation/reconciliation of original work. Restore exact owned configuration to supply current policy. |

Quote acquisition checks enabled funding policy, qualification permission, request
parameters and applicable durable attempt limits. Quote acceptance validates the returned
terms against output and ZEC/overhead safeguards. Fresh spendability and actual input/fee
reservations belong to the existing treasury readiness/preparation owner before mutation.
Do not add a spendable-balance estimator, allocation-time check or pre-quote treasury sync;
the quote supplies the required input, and preparation supplies the authoritative fee/
reservation checks. Each boundary retains its existing current-authority checks. Selecting current quote parameters grants none
of these permissions and never resets attempts. Revalidate current authority before
accepting a quote if policy can change during I/O; do not mutate that quote to make it
fit. Configuration changes cannot rewrite accepted transactions. Reporting old operations
remains possible without current permission to start new work.

For example, after a rejected unsigned $10 quote encounters a $12 bridge minimum,
configure the desired target/ceiling and resume ordinary serving/bootstrap. If the job
is proven clean, its next quote uses that pair automatically. If there is an outstanding
unsigned accepted quote with no preparation effects, use the matrix's existing guarded
refresh automatically. Reservations, preparation intent or uncertainty require ordinary
recovery first. An unsuitable quote is archived, never edited in place.
`wallet recover` remains a guarded recovery/eligibility pass: it fetches no new quote,
allocates no wallet and submits nothing. Exhaustion blocks another funding attempt, not
proven-safe closure; another attempt requires explicitly raising `max_attempts` under
the closure/reauthorization contract below. Other configuration changes cannot bypass
history. Uncertain preparation or unresolved failed-swap refunds remain review-required. Repeated
recovery is idempotent.
Test clean allocation, rejected unsigned quotes, guarded quote refresh, canonical
expiry and uncertain preparation separately. A policy-driven refresh runs once for an
unsuitable saved quote on reopen; it does not create a same-call quote-until-success loop.
Retain existing unsigned backoff. Exhausted transaction recovery history stays exhausted;
refresh cannot replenish it or make an otherwise unauthorized preparation eligible.

### Disabled slots and same-job recovery

Removing large configuration stops new large fills. Its ready wallet becomes retained
without payment or funding. An allocated job preserves its exact wallet/job/claim and
financial ownership. No new quote/reservation/preparation
is allowed while that slot is disabled; accepted uncertain/prepared/submitted work retains existing completion/recovery
obligations. Canonical credit while disabled produces retained capacity in the enabled
group, not a stranded disabled wallet class. Re-enabling reuses the existing incomplete
allocation under current authority;
once completed, a later vacancy may fill once under current authority and the
clean-closure rules below, without inheriting a closed predecessor's attempt count.
Do not create a new job alongside an old disabled allocation.
Small configuration is required; removing the entire group disables its admission and new funding while retaining history.

Separate safe closure from authorization to start another attempt, within the existing
operation/job records:

- Closure requires the applicable guarded-unprepared or canonical-expiry proof, preserves
  exact history, and releases only reservations that proof permits. Attempt exhaustion or
  disabled new-funding policy does not prevent this cleanup. Qualification mutation
  restrictions still apply. Uncertainty, unresolved failed-swap refunds and financial conflicts cannot
  be closed merely because further attempts are forbidden.
- Reauthorization separately checks current funding/claim authority and durable
  recovery history. Closing an operation neither assigns a fresh operation ID nor returns
  it to `ALLOCATED` for dispatch. Preserve retry eligibility/counting in existing recovery
  records, including across restart; repeated closure is idempotent. Unsigned quote refresh
  retains its existing exclusion from transaction recovery counts.
- An incomplete disabled job retains its claim and cumulative recovery history. Safe
  closure of an attempt does not complete an unfunded allocation or create a vacancy.
  A later ordinary fill after a funded reserve is consumed is a distinct job; it does
  not inherit predecessor attempt limits. Do not add a slot-wide recovery ceiling.

`max_attempts` limits recovery of the same funding job, not seller API requests, pricing
probes, all bridge quotes or lifetime fills of a slot. Raising it permits further recovery
of that same exhausted job. Proven clean closure and a separately authorized new job do
not require that increase. Additional bridge quotes remain subject to the existing
current-authority and guarded refresh/recovery rules. Existing observation and accepted-work reconciliation continue.

Factor the guarded archival/cleanup part out of `recover_unprepared_funding`,
`refresh_unprepared_quote` and applicable canonical-expiry recovery. Their current combined
cleanup/reset behavior cannot serve as closure-only machinery: the unprepared helper
assigns another operation ID and resets the phase. Reuse shared proof/archival logic;
invoke reset only after separate authorization. The phase dispatcher selects these actions
from committed facts, without adding a parallel state machine.

For exhausted work, raising `max_attempts` authorizes another recovery of that same job;
it is unnecessary for proven-safe closure. `wallet recover` may finish eligible cleanup
and report later-work eligibility without starting funding. Invalid output ceilings follow
the current-policy matrix. These actions never override uncertainty or refund review.
Exhaustion alone does not release an allocation claim; incomplete disabled allocations
retain their exact ownership under the disabled-allocation contract. An unused destination cannot be replaced merely to retry funding.

### Disabled allocations and partial credits

Slot/group disablement preserves an incomplete allocation's wallet/job/claim. Configuration
already prevents new funding, and the allocation does not consume a scarce count slot.
There is no empty-allocation release, unclaimed state or reclamation transition. Re-enabling
resumes the same job under current authority and durable recovery history. Repeated
remove/re-enable cannot grow wallet history or reset attempts. Preserve accepted work
and independently permitted observation/recovery while disabled; a captured incomplete
bootstrap job remains incomplete. Use existing wallet/job/slot records and add no pause flag,
queue, timer or abandonment CLI.

Continue ordinary bounded balance observation for disabled allocations so external funds
are recognized. A fresh result uses current ownership and runtime evidence; it never
restores obsolete claims or authorizes new funding. Resolve credit and job closure
atomically through the shared evidence/claim owner.

Sub-target external credits remain on the original allocation in both enabled and
configuration-disabled states. Preserve the same wallet/job/claim, original terms and
recovery history. They do not record readiness, change roles, close the job, refresh a
quote, reset attempts or create a successor. Enabled allocations continue their first
funding operation only under current authority; disabled allocations retain independently
permitted observation and safe recovery. Re-enabling resumes the same allocation.

Automatic clean partial detachment is deferred from this delivery. Sub-target external
funds are unavailable through ordinary payment selection until full readiness or an
incident-specific reviewed intervention. This limitation concerns incomplete allocations,
not the retained remainders of previously funded wallets. This full-readiness gate is an
implementation limitation of allocation roles and completion accounting, not an inherent
payment-safety requirement. Independently verified partial capacity could safely fund an
affordable authorization under ordinary ownership, liability and cap checks, but permitting
it here would require readiness accounting that recognizes intervening debits instead of
waiting for the original balance threshold. Defer that change with partial detachment;
do not describe the funding target as payment authority. State the limitation in configuration
and wallet guides. Neither unsolicited dust,
treasury insufficiency nor `auto_fund=false` authorizes claim churn. Add no partial-detachment
outcome, threshold, scheduler, abandonment command or automatic recovery mode.

Removing large during partial bootstrap drops it from desired inventory, not from accepted
work or explicit bootstrap's captured obligations. A funded payer can still serve.

Resolve removed-group jobs by durable group identity rather than generated names or a
fallback-to-one attempt limit. Durable attempt history and accepted quote/journal terms
remain authoritative. Read-only observation and proven-safe closure use their existing
proofs and qualification permissions, independently of an unused recovery attempt.
Reauthorization requires restored enabled configuration and its current `max_attempts`;
historical policy values are audit information, never fallback authority. Do not add a
persisted last-explicit attempt ceiling solely to authorize observation or safe closure.
If current required safeguards or permissions are unavailable, block only the dependent
action and continue independent read-only reconciliation. Missing legacy recovery policy
is not permission to invent it. Restoring the exact profile can supply current policy.
Unsigned quote outages consume no transaction recovery attempts.

The funding-policy matrix governs disabled configuration and exact-operation recovery.
Recovery/status commands never fill vacancies or manufacture new funding authority.
Preserve zero-new-funding restrictions before setup and every supervised reopen, and
old qualification runs as observation-only; current quote-policy selection cannot convert
retained evidence or old permits into new execution authority.

## Retirement accounting and administration

Record each nonempty retirement in its role-change transaction: exact wallet/group,
confirmed/latest amounts and anchors, accepted maximum, observed bound, reason and time.
Use a sanitized warning for committed automatic nonempty retirement. Confirmed-zero/
latest-positive is nonempty even though its recorded confirmed amount is zero.

Report the sum of exact confirmed amounts as recorded balance at retirement, separating
automatic/manual and historical unknowns. Show policy bounds/latest observations
separately; do not add both to the total. This is neither destroyed funds, current
retired holdings, API cost nor treasury replenishment. Later credits preserve history
and do not automatically reactivate a retiree. Keys remain available in backups.

The manual `wallet retire` CLI is deferred. This delivery adds no command, parser/help
surface or manual-retirement fixtures. Automatic retirement under the configured tolerance
remains in scope. Preserve historical retirement records and their known provenance;
deferral does not erase or relabel prior manual/unknown outcomes. Exceptional manual
intervention remains incident-specific reviewed work, not a new administration subsystem.

## One-time configuration and database migration

### Deliberately narrow support and recovery policy

This is an explicit product scope decision: there is one known user, also the developer
and operator. Other possible users are bleeding-edge developers. We do not promise a
self-service upgrade path across historical deployments. Delivery supports one identified
source schema and the known operator's inspected state shape, using a stopped-service
one-time developer utility if conversion is needed. No production migration command,
legacy runtime, option aliases, broad version matrix or generic migration framework.
Future reviews must not expand this scope for hypothetical users or legacy states.
A broader migration product requires a new explicit requirement.

The intended exceptional recovery is operator/agent-assisted investigation of the actual
incident, followed if necessary by a narrowly scoped custom tool written at that time.
The operator accepts that this may require downtime and developer work. Do not prebuild
wallet abandonment, Base evidence assessment, fork repair or general historical recovery
commands as insurance for this migration. Preserved wallet keys are essential to that
recovery; preserve accepted transactions, liabilities and accounting too, because keys
alone cannot establish which work may safely be repeated. Recovery is not guaranteed by
a backup and never grants permission to repeat an uncertain transaction.

### Conversion contract

Before implementing conversion, inspect only the known deployment's schema and relevant
state metadata, without printing keys or seeds. Record the supported source schema and
state preconditions, then create an equivalent unfunded synthetic fixture. Do not inspect
live secrets or migrate real state merely because this plan is being implemented.
Update local configuration manually with an explicit reviewed cap, reserve targets and
per-operation safeguards; remove the obsolete aggregate-budget settings. Preserve
user-owned local files. No automated cap translation or migration projection/reporting API is required; the fresh `2*S + L` formula is not upgrade cost.

Stop serving and acquire exclusive state ownership. Make a consistent backup containing
all wallet/key material, required external key files, database/journals and treasury state.
Use the repository's Rust developer-utility convention for the one-time conversion;
record its exact invocation and supported source schema/state in the operator notes.
Keep the utility outside the production wallet command tree. It rejects unsupported or
ambiguous states before mutation and recognizes an already-converted database without
running the transformation twice. No new financial authority arises from conversion.

Prefer bringing existing work to the supported state using ordinary observation/recovery
before conversion. If the known deployment must retain an unresolved operation, support
and test that exact state while preserving its bytes, identity and obligations. Do not
require forced expiry, fabricate settlement or delete work to meet a migration precondition.
Unexpected state is a diagnostic for developer investigation, not a reason to grow a
universal legacy inference engine during feature delivery.

Use one transactional schema conversion. Preserve group/wallet IDs, key ciphertext/AAD,
addresses, network identity domains, retired holdings, accepted jobs/transactions, payment
liabilities, source/refund records and cumulative accounting. Keep the active wallet; map an
unambiguously eligible READY wallet to small reserve. Other valid previously spendable
wallets become retained. If the actual source state has allocated jobs, map their exact
ownership explicitly in its fixture; reject conflicting or unexplained ownership. Do not
invent jobs, vacant claims or historical authority. Conversion performs no allocation,
signing, retirement or transfer.

Mark old conservative balance snapshots unavailable for new admission/retirement until
ordinary complete verification. Derive debit watermarks from preserved exact USED records;
keep EXPIRED_UNUSED anchors for canonical compatibility. Missing proofs needed for the
known conversion are a preflight diagnostic, not guessed data or a historical repair mode.
Validate schema/foreign-key integrity before commit. Test rollback/reopen and compare
keys/AAD, identity, journals, operation correlations and source reservations/accounting, not just row counts.
For the supported source fixture, map old destination-completion evidence to its exact
operation and compare pre/post-conversion source reservations and cumulative accounting.
Prove old gross totals and UTC rollover no longer gate ordinary funding; preserve
qualification charges and original historical reports. Establish each wallet's compact
financial checkpoint from exact preserved wallet-scoped liability-release/accounting
support or verified bounded initialization before admission. Preserve admission
support for audit without converting fully reserved payment journals into permanent
canonical dependencies. Do not blindly copy the old
pool-wide anchor into every wallet and perpetuate unrelated dependencies. If the known
source fixture lacks the required ownership/proofs, reject conversion before mutation.
Remove legacy ADMITTED rows only after exclusive ownership proves no prior signer or
accepted store job remains; retain every signed liability. The converted schema/runtime
creates only atomic signed admissions. Do not guess missing evidence.
Exercise restored fixture keys by deriving the same addresses without exposing secrets.
Keep the converted schema above the current mutating reader's supported `user_version`
(currently 12); retain its early unsupported-version rejection. A historical executable
matrix and compatibility for obsolete read-only output are not delivery requirements.

After conversion, inspect status and verify balances before ordinary authorized serving.
Keep the source backup, but never restore an obsolete financial snapshot after subsequent
spending to undo an incident. Preserve the latest journals for any later custom recovery.
No downgrade workflow is promised. The utility can be removed after the actual migration;
retain the conversion fixture and operator record of the source/target versions.

Remove the obsolete `wallet pool` allocator with an actionable error directing callers to
managed configuration and `wallet bootstrap`. Audit `Treasury::ensure_pool` and remaining
allocation entry points so all new allocations use the same authority/claim checks. This is a
runtime API cleanup, not a new migration command. New public status fields must describe
the new runtime accurately; old field aliases and a versioned migration-report API are
unnecessary. Historical qualification evidence remains immutable and observation-only.

## Verifier implementation design and fixtures

Share one private verifier between admission and maintenance, with immutable results
applied through the existing owners and store transitions. Separate observation comparison
(scope, anchor order and amount conflicts) from network verification and the payment,
retirement and financial-commit predicates. Add no certificate service or query registry.

The [checkpoint contract](#linear-checkpoint-verification) owns the financial predicates
and advancement rules. The following algorithm is illustrative: certificate layout,
cache updates and branch structure are implementation choices. Test financial outcomes,
scope and races rather than requiring a particular representation or header count.

Let `C = (height, hash)` be the wallet's current financial checkpoint and `Cq` the
checkpoint captured for a query. Compatibility uses the existing trusted-RPC model:
exact-height canonical hash rechecks on one selected endpoint and EIP-1898 state reads.
Verified continuity carries earlier financial facts through later checkpoints without
an ancestry graph. It does not establish independent consensus or irreversible finality.

A private verified result carries its exact wallet/asset/network identity and checkpoint
`Cq` captured from this runtime, its
process namespace and relevant invalidation generations, its network/endpoint/depth scope,
and the exact anchors/facts it verified. Let `A` be its confirmed balance anchor or exact
nonce/accounting proof anchor. A balance result also retains its latest anchor and both
amounts with the original validity time. These are bounded value fields, not references
to a registry of active queries. No caller may manufacture a verification certificate.

1. Under the group gate, capture `Cq`, the wallet's debit watermark, applicable generations
   and the current same-wallet observation, if any. Release the gate before RPC work.
2. On one endpoint, validate the chain and read the required state pinned to exact hashes.
   Recheck `Cq` (when present), `A`, and a balance result's latest anchor at their exact
   heights. Verify captured same-wallet anchors only where needed to detect a replacement
   not already covered by a verified certificate. The accepted checkpoint must cover
   the debit watermark through its established lineage; verify that invariant locally
   and enforce the watermark as the minimum admission-balance height. Do not separately
   requery its header when checkpoint verification already proves that history. Likewise,
   omit captured confirmed-anchor rechecks when established checkpoint coverage suffices.
   Height alone establishes neither relationship. Deduplicate remaining identical anchors.
   All required checks must finish; never combine successful fragments across endpoints.
3. At publication/use/financial commit, first check scope, invalidation generations and
   conflicts. Use a newer compatible complete saved observation when available. For the
   remaining result, canonical compatibility with current checkpoint `C` is established
   by any one of these cases:

   | Case | Why it is sufficient |
   | --- | --- |
   | `C == Cq`, including both absent | The verifier checked the current checkpoint and `A` together. |
   | `Cq` exists and `A.height <= Cq.height` | The verifier established `A` on the chain covered by `Cq`; the store's uninterrupted checkpoint lineage carries that coverage through later advances. Heights alone without this certificate are insufficient. |
   | `A == C` by both height and hash | The exact proof anchor is the current accepted checkpoint. |

   Otherwise the result needs an anchor-only compatibility recheck. A common older
   checkpoint does not prove that two newer anchors are on the same branch. Equality of
   publication sequence numbers is never a condition for acceptance.
4. For that recheck, capture current `C` and generations under the gate, then recheck this
   checkpoint and the result's exact original anchors on its original endpoint outside
   the gate. For balances include the original latest anchor. Preserve amounts and original
   validity; checking headers cannot refresh balance age. The new certificate uses the
   checkpoint just checked as `Cq`. Reapply step 3 at commit. Once `Cq` covers `A`, further
   compatible checkpoint advancement needs no further recheck. If a newer-than-`Cq` result
   loses another race, use already compatible evidence or defer/fail as specified below.
   Endpoint fallback during this recheck requires complete fixed-payer acquisition on the
   new endpoint; it cannot certify state copied from a different endpoint by headers alone.
5. In a checkpoint-bearing disposition commit, retain `C` if it already covers `A`;
   otherwise advance it to `A` atomically with the exact financial fact and supporting evidence. Reject equal-height
   different-hash anchors before this step. An older compatible proof records its fact
   without regressing `C`. Payment admission checks compatibility but never advances `C`.
   A USED resolution also advances only that wallet's debit watermark; EXPIRED_UNUSED never raises the balance floor.

This protocol is inductive: a result either verified the current checkpoint, is already
covered by an accepted predecessor, or exactly matches the current anchor. Only the first
case can extend the lineage with a newer anchor, after verification. Another wallet's
advancement requires no recheck, even for a result not yet committed. A result not yet
covered by its own wallet's lineage may need the single bounded recheck; this is a stated availability tradeoff, not
proof of a reorg or permission to discard a balance. Keep such observations until normal
expiry/eviction, but do not treat them as admission authority until compatibility is proved.

Carry certificates with immutable cache entries/results. After a checkpoint-bearing
disposition commit, the exact supporting observation may receive a certificate covered by the newly committed
checkpoint without extending its lifetime. Captured older handles may use that equivalent
saved observation; they cannot infer coverage from height alone. Other cache entries need
no eager rewrite, and checkpoint advancement never increments invalidation generations.

Deduplicate identical anchor checks and reuse established checkpoint coverage of the
USED watermark; do not add a recurring duplicate header read. Missing coverage during
conversion requires verified initialization before admission. Query work must remain
independent of lifetime resolution count and other wallets' observations. No fixed
header count is an acceptance predicate or a new configurable limit.

A missing compatibility relation defers historical proof application without releasing
liability or blocking other ready work. Background publication may use one bounded
recheck or defer to its ordinary pass; foreground work uses the invocation-wide refresh
allowance. Such a race alone establishes no integrity failure or financial invalidation.

Confirmed history and latest balances have different roles. `C` covers confirmed anchors;
never advance it to an unconfirmed latest tip merely to certify a cache entry. A balance's
latest state was independently verified when acquired and still constrains admission by
min(confirmed, latest). Known latest replacement invalidates the complete observation
under the failure table. A later checkpoint does not turn an unconfirmed tip into a
financial finality requirement. The USED watermark plus unresolved liabilities prevents
release of exposure from making a pre-debit confirmed balance spendable.

## Validation and acceptance

Use temporary state, public unfunded deterministic keys and local protocol fixtures.
Tests must assert forbidden side effects as well as successful output. Harness deadlines
are not runtime policy. The independent financial state model should cover only wallet
roles, reserve claims, liabilities and funding/retirement transitions; keep hint
selection and resource scheduling in separate focused fixtures. Generate reproducible transition
sequences and check conservation against an independent model: credited holdings minus
canonical debits remain accounted across live and retired wallets; unresolved liabilities
are tracked separately and never counted as another debit; retirement transfers no funds
and creates no spendable treasury funds. Preserve history and these invariants across reopen.
Extend the existing store, funding, payment and middleware fixtures with the concrete
cases below; do not create a parallel test framework or duplicate the transition model.
For each row, take expected transitions and failure scope from the [contract map](#normative-contracts-and-implementation-scope).
The table lists regression inputs/assertions, not an alternative policy specification;
shared fixtures may cover several rows. Keep the quantitative workloads below as concrete
acceptance cases rather than repeating them in the implementation/documentation inventory.

| Area | Required regression coverage |
| --- | --- |
| Configuration and authority | Parser/help/output fixtures for the configuration contract, removed fields, rejection of the unimplemented max_live_wallets option, exact decimal boundaries, explicit unlimited cap, equal targets and zero tolerance. The same affordable payment works on every eligible role. Summed holdings never provide single-wallet authority. Old gross totals/UTC rollover do not limit funding; reservations, per-operation safeguards and qualification permits still do. |
| Roles and retirement | Exercise the role/claim and retirement tables through real store callers, including simultaneous claims, growing retained inventory, disabled funding and unresolved obligations. A free response/signing failure creates no vacancy. Test newer higher/lower retirement balances, dual-zero, settlement before/after retention and restart. Retirement excludes routine polling/payment selection while preserving keys/history and independently outstanding source accounting. Allocation and affordable payments proceed without retirement at any inventory size. |
| Small-first allocation | With both slots vacant, allocate small then large without waiting for small to finish. Both retain independent owned jobs under current authority; test when only small is affordable and when small is delayed while large can progress. Existing claims remain owned; insufficiency never causes resizing, claim churn or attempt reset. No retained-wallet count gates either allocation. |
| Ownership and atomic admission | Against the [admission contract](#atomic-admission-and-signed-journaling), inject cancellation, panic, lost acknowledgement and process exit at signing/enqueue/commit/dispatch. Assert atomic rollback and no pre-ack signature escape or duplicate signed submission. Exercise evidence-only, signature-only and combined expiry, including local replacement after an earlier RPC refresh, a rejected recovery commit, and unavailable fresh evidence after refresh exhaustion. Check fresh SDK nonce/exact terms, caller expiry, opaque annotations and definite rollback versus unknown outcome. Assert handoff resets neither allowance and ordinary post-handoff acquisition consumes no race refresh. Pause final validation and exercise every group writer plus independently obtained handles, including startup relay/warm/library callers. Assert ownership violations fail without routine recovery. |
| Lease prerequisite and allocation entry points | If the legacy fix ships separately, pause its signer after ADMITTED while actual funding-credit/reconciliation runs; only owned cleanup removes abandoned unsigned rows. Otherwise reproduce the original interleaving against atomic admission and prove query construction is read-only. Preserve signed journals on cancellation/restart. New runtime creates no unsigned leases. Every allocation entry point uses authority/claim checks; inspection, registration and obsolete wallet pool allocate nothing. |
| Evidence verification | Cover the [failure and ordering tables](#evidence-and-resource-contract): common-ancestor forks, covered/uncovered same-wallet anchors, fixed-depth ordering, supersession, amount conflicts and invalidated captures. Exercise USED versus EXPIRED_UNUSED, missing checkpoint initialization, other-wallet progress, eviction and reopen. Assert financial acceptance and liability outcomes without fixing certificate layouts or header counts. |
| Evidence recovery scope | Exercise replaceable/admission/readiness/retirement conflicts with intact financial support, and conflicts undermining released liabilities/accounting. Recertify intact history after one or multiple latest replacements; race that recheck with another invalidation and include a changed proof anchor and endpoint trust change. Repeat after eviction/reopen. Test actual TLS/malformed-response fallback classification, healthy fallback, all endpoints failing, cancellation and diagnostics; assert no TLS bypass, fragment mixing, fallback concealing contradictions, stale liability release or funding from disputed readiness. Verify unrelated wallets remain usable where the failure table permits. Race a demonstrated shared-endpoint invalidation against admission and liability release in another group, with that group gate already held; exercise both serialized store orders, stale queued commits and late query results. |
| History and resource bounds | Large pending and lifetime histories with few live wallets: captured keyset boundaries, arrivals, restart, slow/unavailable entries and deduplicated in-flight work. Assert independent completed-proof commits, exposure/journal equivalence, checked arithmetic, rollback and idempotence. Exercise actual foreground/history/balance/funding owners under supported load and saturation; detect full-status scans on operational paths. |
| HTTP and cover retention | Traverse over 256 wallet RPC keys while reusing a hot provider client; assert individual eviction, bounded cache size and survival of acquired handles. Cycle over `max_owners` completed cover owners. Hold a cleanup callback after counters reach zero; test all-owned saturation, cleanup, eviction/recreation and finite negative-cache retention. A retained failed entry stays failed across episodes; ordinary eviction permits requalification and a fresh bounded fallback choice without resetting process traffic limits. Exercise churn beyond the owner bound without tombstone growth. Check qualification/request counts and absence of duplicated or cancelled live work against the resource contract. |
| Selection, hints and handoff | Apply the [selection contract](#initial-selection-price-hints-and-admission) to multi-offer challenges, malformed alternatives, capacity/role ties, pending liabilities and reverse read completion orders. Cover cold-cache reserve acquisition and continuation of a captured pass after contention. Exercise actual hint learning, scope/revisions, eviction, variable prices and captured bindings. Check handoff bytes/identity/cover/destination restrictions and pinned middleware handler counts; signed failures and output-conversion failures never replay. Use the workloads below for concrete selection and request counts. |
| Startup and configuration changes | Exercise all two-job/three-job initial completion orders, including large-first with $2/$10 targets; assert the original jobs reach inventory without claim rebinding. Cover unsigned fallback/free calls, initial-active completion during a fallback request, actual reserve consumption, restart with prepared/submitted work, and catalog/bind failure/cancellation. Test affordable or actionable small funding with blocked large funding in both observation orders, job-local insufficiency, outstanding-source waits, enabled vacancies without jobs, all-path obstruction and transient failure. Verify startup uses funding dispositions and one continuous financial owner. Exercise disabled slots/groups and reopen-only policy changes. |
| Disabled allocations and partial credits | Disable/re-enable at preparation and credit boundaries. Exercise empty, one-micro-USDC and $3 credits to a $10 allocation with funding enabled/disabled, unavailable/conflicting evidence, full satisfaction and quote/readiness races. Assert the [disabled-allocation contract](#disabled-allocations-and-partial-credits), including that even an otherwise affordable payment cannot select a sub-target allocation in this delivery; preserve unchanged claims/attempt history and no successor or partial detachment. |
| Funding dispatch and recovery | For every [phase-dispatch](#funding-phase-dispatch), current-policy and provider-compatibility row, assert the next actual backend call and forbidden side effects. Reverse source/status/credit completion, spend before completion, race readiness/preparation and reopen. Test independent backoff, cancellation, closure with exhausted/disabled retry authority, same-job reauthorization and unsigned refresh. Keep the treasury actor idle with an unresolved outgoing operation: independent reads proceed, new preparation waits until canonical resolution. Include removed groups and preserved recovery history. |
| Returned input and failed-swap refunds | Under the [completion/accounting contract](#financial-completion-and-source-accounting-predicates), reverse return/source/status completion; test absent, early, partial and late successful returns after COMPLETE, spending, retirement, archival and restart. Include duplicate/ambiguous bindings and outpoints, known spent/shielded outputs, FAILED through REFUNDED and contradictory terminal statuses. Verify gross/principal/fee accounting and no duplicate credits, speculative return wait, failed-swap closure or replay. Through actual treasury sync, an old attribution-only failure must preserve its obligation while independent returns and another group's otherwise-valid funding progress; contrast canonical/input-ownership and storage failures requiring wider blocking. Exercise large wallet histories and the documented linear discovery pass. |
| Migration, backup and outputs | Use the known-state unfunded fixture only: rollback/reopen/repeat invocation, schema guards, exact keys/AAD/addresses, roles, claims, accepted bytes, liabilities, resolutions and source/refund history. Explicitly handle legacy source-pending COMPLETE records. No network/spending authority arises from conversion. Pin versioned output meanings and immutable observation-only qualification reports; restore fixture keys and derive identical addresses. |

These rows select regression boundaries, not another transition specification. Shared
fixtures may cover several rows. Keep the contract tables authoritative; avoid asserting
incidental helper counts, struct layouts or duplicated phase logic unless a documented
resource or financial invariant requires them.

### Measured consumption and progress

Keep financial correctness and deterministic selection behavior as delivery gates; broad
optimization comparisons are optional tuning. Define fixture balances/liabilities, method
and price sequence, block progression, settlement/publication schedule and RPC completion
order before asserting outcomes. Assert payable cases with compatible completed evidence
at final selection succeed, at most one unsigned handoff occurs, and retained-use fixtures
allocate nothing. Document post-handoff contention and repeated same-wallet checkpoint/debit-watermark/expiry
races without compatible evidence as the bounded algorithm's pre-submission failures. Do not promise success from summed or stale
balances. Use harness limits to detect hangs, not as new financial deadlines.

- Cold fallback retained use: active has $0.05, retained has $0.60 and ready
  reserves each have more than $0.60 available. For a $0.10 GET and a separate POST fixture,
  publish compatible sufficient retained evidence before final selection. Active cannot pay;
  the one handoff binds retained and journals there. Assert unchanged active pointer,
  reserve claims and funding jobs. Begin with an empty runtime cache; use no injected
  selection or new funding.
- Debit versus expiry: capture a still-fresh $10 balance with one $4 unresolved
  authorization. A compatible confirmed expired-unused proof permits a new $8 payment
  without a newer balance merely for proof height. In the used variant, require a
  compatible post-debit balance of $6 and reject $8. Missing nonce proof, incompatible
  history and expired balance never receive the unused-case relaxation.
- Required warm retained use: two $0.60 retained wallets, with active and ready reserves
  each above $0.60 available throughout the retained-use opportunities; interleave
  $0.10 GETs and POSTs with canonical settlement and complete reusable observations.
  Identify each call whose valid hint and saved evidence select a sufficient retained
  wallet; assert those exact choices and balance/liability transitions, with no allocation.
  Each correctly predicted paid call makes exactly one unsigned request and one signed
  submission; initial selection issues no RPC and a free response signs nothing.
  Derive expected consumption from these opportunities rather than an arbitrary six-call floor.
- Required cold/learned use: empty caches, the same funds, faster active reads, one unavailable
  retained wallet and slow unrelated history. The first valid challenge teaches the
  actual hint; a scheduled ordinary balance publication makes the other retained wallet
  usable. Subsequent calls inside hint/evidence validity must select it while it remains
  the smallest sufficient candidate under the shared tie rules.
  Repeat with POST-only calls using real hint learning/publication, not injected payer
  choices. Pin the learning/publication opportunities and resulting expected consumption.
  Variable prices get separate behavioral fixtures without a universal utilization claim.
- Incorrect or unusable hints: price increases/decreases, argument-dependent prices on
  one operation key, stale balance evidence, concurrent reservations, eviction and restart.
  A hint-selected wallet of any eligible role that cannot afford the actual challenge uses the
  ordinary one handoff when another completed candidate can pay. Exercise second-challenge
  price changes and post-handoff contention as bounded unsigned failures. Assert at most
  two unsigned requests and one signed submission, no price probes or extra retries, and
  no role change/allocation on hint selection. Cache expiry/eviction creates no traffic.
- Cross-role smallest fit: with $8 retained, $1 active and $2 small ready, a valid $0.10
  hint chooses active. With $8 active, $0.40 retained and $2 small ready, drain the retained
  remainder through settled hinted $0.10 calls, then select/consume small once the remainder
  is insufficient. Assert exactly one vacancy on small's first journaled payment, no
  selection-time allocation and ordinary authorized refill only. A subsequent hinted $6
  payment uses the now-retained large wallet without changing the small active pointer.
  Check exact balances, liabilities, roles and one unsigned request/one signed submission
  per correctly predicted paid call. Repeat selection with many retained wallets and auto-funding
  disabled: payment succeeds independently of a blocked refill.
- Ordering boundaries: equal available amounts choose active before retained before ready;
  remaining ties follow the selection contract. Liability deductions can reverse raw-balance
  order. Unexpected credits/accepted bridge-minimum increases can reverse reserve target
  order, so choose by actual available capacity. Final selection includes concurrent
  liabilities and newly completed observations; it does not wait for an unfinished smaller
  candidate. Once bound, a sufficient payer stays selected even if a smaller wallet becomes
  usable before signing. Free responses and failed admission consume no reserve.
- Mixed-cost address reuse: use a deterministic cheap-call workload with occasional
  expensive calls and a long cheap-only suffix after a large reserve becomes active.
  With valid hints and smaller sufficient saved capacity, initial selection must move cheap
  calls to that smaller wallet. With missing/expired hints or unavailable smaller evidence,
  default binding and sufficient-bound-payer stability can keep them on the large wallet.
  Assert exact payer/role transitions for both cases; make no universal unlinkability or
  numeric privacy-improvement claim. No privacy-specific selector, tier restriction or
  mandatory comparative benchmark is introduced.
- Optional tuning may compare hint TTLs with identical workloads and predefined
  measurement criteria. No alternate selector, hints-disabled benchmark mode, numeric
  latency improvement or broad comparison study is required. Add no operator toggle
  for benchmark purposes; utilization-only handoffs remain outside this delivery.
- Retirement-independent refill: prove a ready payment, active change and authorized
  refill complete while every retained wallet exceeds tolerance. Repeat with retirement
  observations unavailable and with many retained wallets. Existing affordable retained
  calls still pay; no optional retirement read may delay payment or refill.
- Expensive-only fragmentation outside recommended sizing: small target $2, large $10, cap at least $6,
  tolerance $0.01. Start with $2 active, $2 small reserve and $10 large reserve. Execute
  repeated $6 calls with canonical settlement and authorized refill after each call,
  no bridge output increases or unexpected credits. Each large wallet pays once and
  leaves $4. Eight refills after the initial large wallet create nine large wallets:
  nine successful calls spend $54; holdings are $36 large remainders plus $4 small
  funds, totaling $40 from $94 gross allocation. After the ninth call there are nine
  retained wallets (initial small plus eight large), one $4 large active, one $2 small
  reserve and a vacant large slot. The next authorized $10 fill proceeds without
  retirement; after its tenth $6 call, gross allocation is $104, spend is $60 and
  holdings are $44. Assert all intermediate accounting and further fills beyond the
  old eleven-wallet boundary. A separate fixture with actual spendable treasury
  exhaustion preserves all claims/history and resumes ordinary funding after capacity
  is supplied. Variants use real smaller calls and report the capital-utilization tradeoff.
- Mixed-price cold/expired-hint limitation: exercise the $2.25/$0.50 example in the
  [reserve-sizing guidance](#recommended-reserve-sizing), with hints naturally expired
  between calls under a controlled fixture clock, settled
  authorizations, all relevant evidence ready and authorized large-slot refill. Both
  prices stay within the preferred 25% threshold. Each $10 large wallet serves three
  pairs and leaves $1.75. With $2 small active/reserve, cap at least $2.25 and
  zero tolerance, nine large wallets serve 27 pairs, spend $74.25 of $94 allocated and
  leave $19.75 total before the next refill. No old remainder alone can cover $2.25,
  but another authorized $10 fill proceeds independently of inventory size. After
  another three pairs, spend is $82.50 of $104 allocated and holdings are $21.50.
  Assert intermediate accounting and continued refill without retirement. A
  separate retained-use case starts with active insufficient and demonstrates successful
  small-call reuse. Pair this with a learned mixed-price fixture using two operation
  keys for the two prices and calls within hint lifetime: subsequent $0.50 calls choose
  the smallest sufficient available wallet before provider I/O, including active and ready
  candidates. Assert retained use only where it wins that ordering. Check exact selection
  and accounting against the model, with one unsigned request per correctly predicted paid call. Keep
  a same-key variable-price case separate; it cannot assume accurate predictions.
- Sizing guidance: label $10/$6 as outside the recommendation; compare $12/$6 and $24/$6
  exact-fit cases. Test $10 wallets at $5, $4 and $3.34 to show non-monotonic remainder;
  verify the 50%/one-third/25%/10% bounds with integer micro-USDC arithmetic around
  division boundaries. Mixed prices assert only the weaker next-payment bound.
  Separate remainder from prefunded reserves and unresolved exposure; pending-liability
  variants demonstrate why the settled-payment bounds do not apply there.
- Resource progress: foreground, history, credit and balance publication progress under
  opposing supported load. Include slow continuous progress beyond old bounds, true
  stalls, fallback and cancelled queued requests. Exercise multiple generated groups
  concurrently against the same deployment-wide RPC/query/queue limits; assert bounded
  ownership and cross-group progress under the documented supported load. No-demand architecture
  must let an already accepted vacant-slot job progress while an unrelated candidate
  query remains unfinished. Identify limits when all network capacity is occupied;
  add coalescing/isolation only for measured failures and test ownership if introduced.
  Include actual funding-worker and treasury-actor occupancy: slow source reconciliation
  versus independent Base credit/1Click status for both the same and other operations,
  plus prerequisite-complete quote HTTP work with spare permits. Assert those reads progress,
  and same-operation destination closure can permit retirement while source accounting
  remains owned. Refund-address acquisition and another treasury command wait safely; do not claim arbitrary funding
  completion while the serialized treasury owner is occupied. Also leave the actor idle
  with an unresolved outgoing transaction: another job still cannot prepare or submit,
  while independent status/credit reads can run. Report this distinct source wait and
  verify canonical resolution restores ordinary preparation eligibility.
- Retirement workload reduction: reconcile an eligible $0.009 retained wallet under
  an explicit $0.01 tolerance. Assert its retirement, exclusion from subsequent routine
  balance queries/payment selection, preserved keys/history and continued independent
  source accounting where applicable. Repeat at small and large inventory sizes, with
  and without a vacancy. Contrast zero tolerance (positive remainder retained), dual-zero,
  funding disabled on reopen (eligible retirement still proceeds), group disabled or qualification mutation denied (no automatic retirement), pending destination/payment work,
  shutdown of a selected pass and competing admission. No case makes refill depend on
  retirement, and a later external credit never silently reactivates the wallet.

- Unsuccessful foreground search: use many retained wallets whose combined holdings
  exceed the challenged price but none individually covers it. Capture the pass boundary,
  add wallets concurrently and assert arrivals cannot extend traversal. Exercise a complete
  pass, one failed/stale observation and cancellation with unvisited pages. Assert distinct
  completed-negative versus incomplete results, bounded acquired work/cache memory, early
  success when a sufficient candidate completes, and no funding, retirement or signed I/O
  caused by a negative result. Record traversal work without claiming fixed total latency.

- Cold foreground reserve access: start after reopen with empty balance/hint caches,
  many insufficient retained wallets and a sufficient large reserve. Delay the bound
  payer and small reserve while permits remain available. Assert large-reserve acquisition
  and gated selection before retained traversal completes, with deduplicated reads and
  retained traversal opportunities when search continues. Repeat with reordered wallet
  sequences; acquisition priority must not depend on the large wallet's insertion position.
- Selection contention: pause between provisional sufficiency and gated selection,
  reserve that candidate's capacity in another call, and leave a sufficient wallet on
  an unvisited page. Assert continuation of the original pass to that wallet, unchanged
  captured boundary, no duplicate acquisition and no pre-selection cancellation of needed
  reads. Add a wallet after capture and verify it cannot extend the pass. Contrast exhaustion
  and post-handoff contention, where the fixed-payer admission contract still applies.

Add a deterministic mixed-price/completion-order fixture that checks financial accounting
and the documented selection rules. Broader representative workloads support future
utilization claims, not initial correctness acceptance. Deliver conservative documented
hint TTL/cache bounds with focused boundedness, learning and request-count tests. Broad
tuning does not delay delivery. Measure resource use to select aggregate RPC/query/queue
bounds. Include many retained wallets with zero tolerance and no eligible retirements:
bounded queues/cache memory, incremental traversal, foreground acquisition of uncached
wallets, retained traversal progress under active/reserve load and allocation beyond the
old proposed count ceiling. No implicit replacement count limit is permitted.
Include large authorization histories independently of wallet count and bound their batches. Preserve five-second balance scheduling cadence, independent
history backoff and default Base freshness. Do not add financial timeouts or weaken
freshness to pass tests. Local fixtures establish no real settlement or privacy qualification.

## Implementation and documentation

| Surface | Work |
| --- | --- |
| rotation config, deployment and assignment | Implement the [configuration contract](#configuration-and-authority) and preserve group bindings. Remove obsolete aggregate-budget policy with explicit parser diagnostics. |
| rotation store and allocation/funding modules | Own transactional roles/claims, readiness, completion and [phase dispatch](#funding-phase-dispatch), [current policy](#funding-terms-and-current-policy), [closure/reauthorization](#disabled-slots-and-same-job-recovery) and [disabled allocations](#disabled-allocations-and-partial-credits). Remove alternate allocation entry points; provide the scoped conversion utility. |
| manager, Base and evidence application | Implement [atomic admission](#atomic-admission-and-signed-journaling), [selection](#initial-selection-price-hints-and-admission) and [evidence](#evidence-and-resource-contract) through the shared group owner, verifier and existing journal. |
| payment, catalog_state, routing, pricing, network/cover/relay | Carry trusted operation scope and implement [runtime hints](#runtime-hints-and-trusted-scope) and [all-method handoff](#unsigned-handoff-for-every-supported-method). Use SDK candidate signing under the atomic admission contract. Implement individual HTTP-cache eviction and cleanup-safe inactive cover-owner reclamation under the [resource contract](#evidence-and-resource-contract). |
| deployment/bootstrap, funding worker, treasury return observation/accounting, wallet CLI | Implement [runtime continuity/readiness](#startup-configuration-changes-and-recovery), [independent work](#evidence-and-resource-contract), [completion and scoped return accounting](#financial-completion-and-source-accounting-predicates), [retirement accounting](#retirement-accounting-and-administration) and the output contract below. |
| qualification and shared live driver | Record real group/wallet/slot roles, consumption separately from refill, and cumulative run authority in private evidence. |

Status reports configuration/authority, roles and claims, exact observations/freshness,
liabilities, descriptive inventory counts, vacancies and retirement accounting. Distinguish current
capacity from historical readiness, total holdings and initial funding targets. Show
small-first allocation separately from actual preparation/reservation order.

Give fixed actionable reasons for policy obstruction, exhaustion, treasury-capacity waiting,
treasury-command waits, unresolved-source-transaction waits and transient observation failure. Distinguish historical
readiness/retirement discrepancies, operation-local review and durable financial repair
blocks; ordinary refresh cannot clear invalidated liability-release/accounting support.
Report attached partial credits, their temporary payment unavailability and preserved disabled allocations
with their original target/history. Distinguish successful excess returns from reviewed
failed-swap refunds; shielding remains explicit. Report gross source expenditure, canonically
confirmed returns and derived net cost separately, without asserting a speculative return
amount or final net cost while a reported return remains unverified.
No status field grants retry, new funding, fresh evidence or automatic repair authority.

Pin machine-readable output fixtures alongside the parser, rather than only updating
prose. Replace `WalletSummary.active_and_standby_target_atomic/usdc` with explicit
`initial_inventory_target_atomic/usdc` fields for the fresh-group formula. Offline
`config show` cannot infer migration cost or usable state; add no migration projection API.
Replace pair-based `bootstrapped_wallets` interpretation with explicit
`serving_ready_groups`, `inventory_ready_groups` and incomplete captured-job results
for bootstrap/status, with a documented output schema version. Inventory fields describe
current configured occupancy and may become false after consumption; historical initial-job
and captured-command completion keep separate names. Preserve observation-only
reading of historical qualification reports under their original versions. Check JSON
fixtures and CLI consumers for old field names, total-capacity claims and pair-readiness
assumptions; do not silently retain an old name with new semantics.

Update maintained guides only on delivery; this document remains a plan. Publish the
contracts through the destination inventory below, using links instead of repeating
transition rules. The [contract map](#normative-contracts-and-implementation-scope) defines
the behavior to publish; preserve the stated limitations and qualification boundaries.

Preserve the [selection decision and alternatives](#selection-decision-and-alternatives)
as a durable decision section in `docs/architecture.md`, titled "Wallet selection and
network isolation" (anchor `wallet-selection-and-network-isolation`). Transfer the
chosen priorities, alternatives table, cache/request distinctions, accepted limitations
and grounds for revisiting the decision. Link to that one authoritative section from
the maintained wallet/network/cover guides and AGENTS.md. Do not leave the rationale
only in this plan or create a completed-plan archive. Before removing the plan, verify
that the section and all incoming links exist and still explain why late selection was
declined despite its protocol feasibility.

| Destination | Required update |
| --- | --- |
| docs/README.md and incoming plan links | On delivery, index maintained guides, relocate durable explanations, retain concrete unfinished work in a focused plan, update incoming links and remove this completed plan. No implemented-plan archive. |
| README.md, docs/configuration.md | Publish the [configuration contract](#configuration-and-authority): syntax/rejected fields, sizing and cold/learned fragmentation examples, mixed-cost address-reuse rationale and selector limitations, initial capital versus holdings, treasury-funded authority and retirement controls. Explain reopen-only changes, startup outcomes, temporary unavailability of partial allocations and the narrow migration scope. |
| docs/wallet-rotation.md, docs/architecture.md | Publish lifecycle, evidence, ownership and accounting from the [contract map](#normative-contracts-and-implementation-scope), including their limits and deferred work. Transfer the selection/isolation decision to the architecture section specified above; link from wallet rotation guidance. |
| docs/wallet-cli.md and generated help | Remove wallet pool; document automatic retirement and the deferred manual CLI, readiness/captured-job outcomes, current-policy recovery and actionable status. Explain the one-time utility/backups and limits of ordinary recover versus incident-specific repair. Use the output schema specified above. |
| docs/agent-sources.md | Document trusted BoundTool operation scope, captured snapshot invocation and group sharing; agents cannot select funding authority. |
| docs/network-egress.md, docs/cover-traffic.md | Update network identity, handoff/request-attempt and cancellation behavior, timer inventory, incremental processing/concurrency and hint TTL/cache bounds from their contracts. Explain slow-response evidence acquisition, bounded authenticated fallback for acquisition-only failures, serialized treasury prerequisites and the separate single-outgoing wait. Document individual HTTP-client eviction, cleanup-safe cover-owner reclamation and qualification after eviction. Link to the architecture decision on retaining hints for pooled connections, cover-state reuse and Tor reuse under isolation, including finite negative-cache retention after owner eviction and the rate-limit/cache distinctions. |
| examples/deployments/servers-managed.toml, examples/deployments/agent-sources-managed.toml, examples/deployments/privacy.toml, examples/deployments/servers-auto-wallets.toml | Apply reviewed reserve/cap/per-operation settings; remove aggregate-budget fields. Explain sizing, startup/refill order, automatic retirement and the partial-allocation/accepted-insufficiency limitations; no manual retirement CLI. Preserve user-owned local copies. |
| examples/live-qualification.toml, examples/live-tor-providers.toml | Update managed syntax and reviewed authority; preserve qualification restrictions, funding opt-outs and exclusion of inherited permits. |
| Public demo configs and docs/public-swap-demo.md | Update reviewed caps/per-operation safeguards and remove aggregate-budget fields; preserve funding opt-outs and separate run limits. Keep zero retirement tolerance unless explicitly reviewed; explain that auto_fund=false does not disable eligible retirement. |
| tests/live/INTEGRATION.md, tests/live/INTEGRATION_REFERENCE.md, tests/live/integration/SCENARIO_PRESETS.md and tracked scenario configs | Version group/slot evidence and consumption/refill assertions; preserve observation-only historical runs and prohibit permit inheritance. |
| docs/testing.md, docs/plans/deferred/live_integration_acceptance.md | Describe delivered evidence from [validation and acceptance](#validation-and-acceptance), including actual worker/owner fixtures and the known-state migration. Preserve funded lifecycle/privacy qualification limits; broad utilization benchmarks remain optional. |
| docs/plans/deferred/managed_wallet_followups.md | Replace obsolete double-buffering/resolution descriptions. Retain independent receipt, Permit2 and confidential qualification work; distinguish provider fulfillment reports from canonical transfer attribution. |
| docs/plans/prefer_get_vs_post.md | Align handoff and runtime-hint references with their contracts; retain independent method curation and the limit of pinned middleware qualification. |
| AGENTS.md | On delivery replace obsolete pair/target-cap and wallet-pool guidance with links to maintained lifecycle, ownership, recovery and evidence contracts. Add a short instruction to consult the architecture's "Wallet selection and network isolation" decision before proposing late payer selection, per-invocation isolation or cached-challenge reuse; identify pooling, cover-state reuse and isolated Tor reuse as the chosen priorities. Reflect implemented aggregate-budget removal without reopening settled policy. Preserve treasury/payment/qualification invariants and narrow migration/repair scope. |

Preserve provider inventories, names, methods, full descriptions and schemas. Derive
tracked configuration/snippet updates from `git ls-files`, excluding user-owned local
overrides. Include public-swap-demo and both managed/single live-integration deployment
examples. Validate links, help and output consumers; do not teach unfinished syntax as
current behavior or alter historical evidence. Pin middleware provenance before all-method
handoff. Public reports omit private identifiers, credentials, prose and paths.

## Delivery sequence

Deliver independently reviewable milestones with focused default-Zcash regression checks
at each boundary. Preserve one final configuration/runtime model; intermediate review steps
are not a permanent legacy mode. Do not combine new admission ownership, selection and
funding transitions into one unreviewed change.

Use this sequence as the commit ordering, not a one-commit-per-milestone requirement.
A milestone may span several coherent commits, each with its relevant tests. Keep
intermediate commits buildable and preserve financial invariants. Split by ownership
or behavior boundaries rather than by file or layer; an atomic financial transition
must remain atomic in every intermediate state. Avoid prescribing an exact commit count
or introducing temporary compatibility machinery solely to make a smaller commit.
Remove transitional scaffolding when its replacement lands; the lease prerequisite's
temporary cleanup may remain only until atomic admission replaces it.

Suggested commit boundaries within the larger milestones:

- Atomic admission: shared group ownership, then atomic journaling and cancellation behavior.
- Base evidence/history: verification, cross-group invalidation, then bounded traversal
  and exposure accounting, retaining existing guards until their replacements are complete.
- Treasury returns: original-operation lookup/discovery, then attribution-failure isolation.
- Funding: transitions/completion, independent scheduling, then startup ownership continuity.
- Capacity/selection: configuration and roles, selection/hints/handoff, automatic retirement,
  then network/cover retention. Combine dependent pieces where separating them would expose
  incomplete behavior or violate an invariant.
- Conversion and final integration: keep separate from runtime behavior changes.

Update this plan as milestones land, recording completed scope, focused validation and
remaining work without duplicating the normative contracts. Tests accompany behavior
changes. Publish maintained configuration/help documentation when the corresponding
syntax and behavior work; the final documentation milestone verifies completeness rather
than postponing all documentation until the end. These boundaries guide review and may
be adjusted to actual dependencies without changing the final design or deferred scope.

1. **Independent ownership fix.** Deliver the [lease prerequisite](#existing-implementation-and-dependency-boundaries).
   Include temporary owned cleanup only if the legacy two-step path remains deployed before step 2.
2. **Atomic admission ownership.** Implement [shared ownership and atomic admission](#state-and-ownership)
   before changing selection or moving RPC outside the gate. Run the ownership/lease acceptance fixtures.
3. **Base evidence and history.** Implement per-wallet verification, shared-store endpoint
   invalidation, scoped publication, bounded wallet/authorization traversal and exposure
   accounting from the [evidence/resource contract](#evidence-and-resource-contract), then
   move Base RPC outside the gate. Run evidence, cross-group invalidation, recovery-scope
   and history/resource fixtures. Treasury return accounting and funding scheduling below
   are distinct milestones, not prerequisites hidden in this step's resource-contract link.
4. **Treasury return accounting.** Independently implement the original-operation return
   lookup, keyed discovery/revalidation and attribution-failure isolation specified in
   [source accounting](#financial-completion-and-source-accounting-predicates). Preserve
   gross/principal/fee accounting and the pinned dependency's linear discovery limitation.
   Run actual treasury-sync fixtures for late/archived returns, ambiguity, unrelated progress,
   duplicates and canonical contradictions. This work remains part of delivery.
5. **Funding transitions and completion.** Implement [destination readiness](#applicable-destination-requirement),
   [completion predicates](#financial-completion-and-source-accounting-predicates),
   [phase dispatch](#funding-phase-dispatch), current-policy terms and safe closure versus
   reauthorization. Keep source completion independent of wallet usability and preserve
   return accounting from step 4. Run transition/recovery fixtures through actual backend
   calls before adding concurrent scheduling.
6. **Funding scheduling and runtime continuity.** Implement bounded independent status/credit
   reads and per-observation scheduling within the existing funding owner, then
   [startup continuity](#startup-configuration-changes-and-recovery). Retain the treasury
   actor and durable single-outgoing restriction. Run actual worker/treasury fixtures for
   reverse completion orders, backoff, cancellation, reopen and readiness without worker
   teardown. This remains a distinct milestone in this plan, not deferred work.
7. **Capacity and selection.** Pin parser/output contracts; implement [configuration](#configuration-and-authority),
   [roles/claims](#reserve-consumption-and-refill), [selection](#initial-selection-price-hints-and-admission),
   automatic retirement and network/cover retention. Include finite negative-cache retention
   under ordinary cover-owner eviction. Run lifecycle, selection, startup and cache fixtures
   plus the concrete workloads. The manual retirement CLI is excluded.
8. **Known-state conversion.** Implement and validate only the [supported conversion](#conversion-contract).
   Actual stopped-service conversion remains a separate operator action.
9. **Integration and documentation.** Complete the [implementation/documentation inventory](#implementation-and-documentation)
   and qualification outputs. Run `scripts/check.sh` and link/example/help validation after focused checks.
   No no-default-feature suites without request.

The manual retirement CLI remains deferred. Specialized abandonment, contradictory-evidence
assessment and fork repair remain deferred to actual incidents. None is a final delivery step.

Use tests/rotation.rs, tests/managed_config.rs, tests/payments.rs, tests/network.rs,
tests/wallet_workflow.rs, tests/integration_preparation.rs, existing discovery/provider
contracts and store/funding/Base fixtures. Keep live CLI wiring in the shared driver.
Live-funded validation requires separate operator authority and cannot be inferred from
this plan, migrated state, retained evidence or successful offline tests.
