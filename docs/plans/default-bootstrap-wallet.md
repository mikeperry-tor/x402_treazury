# Default bootstrap wallet

## Status and scope

Implementation and selected validation are complete. Final G2/evidence review
is pending; P6 cleanup remains. Working branch:
`default-wallet-bootstrap`. Starting baseline: `9ca86bdb407152ab1581e14c5d110185d870bb48`.
The unrelated untracked `state` entry is outside this work and must remain untouched.
This planning change does not yet change examples to unsupported TOML or advertise
unimplemented behavior in maintained guides.

The operator wants immediate payment capacity through one ordinary managed group,
then permanent separation as participating groups receive their own initial credit.
Avoid funding every group during setup, avoid waiting for swaps in agent calls,
and weaken the timing correlation of closely spaced initial funding transactions.
This is an availability/privacy tradeoff, not an unlinkability guarantee.

Implement the deployment option, its durable first-use demand and routing,
initial-allocation scheduling, explicit bootstrap CLI behavior, the default small
reserve range reduction from $2–4 to $2–3, examples and maintained documentation.
Do not add shared custody, cross-group payment retries,
group merging, address transfers, top-ups, financial budgets, dependencies or a
global bootstrap shutdown state. Do not implement other active plans.

## Agreed behavior

### Configuration and ordinary group ownership

The optional root deployment table is:

```toml
[wallet_bootstrap]
bootstrap_group = "general"
deferred_groups = ["personal", "health", "financial"]
```

- `bootstrap_group` names one ordinary managed wallet group. It retains its own addresses,
  payment cap, reserve bounds, rotation, retirement, transport identity and journals.
  It can have its own endpoint bindings and remains managed after all participants
  transition. There is no shutdown or merge operation.
- `deferred_groups` explicitly lists participating managed groups. Validate names against
  resolved wallets, including generated groups if explicitly named; reject missing
  or static groups, duplicate participants and self-membership. Do not infer
  participation from all declarations or allow nested/cyclic bootstrap policies.
  Require a nonempty list; an absent table is the opt-out.
- Keep source/listener bindings and generated `auto_v1_` identities unchanged.
  Endpoint and source declarations continue to name their intended destination
  group. Agent/session fields cannot select bootstrap policy or wallet owners.
- Groups outside the list retain ordinary serving funding. With no table, preserve
  current serving and `wallet bootstrap` behavior.
- Show the resolved policy, initial setup reserve bounds, eventual configured
  reserve bounds and effective routing meaning in appropriate configuration/preview
  output without fetching catalogs, allocating wallets or exposing secrets. Do not
  relabel eventual inventory totals as initial bootstrap requirements. Distinguish
  the explicit command's bootstrap-only setup target from ordinary serving, which
  also funds any configured nonparticipating groups on their normal opportunities.

### First use, initial credit and routing

- A validated actual remote API invocation activates its participating destination
  group once, including a free call or an API-response cache hit. Record demand at
  the trusted invocation boundary after binding/argument/permission validation;
  source discovery, tool search/listing and invalid or unresolved calls do not
  activate it. Cover static tools, `tool_call`, restored/dynamic tools and
  `raw_call`, using their existing authenticated source/listener binding.
- Internal catalog, pricing, help/document retrieval, source discovery,
  registration, refresh and cache warming never activate participant funding,
  including when they pay a relay. An actual authored API tool invocation is not
  exempt merely because its tool name contains `help`.
- Persist only the missing durable demand fact, keyed by stable group UUID, using
  the existing serialized Store. Concurrent first uses coalesce. A canceled waiter
  does not undo an accepted Store command. Add no per-endpoint counters or second
  monetary/readiness ledger. Inspect stable-schema/additive-initializer consumers
  before choosing its precise table layout; preserve existing private state.
- Route a new invocation to the bootstrap group until the destination has ever
  recorded its first positive initial credit. Reuse the existing consumed-claim
  fact (`funding_jobs.readiness_requirement IS NOT NULL`) across the group's full
  history, including RETIRED wallets. Current roles, zero balances, advisory price
  estimates, later depletion, reorg repair or refunds cannot reopen bootstrap use.
  Partial positive credit counts; do not introduce a minimum-balance handoff gate.
- Existing credited groups bypass bootstrap immediately, without requiring a new
  activation or changing their original funding ownership. Uncredited allocations
  and accepted swaps remain owned and recoverable across enablement/config changes.
- Select and capture one effective managed group before unsigned API dispatch;
  use its cap, inventory boundary, key ownership, journal, payer transport and cover
  ownership through the call. A call captured before credit may finish with its
  original group; calls routed after credit publication use the destination.
  Do not restart signed or uncertain work because routing changed.
- Bootstrap calls use the bootstrap group's cap even when it is lower or higher
  than the destination's cap. After handoff the destination's cap applies. Keep
  existing Store preparation/journal enforcement; no cap override or minimum of
  two groups' caps. Diagnostics must identify the effective payment authority.
- A first invocation records demand and wakes the existing background owner; it
  does not allocate a wallet, perform swap I/O or wait for funding in its payment
  path. Inventory exhaustion remains best effort. After first credit, a depleted
  destination cannot fall back to bootstrap. Existing within-group unsigned and
  structured insufficient-funds traversal rules remain unchanged.

### Discovery, relay and readiness consumers

- Non-activating work may use existing bootstrap inventory while its implicitly
  assigned participating group lacks credit. Preserve explicit
  `relay_fallback.wallet` selection and existing relay permissions; do not turn
  bootstrap configuration into new relay/disclosure authority.
- Resolve implicit discovery payer selection consistently with normal source
  assignment. Catalog/pricing readiness must not wait for participant funding that
  can only be activated by later API use. Relay payment alone creates no demand.
- Inspect `PaidClient` documentation address, discovery readiness/change
  notification, pinned document authority, catalog cache reuse and relay
  `fetch_when_ready` consumers. Account for group handoff without lost wakeups,
  stale authority reuse for new calls or duplicate in-flight I/O. Existing document
  acquisitions keep their accepted payer and liabilities.
- The aggregate startup announcement considers effective bootstrap inventory/cap
  for uncredited participants, not completion of every dormant reserve. Keep it
  advisory; bind listeners before funding and preserve per-endpoint catalog
  availability and failure scope. Do not add a new startup gate or observation.

### Funding opportunities and explicit commands

- In ordinary serving, an uncredited participating group without demand gets no
  new allocations. Do not disable its Store group to achieve this: authority,
  accepted-work observation and reconciliation remain intact.
- On activation, independently draw an inclusive uniform integer 60–240-second
  opportunity interval. Use the same window on subsequent opportunities until
  first credit. Calls, unrelated wakes and completions do not redraw or advance
  that group's due time. This explicitly authorized cadence spreads initial
  allocations; it is not a resource cap, settlement deadline or privacy proof.
- Both enabled small and large slots may fill at the same opportunity, with the
  existing small-first independent commits. Do not add per-slot schedules. After
  first credit, subsequent rearming uses ordinary 30–120-second opportunities;
  do not advance an already armed opportunity simply because credit arrived.
- Bootstrap and nonparticipating groups retain ordinary scheduling. Restart
  preserves demand/credit and redraws applicable volatile intervals; no catch-up
  bursts, persisted timer ledger, emergency refill or whole-work timeout.
- Reuse the retained funding owner and earliest-due wait. Its existing inventory
  discovery must notice newly activated groups without spawning another worker
  or repeatedly resetting timers. No new foreground funding owner.
- Accepted jobs, quote/source preparation, polling, error backoff, Base credit
  observations, refunds and recovery retain their existing independent schedules.
  New-allocation deferral never delays accepted work or grants another deposit.
- `auto_fund=false` prevents new allocations/proactive rotation; accepted work
  continues. First-use demand may be recorded for a later enabled run without
  silently overriding this setting.
- With the table present, `wallet bootstrap` captures and funds only the bootstrap
  group's enabled reserve claims, promptly and without participant activation,
  catalogs or listeners. It does not wait for every participating or other
  declared group's reserves. Its summary must truthfully identify the selected
  group/claims. Existing accepted work retains its original recovery ownership.
- Keep `wallet sync`, status, preview, configuration inspection and offline
  initialization non-activating. Explicit one-pass allocation and qualification
  must retain their distinct authority: bootstrap policy cannot increase reviewed
  allocation counts or substitute bootstrap wallets for named qualification cases.

### Configuration changes

Use current configuration plus durable UUID-bound demand and credit facts, not a
persisted deployment phase. A new participant follows the ordinary uncredited
rule; a previously credited participant routes directly. Removing the table or
removing a participant restores ordinary allocation/routing for that group.
Re-adding it preserves demand and first-credit history. Changing the bootstrap
group affects new calls only; captured work keeps its original identity. Renaming
groups preserves UUID-bound facts and requires matching TOML references under the
existing CLI contract. Disabled/removed groups keep accepted work under existing
recovery rules; they do not remain phantom participants. No global completion
counter or automatic shutdown/reactivation branch is necessary.

### Default reserve range and funding guidance

- Change only the default small reserve maximum in `src/rotation/config.rs` from
  `"4.00"` to `"3.00"`; keep its minimum `"2.00"`. This applies to omitted/empty
  small slots in all deployments, whether or not bootstrap routing is enabled.
  Explicit fixed targets/custom ranges and the optional $10–25 large default
  retain their configured meaning. Do not rewrite unrelated explicit $2–4 ranges.
- A uniform nominal small allocation has mean $2.50. For a small-only group,
  funding the initial wallet and its first replacement takes $4–6 nominal across
  two allocations, averaging approximately $5 before bridge-floor adjustments,
  fees and other costs. The first wallet starts READY, becomes ACTIVE on its
  first journaled payment and opens a refill vacancy; the replacement becomes
  READY later. This is a preparation/capital estimate, not a requirement to fund
  both at startup or a promise of $5 simultaneously spendable after payments.
- Explain randomization headroom above the $5 average (two unadjusted draws may
  total $6), plus source fees, bridge costs, ongoing use/replacements and any
  separately enabled large reserves. A bridge-minimum floor can bias nominal
  draws upward, so $5 is not a guaranteed all-in cost or universal observed mean.
  The $3 ceiling can reject a route whose required nominal minimum exceeds it;
  retain existing amount-limit reporting/backoff rather than silently increasing
  the bound. This is operator-selected nominal sizing, not a new ZEC spending cap.
- Preserve consumed claims, historical initial credits, original allocation
  identities and accepted/prepared operation terms. Apply changed defaults through
  existing current-claim/unsigned quote refresh rules; preserve in-bounds saved
  draws and do not cancel or reinterpret accepted work above the new default.
- Update README, configuration/reserve-sizing/fragmentation guidance, wallet CLI,
  example guide and default comments in `managed.toml`, `comprehensive.toml` and
  `research_desk.toml`, plus other authored default references found during
  implementation. Small plus optional large initial nominal inventory becomes
  $12–28 instead of $12–29. Correct stale $2–5 wording where it describes defaults;
  retain historical evidence and intentional custom-range examples.

### Examples, setup amounts and privacy explanation

Implement these example changes only with parser/runtime support:

```toml
# examples/deployments/comprehensive.toml
[wallet_bootstrap]
bootstrap_group = "web"
deferred_groups = ["company"]

# examples/deployments/research_desk.toml
[wallet_bootstrap]
bootstrap_group = "general"
deferred_groups = ["personal", "health", "financial"]
```

- Preserve source selection, endpoint bindings, provider permissions, state paths
  and existing per-group caps/explicit reserve sizes. Implicit small slots adopt
  the new $2–3 default. Update surrounding comments about
  separate payers, discovery funding, initial inventory and immediate isolation.
  Review endpoint instructions/search keywords for affected routing claims;
  provider capabilities do not otherwise change.
- Both examples currently use implicit $2–4 small reserves. Comprehensive setup
  will initially fund only web at the new $2–3 nominal default, rather than the
  baseline's $4–8 for both groups. With the new default but bootstrap disabled,
  the two groups' initial small allocations total $4–6 nominal. README's
  current $2–5/$4–10 wording is stale; correct it from resolved configuration,
  including other affected default-range references. Suggest capital covering the
  $3 upper initial nominal bound plus source fees/bridge costs, and explain additional
  capital for company preparation, relay payments and future replacements.
  Distinguish this one-wallet setup amount from the approximately $5 per-group
  ACTIVE-plus-READY preparation estimate above. Nominal sizing is not guaranteed
  credit or the deployment's lifetime cost.
- Comprehensive company/social/toolbox calls initially use web's $0.10 cap,
  then company's $0.50 cap after first credit. Some selected calls may therefore
  be refused during preparation; refusal must not delay/undo activation. Research
  desk health initially uses general's $1 cap rather than health's $0.50 cap.
  Document the selected authority instead of implying destination caps throughout.
- Explain that closely spaced sequential bootstrap swaps provide a funding-time
  correlation signal even for separate addresses. Separate initial addresses still
  avoid direct payer reuse; do not claim they provide no privacy or quantify their
  benefit without evidence. First-use activation and randomized later opportunities
  can weaken timing correlation of later wallets. Shared bootstrap payments use a
  visibly common payer; timing, amounts, providers, request content and settlement
  behavior can still correlate activity. Delayed funding does not erase past links.
- Explain opting out by commenting out the whole table for separate groups from
  the beginning. Existing historical links/funds are not erased or moved. With the
  table absent, the usual all-declared-managed-group funding policy applies.

## Owning contracts and implementation surfaces

Read these guides before changing their areas and update contracts in place:

- [Planning workflow](../planning-workflow.md), [development](../development.md),
  [validation selection/order](../testing.md#selecting-validation-by-change) and
  [complexity review](../../tests/COMPLEXITY.md#review-discipline).
- [Configuration](../configuration.md), [architecture](../architecture.md#wallet-selection-and-network-isolation),
  [best-effort rules](../wallet-rotation.md#best-effort-wallet-rules),
  [observation](../wallet-rotation.md#wallet-observation-policy) and
  [reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy).
- [Agent authority/invocation](../agent-sources.md#trusted-invocation-and-payment-scope),
  [wallet administration](../wallet-cli.md), [local cadence inventory](../local-resource-limits.md),
  [network timer inventory](../network-egress.md#remaining-timers-and-budgets),
  [startup tests](../../tests/STARTUP.md) and [lifecycle tests](../../tests/LIFECYCLE.md).

Expected consumers: `src/deployment.rs`, `src/deployment/{bootstrap,startup,readiness,existing_read}.rs`,
`src/rotation/{assignment,config,manager,store}.rs`, `src/rotation/store/`,
`src/rotation/funding/{scheduler,opportunity_tests}.rs`, `src/payment.rs`,
`src/payment/document_identity.rs`, `src/discovery_relay/`, `src/catalog_state.rs`,
`src/pricing/discovery.rs`, `src/server/` and `src/discovery/` invocation paths.
Inspect actual call chains before choosing helpers. Keep all outbound constructors
in `src/network.rs`; preserve payer-bound transport/Tor and public destinations.
No network-policy exception, dependency or platform-specific mechanism is needed.
Preserve Ironwood-only spending, journal-before-disclosure and one-shot initial
claims. Never rewrite accepted financial identities or load real wallet secrets.

## Ordered commits and review gates

Every implementation commit includes its focused tests and owning documentation;
do not postpone contracts to a final documentation-only sweep. Keep all commits
and independently reviewed `git commit --fixup=<introducing-SHA>` corrections
unsquashed. Update this plan with exact review/validation evidence as work evolves.

| ID | Commit boundary | Acceptance and review |
| --- | --- | --- |
| P0 | Commit this active plan and complete independent design review against baseline and exact plan SHA. | Resolve material design findings before implementation; record any intentionally bounded open implementation choices. |
| P1 | Add root configuration/resolution and durable demand/first-credit projection; leave policy execution unavailable until integrated in P2. | Pure policy-validation/config-show/preview and Store tests for absent policy, invalid groups, history, UUID rename/reopen, idempotent activation and no allocation. Serving/explicit funding with the new table must fail clearly at this intermediate stage rather than silently apply old all-group funding. Document syntax and implementation status truthfully. Independently review exact SHA. |
| P1a | Change the global default small range to $2–3 and update its focused tests, owning guides, README default-range wording and existing example comments together. | Verify omitted/empty slots resolve 2,000,000–3,000,000 atomic units, configured sizing and large defaults remain unchanged, derived inventory totals agree, and unsigned/accepted-work compatibility follows existing policy. Explain $2.50 per allocation and approximately $5 across two small allocations with headroom/cost qualifications. Do not advertise bootstrap behavior before P2–P4. Independently review exact SHA and affected sizing consumers. |
| P2 | Integrate trusted activation, effective routing/readiness/relay consumers and serving opportunity eligibility, enabling serving as one coherent contract. | Do not land a state where routing waits on deliberately unfunded groups or demand accidentally triggers eager allocation. Retain P1's explicit `wallet bootstrap` rejection with the table present until P3, so the old all-group command cannot run under the new policy. Cover effective caps, no metadata activation, atomic route capture, restart and accepted-work ownership. Update configuration, architecture, agent-source, wallet-rotation and both cadence inventories. **Architecture gate G1:** independent combined P0–P2 review and CogC/CycC review across complete affected call chains before building on this integration. |
| P3 | Restrict explicit `wallet bootstrap` capture/funding to the bootstrap group, then remove its temporary policy rejection, preserving other command/qualification authority. | Exercise real CLI summary/capture selection with synthetic funding; absent table retains existing semantics; no participants activated/allocated or catalogs fetched. Update wallet CLI/configuration/qualification references as affected. Independently review exact SHA and consumers. |
| P4 | Enable both example tables and update README, example guide/comments and affected endpoint descriptions. | Parse/resolve actual examples offline; derive funding totals from P1a's $2–3 default and verify effective cap transitions; remove immediate-separation/all-groups-prefunded claims. Distinguish $2–3 bootstrap setup, approximately $5 per-group ACTIVE-plus-READY preparation and additional costs. Include explanatory setup/bootstrap and opt-out usage. Independently review exact SHA. |
| P5 | Complete selected validation and independent final combined review of every accumulated commit/fixup and affected call chain. | **Final gate G2:** no unresolved material findings or compelling simplifications; applicable complexity review and source-matched evidence; distinguish offline behavior from unmeasured privacy/live settlement. Retain plan for user review rounds. |
| P6 | After G2 and applicable user review rounds, remove completed plan and temporary review records, repair links and commit cleanup. | Verify maintained guides own all enduring contracts; independently review exact cleanup SHA and final combined history. Preserve financial evidence and unsquashed plan ancestry; no push/merge authority is granted. |

P1's new state/schema must remain a minimal additive capability with no new
allocation side effects. If P2 is too large for a coherent review, split it only
at dormant internal seams while keeping policy unavailable until all routing,
readiness and allocation consumers are integrated; revise this plan first.

## Validation and concrete failure scenarios

Select meaningful functional/race/side-effect tests; do not add provider inventory
snapshots, benchmark matrices, arbitrary sleeps or fixtures asserting implementation
shape. Wire every new financial test into its actual module and verify nonzero
test counts. Use deterministic barriers and injected opportunity draws.

1. Parse/resolve no table and both real examples; reject invalid/static/self/duplicate
   groups. Inspection/preview and declaration/registration leave demand, allocations,
   signing and outbound network absent. Show eventual versus bootstrap totals.
2. Two endpoints sharing one participant concurrently invoke it: one UUID-bound
   activation, no direct allocation/quote/submission and one independent due time.
   Free/cache-served API calls activate; denied/invalid/discovery/metadata calls do
   not. Failure under the bootstrap cap still leaves accepted demand.
3. Exercise catalog and pricing URL origin failure with paid relay acquisition,
   metadata help, dynamic registration and warming. Existing bootstrap inventory
   can satisfy implicit readiness, explicit relay wallet choices remain intact,
   and participant demand/jobs stay absent. Handoff between readiness subscription
   and wake cannot lose progress or duplicate active acquisitions.
4. Pin unsigned challenge, signature, journal and dispatch to one effective pool,
   cap and payer transport. Credit arriving during a call only changes later route
   captures. Captured accepted calls and bodies survive cancellation/handoff; no
   signed replay or cross-group insufficient-funds fallback. Cover bootstrap caps
   both below and above destination caps and unchanged manual-header restrictions.
5. First positive credit including tiny/partial credit switches later calls;
   depletion, all-wallet retirement, reopen, refund or chain repair cannot switch
   back. Existing credited groups bypass bootstrap. Missing optional evidence
   neither blocks handoff nor grants another funding claim.
6. No first-use demand means no new participant allocations; initial opportunities
   stay within injected 60–240 bounds and normal draws within 30–120. Payment and
   completion wakes do not redraw; both slots commit independently. A failed large
   allocation cannot undo small success. Restart redraws without losing activation;
   suspension skips missed opportunities. Slow accepted funding continues while
   sibling groups progress; allocation cadence does not slow source/credit recovery.
7. Bootstrap exhaustion does not make calls wait for swaps. After handoff, exhausted
   participant inventory never returns to bootstrap. `auto_fund=false` prevents new
   allocations while accepted operations remain observable and owned.
8. Cover add/remove/re-add participant, table removal, changed bootstrap group,
   UUID-preserving rename and disabled groups. Ordinary nonparticipants continue
   funding; group enablement is not misused as a demand flag.
9. Explicit bootstrap with small/large bootstrap reserves captures only that group,
   uses prompt allocations, emits truthful summary and never activates participants.
   Without the table, retain existing capture behavior. Sync/status/preview remain
   non-activating; qualification retains reviewed named allocations/API budgets.
10. Startup sockets/catalog availability remain independent of initial funding;
    effective-cap readiness announcement can complete on bootstrap inventory.
    Source failures remain endpoint-scoped. Check safe, meaningful severity/message
    changes and no credentials/upstream prose in diagnostics.
11. Resolve omitted and explicitly empty small slots to $2–3, optional large to
    $10–25, and both to $12–28 initial nominal inventory. Check two small nominal
    allocations total $4–6 with an unadjusted uniform mean of $5 arithmetically;
    no statistical benchmark is needed. Existing focused draw tests cover
    inclusive bounds. Preserve explicit fixed/custom ranges, previously consumed
    claims and accepted larger operations; cover unsigned quote reuse/refresh and
    bridge-floor redraw within $3 versus amount-limit refusal above it. Derive
    example/README totals from actual resolution, not stale prose or snapshots.

Run cheap socket-free contract checks first. At G1/G2 obtain complete complexity
analysis against the starting baseline using `python3 scripts/complexity.py --base
9ca86bdb407152ab1581e14c5d110185d870bb48`; retain artifacts under ignored `target/`.
Review full JSON/CSV, CogC >20/CycC >40 warnings and unchanged affected hotspots;
record simplification decisions rather than hiding complexity in helpers.

For runtime implementation select production `make build` and all-target
`make clippy`, inspecting dependency/compiler diagnostics as well as exit status,
then affected compatibility and runtime stages under the testing guide's ordering.
Use Make/test-runner pinned tooling. Select affected `check-compat`, startup,
payment/relay/discovery, funding-opportunity, CLI and `check-lifecycle` consumers;
record exact selections rather than blindly running a feature matrix. Run each
selected named stage to completion, preserving valid earlier evidence after fixes.
Coordinate editors/build processes and freeze sources during validation. Choose
socket-capable execution up front for localhost fixtures if required; socket-free
checks remain sandboxed. Do not run funded integration, Docker consensus, real Tor
or reproducibility qualification for this change without separate authorization.
Offline tests do not measure unlinkability or qualify live settlement.

Plan-only validation: review links, syntax examples, source contracts, actual
example amounts and the diff; no application build or lifecycle execution is needed
until implementation. Do not claim planned checks have run.

## Review and execution evidence

- P0 draft: coordinating review traced schema/initial-credit, ownership/caps,
  assignment, funding opportunities, explicit bootstrap and relay-purpose paths at
  the baseline. Earlier independent feasibility discussion informed this plan;
  it is not a review of an implementation.
- Fresh independent plan/design review examined draft SHA-256
  `a9e2bc7c43e5008fdd8c9283eeb09201633d16f40a033821ad85886b4c8bfb42`
  against the starting baseline. It verified assignment, actual invocation,
  ManagedPool caps/readiness, document authority/relay waits, scheduler, bootstrap
  capture and actual example defaults/caps. Findings: explicitly reject policy
  execution at intermediate P1; distinguish command setup costs from serving's
  nonparticipant costs. Both are incorporated above. No other material design
  finding or compelling simplification was reported. Review was read-only, with
  no state access, builds, runtime, privacy or settlement validation.
- Plan-only checks verified relative links, whitespace, TOML snippets and both
  actual example reserve declarations. No application tests were run or needed
  for this planning-only change.
- Independent exact-SHA and combined review examined
  `b5f0a4c284612c0e45e35beb3fa8644fa7cbc556` against the starting baseline.
  Only the plan was changed. It confirmed the earlier corrections and identified
  one remaining staging clarification: keep explicit bootstrap rejected during P2
  until P3 implements its restricted capture. This is addressed in the P2/P3 rows
  by a fixup to that commit. The exact fixup/final combined review is reported at
  handoff; carry its SHA/result into the next implementation update.
- Independent review of exact fixup
  `c5b647384ea23c374649a72d8c460af63da8e95b` and the combined original plan found
  no unresolved material finding; it did not validate runtime behavior.
- User-expanded scope adds the $2–3 global small default and documentation of
  approximately $5 nominal to establish a small ACTIVE plus replacement READY,
  including timing, provider-floor and cost qualifications. Fresh independent
  design assessment found no material objection and identified the derived
  $12–28 large-enabled total and the existing above-ceiling refusal contract.
  This revision updates P1a/P4 and the corresponding acceptance scenarios;
  implementation remains pending. Independent draft review of SHA-256
  `f0b639ec5f944d7e5993affdcd79d9e2a896cebfe93eb3e215fe4140a9820ac0`
  found no material finding or compelling simplification. Plan links/TOML and
  proposed nominal bounds/means were checked; existing relevant default/draw/redraw
  tests should be reused rather than duplicated. No runtime or live bridge
  validation was performed. Exact revision review is reported at handoff.
- Historical initial status: P1/P1a–P6, G1/G2 and implementation validation were pending. Update this section
  with reviewed SHAs, findings/dispositions, fixups, actual nonzero test counts
  and carried-forward evidence.
- Before completion, verify AGENTS.md's existing links still reach the owning
  configuration, observation, reconciliation, network and testing contracts; add
  an owning-guide link only if the maintained boundary needs one. Keep detailed
  feature contracts in those guides, not a duplicate checklist in AGENTS.md.

- Execution begins on `default-wallet-bootstrap` at plan SHA
  `dd62a40082074d39bb92c217c55ef7c3b10df628`; reused all recorded planning reviews.
  Independent read-only verification of that exact plan found no unresolved staging
  issue. P1 adds pure policy resolution/setup projection and UUID-bound demand plus
  all-history consumed-claim credit queries, including an additive schema-23 initializer.
  Serving and explicit bootstrap reject the table at this intermediate boundary.
  Three configuration/staging and three Store durability/concurrency tests authored;
  compiler/runtime validation awaits the frozen integrated snapshot. Exact-SHA review pending.

- P1 implementation committed as `40dbebff`; independent exact-SHA review requested.
  P1a changes default small nominal sizing to $2–3, tests absent/empty slots and
  $12–28 large-enabled totals, and updates maintained default references together.
  Existing saved-choice/unsigned-refresh/accepted-quote contracts remain unchanged;
  targeted runtime evidence remains pending. No new dependency or policy cap added.

- Independent exact reviews found no material findings in P1 `40dbebff` and P1a
  `5334824c`, including combined P0–P1a and saved-draw/accepted-work consumers.
  User naming correction is fixup `8bd205de`: canonical table fields are now
  `bootstrap_group` and `deferred_groups`, with obsolete/mixed names rejected.
  Its independent exact review found no material finding; behavior is unchanged.
- P2 integrates trusted invocation activation, immutable effective payer capture,
  nonactivating documents/discovery/relay, effective-cap advisory readiness and
  serving eligibility/opportunities. Qualification keeps named authority.
  Independent early review found transient eligibility-read propagation and
  offline reads unnecessarily requiring both saved pools; corrected before commit.
  Busy/locked reads use existing inventory retry cadence without changing armed
  opportunities, and existing document reads select only required saved authority.
  A smaller finite/unlimited-cap advisory issue is also corrected. Cover preflight
  checks both possible shared owners under its existing configuration contract.
- P2 focused validation initially passed three configuration tests. Subsequent
  58-test bootstrap run passed 57, including actual relay handoff/explicit payer,
  both cap directions and funding-worker restart/paused/dormant scenarios. One
  authored invalid-input fixture incorrectly assumed missing required query inputs
  are rejected by the existing router; it now exercises an actually invalid missing
  path parameter instead. Earlier fixture synchronization used aggregate health to
  infer completion across clock jumps; deterministic source callbacks and explicit
  owner replacement now establish the intended ordering. EVM identity assertions
  compare canonical lowercase addresses. No production contract was loosened.
  Complete rerun pending; interrupted/failed runs are not counted as suite passes.
- Initial G1 complexity analysis completed (504 files, 11,303 callable spaces,
  no exclusions) against `9ca86bdb`; artifacts under ignored
  `target/complexity-runs/20261110T213053Z-8d11f1f9`. Independent full JSON/CSV and
  unchanged-hotspot review disposition: `validate_selected_cover` CogC 28 checks
  possible shared authorities; `initialize_wallets_mode` CogC 50/CycC 53 retains
  qualification, degraded transport, selected closure and cleanup;
  `schedule_owned` CogC 70/CycC 76 keeps eligibility retry separate from accepted
  I/O and unchanged armed instants. No compelling simplification was identified.
  Renaming/tests invalidate individual source hashes; final source-matched run pending.
- P2 production build, warning gate, compatibility and broader runtime/lifecycle
  stages remain pending until integration gates permit subsequent implementation.
  macOS lib-test linking emitted an oversized `__eh_frame` compact-unwind warning;
  no warning-free compiler claim is made from those test runs. Tests use the
  repository's copied/ad-hoc-signed macOS harness and only temporary unfunded state.

- G1 passed for exact P2 `7752966fd6a7e5e63336476a6d68e6ddf1249435`
  and combined P0–P2 history: independent review found no unresolved material
  finding or compelling simplification. Focused `make test CARGO_ARGS='--lib bootstrap'`
  passed 58/58 using socket-capable copied macOS harnesses. Source-matched metrics
  in `target/complexity-runs/20261110T213920Z-8b1fa46e` cover 504 files/11,309
  callables with no exclusions/hash mismatches. All three warning dispositions
  above remain accepted. General required-query schema checking is pre-existing
  router behavior; the invalid-path test establishes rejected-route activation
  boundaries, not exhaustive schema validation.
- P1a follow-up fixup `8299d678` corrects three missed default-only assertions in
  `tests/managed_config.rs`; fixed/custom sizing assertions remain unchanged.
  Independent exact review and execution pending.
- P3 selects only the configured bootstrap group for explicit setup capture,
  initialization and summary. Its worker allocation-name selection restricts new
  allocations without narrowing Backend configuration or accepted-job observation.
  Added prompt both-slot/other-owned-job and real CLI policy/absent-policy coverage;
  command staging rejection removed and owning guides updated. Exact SHA review
  and selected final validation pending.

- P3 committed `6d60890c`; independent exact review requested alongside P1a
  assertion fixup. P4 enables both example tables with the user-selected canonical
  field names. Maintained README/example guidance distinguishes bootstrap-only
  setup, eventual inventory and two-allocation preparation capital, effective cap
  transitions and timing/common-payer privacy tradeoffs. Pure inspection tests
  resolve actual examples and opt-out bindings. Author's Python TOML comparison
  against pre-P4 files, removing only the new root table, found every other value
  identical (bindings, state paths, permissions, providers, explicit sizing/caps
  and endpoint instructions). Endpoint instructions/search keywords were reviewed;
  no routing/cap claims required agent-facing changes. Parser/runtime execution
  and exact review remain pending; no live privacy claim follows.

- Follow-up P2 consumer correction: offline document reads now obtain effective
  managed profiles through the existing assignment projection, so an explicitly
  named generated bootstrap group works without a matching explicit wallets entry.
  Extended the no-SDK existing-authority regression with that case. A supplemental
  socket-free restored-registry/wrapped-dynamic/raw invocation regression verifies
  registration, search, unknown tools, denied/invalid raw requests do not activate;
  actual valid API invocation records demand even when transport is unavailable,
  without allocations, signers or I/O. Initial test fixture omitted required empty
  listener catalog views; corrected the fixture without production changes.
  Combined focused bootstrap rerun passed 59/59 (copied macOS harness, socket-capable
  only for tests that need TLS). Exact fixup and affected G1 scope review pending.

- Exact independent P3 `6d60890c`, P4 `4d8423b1` and P1a fixup `8299d678`
  reviews found no unresolved material runtime finding. Review initially questioned
  continuing unselected pre-existing ALLOCATED jobs, then reconciled the full agreed
  accepted-allocation contract and withdrew a proposed additional funding gate.
  Durable allocations already express accepted initial-funding intent; selection
  limits new allocations/completeness, while existing jobs retain current-authority
  funding/recovery. Clarified owning CLI/configuration prose and the test's intent;
  no redundant per-step financial enforcement added. Exact clarification fixup
  review pending. Full `managed_config` socket-free target passed 18/18, including
  actual example policies, opt-out bindings and the default-only assertion fixup.
- Final selected validation: production `make build`, full `make check-clippy`
  (production compiler plus all-target regtest-feature Clippy warning gate), full
  `make check-compat`; then library runtime tests, affected startup/CLI/configuration/
  treasury-sync/manual-header integration consumers, and full `make check-lifecycle`.
  Existing default-example evidence carries forward without another managed_config
  execution unless affected inputs change. Final source-matched complexity and G2
  combined review remain required before cleanup. No live-funded, Docker-consensus,
  real-Tor or reproducibility qualification is selected.

- Final warning gate found an eight-argument earliest-wake helper. Moved the
  opportunity/retry earliest-instant projection onto its existing Opportunities
  owner and passed that one value to the general wake aggregator. This reduces
  coupling without another state type, lint exemption, timer or changed due time.
  Existing wake/retry stability tests exercise the same behavior. P2 fixup exact
  review and affected checks/production build/final metrics rerun required.
  Earlier successful managed_config evidence is unaffected; the incomplete warning
  gate is not counted as a pass.

- Focused wake validation exposed an existing ordering-dependent assertion from
  baseline commit `d97cf7c1`: an explicit worker may observe a previously allocated
  job before its newly allocated job because durable IDs do not imply that order.
  The fixture now accepts those known existing jobs while requiring the explicit
  job to progress without clock advancement; its final exact allocation count is
  preserved. No production ordering/policy changed. Baseline-targeted fixup review
  and focused rerun pending; other nine opportunity tests passed in that run.

- Final exact source reviewed and validated: `629a56423c8559656f5a412d92c63a68a04fc934`
  (all preceding unsquashed commits/fixups retained). Independent exact reviews of
  scheduler-projection fixup `418f45fd` and baseline fixture-order fixup `629a5642`
  found no material findings. The focused opportunity rerun passed 10/10.
- Production `make build` passed on that frozen source with no warnings; full
  `make check-clippy` passed its production lib/bin compiler and all-target
  `zcash-regtest` Clippy warning gate without warnings. The earlier failed gate
  remains recorded and is superseded only by these successful corrected-source runs.
- Final complexity artifacts:
  `target/complexity-runs/20261110T215222Z-56dd99ad`, 504 files, 11,323 callables,
  no exclusions or source/lock hash mismatches. Independent full JSON/CSV review
  retained the three owning-function dispositions: cover validation CogC 28,
  initialization CogC 50/CycC 53, scheduler CogC 71/CycC 78. The scheduler's small
  increase preserves selected new allocations with complete existing-job ownership;
  its earliest-wake projection adds no state. No compelling simplification remains.
- Full `make check-compat` passed: crypto vectors 2, payments 15, SDK compatibility
  3, MCP/server 11 (31 total), with its fixed Pepper verification also passing.
- `make test CARGO_ARGS='--lib --test startup --test cli_contracts --test config --test treasury_sync --test headers'`
  passed: library 1,391 (24 existing optional/helper cases ignored), CLI 26,
  configuration 16, headers 11, startup 19 (one helper ignored), treasury sync 27
  (one public-mainnet qualification ignored). No failure; 1,490 parent tests passed.
  The earlier full socket-free managed_config result, 18/18, remains applicable:
  its relevant parser/default/example inputs did not change. No repeated matrix run.
- Full `make check-lifecycle` passed with its fixed `zcash-testutils` selection:
  67 tests passed, one isolated fatal-owner helper ignored by the parent harness.
  The complete synthetic SDK reorg test finished successfully; no work deadline,
  skip or contract relaxation was introduced. This is offline synthetic-chain
  evidence, not live settlement, Docker consensus, Tor qualification or measured privacy.
- `make fmt-check`, tracked whitespace, maintained local Markdown file targets,
  AGENTS owning-guide headings and exact example TOML-value preservation checks
  passed. The only untracked entry is the original untouched `state`.
  Clarified wallet rename guidance to update both new policy name references while
  preserving UUID-bound demand/credit; this prose-only addition changes no runtime
  or test input. No AGENTS duplication or additional owning-guide link is needed.
- Compiler/runtime logs are retained under ignored
  `target/default-bootstrap-wallet-validation/` (`build-final.log`,
  `check-clippy-final.log`, `check-compat.log`, `runtime-default.log`,
  `check-lifecycle.log`). Runtime harnesses use copied/ad-hoc-signed macOS binaries.
  Lib-test linking emitted the existing oversized `__eh_frame` compact-unwind
  diagnostic; production/compiler/Clippy gates were clean. Ignored helpers and
  optional/live cases are not counted as passes. Build/executable revision stays
  `629a5642`; evidence carries forward through prose-only review/cleanup commits
  without claiming those binaries embed a later Git SHA.
- P5 final exact evidence/G2 review is requested on this update; P6 begins only
  after no material finding or compelling simplification remains. No push or merge
  is authorized. Completed plan deletion will retain all history and private evidence.
