# ZEC cost policy simplification

## Intent, baseline and preserved contracts

Implement the user's requested removal of arbitrary ZEC transaction-fee ceilings,
caller `max_source_zatoshis`, derived `max_input_zatoshis`, and managed swap
commission-metadata gates. Baseline is `fa59929469250da3253ec5ef09cdb4ad1ace84a6`
on `direct_swap_control`. Preview must report whether estimated principal plus
SDK fee fits the lesser of optional operator `max_send_zec` and SDK spendable ZEC.
The user additionally requests a final independent inventory of remaining limits.
Conversion-overhead removal is awaiting the user's separate answer after Zodl research;
do not silently expand that policy while implementing the authorized removals.

Preserve the modified `examples/deployments/research_desk.toml`, untracked `.vscode/`
and `TODO`, private wallet/qualification evidence and all reference checkouts.
No live-funded test, operator wallet inspection, restart, push or merge is authorized.
No new dependency, reservation ledger, arbitrary cap, queue or timeout.

Owning contracts: [planning workflow](../planning-workflow.md),
[wallet reconciliation/observation](../wallet-rotation.md),
[Ironwood policy/SDK boundary](../zingo-integration.md),
[configuration](../configuration.md#direct-swap-control),
[network policy](../network-egress.md), [testing](../testing.md),
[complexity review](../../tests/COMPLEXITY.md#review-discipline), and
[independent reviews](../../AGENTS.md#independent-subagent-workflows).

Keep original accepted payment amounts, recipients, transport identities, raw terms,
quotes, selected-input ownership and signed bytes. The user expressly removes old
saved fee-ceiling enforcement; this does not remove accepted optional direct total-debit
authority. Historical JSON can contain obsolete fields without retaining their
runtime meaning. No financial evidence deletion or replacement/replay authority.
Native ZEC remains Ironwood-only, with no NEAR dependency or transparent RPC scans;
the sole transparent output exception remains actual NEAR-issued deposit addresses.

## Design findings and reference evidence

Independent `zec_fee_limits_audit` accepted removal with two resolved requirements:

- Historical send terms contain `max_source_zatoshis`, including null on native sends.
  Preserve raw terms and compare active send terms ignoring only that obsolete field;
  retain strict new arguments, changed-amount/recipient rejection and unchanged receive
  semantics. Test old native/foreign retries, original operation/bytes and restart.
- Read SDK Ironwood spendable balance and an unsigned proposal under one actor-owned
  wallet guard. SDK balance does not exclude advisory locks whereas proposal selection
  does. Insufficient proposal means `fits=false` even with an apparently sufficient
  balance. Unknown balance/fee remains unknown; do not subtract reservations into a
  separate monetary ledger. Successful fee estimation and known effective limit may
  produce `fits=true`, advisory until execution. No address derivation, sync, signing,
  stored proposal, cache clearing, proof or wallet mutation for previews.

Independent `near_cost_limits_audit` accepted deleting managed appFees/rebates metadata
restrictions while preserving actual routing, exact quote echo, amount, slippage,
network/address, deadline and saved-quote bindings. Raw metadata remains reportable;
changed fee metadata cannot veto saved status reconciliation.

Zodl examined clean revisions: iOS `bfce63e808f595f716db56d3e2c87cb4f002c259`, Android
`2beecca837bed599623a167947f5ab96cd1025e7`, public Swift SDK
`1e632565b2d8ce1779453114b94a8a959894cda3`, public Android SDK
`c137875396efe1fdb58916827adc34f5f1c003cb`. No equivalent hard all-in USD overhead
gate was found in examined quote-to-confirmation paths. Swaps display economics and
seek confirmation. iOS actually references an unavailable private SDK; Android's
Maven snapshot is not proven identical to its public checkout. SDKs do not own NEAR
pricing. Do not copy Android's contradictory exact-output minAmountIn comment.
Permanent guidance receives relevant evidence, not a completed-plan archive.

## Ordered work and gates

| Step | Work and acceptance |
| --- | --- |
| P0 | Commit this plan; independent design findings above resolved before affected implementation. Honor pending overhead choice separately. |
| P1 | Remove fee setting, runtime/saved fee/input fields and derived gates through deployment, managed funding, treasury selection/calculation/publication and direct source ownership. Remove caller source field/schema/new terms/enforcement; retain optional direct actual total-debit check. Historical obsolete JSON remains readable and retries semantic-compatible without rewriting raw evidence. Remove stale examples/docs including shielding-fee claims, preserving unrelated files. Tests: standard fee above old30k succeeds, direct total cap still denies excess, historical fee/source fields no longer veto progress, input/byte/recipient integrity unchanged. |
| P2 | Treasury preview observation returns advisory spendable Ironwood balance and SDK fee outcome from one wallet read. Service exposes effective available debit and tri-state fits for native/foreign, no caller math or new authority. Tests: sufficient/insufficient/unknown, operator cap/balance min, lock discrepancy, concurrent accepted work/no mutation and static safe unavailable reasons. |
| P3 | Remove managed commission recipient/count/type/rate/rebate restrictions in quote and status consumers; retain exact actual transfer bindings. Regression covers arbitrary/unknown fee metadata and forbidden routing changes. Resolve overhead scope from user answer before implementing affected policy. |
| G1 | Independently review every exact implementation/fixup SHA and complete accumulated call chains, historical compatibility, financial ownership/network/privacy invariants and updated permanent docs. Source-matched CogC/CycC review, cheap affected contracts, production compiler/all-target Clippy and compatibility, then selected synthetic financial/lifecycle tests. No competing builds/source edits during validation; continue failures/fixups with focused revalidation. |
| P4 | Independently inventory remaining fee/price/spending/funding-size/attempt/expiry restrictions, distinguish policy from SDK/consensus correctness and report necessity/unproven bounds. Record findings, dispositions and actual validation in plan. After G1, remove plan, commit cleanup and independently review exact cleanup and final combined branch, preserving unsquashed history. |

## Ownership and evidence

Root coordinates application configuration/deployment, direct service/store, maintained
docs and examples. Treasury implementation and managed quote implementation may be
delegated with exclusive file ownership; reviewers remain non-authors and read-only.
No source compilation/runtime evidence is claimed yet. Reference investigations were
read-only, with no live quotes, secrets or wallet access. All required checks and
final remaining-limits inventory remain outstanding.
