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
- **Never-credited wallets remain protected.**
  `destination_obligation_pending` currently returns true until first positive
  initial credit, including completed, abandoned funding whose source is already
  confirmed. Newly retained predecessors of reserve replacement therefore remain
  available for late bridge credit. Integrating retirement must not remove this
  guard or treat the old completion label as proof of delivery or permanent loss.
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
| Required stored records malformed or inconsistent | Preserve obligations, surface a scoped operator requirement, and continue independent wallets; avoid repeated proofs that cannot repair those records |
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

### G0 decision (implementation scope selected from `1e880388`)

The independent G0 review traced scalar acquisition, refresh flags, role/claim
transitions, authorization paging and scoped publication. The implementation
uses the requested revision's scope: never-credited delivery obligations retain
the existing guard. Later original-obligation closure proposals are separate
work; the maintained reconciliation policy still governs all existing behavior.

Reuse of `refresh_pending` alone was rejected: it clears before scalar I/O,
scalar success completes it, and canonical availability saves the minimum. A
latest-above-tolerance proof can therefore leave a low saved minimum permanently.

Keep a minimal Store-lifetime retirement marker per considered wallet: remember
local eligibility independently of pending demand. Local metadata pages discover
false-to-true eligibility transitions (including retained role, claim/source
closure and configured tolerance); a fresh low scalar observation also requests
work. Canonical minimum publication and unrelated wakes never request work.
Remember completed demand independently of disposable certificate/scalar caches.
No schema conversion, monetary ledger or durable completion tombstone is needed:
reopen supplies one new finite evaluation from durable facts.

Consume captured demand at acquisition start; overlapping demand remains pending.
Restore captured demand only for unresolved authorizations, confirmation progress
or transient acquisition/publication failure. Stop ineligible and malformed work
locally; relevant new evidence or explicit inspection can request it again.
A verified latest-above-tolerance result completes even with a low saved minimum.

The existing group reconciliation owner schedules work. A spawned wallet owner
retains the whole authorization-page/balance/publication stage through waiter
cancellation, sharing wallet serialization with scalar reads. Proof demand owns
its completion-based 60-second floor and existing error-backoff shape separately
from scalar result state. Neither lane changes the other's due time, outcome or
failure streak, and a proof retry never waits sequentially on both schedules.
This separation resolves the independent implementation review's finding that a
proof-only cooldown on an evicted scalar result could fabricate advisory failure. No group
publication gate spans remote I/O. Local reevaluation uses the existing maintenance
cadence, including source changes without capacity notifications.

Finite-work review scenarios: latest-high completes; confirmed-high/latest-low
retains progress; valid unused authorizations retry; demand during acquisition
survives completion; disconnected waiters cannot duplicate accepted page I/O;
malformed wallet work stops without stopping siblings; explicit sync attempts once
and reports pending work; restart grants only a fresh finite evaluation. R2 must
prove these scenarios through the ordinary producer/publication path before G2.

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
- Never-credited abandoned predecessors remaining retained, original late credit
  becoming usable, pending current/archived source work, and reserve-replacement
  generation ownership remaining intact.
- Incoming credit, concurrent payment/role reuse, changed eligibility and stale
  evidence during proof acquisition; exact atomic retirement evidence and reopen.
- Cancellation/disconnected waiters, slow progress versus a true stalled response,
  owner replacement, restart, unrelated concurrent work and independent backoff.
- Auto-funding paused, disabled group, qualification mutation denial, Store-only
  operation, finite explicit sync, startup SOCKS unavailability and no direct fallback.
- Stable idle RPC counts: no additional retirement-driven canonical reads of
  unrelated/above-tolerance/never-credited wallets; existing finite scalar startup
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
