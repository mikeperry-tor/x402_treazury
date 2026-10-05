# Bounded mutation-testing pilot

The [coverage map](../../../tests/COVERAGE.md) describes existing regressions.
Mutation qualification remains unperformed; check local tooling before starting.

Install/pin a reviewed cargo-mutants release and inspect its CLI before using it.
Work in a disposable committed-source copy, excluding secrets, live state,
reference checkouts, existing target directories, vendor code, generated providers
and test helpers from mutation selection. Keep the production Rust compiler and
lockfile. A RAM disk is optional after measuring the build footprint; avoid enough
parallel copies to cause swapping.

1. List candidate mutations without executing them. Select at most twelve from
   the decisions below; retain the exact list and source revision.
2. Run the unchanged baseline with precisely the selected test targets. Abort on
   baseline failure rather than attributing it to mutations.
3. Use one worker, a thirty-minute total budget, and a per-mutant deadline of twice
   measured baseline duration plus thirty seconds. Budget exhaustion is incomplete
   evidence, not a caught mutant.
4. Report caught, surviving, unviable and timed-out outcomes separately, including
   compiler/tool versions, commands and duration. Review each survivor for a missing
   behavioral assertion versus equivalent behavior.
5. Add only meaningful regression assertions and commit the evidence/disposition.
   Do not establish a repository-wide mutation score gate or remove safety guards.

| Decision | Baseline targets | Invariant |
| --- | --- | --- |
| `payment::SpendPolicy::select` amount/asset/network predicates | `--no-default-features --test payments` | Equality to cap allowed; larger amount or unsupported asset/network cannot sign. |
| `rotation::base::validate_timestamp` comparisons | `--no-default-features --lib rotation::base::tests` | Future/stale anchors rejected at defined boundaries. |
| `network::public_ip` classification | `--no-default-features --lib network::` | Private/reserved answers cannot become public-only destinations. |
| Discovery target/persistence grants | `--no-default-features --lib discovery::tests::permissions` | Denial changes neither catalog nor durable records. |

Confirm function/test names against the checkout before execution. Start with the
first two rows; add the others only within the same budget. Refund, journal,
consensus, async-network and proving mutations require a separately bounded plan.

Use one commit for any reusable pilot tooling/configuration and another for the
measured evidence and resulting assertions. Ordinary test/region qualification
must not wait for this optional pilot. Raw output remains in ignored `target/`;
update the mutation section of `tests/COVERAGE.md` with actual results and limits.
