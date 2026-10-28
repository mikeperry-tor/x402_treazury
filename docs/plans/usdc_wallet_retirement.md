# Ordinary USDC wallet retirement

## Intent, baseline and scope

Integrate automatic retirement into ordinary managed-wallet serving. Change the
omitted `max_retirement_remainder_usdc` default from `"0"` to `"0.001"`
(1,000 atomic USDC units), while preserving independent overrides on every
`[wallets.NAME]` managed group. Explicit `"0"` continues to require exact zero.
This document plans implementation; it changes no runtime defaults or wallet state.

Starting baseline: `b61ec526f04309984d6d9abde5135514baaf3345`, including the
standalone diagnostic-test correction `7c163bfa`. Ordinary serving and explicit
`wallet sync` currently perform scalar observations, which cannot retire a
wallet. Canonical retirement and authorization-resolution machinery exists;
its runtime producers currently belong to qualification. Do not enable the full
qualification traversal as an ordinary maintenance loop.

This work excludes transfers, sweeping/consolidation, replacement deposits,
manual force-retirement, new payment authority and funded qualification. Preserve
the automatic reserve-replacement behavior already implemented at this baseline.

Owning guides are [wallet rotation](../wallet-rotation.md#automatic-retirement),
[reconciliation policy](../wallet-rotation.md#recovery-and-reconciliation-policy),
[wallet observation policy](../wallet-rotation.md#wallet-observation-policy),
[configuration](../configuration.md), [wallet CLI](../wallet-cli.md),
[network policy](../network-egress.md), [resource limits](../local-resource-limits.md),
[logging](../development.md#logging-and-diagnostics), and
[testing](../testing.md#validation-order-and-continuation).

## Required behavior and preserved invariants

- Only enabled-group `RETAINED` wallets can retire. Low estimates select work;
  they never prove retirement eligibility or exclude an otherwise permitted payer.
- Reuse compatible confirmed/latest balance evidence. Both balances must be at
  or below the group's tolerance, inclusively; use their maximum for the recorded
  remainder. Preserve existing minimum-based advisory/canonical availability
  semantics instead of changing them to simplify scheduling.
- Reuse canonical nonce-used and confirmed-chain-time expiry proofs to resolve
  the candidate's original authorizations. Local time, a seller receipt, HTTP
  failure, or an estimated zero cannot release payment exposure. Retire only
  after all outstanding authorizations are resolved.
- Preserve exact role, claim, capture and evidence checks at publication. An
  enabled reserve claim or an existing destination/source obligation blocks
  retirement. A payment, incoming credit or role change during acquisition must
  be revalidated through the existing scoped publication contract.
- **Never-credited wallets remain protected until their original delivery
  obligations are resolved.**
  `destination_obligation_pending` currently returns true until first positive
  initial credit, including completed, abandoned funding whose source is already
  confirmed. Replace this blanket guard only with the evidence-backed closure
  rules below. A completion/abandonment label, missing legacy records, quote age
  or an observed zero is insufficient. Preserve unresolved predecessors for late
  bridge credit; do not manufacture a positive initial-credit observation.
- Commit retirement role and supporting evidence atomically through the existing
  publication path. Preserve keys, funds, old authorizations and funding history.
  Retirement neither transfers dust nor creates reserve vacancies or funding
  authority. Later contradictory evidence must not silently reactivate a signer.
- Retirement authority stays independent of `auto_fund` and treasury capacity.
  Preserve qualification restrictions, Store-only serving, disabled-group behavior,
  listener ownership, identity-bound Base transports and Tor/no-direct-fallback
  policy. No new network constructors, monetary accounting or worker framework.

## Scoped observation and completion design

Use the existing group reconciliation owner, per-wallet observation ownership,
publication gate, local paging and network recovery. A retirement request concerns
one candidate wallet and its outstanding authorizations, not the whole pool.
Local candidate discovery may page durable metadata; it must not turn into a
recurring network inventory sweep.

Trigger candidate consideration from the finite serving startup pass, meaningful
scalar observations, transitions to retained inventory, and relevant claim/source
changes through existing notifications. Reopening configuration reevaluates the
new group tolerance. Unknown or above-tolerance balances wait for ordinary
observation; arbitrary external transfers do not create an incoming-funds watcher.
Never-credited or source-obstructed wallets do not justify useless balance polling.
Resolve their specific original obligation where useful; do not use repeated
balance reads as a substitute for unavailable source or provider evidence.

### Closing original destination obligations

Evaluate every original current and archived attempt that could still deliver.
Reuse accepted quote/operation bindings, source journals and canonical proofs;
do not add a parallel financial ledger or infer negative evidence from missing
legacy history. Closure permits ordinary retirement evaluation; it does not
itself retire the wallet, release source inputs or authorize replacement funding.

| Evidence | Permitted conclusion |
| --- | --- |
| Complete journal provenance establishes that no original attempt reached a state capable of submission or disclosure | That attempt cannot deliver; missing records alone do not establish this for legacy work |
| Existing canonical consensus-expiry and original-unspent-input recovery proves the source attempt cannot execute | That attempt cannot create a future destination delivery |
| Delivery bound to the original quote and independently verified by the existing destination USDC transfer proof | That delivery completed, even if its credit was missed and the current balance is zero |
| Original-quote-bound terminal refund whose protocol contract establishes completion of refund processing and no outstanding destination delivery | That swap no longer promises destination credit; account for contractual fees rather than requiring refunded amount to equal the gross deposit |

These are alternative grounds for closing an attempt, not a cumulative checklist.
Use an already validated, exactly bound terminal observation or existing canonical
proof when it establishes the required fact; do not require a fresh provider or
chain lookup merely to agree with it. Each genuinely unresolved attempt and
authorization remains independently owned.

Preserve the existing first-positive-USDC-credit contract when recovering missed
historical delivery. Use the actual bound receipt, not equality to the quoted
target, to establish that credit. Do not introduce a new wait for the expected
target amount; actual unresolved accepted source work remains independently owned.

Complete provenance means the existing journal contract and retained facts, not
a new provenance-certificate framework. A refunded current attempt cannot close
an unresolved archived attempt or an accepted source attempt still able to deliver.
Use the provider's documented terminal semantics, exact saved deposit/request
bindings and existing receipt verification. An unbound status enum or Boolean
does not establish those facts. A partial refund with an outstanding contractual
delivery leg, genuinely conflicting delivery evidence, unverified delivery,
`FAILED`, `PENDING_DEPOSIT`, HTTP errors and timeouts alone do not establish closure.
A net refund below the gross deposit or a difference between provider-reported and
observed amounts is not itself evidence of an outstanding delivery leg. Expired
quote time alone does not prove source nonexecution or release an accepted transaction.
Independently resolve source liabilities through existing source recovery; a
terminal provider result never releases source inputs. Do not require a new
refund-attribution ledger or artificial positive-credit event to close an otherwise
proven destination obligation. SDK/blockchain reconciliation continues to own
source spendability and discovery of returned funds.

#### Reconcile from existing ZEC evidence

Apply the maintained [reconciliation policy](../wallet-rotation.md#recovery-and-reconciliation-policy).
In this implementation, reuse exact source confirmations, bound refund outputs
and confirmed shielding spends with existing SDK ownership validation to repair
derived outcomes and finish the work they establish. They do not independently
prove absence of future USDC delivery. A bound terminal refund that already
closes that destination obligation needs no additional receipt, shielding,
amount-equality or fee-recovery gate. Do not add explorer dependencies. The R2
acceptance cases below verify this separation and absence of redundant I/O.

Inspect bound provider status or existing chain evidence only while it can advance
this specific unresolved obligation, using the existing observation owners,
cadence and independent backoff. Stop that lane on proven closure. Missing legacy
provenance or contradictory evidence requiring operator investigation must appear
as a scoped unresolved status, without an indefinite balance-polling loop or an
age-based automatic abandonment. New relevant evidence or explicit inspection
can reopen evaluation; elapsed time alone grants no retirement authority.

For a candidate, page only its outstanding authorizations through existing
canonical proof/publication machinery, then publish the required dual balance
proof with final eligibility checks. More than 64 authorizations must be processed
in pages without truncation, a new total cap, or scanning unrelated wallets.
A still-valid unused authorization is pending work, not failure or permission to
retire. A resolved authorization never authorizes another provider request.

Define completion independently of the saved scalar:

| Result | Required continuation |
| --- | --- |
| Retired | Finish the request; stop ordinary selection and balance discovery for that wallet |
| Verified latest balance above tolerance | Finish this retirement request; wait for new relevant demand |
| Latest within tolerance but confirmed balance still above it | Retain confirmation-progress work under the existing cadence; retire only after both qualify and other blockers resolve |
| Unresolved original authorization or transient proof/publication failure | Retain scoped work and retry under existing observation cadence/backoff |
| Role, group, reserve-claim or destination eligibility changed | Recheck current facts; stop ineligible network work and reconsider on its relevant transition |
| Required facts cannot be recovered from accepted inputs and authoritative observations, or execution/ownership evidence genuinely conflicts | Preserve the affected obligation, surface a scoped operator requirement, and continue independent wallets; reconcile recoverable derived state instead of treating reporting differences as blockers |
| Cancellation, restart or owner replacement | Keep accepted work owned; rederive unfinished work from durable facts without replaying payments; restart may supply one fresh finite startup evaluation |

A low saved minimum can coexist with a latest balance above tolerance. It must not
cause perpetual canonical polling. Within a serving lifecycle, cache eviction and unrelated capacity wakes must not
resurrect a completed request or reset retry backoff. Restart itself is new finite
startup demand and may evaluate eligible wallets once again; no durable
completed-request tombstones are required. Do not depend on retaining a disposable
balance certificate to distinguish completion from demand.

### G0 — scheduling and ownership design gate, before implementation

Independently review the complete scalar acquisition, requested-refresh,
retained-role transition, pending-authorization and canonical publication chains.
Choose and document the smallest concrete way to represent retirement request
completion and retries within the existing owners. First assess reusing existing
refresh-demand state with a retained-wallet proof stage. Do not conflate a
completed scalar refresh with completed retirement work or change advisory RPC
failure policy to make this reuse convenient.

Review original source/destination closure alongside this scheduling decision:
identify which existing persisted proofs establish each closure case, how exact
quote bindings survive restart, and how legacy gaps and contradictions remain
visible without repeated useless I/O. Identify where existing SDK/chain facts can
replace stale derived state and finish work without adding a consistency gate.
Establish the terminal-refund contract from
[the provider status API](https://docs.near-intents.org/api-reference/oneclick/check-swap-execution-status)
and its [status semantics](https://github.com/near/agent-skills/blob/main/skills/near-intents/references/concepts.md).
Do not treat an enum's spelling or a successful diagnostic lookup as sufficient
production proof. Resolve material contract gaps before implementing that case.

If existing state cannot preserve the completion rules above across overlapping
requests and rederive unfinished work on restart, justify a minimal work marker with its exact ownership,
set/clear transitions, restart semantics and compatibility strategy. It must not
be a monetary ledger, generic queue, generation framework, new admission cap or
certificate-retention requirement. Resolve this decision before building R1/R2;
prove the design with finite-work and cancellation scenarios in review.

Reuse the existing 60-second observation floor and error backoff, without stacking
backoffs, adding whole-work deadlines or letting sibling progress renew another
request. Any actual change to cadence or resource responsibility must first satisfy
[resource policy](../local-resource-limits.md) and update both resource/timer
inventories. Shared read/publication ownership must survive a disconnected waiter;
no group publication gate may be held across remote I/O.

## Ordered implementation commits and acceptance

### R1 — default and per-group configuration contract

Change the managed profile's omitted tolerance to `"0.001"` using exact existing
USDC parsing. Preserve explicit zero, other finite nonnegative overrides, maximum
validation and per-group independence. Existing serialized values must not be
rewritten as though they were omitted; normal configuration application updates
the selected pools on reopen. Propagate the default through templates and effective
configuration as applicable, without adding another setting.

Acceptance: focused parsing/default, explicit-zero, multiple-group, configuration
resolution and reopen tests show exactly 1,000 atomic units when omitted. Test
inclusive below/equal/above threshold behavior without floats. Review authored
README/example values: label intentional exact-zero examples as overrides, and
update default-demonstration values/comments. Never edit local operator overrides.

Update the default and implications in `docs/configuration.md`,
`docs/wallet-cli.md`, `docs/wallet-rotation.md`, and affected README/examples in this
commit. Explain that tolerated funds remain owned at their addresses, not spent,
and that positive tolerance has no cumulative stranded-funds cap.

**G1:** independent review of the exact R1 SHA and its effective-configuration and
persisted-pool consumers. Resolve material findings before R2; review each fixup.

### R2 — complete ordinary retirement integration

Implement the G0 owner/completion decision as one coherent behavior change:
candidate discovery, candidate-scoped canonical authorization resolution, dual
balance publication, retry/stop behavior and ordinary runtime scheduling. Reuse
`retire_with_support` through canonical publication; do not create a parallel
retirement mutation. Support Store-only serving without requiring Zcash/NEAR.
Startup follows the existing SOCKS prerequisite. Explicit `wallet sync` performs
only a finite eligible-candidate pass, reports unresolved work and creates no
watcher; it remains usable with auto-funding paused.

Include the original-obligation closure rules in the same coherent integration.
Reuse existing proof/publication paths and preserve all current/archived attempt
ownership. No closure path fabricates initial credit, reopens a consumed funding
claim, resubmits an old deposit or allocates another wallet. Publish supporting
closure facts and eligibility under the existing Store transaction discipline;
revalidate relevant changes before committing retirement.

Include diagnostics with this implementation: INFO for committed retirement and
meaningful deferred/recovered work, DEBUG for individual checks and ordinary
cooldowns. Replace the touched routine-success WARN for authorized nonzero-dust
retirement with policy-consistent logging. Report wallet/group, observed remainder,
configured tolerance and known blocker safely; never log credentials, signed
payloads, raw RPC/prose, or claim that retired dust was spent. Avoid one reminder
per polling iteration. Zero retirement must also be visible as a lifecycle change.

Permanent documentation belongs in the automatic-retirement, payment-reconciliation
and observation-policy sections of `docs/wallet-rotation.md`; configured authority
in `docs/configuration.md`; finite sync and status in `docs/wallet-cli.md`; network
scope and scheduling responsibilities in `docs/network-egress.md` and
`docs/local-resource-limits.md`; focused evidence/limits in `docs/testing.md`.
Update the managed-wallet deferred plan's runtime-gap claim. These owning guides
are already linked by root `AGENTS.md`; verify those links rather than duplicating
policy in the instructions.

Acceptance must exercise the real ordinary producer-to-publication path, including:

- Default/overridden dust, exact-zero override, both balance directions disagreeing
  at the threshold, retained eligibility, and active/ready/current-claim exclusion.
- More than one authorization page; used, confirmed-expired-unused and still-valid
  unused nonces; no replay, release on local clock alone, or collateral history scan.
- Never-credited abandoned predecessors remaining retained while unresolved,
  original late credit becoming usable, pending current/archived source work,
  and reserve-replacement generation ownership remaining intact.
- Complete never-started provenance versus missing legacy history; canonical
  expired/unspent source recovery; verified delivery missed by scalar history;
  bound terminal refunds including fees; refunds with unresolved delivery legs,
  genuinely conflicting execution evidence, and bare
  failed/pending/expired-quote results retaining their obligations. Exercise
  closure with current balance zero without fabricating first positive credit,
  refund attribution, source release, replay, top-ups or new allocations.
- Scoped unresolved legacy status, useful status/receipt observation and terminal
  completion across cancellation/restart; no recurring balance reads that cannot
  resolve a source/provider blocker. One attempt's closure cannot conceal another
  current or archived attempt capable of delivery. Reject wrong deposit, recipient,
  asset or quote bindings; retain genuinely conflicting execution evidence while
  reconciling ordinary status progression. Revalidate concurrent source ownership
  changes before publishing closure.
- Refund received while offline, discovered after restart and later shielded;
  exact outpoint linkage, an empty refund address after shielding, and already
  spent recovered capital. Reuse existing confirmed records without an additional
  chain/status lookup; missing optional receipt/shielding evidence must not defeat
  independently established terminal closure.
- Provider/refund amount differences and fee deductions, stale phases and older
  nonterminal status followed by bound terminal evidence: correct derived state
  and finish proven work without a repair halt, amount-equality gate or repeated
  acquisition. Assert unrelated authorized payment/funding work continues. Keep
  genuinely unresolved source inputs and destination delivery independently owned;
  refund receipt/shielding alone grants neither retirement nor another deposit.
- Verified historical positive USDC receipt below the quoted target consumes the
  original claim under existing semantics; no target-equality or additional
  expected-credit gate, and no release of independent accepted source work.
- Incoming credit, concurrent payment/role reuse, changed eligibility and stale
  evidence during proof acquisition; exact atomic retirement evidence and reopen.
- Cancellation/disconnected waiters, slow progress versus a true stalled response,
  owner replacement, restart, unrelated concurrent work and independent backoff.
- Auto-funding paused, disabled group, qualification mutation denial, Store-only
  operation, finite explicit sync, startup SOCKS unavailability and no direct fallback.
- Stable idle RPC counts: no additional retirement-driven canonical reads of
  unrelated or above-tolerance wallets, or never-credited wallets with unresolved
  original delivery obligations; existing finite scalar startup
  and accepted-credit observations remain valid. A verified latest balance above
  tolerance stops retirement work even when the saved minimum stays low. The
  opposite disagreement retains progress until confirmed balance catches up;
  cache eviction and unrelated wakes do not restart completed work. Restart may
  supply one new finite evaluation, without creating an idle watcher.
- Safe lifecycle severity and blocker visibility in isolated diagnostic fixtures.

**G2:** before broad runtime suites, independently review all R1/R2 changes and
fixups against the baseline, including complete financial ownership, observation,
network, publication and documentation handoffs. Use source-matched CogC/CycC
baseline comparison; inspect existing high-risk maintenance, observation,
canonical publication and history consumers. Evaluate concrete ways to remove
responsibilities or branches instead of extracting helpers solely for scores.
Resolve material design/complexity findings before proceeding.

## Validation, review and final integration

Follow the repository's validation order: cheap affected contracts, applicable
static/tooling checks and source-matched complexity review, production compiler
and full all-target Clippy warning gate, `check-compat`, then the relevant broader
library/CLI/network and wallet lifecycle suites. Use socket-capable execution up
front for local RPC/SOCKS fixtures. Use only temporary unfunded deterministic
wallets; this plan authorizes no live payments, deposit probes or mainnet mutation.

Record commands, exact source/executable/feature/profile pins, nonzero test counts,
warnings and carried-forward evidence under ignored `target/`. Revalidate affected
consumers after fixes; resume unfinished named check stages and repeat passed
stages only when their evidence is invalidated. Offline tests do not qualify live
settlement, Tor privacy or permanent absence of future external credit.

Independently review every exact implementation/fixup SHA with user intent,
baseline and validation evidence. Preserve unsquashed history through review.
Before integration to main, give any review fixups proper standalone messages as
requested by the user; verify each rewritten SHA and its unchanged tree again.

**G3 — final integration:** review the full branch from `b61ec526`, including all
prior commits/fixups, complete consumers and the reasons behind corrections.
Resolve recurring assumptions, hazards and test gaps; widen the audit or simplify
if the evidence requires it. Confirm permanent guidance and AGENTS links, remove
this completed plan, and independently review that exact cleanup SHA and final
combined branch. Do not retain a completed-plan archive or delete financial evidence.
