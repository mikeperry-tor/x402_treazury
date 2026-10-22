# Vendor retirement assessment

## Selected scope and baseline

Assess whether application-owned RPC adaptation, direct Pepper synchronization
and backend deposit calculation can retire **both `zingolib` and
`zingo-netutils`**, with less total maintenance and unchanged runtime policy.
Plan implementation only after that assessment passes independent review.
This document plans the work; its creation does not start runtime implementation,
upstream submission, live-funded execution or another retirement track.

The plan's original baseline was `9ec7f318cf36a8f9af9f6acb71dc12dceba38f97`.
Z0 assesses `7097bf1fcb9de3a07aadb20879ea9cbd4f726727`; the intervening
commits include the v7 cohort upgrade and Alloy retirement documentation.
The assessment starts with no tracked modifications; unrelated untracked user
files are outside its scope.
Six patched checkouts remain; both Alloy checkouts have already been retired.
The selected Zingo revision is `bfb0a2156819ea7f75a9b09b1724abfd06c8925e`
(v7.0.0). Reconfirm the working tree, exact sources and both Cargo graphs when
assessment begins; record any new baseline before comparing implementation.
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
  unchanged; public proposal cleanup releases SDK locks correctly despite private
  proposal-management APIs. Preserve ordinary expiry, network-upgrade boundaries,
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
Keep the coherent Z1 commit: changing the factory's concrete indexer also changes
its sync, calculation and submission consumers.

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

This is conditional on Gate A and authorization to implement. Use one coherent
commit for the adapter, direct sync, backend deposit calculation, retirement of
both checkouts and necessary consumer/test/tooling/documentation changes. If Z0
demonstrates an independently useful split, revise this plan into ordered stable
commit IDs with their own evidence and review gates before implementation.
Never remove existing behavior separately from its required replacement.

Update both manifests and lockfiles, source verification, patch/provenance files,
network audit, compatibility dependencies and release staging together. Keep
remaining source verification effective, including a deliberate scratch-source
mutation rejection. Preserve upstream attribution for copied RPC/prover code
when removing checkout license files. Search optional-feature and release paths
for stale retired paths. Do not delete ignored parameters, user files, wallet
state or evidence.

Acceptance includes nonzero execution of the affected real consumers:

- Direct and fixture-SOCKS gRPC, identity/clone isolation, queued cancellation,
  progressing large messages versus true stalls, and no fallback/bypass through
  `tests/network.rs`, `tests/network_audit.rs` and the owning network tests.
- Finite sync, early failure, periodic checkpoints, cancellation, supervisor EOF,
  no surviving I/O and restart through `tests/treasury_sync.rs`. Keep the engine
  independently polled while checkpoint acquisition waits for its wallet lock;
  add a regression with a checkpoint due during a lock-held asynchronous engine
  operation. An inline engine/checkpoint select can deadlock.
- Real deposit proving, ordinary-expiry/upgrade-boundary assertions, journal and
  calculation/publication failure recovery, exact bytes/inputs after restart,
  confirmation, tip/deeper forks, expiry recovery, and unchanged shielding via
  `treasury::send::proving_tests`, `treasury::lifecycle_tests` and affected Store tests.
  Retain Z0's real Sapling and Orchard-only proving coverage, explicit pre/at-NU7
  expiry, public proposal cleanup and lazy embedded-parameter behavior.
- Wallet format/read-save-reopen compatibility, current diagnostics/redaction,
  client-scoped synthetic codec and Pepper's actual merge fixture. Keep SDK/mock
  activation schedules aligned and strict mock submission validation enabled.
- Both dependency graphs, crypto/payment signing, default application tests,
  sequential opt-in proving/lifecycle/codec tests, feature Clippy, format and
  remaining vendor checks. Use `scripts/check.sh` or its verified component
  commands; choose socket-capable execution up front for socket fixtures.
- Release-tool parameter staging/integrity/network-denial regressions. Retain
  descriptor and upstream Git metadata as release inputs: the current vendored
  archive's version-only fallback does not apply to the upstream Git checkout.
  Shared-cache comparison is not cross-cache reproducibility. Existing
  qualification limits remain: offline tests do not qualify live settlement,
  real Tor, Docker consensus or a fresh reproducible release. Those runs require
  their own prerequisites and applicable authorization.

Update enduring explanations with Z1 in `vendor/README.md`, `vendor/ZINGO.md`,
`docs/network-egress.md`, `docs/development.md`, `tests/LIFECYCLE.md` and affected
release/tooling guides. Update wallet ownership/recovery guidance where integration
details change. Intended resource and timing policy is unchanged; any proposed
policy change reopens design review and both owning inventories.

The selected design must also remove the actor preparation's unused connection:
canonical validation already occurs through scan reuse or a new scan, and backend
calculation no longer needs an installed SDK indexer. Preserve direct preparation's
network validation and the existing deadline/cleanup path. Verify that actor
preparation issues no additional connection/RPC after canonical validation, while
direct wrong-network preparation still defers without accepting inputs. The Z0
scratch calculation retains that redundant connection; its tests do not qualify
this final simplification.

**Gate B, after Z1:** independently review every plan commit/fixup together with
complete network, sync, financial and build/release call chains. Exit requires
both whole-checkout removals, exercised failure/restart boundaries, fewer justified
responsibilities, reviewed CogC/CycC warnings and no unresolved material findings.

### Z2 — Complete the selected track and preserve unfinished assessments

Verify permanent documentation and the AGENTS ledger link before cleanup. Remove
only completed planning material; keep the unfinished assessments below in a
focused plan until individually resolved or transferred with their validation
requirements and reopening conditions. Completing Zingo does not complete them.
Keep no completed-plan archive. **Gate C** reviews the exact cleanup SHA and final
combined branch after cleanup, including all prior commits/fixups and handoff.

For Z0/Z1/Z2, obtain independent review of every exact committed SHA and relevant
consumers; the reviewer must not be its author. Address findings with
`git commit --fixup=<introducing-SHA>` and independently review each fixup. Preserve
unsquashed history through final review. Every gate examines recurring assumptions,
fixup causes, hazards and coverage gaps across all prior work and decides whether
they require broader audit, regression coverage, simplification or design changes.
Later material changes reopen affected gates and validation. Do not build on
unresolved material findings or use metric-driven helper extraction as a substitute
for simpler ownership. Existing prior-track tests do not qualify new implementation.

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
