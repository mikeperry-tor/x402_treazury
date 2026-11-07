# ZEC cost policy simplification

## Intent, baseline and preserved contracts

Implement the user's requested removal of arbitrary ZEC transaction-fee ceilings,
caller `max_source_zatoshis`, derived `max_input_zatoshis`, and managed swap
commission-metadata gates. Baseline is `fa59929469250da3253ec5ef09cdb4ad1ace84a6`
on `direct_swap_control`. Preview must report whether estimated principal plus
SDK fee fits the lesser of optional operator `max_send_zec` and SDK spendable ZEC.
The user additionally requests a final independent inventory of remaining limits.
The user explicitly approved conversion-overhead removal after Zodl research; remove
that setting and its enforcement too. No replacement price gate.

Preserve unrelated changes in `examples/deployments/research_desk.toml` while deleting
its retired overhead settings, plus untracked `.vscode/`
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
runtime meaning. The user rejected extra retry-compatibility projection; strict changed
argument shapes may fail explicitly, with original operations available via status. No financial evidence deletion or replacement/replay authority.
Native ZEC remains Ironwood-only, with no NEAR dependency or transparent RPC scans;
the sole transparent output exception remains actual NEAR-issued deposit addresses.

## Design findings and reference evidence

Independent `zec_fee_limits_audit` accepted removal with two resolved requirements:

- Superseded: initial design proposed ignoring the obsolete caller field when matching
  old send retries. The user rejected that convenience as unnecessary legacy code.
  Keep strict exact terms comparison, preserve raw evidence and status lookup by
  original request ID; reject changed argument shapes without a new payment.
- Read SDK Ironwood spendable balance and an unsigned proposal under one actor-owned
  wallet guard. Both SDK balance and the concrete pinned note selector can ignore generic advisory
  locks (the backend trait policy alone does not establish implementation behavior). Insufficient proposal means `fits=false` even with an apparently sufficient
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
| P0 | Commit this plan; independent design findings above resolved before affected implementation. Overhead removal is explicitly authorized; initial retry compatibility refinement is superseded by the user's simpler strict-failure/status-retrieval choice. |
| P1 | Remove fee setting, runtime/saved fee/input fields and derived gates through deployment, managed funding, treasury selection/calculation/publication and direct source ownership. Remove caller source field/schema/new terms/enforcement; retain optional direct actual total-debit check. Historical obsolete JSON remains readable; mismatched new retry shapes fail explicitly without rewriting raw evidence or creating another payment. Remove stale examples/docs including shielding-fee claims, preserving unrelated files. Tests: standard fee above old30k succeeds, direct total cap still denies excess, historical fee/source fields no longer veto progress, input/byte/recipient integrity unchanged. |
| P2 | Treasury preview observation returns advisory spendable Ironwood balance and SDK fee outcome from one wallet read. Service exposes effective available debit and tri-state fits for native/foreign, no caller math or new authority. Tests: sufficient/insufficient/unknown, operator cap/balance min, advisory-balance/proposal discrepancy, concurrent accepted work/no mutation and static safe unavailable reasons. |
| P3 | Remove managed commission recipient/count/type/rate/rebate restrictions in quote and status consumers; retain exact actual transfer bindings. Regression covers arbitrary/unknown fee metadata and forbidden routing changes. User approved overhead removal: delete the required setting, USD metadata parsing/cost checks and aliases; preserve nominal output sizing and actual transfer bindings. |
| G1 | Independently review every exact implementation/fixup SHA and complete accumulated call chains, historical compatibility, financial ownership/network/privacy invariants and updated permanent docs. Source-matched CogC/CycC review, cheap affected contracts, production compiler/all-target Clippy and compatibility, then selected synthetic financial/lifecycle tests. No competing builds/source edits during validation; continue failures/fixups with focused revalidation. |
| P4 | Independently inventory remaining fee/price/spending/funding-size/attempt/expiry restrictions, distinguish policy from SDK/consensus correctness and report necessity/unproven bounds. Record findings, dispositions and actual validation in plan. After G1, remove plan, commit cleanup and independently review exact cleanup and final combined branch, preserving unsquashed history. |

## Ownership and evidence

Root coordinates application configuration/deployment, direct service/store, maintained
docs and examples. Treasury implementation and managed quote implementation may be
delegated with exclusive file ownership; reviewers remain non-authors and read-only.
Initial integration compiler found only a new test assertion comparing a zeroizing byte
wrapper with a Vec; corrected to compare slices. No compiler pass or runtime evidence
is claimed yet. The overhead-only refresh error/worker handler and obsolete fixture
were also removed, leaving ordinary claim/expiry refresh and accepted work unchanged.
P1-P3 are committed together because the removed types and actor preview contract span
both callers and consumers; this avoids a knowingly non-compiling intermediate commit. Reference investigations were
read-only, with no live quotes, secrets or wallet access. All required checks and
final remaining-limits inventory remain outstanding.

## Implementation review and pending validation

P1-P3 exact `2f0297fc` independently reviewed by zec_fee_limits_audit; remaining-limit
inventory independently audited by asset_search_review. No additional actual SDK
fee/publication/ownership defect was found. Metrics source-matched every file to
2f0297fc, complete 486 files/10727 spaces, no exclusions/new warnings. Existing
source_step44/72 and prepare_with_observation18/53 inspected; no additional
simplification required beyond the removed overhead-only handler.

Review corrections for fixup: compare known total (not only principal) against a
known operator cap when balance is unknown; remove stale commented fee setting;
replace invalid-config no-op/empty-pattern replacements with actual unknown-field
insertion; update old USD/appFee rejection assertions in shared near_boundaries.
Initial focused contracts passed72, failed1 stale removed-policy assertion, ignored1
explicit child helper. No complete runtime pass claimed. Repeated cause: policy
removal must audit external shared test consumers and commented templates, not only
inline tests/config structs. Independent inventory also found direct commission
shape/rebate checks contradicting advisory semantics: remove those matching passive
metadata gates too, with direct quote/status and unchanged transfer-binding coverage.
Add the existing x402300-second signing-authority bound, confirmation defaults and
numerical accounting domains to the permanent inventory. These are reported, not
new runtime restrictions. All corrections require exact fixup/combined review and
affected revalidation before G1. No old fee-cap compatibility projection is added.

Exact fixup `eff27c69` and combined source independently accepted by both auditors:
financial/SDK/quote call chains, remaining-control scopes and source-matched complexity
have no unresolved material findings. Focused direct/NEAR contracts now passed73,
ignored1 explicit child helper (`target/zec-cost-policy-contracts-fixed.log`).
Configuration/CLI/assignment/integration-preparation contracts passed all selected
targets (`target/zec-cost-policy-config-contracts.log`). Eight changed TOML files
parsed; format/diff checks pass. Final source-matched metrics are under
`target/complexity-runs/20261107T074336Z-15594548`, complete486/10727 with no
exclusions/new warnings. Additional documentation-only correction clarifies the
public demo reports provider fees without local quote cost gates, describes native
Ironwood versus bound NEAR output shape correctly, and links qualification-only
bounds rather than duplicating them. Compiler/all-target Clippy, compatibility,
selected SDK/proving/lifecycle checks and final executable remain pending.

Financial selection executed13 tests: twelve passed, including SDK standard fee
above30k and all selected proving/native/foreign lifecycle cases; one failed at
fee_preview.rs:224 because the SDK reports Some(0), not None, when scan data is
missing (`target/zec-cost-policy-financial.log`). Investigation verified pinned
SDK balance.rs475-483 returns zero without spend horizon/anchors even for funded
wallets. Independent design review accepted a presentation-only correction: reuse
already computed ScanRequired to expose unknown spendability. No new scan gate,
network read or state. Preserve scanned-zero/InsufficientFunds and known balances
on unrelated fee errors. Add a populated500k Ironwood clear/restore-scan regression;
retain the original failing assertion rather than changing it to accept false zero.
This fixup requires exact review and focused revalidation; unaffected successful
signing/ownership cases remain valid. Compiler/all-target Clippy and fixed
compatibility stage passed on b8ab6a7d; final affected compiler/lint continuation
and preview consumers remain required. The financial compile was restarted with
socket-capable permissions before any fixture execution; no sandbox-denied test
was used as a permission probe.

## Final integration evidence and completion

Final Rust source: `7790f08b120726d9be5276a5263106eb9c58e70e`. All exact
implementation/fixup SHAs (2f0297fc, eff27c69, b8ab6a7d, 7790f08b) and accumulated
financial/SDK/quote/preview consumers independently reviewed and accepted. Separate
remaining-controls audit accepted the inventory and its corrected examples/metadata
semantics. No unresolved material finding, compelling simplification or implementation
portion is deferred. G1 source/review/selected-validation closure independently
accepted by zec_fee_limits_audit after inspecting actual logs and hashes.

| Evidence | Result |
| --- | --- |
| Final production compiler/all-target zcash-regtest Clippy | Passed with clean diagnostics (1m50/5m30), `target/zec-cost-policy-clippy-final.log`. |
| SDK missing-scan/fee/balance preview continuation | Passed5, `target/zec-cost-policy-preview-fixed.log`; includes original failing assertion and populated-wallet restoration. |
| Final native/foreign service consumers | Passed2 in30.95s, `target/zec-cost-policy-consumers-final.log`. |
| Earlier selected synthetic financial/proving/lifecycle | Nine successful non-preview cases carry forward; original13 selection was12pass/1previewfailure now resolved above. SDK fee>30000 accepted, explicit direct actual total-debit limits and signed-byte ownership/cancellation/restart tested. `target/zec-cost-policy-financial.log`. |
| Fixed compatibility stage | Passed31 (crypto2/payment15/SDK3/MCP11), plus Pepper35-file verification, `target/zec-cost-policy-compat.log`. Unaffected by the final private preview-only correction. |
| Cheap direct/NEAR contracts | Passed73/ignored1 explicit child helper, `target/zec-cost-policy-contracts-fixed.log`. |
| Configuration/CLI/assignment/integration-preparation | Passed299/ignored8 separately authorized cases, `target/zec-cost-policy-config-contracts.log`; eight changed tracked TOMLs parsed. |
| Complexity | Exact final source hashes verified independently; complete486 files/10728 spaces, zero exclusions/new warnings, `target/complexity-runs/20261107T080157Z-c18dee5b`. wallet_preview1/3; other financial hotspot dispositions carry forward. |
| Formatting/default development executable | Final fmt-check passed (`target/zec-cost-policy-fmt-final.log`); `make build-dev` passed cleanly in1m23 (`target/zec-cost-policy-build.log`). |

Test executables emit the existing macOS large __eh_frame linker performance
warning; it remains visible. Production compiler/Clippy/build diagnostics are clean.
No full make check, Docker consensus, live settlement or Tor qualification is claimed.
The initial test compile was restarted with socket permissions before fixture
execution, not after a denied socket attempt. All test keys/state were public
unfunded fixtures, not operator wallet state.

Enduring rules, complete remaining-control inventory, configuration/tool descriptions
and examples are maintained in AGENTS, README, configuration, wallet rotation,
wallet CLI, SDK integration, agent-source and testing/live guides. The inventory
explicitly distinguishes optional direct debit authority, nominal USDC sizing,
API authority/300s signing lifetime, retirement thresholds, managed recovery limits,
slippage (including the unproven application10%maximum), provider/signed expiry,
confirmation and monetary domains, and qualification-only budgets/counts/windows.
Observation cadence/health reporting are identified as scheduling, not money limits.
No local price oracle, USD-overhead gate or commission-metadata veto remains.

The operator's unrelated four tool-enablement changes in research_desk.toml remain
unstaged exactly; only its four retired overhead lines were removed in the commit.
Untracked .vscode/TODO, ignored local overrides, reference checkouts and private
financial evidence remain untouched. Local overrides that still use retired fields
must remove them; strict unknown-setting validation is intentional, with no legacy
adapter. No runtime restart, push or merge occurred.

P0-P3 and G1 are complete. P4 final remaining-controls audit is complete; delete
this completed plan, commit cleanup and independently review exact cleanup and
final combined branch. Preserve unsquashed history and all financial evidence.
