# Tor isolation retries before discovery relay and API failure

## Intent, baseline and status

Proposed runtime work; this plan does not implement retries or authorize funded
qualification. Starting code baseline: `970f6bde1334eb42a540288ea1432381e1b9ae64`.
Accompanying documentation clarifies the already accepted unsigned x402 POST
semantics; it must not describe the proposed runtime behavior as delivered.

Try up to two additional Tor isolation groups after selected unsigned failures.
Separate catalog/pricing delivery from the more involved API payment/cover path.
Reduce avoidable paid discovery relay use and recover API calls affected by an
unusable exit, without replaying signed work or changing payer ownership. Retain
a successful replacement in the existing provider transport-cache entry; after
eviction or restart create a fresh isolation rather than reconstructing a failed
primary. Other provider origins on the same wallet retain their own transports.
Different circuits can still select the same exit IP; this is best-effort circuit
separation, not guaranteed exit diversity or a cure for all Tor blocking.

Owning guides: [network policy](../network-egress.md),
[resource limits](../local-resource-limits.md),
[selection/isolation](../architecture.md#wallet-selection-and-network-isolation),
[unsigned GET/POST semantics](../architecture.md#unsigned-x402-requests-and-post-retries),
[wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[wallet observation](../wallet-rotation.md#wallet-observation-policy),
[configuration](../configuration.md#paid-discovery-relay),
[cover policy](../cover-traffic.md), [source management](../agent-sources.md),
[development](../development.md), [testing](../testing.md),
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline) and
[Tor evidence](../../tests/TOR.md). Read affected guides before implementation.

Current behavior:

- `network.rs` derives stable SOCKS credentials and pooled clients from canonical
  owner identities. There is no request-scoped circuit retry discriminator.
- `catalog.rs::prepare_json_discovery` and `pricing.rs::probe_request` select the
  configured paid relay after one failed origin attempt. Fresh caches can avoid
  origin I/O. Pricing discovery is one-shot/coalesced, not a recurring probe.
- `payment.rs` obtains an unsigned response through the selected payer's client,
  then uses that same client for its signed request and associated cover owner.
  Generic unsigned status failures do not currently rotate circuits. Recognized
  structured signed insufficient funds permits the existing eligible-wallet
  traversal; generic signed errors do not.

## Accepted behavior and boundaries

Acquire the currently selected transport from the existing client cache, then try
at most two fresh attempt scopes sequentially after eligible unsigned failures.
Keep the retry sequence local to its existing operation owner; no persistent
retry ledger, separate winner registry, global circuit epoch, control port,
NEWNYM, new readiness gate or general workflow framework. Two additional attempts
bound the newly introduced speculative request amplification, not financial
capital or the duration of progressing work. Do not add a whole-work deadline,
shorten existing timeouts or tune unrelated resource caps.

### Selected transports, pooling and lifetime

Keep the canonical cache key: owner identity, provider origin, runtime, protocol
permissions, public-destination policy and timeout. For participating Tor HTTP
paths, its value owns a coherent immutable transport record containing the client
and its effective isolation scope. Return that record together, rather than a
bare client plus separately reconstructed identity. The canonical owner still
identifies the payer/discovery principal; it is not replaced by the fresh scope.

On every participating cache miss, including first acquisition after restart,
create a fresh owner-bound scope. Retry candidates also use unique fresh scopes.
This deliberately replaces deterministic primary SOCKS credentials for these
participating paths: retain existing credential/token domains and canonical owner
formats, but do not require their former primary token values. Direct mode and
nonparticipating HTTP/gRPC constructors keep their existing behavior. Ensure
participating and nonparticipating lookups cannot accidentally consume or replace
each other's cache records; represent the transport-selection policy explicitly
where the same otherwise canonical key can be used by both. Such separation
must not broaden which callers can retry.

Each candidate client has its own reqwest pool and scope-specific SOCKS credentials.
Changing only auth metadata or cloning the previous client is insufficient: an
existing HTTP/2 connection performs no new SOCKS handshake. Keep private retry
candidates outside the canonical selected-entry slot until eligible success; do
not add a second cache indexed by every historical attempt.

On eligible fresh-candidate success, conditionally replace the canonical entry
with that candidate's transport record. Replace only if the entry still holds
the exact record acquired when this sequence began; immutable record identity
(e.g. Arc identity) is sufficient, with no counters. Never insert on promotion
when the slot is absent. Eviction/reinsertion, another successful promotion or a
late completion from an old selected transport must not restore a stale choice.
A candidate losing this replacement race still finishes its successful operation
through its pinned transport; it incurs no extra retry, failure or payer acquisition. Failed candidates and paid relay successes never
change the origin's selected transport.

Discovery promotion requires a complete valid catalog or a successfully
interpreted pricing result, including existing successful no-challenge outcomes.
API promotion occurs on a valid accepted unsigned 402 challenge, or after a
successful unsigned response completes normal consumption/output validation.
A 402 proves unsigned connectivity, not payment acceptance or future paid success;
normal payment checks still apply. Malformed challenges, rejected/partial output,
cover qualification alone and signed outcomes cannot trigger promotion. Promotion
authorizes no extra request, signature, cache refresh or financial replay.

Subsequent logical operations reacquire the canonical selected record rather than
reusing a caller-held obsolete client. Inspect pre-acquired catalog/pricing clients
and deferred loaders explicitly. Already accepted/in-flight operations retain their
exact record; a valid challenge's signed dispatch and response handling stay pinned
even if another operation promotes or evicts the entry meanwhile. Dropping the
failed response releases that response/stream; do not force-close the old client
or connection, flush the provider's pools, or disrupt sibling HTTP/2 streams.

Selection is scoped by the full cache key, not wallet-wide. Other origins using
the same wallet do not migrate. Source aliases sharing an owner, origin and all
other key settings intentionally share selection; different transport/public/
timeout policies retain separate selections. Discovery-origin and payer identities
remain separate even for one provider; catalog success does not migrate API traffic.
Optional cover attached to a participating API follows its effective scope without
turning cover fetches into independent retry/promotion consumers.

Retain the existing LRU capacity and eviction rules: replacement updates one slot,
not a growing history. The winner is retained only while its entry is cached.
After eviction, a new acquisition uses a fresh scope, not the forgotten winner or
the original deterministic scope. Restart behaves the same; selection is not
persisted. Fresh scope does not promise a different Tor exit IP. Old acquired
records can outlive eviction solely for their existing operation/cover ownership;
they must not repopulate or bypass selection for a new logical operation.

All constructors stay in `network.rs`, retaining remote DNS, no direct fallback,
public-destination rules, runtime/origin/protocol/timeout separation and existing
cache retention. Do not change persistent state formats, environment contracts
or wallet roles.

### Request and financial boundaries

The accepted x402 model gates protected execution on payment acceptance. Unsigned
POST and GET are equally eligible; preserve exact business arguments and body
bytes and add no POST-specific idempotency proof or business-side-effect gate.
This policy concerns x402 calls, not arbitrary infrastructure HTTP requests.

| Outcome | Planned handling |
| --- | --- |
| Unsigned HTTP 400, 403, 429 or 503 | Eligible for the finite isolated retry sequence in both phases; no upstream prose classifier. A repeated 400 may simply be a bad request, and account-wide throttling may survive rotation. |
| Valid unsigned 402 | Stop circuit retries; validate the challenge normally. For API calls, retain its exact transport through signing and signed response consumption. |
| Other completed HTTP statuses | No new circuit retry, including success, redirects, 401, 404 and other 5xx. Existing discovery relay eligibility remains separate. |
| Discovery connection/reset/read-inactivity failure | Eligible while unsigned; never interrupt a response still making progress. Do not repeat a proven local SOCKS-listener outage expecting exit rotation to repair it. |
| API transport failure without an eligible completed status | Outside the initial API phase; preserve current error behavior rather than expanding uncertainty handling. |
| Policy, TLS trust, size, parse or challenge-validation failure | No new circuit retry. Preserve existing rejection and discovery relay handling; never weaken validation to recover. |
| Signed status or transport/output failure | No new generic retry, fresh challenge or signature. Preserve journaled liabilities and existing structured insufficient-funds traversal. |

For eligible 429/503 responses, honor valid `Retry-After` delay-seconds or HTTP-date
before the next network attempt, including relay fallback to that same target.
Missing, malformed or past values add no delay. Use no arbitrary maximum delay or
whole-work deadline. Scheduling must remain asynchronous and independent of other
operations; qualification authority is rechecked after any wait. Discard unused
error bodies without waiting for them. Never log credentials, private request
arguments or credential-bearing upstream prose. Report stage, ordinal, fixed
reason and available status through the existing safe diagnostic conventions.

Fresh valid caches still win without retries. Retries must not refresh failed
cache entries, persist challenges as reusable authority, change provenance or
turn one-shot pricing into background work. Waiter disconnect does not cancel an
already accepted payment owner. Explicit force/owner termination cannot launch
later dispatches or orphan disclosed financial work. Do not extend a signed
expiry, response inactivity budget or qualification allowance through retries.

## Ordered commit boundaries and acceptance

The identifiers below remain stable even if titles change. Resolve material
independent design findings before implementing the affected decision. Keep each
commit coherent with its tests and permanent documentation; do not ship an
unprotected intermediate retry path. No commits are requested merely by writing
this plan.

### T1 — Shared attempt scope and catalog/pricing retries

Implement the owner-preserving selected transport record, fresh-on-miss creation,
conditional promotion and bounded retry scopes in `network.rs` together
with their first consumers in `catalog.rs` and `pricing.rs`. Factor only common
transport/eligibility responsibilities; keep catalog cache validation and pricing
challenge interpretation with their existing owners. Resolve local proxy failure
classification using existing typed transport evidence, without probing Tor or
introducing new health state.

Try the sequence before creating/resuming the existing paid relay continuation.
On exhaustion or an ineligible failure, retain existing relay eligibility,
readiness, payment ownership and breaker semantics. A successful origin attempt
must not contact the relay. Catalog/pricing 402 interpretation remains distinct:
a valid price challenge is discovery success, not authorization to pay the target.
Malformed pricing headers still follow existing relay behavior without cycling.

Consumers to inspect: deployment startup/catalog publication, standalone loading,
strict inspection, catalog warming (including explicit direct warming), pricing
coalescing and disk caches, relay continuations and source-assigned relay payers.
Direct mode, local files and qualification restrictions remain unchanged. Lazy
help, dynamic import, `source_url_info`, RPC, treasury, cover fetches and internal
relay-service requests are outside this phase; do not silently inherit retries
through a generic reqwest hook. No new paid relay fallback is added anywhere.

Acceptance evidence:

- Fake SOCKS captures distinct auth/pools for all three attempts and unchanged
  canonical owner/token domains, different owners and concurrent retry sequences;
  destination names remain remote and public/protocol enforcement remains active.
- A successful replacement is reused by the next logical operation without
  revisiting the failed scope. Other origins on the same wallet, other wallets,
  discovery versus payer identities, and distinct transport/public/timeout policies
  stay separate. Shared aliases with the same full key share selection.
- Force ordinary LRU eviction, then verify fresh credentials on reacquisition;
  restart likewise starts fresh, never reconstructing the original deterministic
  primary. Assert unchanged cache capacity and no retained winner/history table.
- Deterministically interleave two recoveries, late original success, eviction and
  reinsertion: stale promotions neither overwrite a newer record nor resurrect an
  absent slot. A failed promotion does not fail the candidate operation.
- Hold a sibling HTTP/2 stream open on the original connection while another
  stream receives an eligible status and retries. Verify a distinct SOCKS handshake
  and physical connection for the retry, continued original-stream progress, and
  subsequent acquisitions using the promoted pool. Client cloning/auth-only
  changes must not satisfy the test.
- Verify promotion only after complete valid catalog/pricing success; malformed,
  oversized, interrupted or failed candidates and paid relay results cannot
  promote. Deferred/pre-acquired clients cannot bypass selection on later work.
- Catalog and pricing fixtures cover each eligible status, transport failures,
  success at each position, exhaustion, valid 402, 2xx pricing without challenge,
  ineligible statuses, malformed content, body limits and local proxy outage.
- Assert actual origin/relay counts, one fallback sequence per coalesced operation,
  no relay spend after origin success, cache provenance/validators, and no repeated
  origin I/O when a relay continuation waits/resumes.
- Exercise slow body progress versus inactivity, Retry-After dates/delays,
  independent concurrent progress, disconnect/cancellation and deferred startup.
  Listeners and already usable catalogs remain available during degraded discovery.
- Update network, configuration, resource inventories and testing guidance in T1;
  describe expected extra latency, cache-lifetime selection, fresh-on-miss behavior,
  policy separation and the absence of guaranteed exit diversity.

**Gate G1, after T1 and its fixups:** independently review complete discovery,
cache, startup and relay call chains plus the shared transport representation.
Verify all acceptance claims and unchanged financial/network enforcement. Inspect
selected-slot ownership, participating/nonparticipating policy separation, bounded
retention, stale/evicted-record races, pre-acquired consumers and real HTTP/2 pool
separation with uninterrupted sibling streams. Reject hidden winner registries or
deterministic fallback after eviction. Examine complexity against the baseline,
including unchanged hotspots. Exit only with material findings resolved, documented
scope and no accidental wider consumers.
API work must not build on unresolved transport or discovery findings.

### T2 — Unsigned API status retries with payer and cover ownership

Integrate the same attempt scope into `PaidClient`'s unsigned-response stage for
ordinary x402 API invocations, including POST, static and managed payers, generated
MCP tools, directory calls and source-management `raw_get`/`raw_post`. Internal
paid discovery relay calls retain their existing attempt accounting and are
explicitly excluded; avoid stacking new retries with their own unsigned recovery.
Unsigned-only qualification retains its challenge refusal and original authority.

Pin the existing payer during the sequence. Do not switch wallets, allocate,
fund, reconcile balances synchronously or reacquire application ownership merely
because the circuit changes. On a valid 402, carry the selected client and its
matching cover scope through admission, journaling, signed dispatch, response
collection and output conversion. Preserve ordinary pre-signing payer handoff:
any newly selected payer obtains its own unsigned challenge. Keep the existing
structured-insufficiency traversal; each explicitly authorized next-wallet
unsigned stage may use its own finite circuit sequence, without replaying the
preceding signed request. Generic signed responses can never enter that sequence.

Inspect `cover::registry::Owner` and the full engine/episode lifecycle: its key
must include effective scope separately from canonical payer identity. Different
scopes cannot share incompatible cover qualification, sessions or retained clients.
After promotion new calls use cover matching the selected record, while old
call/session cleanup retains its original record through replacement and eviction.
No global cover reset or forced migration of sibling providers is needed. Drop
failed unsigned responses and finish/release their optional cover calls correctly;
optional qualification or cleanup must not gate ordinary API availability.
Document any extra cover requests rather than claiming the three API attempts
bound total traffic. Prefer passing a coherent attempt value over independent
counters/flags scattered through payment and cover state machines.

Acceptance evidence:

- Exercise 400/403/429/503 before 402 and success, exhaustion and direct-mode
  behavior with real `PaidClient` fixtures; verify exact POST bytes/query/method,
  pinned payer, distinct isolation and matching challenge/signed/cover transport.
- Verify a valid unsigned 402 promotes its exact record before normal payment
  handling; signing uses that pinned record despite concurrent replacement/eviction.
  Complete unsigned success can also promote; malformed 402, failed output and
  signed outcomes cannot. A later logical call uses the selected record, while
  same-wallet other-origin calls retain their own record and cover session.
- Assert no signature, journal liability, wallet allocation or funding occurs on
  rejected unsigned attempts; a later valid 402 follows normal amount authority.
- Signed versions of all four statuses, ambiguous submission, stalled/interrupted
  paid bodies and output-conversion failure produce no generic replay or new
  authorization. Retained accepted work survives waiter cancellation/restart.
- Preserve structured insufficient-funds traversal, original identities, all
  possible charges, independent reconciliation and qualification accounting.
  Test payer handoff and retry scopes together, not only each helper separately.
- Cover tests exercise retirement, concurrent same-payer calls, pool eviction,
  optional qualification failure and unrelated response progress. In particular,
  retain old active sessions during promotion, prevent new calls from attaching to
  obsolete sessions, and independently reconstruct cover after a fresh-on-miss
  acquisition. Internal relay fixtures prove existing spending breaker/attempt rules remain intact.
- Update architecture, network, cover, source-management, wallet and testing
  guidance with delivered unsigned semantics and precise financial boundaries;
  update both resource inventories for the new attempt bound/scheduling behavior.

**Gate G2, after T2 and its fixups:** independently review all T1/T2 changes and
fixups with full MCP/raw/directory → payment → transport/cover → signing/journal →
response paths, and discovery → relay paths. Verify authority is stage-based,
never method-based; no signed generic retry or duplicate active I/O exists.
Recheck every G1 selection/pooling race through real PaidClient and cover call
chains: exact challenge/signing handle, promotion versus payment meaning,
other-provider independence, retained old sessions, caller reacquisition, and
eviction/restart without deterministic rollback or additional history tracking.
Compare CogC/CycC with the original baseline and inspect unchanged payment/cover
hotspots and new/worsening warnings. Resolve material findings before cleanup.

### T3 — Documentation handoff and plan removal

Confirm enduring rules live in the owning guides named above, with AGENTS.md
links to unsigned semantics, network policy and complexity review intact. Update
existing GET/POST and deferred polling plans only where they contradict delivered
behavior; preserve paid polling/submission no-replay requirements. Remove this
plan only after both phases and their validation are complete; retain focused
unfinished work instead if scope remains. Fix incoming links in the same commit.
Do not retain a completed-plan archive or move metrics/evidence into tracked docs.

**Final integration gate, after T3:** independently review the exact cleanup SHA
and combined branch, including every prior commit/fixup and relevant unchanged
consumers. Confirm documentation links and claims, retained evidence, no scope
leaks and all earlier gate concerns after cleanup. Verify selected-entry
replacement, fresh-on-miss lifetime, per-origin scope and pinned financial/cover
handles are explained in permanent guidance and backed by the gate evidence.
Material later changes reopen affected concerns and require appropriate revalidation before completion.

## Review and validation ordering

Before substantive implementation, give an independent read-only design reviewer
the user intent, accepted POST semantics, baseline, this plan and preserved
invariants. Ask them to challenge unnecessary gates, state and enforcement, not
merely approve the proposed arrangement. Parallelize distinct review concerns
where useful; coordinate edits explicitly and freeze sources/executables during
validation to avoid competing builds.

Independently review every exact committed SHA, including T3; reviewers must not
be authors. Address findings with `git commit --fixup=<introducing-SHA>`, review
each exact fixup independently and preserve unsquashed history through final
integration review. At each gate inspect all prior fixups in surrounding call
chains: identify root causes, repeated assumptions, hazards and coverage gaps;
add regression coverage and decide whether wider audit, simplification or design
change is needed. Prior per-commit approval is not gate approval.

Capture an agreed original-baseline complexity report with the pinned analyzer
before substantive Rust edits. At G1/G2 compare changed functions and full call
chains with:

```sh
python3 scripts/complexity.py --base 970f6bde1334eb42a540288ea1432381e1b9ae64
```

Use source-matched artifacts and local fixup comparisons where needed. Explicitly disposition relevant
CogC/CycC warnings and unchanged high-risk hotspots after evaluating concrete
simplifications; helper extraction or passing tests alone is not justification.
Retain reports under ignored `target/`, per the complexity guide.

Run focused default-Zcash tests through `scripts/zcash.sh`, check that each named
financial test is wired and executes nonzero cases, and run formatting plus
required checks appropriate to changed modules. Do not run no-default-feature
configurations, live funded calls or Docker consensus tests without separate
explicit authorization. Use existing network/relay/payment/startup fixtures and
record exact commands/results for reviewers. No broad rerun after a localized
fix unless changes or unresolved concerns justify it. Fake SOCKS evidence proves
credentials and routing policy, not actual Tor circuit/exit selection; any claim
about real circuits needs separately enabled controlled Tor observation.
