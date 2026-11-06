# Optional managed-wallet extensions and qualification

The public-swap treasury, capacity-based wallet groups, admission and Tor integration
are implemented. Their current contracts are in
[wallet architecture](../../wallet-rotation.md), the [README](../../../README.md),
and the [qualification scope](../../testing.md#qualification-status).
These are optional capability, reporting and operator-tool extensions, not blockers
for ordinary supported payments or the achieved shielded-refund/address-query
privacy contract. NU7 support does not establish their separate live qualification.
None authorizes mainnet transactions, private-key access or commercial signup.

The source assessment baseline is `ad210e0ac527859cc4ed091b85c3808ddb541883`.
When selecting a track, capture the current branch and exact baseline, define its
operator outcome and independently review the remaining gap against current code.
Refine the selected track into coherent commits with owning-guide changes, tests,
exact-SHA/fixup and combined review under the
[planning workflow](../../planning-workflow.md). Do not implement every extension
merely because it is listed here.

## Structured seller receipt evidence

The current client journals authorizations and reconciles canonical Base balance/
nonce evidence. It already parses seller settlement headers for the narrow
structured insufficient-funds retry contract. The
[receipt classifier](../../../src/qualification/receipt.rs) also handles missing,
duplicate, malformed, oversized and transport-unknown qualification outcomes,
retaining safe seller-claim projections. Existing
[qualification evidence](../../../src/qualification/payment.rs) persists correlated
per-attempt receipt events, and qualification reports consume them. Those claims
are explicitly unverified; these foundations do not need a second parser or journal.

The remaining extension is ordinary-serving per-request receipt reporting or
accounting, if an operator needs it. Define the useful result and reuse the existing
classification and immutable attempt/generation identity. An internal paid response/
evidence result may carry status, body, selected requirements and parsed claims;
keep direct MCP and fallback invocation on the same path. Persist only necessary
safe projections protected like other sensitive financial records, without logging
authorization headers, full signed payloads or arbitrary seller bodies.

New reporting must not grant release/replay authority or replace the existing
narrow insufficient-funds retry contract. Distinguish provider claims, application
errors, transport uncertainty and chain-confirmed outcomes. Use original address/
nonce identities and coherent canonical evidence; do not mix endpoints or
subtract a debit again because a receipt was observed. Durable canonical `USED`
versus `EXPIRED_UNUSED` resolution and immutable admission ownership remain the
financial authority. Provider fulfillment and token-transfer attribution remain
separate facts.

First implement only missing response/ownership integration and offline cases;
add ordinary journal/report integration only when the selected outcome requires
it, with migration/restart coverage. Reuse existing parser and qualification cases.
Cover the actual new handoff with receipt-before-confirmation, missing/malformed
claims, paid body loss, unused expiry, reorg contradiction and duplicate observation
where relevant. Preserve uncertain historical attempts lacking receipt data.

## Authenticated confidential swap qualification

Confidential configuration, explicit request modes, binding validation, API-key
and bearer user-session headers already exist. Authenticated funded execution
remains unqualified. If this capability is selected, verify the current provider
entitlement/authentication and session lifecycle before adding an operator runbook.
A public quote or configured token proves neither confidential access nor
entitlement to commissions. Current requests add no application fees.

Reuse existing configuration/request tests and add sanitized fixtures for any
missing auth combination, expired credentials and exact recipient/refund/amount/
mode bindings, route minimums, fees and deadlines. Refuse public fallback. Then
perform an independently authorized bounded live qualification using supplied
credentials and funds; keep it separate from public-swap evidence. Shielded refund
receivers alone do not qualify provider confidentiality or funded execution.

Any future commercial tier needs an explicit fee policy and user-visible terms,
validated collector/amount limits and separate tests. Do not silently introduce
application fees as part of enabling authentication. Public swap and Tor identity
separation must not be advertised as financial unlinkability.

## Managed Permit2 / upto

Managed profiles currently reject upto; static signing support is a separate
capability. If managed upto is selected, first design the per-address USDC allowance,
necessary gas/approval funding and maximum-versus-settled liability lifecycle.
Assess existing SDK and owner enforcement before proposing any additional readiness
or funding gate. This plan does not authorize a new spending budget, reserve mirror,
financial ledger or duplicate balance/allowance check; follow the
[best-effort](../../wallet-rotation.md#best-effort-wallet-rules) and
[reconciliation](../../wallet-rotation.md#recovery-and-reconciliation-policy) rules.

Qualification must cover actual approval failure, allowance reorgs, maximum versus
settled amounts and outstanding liabilities through retirement. Approval failures
cannot churn through new wallets. Until separately designed and qualified, retain
the explicit managed rejection; NU7 and shielded refunds do not change it.

## Manual retirement and incident repair

Ordinary serving and finite explicit sync already invoke candidate-scoped canonical
[automatic retirement](../../wallet-rotation.md#automatic-retirement), including
the configured per-group tolerance and original destination/delivery guards.
Proven nondisclosure and closed local source work retain their separate meanings;
source expiry/confirmation or a completed/abandoned label alone cannot establish
bridge settlement. These are maintained runtime contracts, not unfinished manual
repair work.

There is no manual retirement CLI. If an operator needs one, define explicit wallet
selection and reuse the existing retirement owner, canonical proof and unresolved-
liability checks with durable transition evidence. Do not add a parallel proof
system, force extra agreement reads, sweep funds or infer replacement-funding
authority merely to implement an administrative command.

Ordinary serving already reconstructs derived chain checkpoints and authorization
dispositions from original identities and canonical evidence, retaining replaced
proofs as history; see the
[reconciliation policy](../../wallet-rotation.md#recovery-and-reconciliation-policy).
Specialized abandonment and unreconstructible original-record corruption remain
incident-specific operator work. An incident utility should start from a concrete
failure and exact reviewed source shape, preserving original bytes and support.
Recovery cannot manufacture missing accepted bytes or immutable authorization
identities. No general-purpose migration or proof override is selected.

## Qualification and completion boundaries

Canonical chain replacement grants no replay or replacement-funding authority.
Unrecoverable original records still require scoped operator investigation.
Live treasury exhaustion, arbitrary paid-media routes and mainnet crash injection
are not established by SDK upgrades or offline privacy regressions. Keep those
limits in [qualification guidance](../../testing.md#qualification-status);
reusable live scenarios belong to the
[live integration runner](../../../tests/live/INTEGRATION.md), without automatically
repeating the completed public-swap experiment.

For a selected track, independently review its exact commits/fixups and complete
affected payment, ownership and recovery chains, with applicable complexity and
ordered validation. Keep financial evidence private. After implementation and
combined review, hand enduring contracts to their owning guides and remove only
the completed planning material through reviewed cleanup; retain genuinely
unfinished selected work without a completed-plan archive.
