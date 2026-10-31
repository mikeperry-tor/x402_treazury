# Randomized reserve funding

## Intent, scope and baseline

Add randomized nominal USDC funding targets alongside the existing fixed-target
mode. A range uses `min_funding_amount_usdc` and `max_funding_amount_usdc` without
`funding_amount_usdc`. When NEAR reports that a selected amount is below its
bridge minimum, raise the operation's effective lower bound and draw again within
the remaining authorized range. Keep executable swaps as `EXACT_INPUT` with the
current default 500-bps slippage.

Default ranges are **$2–5 for small** and **$10–25 for large**. Migrate all managed
deployment configurations to the default small range. Large remains opt-in;
introducing defaults must not enable an additional large reserve.

This document plans implementation; it does not implement it or authorize live
funding, qualification runs, deposits, state rollback or modification of accepted
swaps. The starting revision is
`de53253107e465b3fdba84f9202286890590413f`. Reconfirm the revision and intervening
changes before execution. Preserve unrelated work, including `TODO`,
`swap_logs.txt`, ignored local overrides, secrets and financial evidence.

Current implementation:

- `src/rotation/config.rs::ReserveConfig` requires a fixed amount, optionally with
  a maximum for bridge-minimum adjustment. `ReservesConfig` requires small and
  optionally enables large; its current comparison orders their nominal targets.
- `store/capacity.rs` stores current reserve bounds separately from immutable
  allocation/job intent. `store/funding.rs::select_quote_terms` currently requires
  the configured target and overwrites unsigned selected terms on each call.
- `funding.rs` captures quote prerequisites, then `near.rs` obtains a zero-slippage
  dry `EXACT_OUTPUT` preview and an executable `EXACT_INPUT` quote. The preview
  supplies the ZEC principal; no separate exchange-rate service is involved.
- The existing minimum-adjustment helper retries a rejected output amount at the
  provider's suggested minimum. It currently permits three sizing attempts.
- `assignment.rs::WalletSummary`, inspection/preparation consumers and some CLI
  descriptions assume one exact initial inventory target. Actual funding readiness
  is already the first positive destination observation, independently of target.

## Owning contracts and invariants

Read [funding and ownership](../wallet-rotation.md#funding-and-source-safety),
[best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[observation](../wallet-rotation.md#wallet-observation-policy),
[reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy),
[wallet administration](../wallet-cli.md), [configuration](../configuration.md),
[runtime architecture](../architecture.md), [network policy](../network-egress.md),
[resource limits](../local-resource-limits.md),
[validation](../testing.md#validation-order-and-continuation),
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline) and
[independent workflows](../../AGENTS.md#independent-subagent-workflows).
Live-runner changes additionally require the [runbook](../../tests/live/INTEGRATION.md)
and [technical reference](../../tests/live/INTEGRATION_REFERENCE.md).

Preserve these boundaries throughout every commit:

- SDK/blockchain facts own ZEC spendability, selected inputs and source closure.
  Preserve accepted inputs, exact signed bytes, operation identities, recipients,
  refund addresses and liabilities through cancellation, restart and reorgs.
- Bounds authorize **nominal sizing**, not a minimum credited balance or a maximum
  received amount. Favorable executable output or actual credit above the nominal
  maximum remains valid. Slippage can leave credit below the configured minimum.
- Do not add input slippage to the dry sizing preview, shrink the sampling range
  by 5%, or impose a second output floor/ceiling. Keep existing source principal,
  actual fee, conversion-overhead and qualification safeguards.
- First positive credit consumes the initial claim; neither low credit nor an
  empty later balance grants a top-up. Refunds alone grant no replacement deposit.
- Add no minimum cache, monetary ledger, daily/cumulative budget, database columns,
  network constructor, timer, readiness gate or RNG dependency. Use the existing
  `rand` dependency and quote fields. Preserve Tor and network-owner boundaries.
- Unsigned quote failures do not consume signed recovery attempts. Missing optional
  observations and stale reporting projections must not block proven work.

## Proposed configuration contract

Normalize user-facing settings into a clear fixed or ranged sizing policy. Do not
infer the mode merely from unequal lower/upper bounds: fixed mode can have bridge
headroom too. Keep names at the reserve level.

| Reserve settings | Meaning |
| --- | --- |
| No amount keys | Slot-specific randomized defaults: small $2–5; explicitly enabled large $10–25 |
| `funding_amount_usdc` only | Fixed nominal target; maximum equals that amount |
| `funding_amount_usdc` plus `max_funding_amount_usdc` | Fixed target with existing bridge-minimum headroom |
| `min_funding_amount_usdc` plus `max_funding_amount_usdc` | Uniform randomized nominal amount within the inclusive range |
| Equal explicit range endpoints | Valid degenerate range; no random draw is needed |
| Fixed amount plus minimum, or an incomplete custom range | Configuration error identifying the reserve and conflicting/missing keys |

An omitted managed `reserves` block, an empty block, or an empty small definition
uses the default small reserve. An absent large definition stays disabled; an
empty large definition explicitly enables its $10–25 default. Resolve defaults
by slot, rather than accidentally applying the small default to a large table.
Do not apply randomized defaults to static wallets.

All amounts remain positive finite micro-USDC decimals, parsed exactly without
floating point. Each custom maximum must be at least its minimum/fixed amount.
Do not add ordering constraints across independently configured small/large
ranges. Review and remove the current cross-slot nominal-target comparison after
confirming its consumers use slot ownership and per-payment authority rather
than relying on amount ordering. Overlap is not a financial inconsistency.

Canonical default-small deployment spelling:

```toml
[wallets.research.reserves.small]
# Default randomized nominal funding: $2–5.
```

Custom range spelling:

```toml
[wallets.research.reserves.small]
min_funding_amount_usdc = "3.00"
max_funding_amount_usdc = "7.00"
```

Large defaults are available through an explicit empty `reserves.large` table,
but configuration rollout in this plan enables only the default small slot.

## Selection, retries and persistence

1. At the existing serialized unsigned quote boundary, obtain the current sizing
   policy and claim. Choose an unbiased integer micro-USDC amount in the inclusive
   range, using the existing RNG. Keep RNG injection small enough for deterministic
   boundary tests; do not create a sampling framework or seed ledger.
2. Save the chosen amount in existing `funding_progress.quote_target` and the
   authorized ceiling in `quote_ceiling`, returning the actual chosen terms to
   quote acquisition. Preserve the allocation/job's original intent; do not use
   `accepted_output` to store the draw or mutate original job targets on retries.
   Current reserve claim bounds continue to represent current configuration.
3. Reuse a valid saved unsigned choice across ordinary failures, cancellation and
   restart. If changed configuration makes that unsigned choice invalid, repair
   it under current policy at the same clean boundary. Do not introduce a stale
   projection veto or historical mode ledger just to force a reroll.
4. Request the normal zero-slippage dry output preview. On the existing recognized
   bridge-too-low response, raise the operation-local lower bound to
   `max(configured minimum, suggested USDC minimum)` and redraw uniformly from that
   bound through the authorized maximum. Persist the replacement choice before
   the next request, so a restart after rejection does not lose it. Do not clamp
   every rejected draw to the floor, which would concentrate results there.
5. If the reported minimum exceeds the maximum, stop that unsigned acquisition
   with a clear range-too-low explanation and no deposit. Do not misreport it as
   an empty treasury or ask for ZEC when the problem is configuration/route size.
   Reject malformed/non-increasing suggestions through existing validation.
6. Keep fixed-target bridge adjustment working as before. Keep USDC output-unit
   minimum handling separate from executable `EXACT_INPUT` errors expressed in
   ZEC atomic units; never reinterpret one as the other or silently increase the
   fixed source input.
7. Obtain the executable fixed-input quote using the preview's ZEC principal and
   configured slippage once. Preserve the accepted preview/request/response pair
   in the existing encrypted quote. After acceptance, selected-term metadata must
   not become a new equality gate on preparation, restart or reconciliation.
8. Fresh operations and existing safe quote refresh/recovery paths can obtain a
   fresh draw. Obsolete completions must remain unable to change a replacement's
   terms or publish its quote. Existing exact-operation and source-owner checks
   still govern disclosure; randomness grants no retry or replay authority.

There is no additional minimum-discovery request: normally sizing plus execution
remain two quote requests. A below-minimum response requires another dry request.
Provider minimum guidance can change and is not a universal trade-size guarantee.
Do not promise liquidity, anonymity, settlement speed or fewer refunds from RNG.

Before implementation, review the existing three-sizing-attempt bound against
the resource/contract guidance. Retain it only with a concrete rationale, or remove
it coherently with the existing monotonic-minimum/error handling. Do not add a
second randomization cap, a whole-work deadline or a new financial-attempt charge.
Any change to that existing policy updates both resource/timer inventories and
owning retry documentation in the same behavior commit.

## Consumers and configuration rollout inventory

Update bootstrap, status, inspect/preparation output and qualification consumers
that currently assume a single exact configured target. Expose nominal bounds and,
when available, the selected/accepted amount from existing facts. Do not present
the minimum, midpoint or maximum as a guaranteed exact inventory total. Existing
readiness remains first positive credit, including credit below nominal minimum.
Use additive output fields where truthful; if an exact-only JSON contract must be
replaced, update its owning schema version and every consumer/fixture together.
Do not rename monetary database columns merely to improve output terminology.

The implementation inventory must cover every TOML and generated deployment
template, not just the stock research desk. Starting tracked managed deployments:

- `examples/deployments/managed.toml`
- `examples/deployments/research_desk.toml`
- `examples/deployments/comprehensive.toml`
- `tests/live/deployments/live-qualification.toml`
- `tests/live/deployments/live-tor-providers.toml`
- `tests/live/deployments/public-swap-demo.toml`
- `tests/live/integration/single-deployment.example.toml`
- `tests/live/integration/managed-deployment.example.toml`

Migrate each managed group in these files to the canonical default small range;
do not leave the demo at a fixed $5 or leave bridge-headroom comments describing
old $2/$3 behavior. Do not add large slots. Inspect generated/inline deployment
templates under `tests/live/runner/`, preparation examples and documentation
snippets for the same assumptions. Preserve deliberately fixed/small test inputs
when they exercise a named contract; distinguish those fixtures from deployable
defaults rather than globally replacing every amount literal.

Inventory ignored workspace overrides separately. At planning time,
`examples/deployments/comprehensive.local.toml` has the old small funding stanzas;
`examples/deployments/public-demo-wallet.local.toml` also exists and needs
classification. Update applicable managed funding stanzas to the default small
range during rollout, keeping these files ignored/untracked and preserving all
unrelated paths, credentials, endpoints and operator overrides. Never dump their
contents into reports or commit them. Refresh the filename/stanza inventory
before editing; ignore status alone must not silently exclude an applicable
configuration from the user's request.

Other TOMLs, including static-payment demos, provider definitions and integration
authorization manifests, need classification rather than blind edits. The
`providers/locus/provider.toml` text match is not a managed reserve definition.
Do not invent treasury settings for static profiles or alter unrelated providers.

Live qualification limits remain separate authority. Adapt preparation estimates
and range-aware validation in `tests/live/runner/`, manifest examples and runbooks
to account for an upper nominal amount of $5 per small claim, without treating a
midpoint/minimum as sufficient funding authority. Do not automatically enlarge
registry spending limits, enable `auto_fund`, add `--allow-funding`, reuse captured
authorizations or run funded cases. An insufficient existing qualification permit
must report that mismatch; changing reserve defaults is not permission to raise it.

## Phases, ordered commit map and review gates

Only plan creation is authorized now. The following are future implementation
boundaries; preserve coherent behavior and its recovery path in each commit.

| Phase / stable commit ID | Coherent scope and owning documentation | Acceptance evidence / gate |
| --- | --- | --- |
| P0 — design and inventory | Reconfirm starting SHA and local changes; independently challenge default activation, mode normalization, sampling/persistence ownership, cross-slot ordering, range reporting, qualification authority and the existing sizing retry bound. Inventory all config/template consumers. | G0: material design decisions resolved before affected implementation; record agreed source-matched CogC/CycC baseline and concrete simplifications considered. |
| P1 / C1 — complete randomized funding behavior | Implement slot-aware defaults, fixed/range parsing, serialized select/reuse/repair, provider-floor redraw and persistence, executable quote acceptance, startup/restart/recovery, and all status/bootstrap/inspection/qualification code consumers. Include focused tests and permanent contract updates in configuration, wallet CLI, wallet rotation, testing/LIFECYCLE and applicable resource/network guides. | G1 after exact C1 and its fixups: independent exact-SHA review, complexity review and full config → claim → quote → SDK → disclosure → reconciliation/credit call chains; focused contracts plus compiler/compatibility gates pass. No partial parser-only range mode that still funds a fixed minimum. |
| P2 / C2 — configuration rollout | Migrate all tracked deployments, applicable generated templates and documentation examples to default small ranges; update local managed stanzas separately without tracking them. Include range-aware live preparation/reporting docs and tests; preserve qualification permits and unrelated configuration. | G2 after C2: exact-SHA review and combined architecture/financial review of C1, C2 and every fixup; inventory complete, full named validation stages pass, local rollout separately recorded without secrets. |
| P3 / C3 — maintained handoff and plan removal | Confirm enduring rules and examples are in owning guides, verify AGENTS links and documentation references, resolve remaining work, then remove this completed plan and any temporary index link. Do not create a completed-plan archive. | G3: independently review exact cleanup SHA and final combined branch, including all prior commits/fixups and full call chains. Revalidate cleanup/link changes; carry unchanged runtime evidence with explicit provenance. Completion requires no material findings or deferred required work. |

Reviewers are read-only by default and must not author the changes they approve.
Give each reviewer user intent, scope, exact baseline/SHA, full consumers and
validation evidence. Resolve findings with `git commit --fixup=<introducing-SHA>`;
independently review each exact fixup and retain unsquashed history through G3.
Material later changes reopen the affected gate before dependent work proceeds.

Every gate reviews all earlier implementation commits and fixups with surrounding
consumers, not only its latest diff. Analyze causes of findings, repeated authority
or representation assumptions, concurrency/cancellation hazards and coverage gaps.
Decide whether they need wider audit, regression coverage, simplification or a
design change. Per-commit approval does not waive combined review. In particular,
do not repeat the earlier mistake of treating derived amount projections as extra
spending authority or allowing optional evidence to block valid accepted work.

## Validation criteria and execution order

Plan creation needs source/inventory review, link checks and whitespace validation,
not a runtime test run. During implementation use pinned Make entrypoints from
the repository root. Freeze sources and coordinate all builders; execute feature
configurations sequentially because subprocess tests share executable paths.

1. Complete G0 and applicable design review. Run `make check-static` with Python
   3.11+ and an explicit agreed `COMPLEXITY_BASE_REF`. Independently review the
   source-matched CogC/CycC report, changed functions and full chains, including
   unchanged high-risk scheduler/store hotspots. No parser exclusions or silent
   dismissal of advisory warnings; do not extract helpers just to lower scores.
2. Run the production compiler and all-target Clippy warning gate through
   `make check-clippy`, plus `make check-compat`, before broad runtime suites.
   Inspect emitted application/dependency diagnostics as well as exit status.
3. Run affected cheap contracts before broader runtime selections. Exercise:
   - Slot-aware omitted/empty defaults, large disabled by omission, fixed mode
     with/without headroom, explicit ranges, equal bounds, conflicting/missing
     fields, precise fractional units, reversed/invalid bounds and static wallets.
   - Inclusive RNG endpoints and deterministic interior draws without statistical
     flakiness; existing RNG dependency only. Allocation identity/original intent
     and current policy remain separate from selected and expected output amounts.
   - Ordinary quote rejection retry/restart reuse; persisted provider-floor redraw
     followed by cancellation/restart; config edits repairing only invalid unsigned
     selections; no rewrite of accepted inputs or signed bytes.
   - Correct uniform redraw after a higher USDC floor, equal floor/maximum,
     out-of-range/malformed/non-increasing floors, fixed-mode compatibility and
     ZEC-unit input minimum errors. Verify actual request and deposit counts.
   - Zero-slippage sizing and 5% execution slippage once; favorable quote/credit
     above maximum succeeds; output below nominal minimum can consume the claim.
     No extra top-up, deposit, signing, minimum probe or price-service request.
     Genuine source/fee-cap rejections remain enforced. Keep other errors on their
     existing recovery paths rather than silently changing amounts or increasing
     configured ZEC spending safeguards.
   - Stale quote completion versus replacement/config changes, shared-source
     serialization, progress versus stalls, cancellation/restart and unrelated
     status/credit progress. Unsigned errors consume no financial recovery attempt.
   - Range-aware bootstrap/status/inspection output, generated bindings, disabled
     reserves, old encrypted accepted quotes and existing state reopening without
     a schema migration. Missing reporting fields do not become monetary vetoes.
   - All migrated configs and generated templates resolve default small $2–5,
     preserve explicit payment/fee/qualification authority, and enable no large
     reserve. An explicitly enabled empty large table resolves $10–25 in tests.
4. Run `make check-default`, `make check-proving`, `make check-lifecycle` and
   `make check-codec` sequentially. Verify changed tests are wired and execute
   nonzero cases; preserve forbidden-side-effect assertions. Localhost fixtures
   use socket-capable execution up front; socket-free checks stay sandboxed.
   Temporary fixture wallets/public unfunded keys only. No live-funded, Tor or
   ignored Docker/consensus qualification is inferred from these checks.
5. Keep commands, exact revision/input hashes, feature/profile, diagnostics, test
   counts and results under ignored `target/` or private evidence. After a fix,
   revalidate affected consumers and resume unfinished named stages. Repeat passed
   stages only when their evidence is invalidated; justify carried provenance.
   Report assembled checks as staged validation rather than one uninterrupted run.
6. Obtain G2 and G3 independent combined approvals with all required findings
   resolved. Parsing defaults, successful dry quotes and synthetic tests do not
   establish live settlement reliability or authorize operator spending.

## Permanent documentation handoff

Maintain the new configuration/default grammar in [configuration](../configuration.md),
sampling/claim/accepted-work semantics in [wallet rotation](../wallet-rotation.md),
amount labels and JSON contracts in [wallet CLI](../wallet-cli.md), and focused
coverage in [testing](../testing.md), [LIFECYCLE](../../tests/LIFECYCLE.md) and the
live runbooks/demo guide where their templates change. Keep both inventories
current if the existing retry policy changes. Update README/examples and document
the new funding-range diagnostic fields using the existing severity/privacy rules;
logging-only edits do not justify new diagnostic test scaffolding.

The existing [AGENTS.md](../../AGENTS.md#read-the-relevant-guide-before-editing)
links these owning guides. Verify their anchors and add a missing owning-guide
link there only if implementation introduces a genuinely new destination. C1/C2
must update maintained guidance alongside behavior; do not defer it all to C3.
Commit plan removal only after this handoff; completion then requires review of
the exact cleanup SHA and final combined branch. Preserve financial evidence and
unfinished independent plans.
