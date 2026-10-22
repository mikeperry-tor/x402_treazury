# Vendor retirement assessment

## Selected scope and baseline

Assess whether application-owned RPC adaptation, direct Pepper synchronization
and backend deposit calculation can retire **both `zingolib` and
`zingo-netutils`**, with less total maintenance and unchanged runtime policy.
Plan implementation only after that assessment passes independent review.
This document plans the work; its creation does not start runtime implementation,
upstream submission, live-funded execution or another retirement track.

The plan's original baseline was `9ec7f318cf36a8f9af9f6acb71dc12dceba38f97`.
Z0 assessed `7097bf1fcb9de3a07aadb20879ea9cbd4f726727`; the intervening
commits include the v7 cohort upgrade and Alloy retirement documentation.
The assessment started with no tracked modifications; unrelated untracked user
files are outside its scope.
The Z1 planning baseline is `822261bf0a77d4610c6084b22870b3557d7c6a3f`,
the independently reviewed Z0 documentation commit; its Rust sources are unchanged
from the assessed baseline. Reconfirm implementation HEAD, working-tree state,
source identities and both locked graphs before Z1a. Reuse Z0 metrics only for
verified identical sources; capture a new comparison baseline for any source drift.
Six patched checkouts remain; both Alloy checkouts have already been retired.
The selected Zingo revision is `bfb0a2156819ea7f75a9b09b1724abfd06c8925e`
(v7.0.0). Record any new baseline before comparing implementation.
The permanent [checkout ledger](../../vendor/README.md#checkout-retirement-tracking)
owns current blockers, qualified candidates and upstream follow-up. This plan
owns assessment ordering and acceptance work, rather than a second status inventory.

Read [Zingo patch reasons](../../vendor/ZINGO.md),
[network policy](../network-egress.md), [resource limits](../local-resource-limits.md),
[wallet ownership](../wallet-rotation.md),
[best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[wallet observation](../wallet-rotation.md#wallet-observation-policy),
[development](../development.md), [testing](../testing.md),
[lifecycle coverage](../../tests/LIFECYCLE.md),
[build tools](../../scripts/README.md),
[release staging](../reproducible-builds.md) and
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline).

## Current behavior and proposed replacement

Today patched `GrpcIndexer` transports are installed on `LightClient` for SDK
sync and deposit calculation. Sync runs on an application-owned runtime;
the SDK startup patch acknowledges first polling without a launch deadline.
Connected deposit calculation retains ordinary transaction expiry. Shielding
already uses the backend's public calculate-only API. Exact selected inputs and
accepted intent are journaled before calculation and survive restart. The process
retains completed bytes and snapshots through failed publication and cancellation;
those results survive restart after their durable commit. Process loss before
publication leaves an uncertain accepted calculation, not permission to recalculate.

The candidate replaces the two crates' local changes with:

- An application implementation of public `Indexer`/`TransparentIndexer` traits
  around the generated gRPC client and existing network transport.
- Direct public `pepper_sync::sync` execution using the existing session owner,
  wallet, progress and checkpoint machinery.
- Backend calculation of deposit proposals, preserving ordinary expiry and the
  accepted selected-input binding, using the shielding integration as precedent.

These are assessment candidates, not production replacements. The generic
Pepper entry point predates v7. New status/wait helpers do not bypass the existing
LightClient launcher. Simply keeping LightClient offline changes its high-level
calculation to epoch-end expiry; that is not this plan's intended behavior.

Keep all production outbound constructors in `src/network.rs`. Preserve direct/Tor
identity, remote DNS, public-destination policy, no direct fallback, clone/cache
ownership and per-response inactivity below protobuf assembly. Do not add adapter
retries, monetary tracking, queues, caps or whole-work deadlines. Preserve current
request metadata handling, including removal of total gRPC deadlines by the
progress transport; copying upstream RPC wrappers must not restore them.

SDK/blockchain state remains authoritative for current spendability. Preserve
journal-before-disclosure, exact accepted inputs/bytes/identities, liability fences,
ordinary expiry, canonical release proofs and current recovery authority. NEAR
status, Base credit and refunds can inform observations but cannot prove ZEC
non-inclusion or authorize replacement deposits. Keep state and environment
contracts stable. Canonical recovery simplification is a separate future
assessment, requiring equivalent SDK evidence rather than weaker status hints.

`pepper-sync` remains vendored in this candidate. Its complete-message timers,
loader/worker drain ownership and diagnostic context are separate blockers;
the RPC adapter cannot change them. Preserve the executable merge regression and
root workspace membership while that checkout remains.

## Ordered commit boundaries

### Z0 — Assess the complete two-checkout exit

Before substantive implementation, an independent read-only design reviewer
challenges the candidate, unnecessary checks and opportunities to remove existing
responsibilities. Resolve material findings before developing the affected option.

Map every hunk, manifest adaptation, consumer, optional feature, fixture and
build/release dependency for each of the three Zingo checkouts. Inspect public
upstream APIs and source identities at the exact candidate revision. Include
`compat/zingolib`'s direct dependency, retained Pepper path overrides and upstream
sibling revision coherence; a root-only build is insufficient.

Use isolated development probes where source inspection cannot establish:

- **RPC adaptation:** all active trait methods, generated message defaults,
  status/error-code and transaction-ID parsing, streaming types and cloned
  transport ownership work through the existing factory. Compare roughly twenty
  delegations with supported upstream injection APIs and the current fork burden;
  avoid copying a second networking or retry implementation.
- **Direct sync:** the real wallet satisfies the public engine traits; finite
  completion, protocol checks, progress, periodic encrypted checkpoints and
  successful witness capture are preserved. Retain cancellation/runtime teardown,
  worker joins and final snapshot ordering, without a second sync controller.
- **Deposit calculation:** the selected proposal survives authority acceptance
  unchanged; public cleanup clears the stored proposal and releases its pause
  after the wallet guard drops, without copying private proposal-management APIs
  or inventing output locks. Preserve ordinary expiry, network-upgrade boundaries,
  fee/output checks, CSPRNG/prover behavior and exact original selected inputs.
  Exercise real proving and retained publication/restart behavior; shielding
  precedent alone does not establish deposit compatibility.
- **Source replacement:** both dependency graphs can use ordinary unmodified
  upstream sources for the two crates, with the same retained Pepper patches.
  Standard-cache parameter staging, hashes and network-denied release builds
  remain supported without a local Zingolib source path.

Keep experimental outputs and metrics under ignored `target/`; never patch Cargo
caches or reference checkouts. Freeze sources during validation and serialize
feature builds. Record a durable proceed/defer decision and concrete source/API
blockers in the ledger and owning guides; retain a focused regression with the
implementation that needs it, rather than committing a probe framework or report.

**Gate A, after Z0:** independent review examines the whole proposal, both graphs,
the sync/proposal/calculation/submission/recovery chains and actual probe evidence.
Compare CogC/CycC for changed functions and full chains against the agreed baseline,
including unchanged high-risk consumers. Compare total ownership and maintenance
responsibilities, not just deleted lines. Exit with a supported, tested replacement
design and concrete Z1 acceptance mapping, or a specific blocker/reopening
condition. A negative assessment is a valid result. Do not weaken expiry, network
or recovery policy to force an exit; do not automatically switch to another track.

**Z0 complete; Gate A passed:** proceed with the paired Z1 design at the pinned
revision after separate implementation authorization. Independent assessment
review found no unresolved material finding or public source/API blocker. The
[replacement boundary](../../vendor/ZINGO.md#two-checkout-replacement-boundary)
maps all three checkouts' patch/manifest responsibilities; the
[ledger](../../vendor/README.md#checkout-retirement-tracking) owns the decision.
The ordered Z1 sequence below separates behavioral prerequisites from the factory
and source transitions. The factory's concrete indexer and all affected concrete
consumers still change together. Z0 qualified the combined candidate; it did not
qualify each intermediate state.

Isolated probes compiled ordinary upstream Zingolib/netutils in both dependency
graphs with the retained Pepper override. Real Orchard/Sapling calculation,
ordinary expiry before/at NU7 and across an upgrade boundary, cleanup after
calculation failure, retained publication/restart, network/sync/wallet regressions,
synthetic lifecycle, scoped codec, Pepper merge and independent compatibility
checks passed. Release-tool regressions passed; the unmodified upstream build
script also passed under actual OS network denial with hash-verified isolated
standard-cache parameters. Source/lock identities, logs and same-analyzer CogC/CycC
comparisons remain under ignored `target/z0-assessment/`; no probe framework is
committed. Existing SDK scan/build hotspots remain SDK-owned. Z1 will delete
launcher choreography and the unused actor connection rather than copying
algorithms or extracting helpers for scores. Production integration remains Z1
work; these probes do not qualify a fresh release or live execution.

### Z1 — Implement and qualify the selected replacement

This remains conditional on implementation authorization. Independently review
this revised plan and its exact committed SHA before Z1a; updating the plan does
not start implementation. Execute Z1a through Z1d in order. Each commit must build
and preserve complete behavior, with its necessary replacement, tests and owning
documentation together. Resolve its material findings before dependent work.

All financial, network, state/environment and timing invariants above apply to
every intermediate state. Z1a/Z1b use the existing patched indexer factory; Z1c
activates the application adapter while retaining existing source overrides until
Z1d. This separates runtime adaptation from source/build changes without a second
factory, temporary runtime flags, fallback paths or unused adapter scaffolding.
Neither checkout is counted as retired before Z1d qualifies its removal.

#### Z1a — Backend deposit calculation

Replace high-level stored-proposal calculation with backend calculation of the
exact public proposal retained through authority acceptance. Preserve original
selected inputs, fee/output checks, CSPRNG, sender OVK, ordinary NU7-aware expiry
and lazy proving from embedded parameters. Clear the stored proposal/pause through
the public client API after dropping the wallet guard on success or failure.
Introduce upstream attribution with copied prover code. Preserve SDK pending-state
storage and existing snapshot/publication ownership rather than duplicating them.

Remove deposit calculation's `set_indexer` use and the actor's now-unused
connection in this same commit. Canonical validation already occurs through scan
reuse or a new scan. Retain direct preparation's network validation and the shared
deadline/cleanup path. The Z0 scratch calculation retained the redundant
connection, so its passing probes do not qualify this deletion.

Consumers/evidence: `src/treasury/send.rs`, input binding, retained publication,
submission/recovery and affected Store paths; execute real Orchard/Sapling proving,
pre/at-NU7 and upgrade-boundary expiry, public cleanup, journal/calculation and
publication failure/restart through `treasury::send::proving_tests`, focused
`treasury::lifecycle_tests` and Store tests. Assert exact original inputs/bytes,
no automatic replay, no extra actor connection/RPC after canonical validation and
direct wrong-network deferral before input acceptance. Update both lockfiles for
any new direct dependencies and verify both graphs without changing their source
overrides.

Documentation with Z1a: calculation/attribution details in `vendor/ZINGO.md`,
ownership explanations in `docs/wallet-rotation.md`, coverage in
`tests/LIFECYCLE.md`, and the ledger's prerequisite status in `vendor/README.md`.

#### Z1b — Direct Pepper execution

Run public Pepper sync with the existing patched `GrpcIndexer` inside the current
session/runtime owner. Keep one independently scheduled engine task with an owned
join handle, finite completion, protocol checks, local progress/mode state and
successful public witness capture. Remove the SDK launcher/first-poll choreography;
do not add another sync controller. Retain checkpointing, cache release, runtime
teardown and blocking-worker joins before the final snapshot. `LightClient`
remains the offline wallet/public API host; this does not retire its checkout yet.

Consumers/evidence: `SyncSession`, actor/CLI/supervisor callers, diagnostics and
wallet snapshots. Execute finite sync, early/protocol failures, periodic encrypted
checkpoints, cancellation, supervisor EOF, no surviving I/O and restart through
`tests/treasury_sync.rs` and focused lifecycle tests. Add the regression where a
checkpoint becomes due during an asynchronous engine operation holding the wallet
lock; an inline engine/checkpoint select can deadlock. Preserve wallet
read/save/reopen compatibility and sanitized diagnostics.

Documentation with Z1b: current sync integration in `vendor/ZINGO.md`,
`docs/network-egress.md` and `docs/wallet-rotation.md`, coverage in
`tests/LIFECYCLE.md`, and prerequisite status in `vendor/README.md`.

**Gate B1, after Z1b — Treasury ownership:** review Z1a/Z1b and all earlier plan
commits/fixups across proposal selection, acceptance, calculation, sync,
checkpointing, publication, submission and canonical recovery, including unchanged
shielding consumers. Audit the causes of expiry/prover drift, lock-held awaits and
stale connection requirements. Exit requires exercised failure/restart boundaries,
unchanged financial authority, complete task/snapshot ownership and resolved
complexity/design findings before Z1c. Apply the accumulated review discipline
below, not just the latest commit's tests.

#### Z1c — Activate the application RPC adapter

Implement the nineteen active public-trait delegations over the generated client
and existing progress transport. Change the factory/cache return type and every
concrete consumer together, including submission and the private network fixture.
Preserve generated defaults, streaming types, status/error-code and transaction-ID
parsing, request metadata removal, clone/cache ownership, identity isolation and
all direct/Tor policy. Keep constructors in `src/network.rs`; add no retries,
networking implementation, controller or deadline owner. Introduce attribution
with copied RPC code. The adapter must serve real consumers in this commit while
the existing checkout overrides and their verification remain intact until Z1d.

Consumers/evidence: `src/network.rs`, the adapter, `src/treasury/submission.rs`,
all factory callers and `tests/support/network_boundary.rs`. Execute RPC/submission
semantics, direct and fixture-SOCKS gRPC, identity/clone isolation, cache eviction
and release, queued cancellation, progressing large messages versus true stalls,
and no fallback/bypass through `tests/network.rs`, `tests/network_audit.rs` and
owning network tests. Revalidate affected treasury consumers through the active
adapter, including exact-byte submission and cancellation; passing lower-level
transport tests alone is insufficient.

Documentation with Z1c: factory/adapter boundaries in `docs/network-egress.md` and
`vendor/ZINGO.md`, affected coverage in `tests/LIFECYCLE.md`, and the ledger's
still-vendored source status in `vendor/README.md`.

**Gate B2, after Z1c — Transport and financial consumers:** review all preceding
plan commits/fixups across the factory, every RPC/clone, sync, preparation,
submission, cancellation and teardown. Reopen B1 concerns affected by the adapter;
trace any metadata, error-mapping or ownership fixes through sibling consumers.
Exit requires equivalent network/financial contracts through the active adapter,
no unused replacement scaffolding or duplicate ownership, and resolved
complexity/design findings before Z1d. Apply the accumulated review discipline.

#### Z1d — Retire both checkouts and qualify the source transition

Switch both manifests and lockfiles to ordinary unmodified upstream Zingolib and
netutils at the coherent pinned revision; replace the compatibility workspace's
direct Zingolib path as well as its override. Remove both tracked checkout sources
and obsolete patch/provenance entries together. Preserve copied-code attribution,
the Pepper override/patches, root membership and executable private merge fixture.
Keep remaining source verification effective, including deliberate mutation
rejection in scratch sources. Search optional-feature and release paths for stale
retired references. Do not delete ignored parameters, user files, wallet state or
financial evidence.

Update release tooling/evidence at this actual source transition: parameter staging
and hash verification must work without a local Zingolib source path under network
denial. Record observed upstream Git metadata and the actual generated descriptor
as release inputs; the vendored archive's crate-version fallback (which can include
`_dirty` when Git finds no repository) no longer applies. Do not alter Cargo caches,
toolchain/profile pins or the release sandbox to obtain matching output.
Shared-cache comparison does not establish cross-cache reproducibility.

Consumers/evidence: both Cargo graphs, optional-feature compilation paths,
compatibility dependencies, source verification, release staging and all migrated
runtime consumers. Execute wallet-format/read-save-reopen, crypto/payment signing,
default application tests, sequential opt-in proving/lifecycle/codec tests,
Pepper's actual merge fixture, feature Clippy, format and remaining vendor checks.
Keep SDK/mock activation schedules aligned and strict mock submission validation
enabled. Lifecycle evidence must include confirmation, tip/deeper forks, expiry
recovery and unchanged shielding. Run release-tool staging/integrity/network-denial
regressions. Use `scripts/check.sh` or its verified component commands; choose
socket-capable execution up front and keep sources/executables frozen during
validation. Reuse earlier evidence only where source/configuration identity and
the source transition justify it; do not repeat unaffected full suites at every
earlier commit by default.

Documentation with Z1d: final retirement/provenance in `vendor/README.md` and
`vendor/ZINGO.md`; source/verification integration in `docs/development.md` and
`scripts/README.md`; release inputs in `docs/reproducible-builds.md`; affected
network, wallet and lifecycle guides updated with the final source integration.
Earlier commits must already document their implemented behavior rather than
deferring all permanent explanations to this commit.

**Gate B, after Z1d — Full retirement/integration:** review every preceding plan
commit/fixup and the complete network, sync, financial, both-graph and build/release
chains together, reopening B1/B2 concerns affected by source replacement. Exit
requires both whole-checkout retirements, exercised failure/restart boundaries,
fewer justified maintenance responsibilities, reviewed CogC/CycC findings,
effective remaining verification and completed permanent-documentation handoff,
with no unresolved material finding. Apply the accumulated review discipline.

Existing qualification limits apply throughout: offline tests do not qualify
live settlement, real Tor, Docker consensus or a fresh reproducible release.
Those runs need their own prerequisites and applicable authorization. Intended
resource/timing policy is unchanged; a proposed policy change reopens design
review and both owning inventories.

### Z2 — Complete the selected track and preserve unfinished assessments

Verify permanent documentation and the AGENTS ledger link before cleanup. Remove
only completed planning material; keep the unfinished assessments below in a
focused plan until individually resolved or transferred with their validation
requirements and reopening conditions. Completing Zingo does not complete them.
Keep no completed-plan archive. **Gate C** reviews the exact cleanup SHA and final
combined branch after cleanup, including all prior commits/fixups and handoff.

### Review ordering and accumulated evidence

For Z0, this revised plan, Z1a–Z1d and Z2, complete the relevant validation on frozen
sources, then obtain independent review of the exact committed SHA and surrounding
consumers; the reviewer must not be its author. Give reviewers intent, scope,
baseline and actual nonzero test evidence. Address findings with
`git commit --fixup=<introducing-SHA>`, revalidate the affected behavior and obtain
independent exact-SHA review of each fixup. Resolve material findings before the
next dependent commit; pass each wider gate before crossing its boundary.
Preserve unsquashed history through final review.

Every wider gate examines all preceding plan commits and fixups, including the
revised plan and Z0 findings, alongside complete relevant call chains. Review
the bug/fixup history explicitly: identify the failed assumptions, search sibling
consumers and failure paths for similar instances, and check whether fixes exposed
regression gaps. Decide whether recurring patterns require a wider audit, new
regression coverage, simpler ownership or a changed design. Per-commit approvals
do not exempt any earlier code or fixup from accumulated review. Scope independent
audit concerns distinctly rather than duplicating identical checks.

Use the same pinned analyzer to compare CogC/CycC with the confirmed implementation
baseline and, where relevant, the preceding approved commit/fixup. Verify source
identities before reusing artifacts. Inspect changed functions, complete call
chains and unchanged high-risk SDK/Pepper/backend/application hotspots; review
warnings and concrete simplifications rather than summing scores or extracting
helpers merely to lower them. Passing tests and advisory scores alone do not
justify the current ownership/control flow. Keep generated metrics/evidence under
ignored `target/`; durable contracts belong in the owning guides named above.

Later material changes reopen affected gate concerns and validation. Do not build
on unresolved material findings. Existing prior-track tests or Z0's combined
probes do not qualify a new intermediate implementation state.

## Deferred assessments retained from the broader plan

These preserve unfinished work, not authorization to execute it alongside Zingo.
Select a track explicitly, capture a fresh baseline and define its implementation
commits/gates after assessment. Current status and candidates stay in the ledger.

| ID | Assessment and exit evidence |
| --- | --- |
| H / H2 | Assess Hyper and h2 independently against exact upstream revisions; inspect both sides of pending-open cancellation even if only one checkout can exit. Preserve raw HEADERS absence, zero-peer-capacity cleanup, explicit/implicit cancellation, no stale bodies and healthy multiplexed reuse. Require queue/timeout tests in `src/network/`, HTTP/gRPC cases in `tests/network.rs`, network audit and independent compatibility. Combine removals only for a demonstrated version/API dependency. |
| R | Assess a complete Reqwest replacement for post-connect header timing, nonempty body progress, backpressure and disabled unstarted-request recovery. Compare supported hooks/adapters and any proposed policy change across each affected request type, with before/after behavior and whole-checkout payoff. Use the owning HTTP timeout/queue and network/compatibility regressions; reducing one retry hunk alone is not a retirement. |
| P | Assess Pepper message-timeout control, truthful retained loader/worker drain and typed sanitized diagnostics together, including standalone manifests and the private merge fixture/workspace membership. Require progress/stall, cancellation/restart, diagnostics and synthetic lifecycle/merge evidence. Relaxing logs alone does not remove this checkout; do not replace sync algorithms locally merely to avoid a dependency fork. |
| E | Watch for upstream typed canonical recovery evidence that can replace local proof plumbing while preserving exact accepted inputs/bytes and release authority. A network adapter, NEAR status or Base credit alone is not such evidence. Assess a concrete API when one exists; do not add a watcher, ledger or new proof framework in anticipation. |

For upstream work, first inspect contribution instructions and existing issues/PRs.
Prepare independently useful fixes/APIs with small faithful upstream regressions,
separately from application financial-ownership fixtures. Actual submission needs
its own authorization. Record links, merge revisions, exact candidate versions and
remaining blockers in the ledger. An accepted PR or newer release is not qualified
replacement evidence. Preserve both workspace graphs, regression/provenance
coverage and relevant build/release inputs with every eventual retirement.
