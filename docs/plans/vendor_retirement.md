# Vendor checkout retirement

## Intent and baseline

Minimize locally maintained dependency checkouts. Prioritize upstream contributions,
compatible dependency selections and simple supported adapters that can remove an
entire checkout. Reducing a patch is intermediate progress only when it advances
a credible complete removal route. The current task establishes tracking and this
plan; it does not change runtime behavior or submit upstream contributions.

The starting baseline is `5cc4a80444e1d81d2052c326c36618c4403a752a`.
There are eight patched crate directories, used by both the application and the
independent compatibility workspace. No replacement candidate is qualified.
The [checkout ledger](../../vendor/README.md#checkout-retirement-tracking) owns
current blockers, upstream links, candidate revisions and next actions. Update it
in place rather than keeping a second status inventory here. Reconfirm the actual
baseline and complete blocker set before each implementation branch.

Owning guides are [vendor provenance and reasons](../../vendor/README.md),
[Zingo patches](../../vendor/ZINGO.md), [network policy](../network-egress.md),
[retry ownership](../network-egress.md#retry-ownership),
[resource limits](../local-resource-limits.md),
[wallet ownership](../wallet-rotation.md),
[best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[wallet observation](../wallet-rotation.md#wallet-observation-policy),
[development](../development.md), [testing](../testing.md),
[lifecycle coverage](../../tests/LIFECYCLE.md),
[complexity review](../../tests/COMPLEXITY.md#review-discipline) and
[reproducible builds](../reproducible-builds.md).

Preserve journal-before-disclosure, exact accepted inputs/bytes/identities,
financial uncertainty and restart recovery. SDK/blockchain state owns current
spendability; do not add historical accounting gates, capital budgets or replay
authority. Keep identity-bound transports, Tor remote DNS/no direct fallback,
connection versus response inactivity, progressing work and cancellation/drain
ownership. Existing application retries remain distinct from disabled automatic
HTTP transport recovery. Keep toolchain pins, state formats and qualification
claims intact. None of this work authorizes live-funded qualification.

## Selection and upstream work

Start with Alloy compatibility and the independent Hyper/h2 correctness fixes:
these offer concrete routes to whole-checkout removal. Priority is provisional;
candidate availability, remaining blockers and total implementation burden govern
the next choice. Do not hold an independently removable checkout behind an
unrelated blocked track.

For each candidate, identify the exact checkout(s) it can eliminate and every
remaining source, manifest, test and tooling blocker. Inspect current upstream
source and releases; record exact revisions rather than treating a feature name,
accepted PR or newer version as equivalence. Check both workspace graphs and
the application/x402 version constraints. Record an infeasible candidate with its
concrete blocker and next reopening condition in the ledger, then pursue another
viable exit rather than weakening behavior to force a removal.

Prepare upstream contributions as independently useful changes with small faithful
regressions: cancellation/reset correctness, reorg handling and fixture branch
hashes; or supported transport/configuration APIs for application policy. Do not
submit the entire cross-crate application patch as one upstream feature. Within an
authorized contribution task, inspect repository contribution instructions and
existing issues/PRs, prepare and independently review the contribution, then submit
and record its link and dependencies in the ledger. Submission and acceptance are
prerequisites, not retirement evidence. Keep minimal upstream tests separate from
application ownership and financial regressions that must remain here.

Evaluate local behavior changes only against the complete checkout blocker set.
State the affected request types, before/after behavior, safety implications and
expected directories removed. Prefer supported APIs and a small adapter in the
existing owner over new proxy, retry, connection or workflow machinery. Compare
total responsibilities and complexity, including failure paths. Do not enable
unstarted HTTP recovery, weaken cancellation, restore forced-success drain
timeouts or reduce diagnostic detail merely to delete isolated hunks while the
checkout remains. Any selected policy change needs a revised, independently
reviewed coherent commit boundary and updates to its owning guides before work.

## Ordered delivery boundaries

These boundaries describe future work. V1 chooses available candidates before
implementation; V2/V3 are initial priorities, not promises of removal. If one is
blocked, record why and proceed to a ready track. Independently review each design
before substantive implementation, including necessity of checks and opportunities
to remove responsibilities. Freeze sources and avoid competing builds during
validation; run feature configurations sequentially.

1. **V1 — Qualify and rank complete exits.** Inspect candidate graphs and upstream
   code for all blockers of the selected checkout. Prepare focused upstream work
   where necessary. Commit durable ledger updates and any required regression
   coverage with accurate ownership/verification documentation; generated
   investigation logs stay under ignored `target/`. Exit with an exact candidate
   and acceptance checklist, or a specific external blocker and reopening condition.
   Do not claim a checkout can be removed from source inspection alone.
2. **V2 — Retire compatible Alloy checkouts.** Qualify an unpatched graph for the
   runtime primitives and macro expander. Alternative Keccak features alone do not
   remove their current unconditional SHA3 dependencies. Remove both overrides
   together when the coherent graph permits; an independently viable single-crate
   removal may be selected explicitly. In the same commit update both manifests
   and lockfiles, delete only retired sources/provenance, adapt verification and
   update `vendor/README.md` and `docs/development.md`. Run crypto vectors, payment
   signing including Permit2, the independent compatibility suite, and remaining
   vendor verification. A crypto dependency migration also needs offline wallet
   restore/serialization and relevant treasury coverage; do not loosen pins to
   suppress a resolution failure.
3. **V3-H and V3-2 — Retire Hyper and h2 independently.** Each identifier is one
   coherent removal commit for a qualified upstream candidate. Inspect both sides
   of pending-open cancellation even when only one checkout exits. Preserve raw
   HEADERS absence, zero-capacity cleanup, explicit/implicit cancellation, no stale
   bodies and healthy multiplexed connection reuse. Run the queue/timeout tests in
   `src/network/`, relevant HTTP and gRPC cases in `tests/network.rs`, network audit
   and compatibility suite. Update both workspace graphs, provenance verification,
   the ledger and network guidance with each removal. Combine these commits only
   when a real version/API dependency makes their replacement inseparable.
4. **V4 — Select the next complete Reqwest or Zingo exit.** Before implementation,
   revise this boundary into ordered checkout-specific commits with stable IDs,
   exact candidates, full consumers, tests and owning documentation. Do not use V4
   as a generic authorization to refactor. Reqwest must replace every timing,
   progress/backpressure and recovery-control hunk. Each Zingo checkout must replace
   all mapped hunks and reproduce the standalone manifest/dependency graph;
   preserve coherent sibling Git revisions/source identities. A supported injection
   API alone does not retire Pepper's sync/reorg changes or LightClient ownership.
   Removing `zingolib` must also replace `compat/zingolib`'s direct path dependency
   and `scripts/reproducible.py`'s local parameter-staging assumptions in that same
   coherent commit, preserving hash verification and network-denied builds.
   Select focused sync, diagnostics, cancellation/restart, synthetic lifecycle,
   network and release-tool tests according to the complete affected chain.
5. **V5 — Finish the selected retirement cycle and remove this plan.** Complete
   qualified removals and transfer remaining externally blocked work to concrete
   ledger actions/reopening conditions. Verify permanent guide and AGENTS links,
   remove this plan and its transient references, and independently review the
   exact cleanup SHA and final combined branch. Keep no completed-plan archive.

Every removal commit includes its replacement, consumer changes, necessary
recovery path, tests, permanent documentation and verifier/tooling adaptations.
Do not temporarily disable provenance checks or delete a regression because its
original fixture/source was vendored. Verify that remaining-source mutation is
still rejected when verification code changes. Search for residual path references,
including optional test features and release inputs; do not edit Cargo caches or
reference checkouts. Removing checkout files never deletes private parameters,
wallet state, financial evidence or retained qualification artifacts.

## Review and acceptance gates

For every implementation commit, obtain independent review of the exact committed
SHA and relevant consumers; the reviewer must not be its author. Provide intent,
starting baseline, actual commands/results and remaining limitations. Resolve
findings with `git commit --fixup=<introducing-SHA>`, independently review each
exact fixup and preserve unsquashed history through final integration review.
Do not build on unresolved material findings.

- **G1, after each selected V2/V3 retirement track: dependency and transport gate.** Review all
  prior plan commits and fixups with both Cargo graphs and complete payment,
  discovery, Base, HTTP/gRPC and cancellation call chains. Exit requires verified
  whole-checkout removals, effective remaining provenance checks, preserved
  financial/network contracts and no material findings. Skipped tracks remain
  explicitly blocked in the ledger rather than counted as removals.
- **G2, after each selected V4 SDK or HTTP ownership boundary.** Review all prior
  plan commits/fixups with full relevant sync, proposal, submission, observation,
  restart, parameter/build and diagnostic paths. Reassess whether adapters truly
  simplify ownership. Exit requires exercised failure/cancellation boundaries,
  coherent sources, truthful build/qualification claims and no material findings.
- **G3, after V5: final integration gate.** Independently review the accumulated
  branch, all fixups, the exact cleanup SHA and permanent documentation handoff.
  Confirm actual checkout count/graph removal, preserved local changes/evidence
  and actionable remaining external blockers. Earlier approvals do not exempt code.

Each gate analyzes fixup causes, recurring assumptions, hazards and testing gaps
across all earlier commits and complete call chains. Decide explicitly whether
they require wider audit, regression coverage, simplification or design changes.
Material later changes reopen affected gates and validation. For substantive Rust
refactors and architecture gates compare CogC/CycC against the agreed starting
baseline using the pinned analyzer, covering changed functions, full chains and
unchanged high-risk hotspots. Review warnings and concrete simpler alternatives;
neither passing tests nor helper extraction justifies moved complexity.

Use socket-capable execution up front for socket fixtures under the
[test sandbox guidance](../testing.md#sandbox-permissions). Require nonzero
execution of the intended tests. Run remaining vendor checks and compatibility
checks for graph changes, with broader application/Clippy/feature validation when
the changed consumers require it. Evidence from offline fixtures does not qualify
live settlement, real Tor confinement or a reproducible release. Adapted release
tooling must retain network-denial and parameter-integrity controls; do not claim
release reproducibility without its dedicated qualification.

Keep each delivered rule in its owning guide with its implementation commit.
`AGENTS.md` must retain its link to the permanent checkout ledger, not this plan.
Keep current patch reasons and source-verification boundaries in `vendor/README.md`
and Zingo details in `vendor/ZINGO.md`; update network/resource inventories together
if policy is deliberately changed. Before V5, verify these destinations and all
plan links so cleanup leaves maintained guidance and focused remaining work.
