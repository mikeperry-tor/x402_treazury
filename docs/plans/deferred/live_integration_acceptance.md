# Deferred live integration lifecycle and cover acceptance

## Status and re-entry

The runner and presets are implemented and the provider workflow has been exercised.
This plan defers broader live qualification, not safeguards or defects blocking
ordinary use. See [qualification status](../../testing.md#qualification-status).
Manual refill/restart observations and offline assertions do not establish a full
confirmed-refill/two-round restart through this runner. Unsigned cover observations
do not qualify combined paid traffic and cover shaping.

Use the [runbook](../../../tests/live/INTEGRATION.md) and
[scenario presets](../../../tests/live/integration/SCENARIO_PRESETS.md). Require fresh
treasury sync, canonical balances and adequate reviewed authority. Historical saved
balances never establish current funding readiness. No new architecture is required.

## 1. Confirmed refill and two-round lifecycle

Review current bridge minimums, input/fee bounds, API prices, allocation slots,
existing liabilities and cumulative remaining authority. Retain all declared pools.
Each new funding target respects max($2, the validated current bridge minimum);
existing jobs/targets are immutable. Do not lower safety bounds, sweep retired
balances, reuse completed cases or manufacture a slow refill to force a pass.

Use the existing rotation/refill-service preset first, then the two-round lifecycle
preset when feasible. Require:

- One natural promotion and a replacement with a new address, unchanged job binding,
  one source submission and confirmed target credit before READY.
- Canonically paid service from the promoted payer and each declared peer while
  the same refill is pending, using admission/final-response observations.
- Two consecutive rounds with unchanged generation between the previous confirmed
  refill and the next depletion baseline.
- One graceful application restart at the reviewed queued/submitted checkpoint,
  retaining Tor and registered process/session boundaries. Verify durable operation,
  recipient and prepared-byte continuity without extra submission or authorization.
- Explicit failed/incomplete results if refill is too fast to observe, funds are
  insufficient, service fails or a checkpoint is missed. No implicit retries.

Record source principal/fees, canonical API debits, active/standby/retired balances
and reserved liabilities separately. Preserve provider errors alongside independent
lifecycle observations. Historical manual evidence and offline invariants do not
replace the runner's actual process and payment evidence.

## 2. Combined cover and payment

Use a provider/resource that supports the reviewed Range behavior. Verify range
sizes, request padding, requested distributions/concurrency bounds and actual
activity separately from payment. Ignored Range responses, exhausted deadlines or
missing episodes remain failed/incomplete cover evidence even when payment succeeds.

Require a canonical debit and the requested cover modes in the same reviewed run,
with complete Tor/control/confinement evidence and unchanged wallet identity binding.
Connection reuse/affinity needs its own observation; HTTP/2 support alone is not
proof. Do not weaken cover assertions or replay paid calls to hide incomplete results.
The [deferred cover transport plan](http2_cover_traffic.md) retains the broken-pipe
investigation, connection-affinity work and further shaping ideas. A defect that
interferes with normal requests should be fixed when found, rather than postponed
solely because broader cover qualification is deferred.

## Optional later breadth

Cross-day reliability, additional bundled providers/routes, and bounded image/output
samples may use separate reviewed runs when needed. Preserve every failed attempt;
never automatically replay uncertain work or infer Tor causation from Tor-only
samples. AgentUtility semantic review belongs to the active provider sweep; it is
not a prerequisite for unrelated lifecycle or cover observations. Async jobs and
artifact storage remain their own plans.

## Execution and evidence rules

Reuse production ownership, admission, funding, network identities and safe drain.
The treasury-local registry retains cumulative reservations across runs, failures
and restarts. New IDs grant neither authority nor a budget reset. Started/reserved
runs remain observation-only; new runs may adopt only reviewed, resolved lineage
under production checks. Positive funding requires both registry authority and the
explicit execution/production funding settings.

Use installed Tor with dedicated persistent state and authenticated control under
supported confinement. Keep exact executable/config/catalog pins. A Tor-outage test
stays independently keyless; do not introduce a managed-to-keyless handoff. No new
paid resume, automatic scheduling, second ledger or deliberate mainnet ambiguous-send
injection is part of this plan.

Each discovered code correction gets focused offline regression coverage and a
separate commit. Revalidate changed behavior before another paid attempt. Preserve
private state/evidence and original case records. Bound output explicitly; missing
or truncated evidence cannot pass. Unknown settlement stops dependent dispatch and
retains exposure. Observed preparation-bound insufficiency is not literal zero ZEC,
and retired balances are not available active funds.
