# Remaining vendor retirement assessments

The [checkout ledger](../../vendor/README.md#checkout-retirement-tracking) owns
current checkout blockers, candidate revisions and upstream follow-up. This plan
preserves unfinished assessment work. The maintained
[Zingo integration guide](../zingo-integration.md) owns the application RPC adapter,
direct Pepper synchronization, backend calculation and remaining Pepper patches.

Before selecting another track, read the [network policy](../network-egress.md),
[resource limits](../local-resource-limits.md),
[wallet ownership](../wallet-rotation.md),
[best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[wallet observation policy](../wallet-rotation.md#wallet-observation-policy),
[development](../development.md), [testing](../testing.md),
[build tools](../../scripts/README.md),
[release staging](../reproducible-builds.md) and
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline).

## Assessment and implementation ordering

Capture the selected track's exact HEAD, working-tree state, both locked dependency
graphs, source identities and affected consumers. Preserve unrelated local files.
An independent read-only design reviewer must challenge the proposed replacement,
unnecessary checks and opportunities to remove responsibilities before substantive
implementation. Assess every checkout blocker, optional feature, fixture and
build/release input against public APIs at the exact candidate revision. Keep
probes and generated evidence under ignored `target/`; never patch Cargo caches.

Preserve journal-before-disclosure, original accepted inputs, signed bytes,
identities, liabilities and recovery authority. SDK/blockchain state owns current
ZEC spendability. Preserve ordinary NU7-aware transaction expiry, state and
`X402_*` environment contracts. All production HTTP/gRPC constructors remain in
`src/network.rs`, preserving identity-bound direct/Tor transport, public destinations,
remote DNS, no direct fallback and per-response inactivity. Do not add retries,
monetary tracking, caps, queues or whole-work deadlines to obtain a retirement.

The assessment gate reviews both graphs and complete affected financial, network,
concurrency and build/release chains. Exit with concrete blockers and reopening
conditions, or a supported replacement and acceptance mapping. A negative result
is valid. Do not automatically switch tracks or weaken preserved contracts.

After assessment and implementation authorization, revise this plan around ordered
commits with stable identifiers, coherent behavior, relevant consumers, nonzero
test evidence and owning documentation. Keep necessary replacement and recovery
with each removal. Place wider gates at meaningful ownership boundaries and final
integration; identify each gate's combined scope and exit criteria. Freeze sources
and serialize builds during validation. Offline fixtures do not qualify live
settlement, real Tor, Docker consensus or a fresh reproducible release.

Independently review every exact committed SHA and its consumers; reviewers must
not author the change. Fix findings with `git commit --fixup=<introducing-SHA>`,
revalidate affected behavior and independently review each fixup. Preserve
unsquashed history through integration. Resolve material findings before dependent
work. Every wider gate examines all preceding commits and fixups alongside full
call chains, tests and documentation. Analyze failed assumptions, search sibling
paths for similar hazards and decide whether recurring patterns require regression
coverage, wider audit, simplification or design changes. Per-commit approval does
not exempt earlier work. Material later changes reopen affected gates.

Compare CogC/CycC against the confirmed baseline with the same pinned analyzer and
verified source identities. Review changed functions, full chains, warnings and
unchanged high-risk hotspots. Evaluate concrete simpler ownership/control flow;
do not extract helpers merely to lower scores or treat passing tests as sufficient
justification. Keep metrics under ignored `target/`.

Maintain enduring rules in their owning guides with applicable AGENTS.md links in
each implementation commit. Before removing completed planning material, verify
the handoff and preserve unfinished assessments with their validation and reopening
conditions. Independently review the exact cleanup SHA and final combined branch
after cleanup, including all prior commits and fixups. Keep no completed-plan
archive.

## Unfinished assessments

These assessments are unfinished and do not authorize implementation or upstream
submission. Select a track explicitly, capture a fresh baseline and complete an
independent assessment before defining its implementation commits and gates.
Current status, blockers and candidate revisions stay in the ledger.

Prioritize H (Hyper), followed by H2 (h2). Hyper has one contained pending-open
cancellation responsibility; h2 additionally changes queue reclamation, reset
handling and scheduler wakeups. Inspect the combined cancellation chain in each
assessment, but qualify Hyper with the retained h2 patch if supported. Do not
couple retirements without a demonstrated version/API dependency. Reqwest and
Pepper remain later assessments because each has multiple independent blockers.

| ID | Assessment and exit evidence |
| --- | --- |
| H | Recheck existing Hyper reports and contribution instructions, then prepare a minimal pending-open cancellation reproducer distinguishing the gap from the dispatched body-task cancellation fixed by [#4042](https://github.com/hyperium/hyper/pull/4042) for [#4040](https://github.com/hyperium/hyper/issues/4040). No exact pending-open report was identified by the initial search; do not treat that as proof none exists. Assess exact upstream revisions for cancellation observation and waker registration before waiting for h2 readiness, retaining the h2 patch during independent Hyper qualification. |
| H2 | Review and coordinate with [h2 #962](https://github.com/hyperium/h2/pull/962), which addresses cancelled pending-open streams, rather than prepare a duplicate. At the ledger's reviewed head, cleanup still requires available peer capacity, recognizes only scheduled resets and adds no cancellation scheduler wakeups. Assess zero-capacity reclamation, explicit reset state and explicit/implicit wakeups with focused regressions; the existing PR regression releases the occupied stream and does not establish zero-capacity cleanup. |
| R | Assess a complete Reqwest replacement for post-connect header timing, nonempty body progress, backpressure and disabled unstarted-request recovery. Compare supported hooks/adapters and any proposed policy change across each affected request type, with before/after behavior and whole-checkout payoff. Use the owning HTTP timeout/queue and network/compatibility regressions; reducing one retry hunk alone is not a retirement. |
| P | Assess Pepper message-timeout control, truthful retained loader/worker drain and typed sanitized diagnostics together, including standalone manifests, the constructible public sync payload and application-owned merge fixture. Require progress/stall, cancellation/restart, diagnostics and synthetic lifecycle/merge evidence. Relaxing logs alone does not remove this checkout; do not replace sync algorithms locally merely to avoid a dependency fork. |
| E | Watch for upstream typed canonical recovery evidence that can replace local proof plumbing while preserving exact accepted inputs/bytes and release authority. A network adapter, NEAR status or Base credit alone is not such evidence. Assess a concrete API when one exists; do not add a watcher, ledger or new proof framework in anticipation. |

For both H and H2, preserve raw HEADERS absence, zero-peer-capacity cleanup,
explicit/implicit cancellation, no stale bodies and healthy multiplexed reuse.
Require queue/timeout tests in `src/network/`, HTTP/gRPC cases in
`tests/network.rs`, network audit and the focused SDK/payment tests. Review both sides
of pending-open cancellation even if only one checkout can exit. Initial upstream
source and PR inspection is not executed regression or retirement evidence.

For upstream work, first inspect contribution instructions and existing issues/PRs.
Prepare independently useful fixes/APIs with small faithful upstream regressions,
separately from application financial-ownership fixtures. Actual submission needs
its own authorization. Record links, merge revisions, exact candidate versions and
remaining blockers in the ledger. An accepted PR or newer release is not qualified
replacement evidence. Preserve application and compatibility regression/provenance
coverage and relevant build/release inputs with every eventual retirement.
