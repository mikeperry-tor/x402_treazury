# Managed wallet follow-ups

The public-swap treasury, capacity-based wallet groups, admission and Tor integration
are implemented. Their current contracts are in
[wallet architecture](../../wallet-rotation.md), the [README](../../../README.md),
and the [qualification scope](../../testing.md#qualification-status).
This plan contains only remaining extensions and qualification, not a rewrite of
that implementation. None authorizes mainnet transactions or commercial signup.

## Structured seller receipt evidence

The current client journals authorizations and reconciles canonical Base balance/
nonce evidence. It does not provide full per-request seller-receipt accounting.
Add an internal paid response/evidence type carrying status, body, selected
requirements, immutable attempt/generation identity and parsed settlement evidence.
Keep direct MCP and fallback invocation on the same path.

Persist only bounded necessary evidence, protected like other sensitive financial
records. Do not log authorization headers, full signed payloads or arbitrary
seller bodies. Distinguish missing, malformed and contradictory receipts from
provider application errors, transport failures and chain-confirmed outcomes.
A receipt alone cannot release a reservation or authorize a second signed request.
Use the original address/nonce and canonical confirmed anchor for reconciliation;
do not mix observations from different RPC endpoints or double-subtract a debit.

Implement as a commit for the type/parser and offline cases, then a commit for
journal/report integration with migration/restart coverage. Test a receipt before
confirmed credit/debit, success without receipt, malformed receipt, paid body loss,
expiry-unused, reorg contradictions and duplicate observation. Preserve compatibility
with uncertain historical attempts lacking receipt data. Use the durable canonical `USED` versus `EXPIRED_UNUSED` resolution and immutable
admission role/claim support. A provider fulfillment report remains separate from
canonical token-transfer attribution; neither replaces the other.

## Authenticated confidential swap qualification

Confidential configuration and request validation exist, but authenticated funded
execution has not been qualified. Resolve partner/API authentication and any
required user-session lifecycle against current provider contracts before adding
an operator runbook. A public quote or a partner token is not proof of confidential
access or entitlement to commissions. Current requests add no application fees.

Capture sanitized token/quote/status fixtures and test the full configured auth
combination, missing/expired credentials, exact recipient/refund/amount/mode
bindings, route minimums, fees and deadlines. Refuse public fallback. Then perform
an independently authorized bounded live qualification using supplied credentials
and funds; keep it separate from the public edition's recorded success.

Any future commercial tier needs an explicit fee policy and user-visible terms,
validated collector/amount limits and separate tests. Do not silently introduce
application fees as part of enabling authentication. Public swap and Tor identity
separation must not be advertised as financial unlinkability.

## Managed Permit2 / upto

Managed profiles currently reject upto. Supporting it requires each ephemeral
address to have a confirmed USDC allowance, a gas/approval funding strategy and
metered settlement accounting. Those costs and permissions become part of readiness,
reserve readiness and per-operation funding safeguards. Static upto signing is not sufficient qualification.

Define that lifecycle before implementation. Tests must cover approval failure,
allowance readiness/reorgs, maximum versus settled amounts and outstanding
liabilities across retirement. Approval failures cannot churn through new wallets.
Until this work is separately designed and qualified, retain the existing rejection.

## Manual retirement and incident repair

Ordinary serving and finite explicit sync invoke candidate-scoped canonical
retirement under the configured per-group tolerance, default 0.001 USDC. See
[automatic retirement](../../wallet-rotation.md#automatic-retirement).
Never-credited predecessors retain their destination guard while original delivery
remains possible. A completed job can close it only when every current and archived
operation has proven source expiry or never accepted calculation/submission; a
completed or abandoned label, refund or source confirmation alone proves neither
delivery nor loss. The maintained guide describes the implemented proof boundary.
There is no manual retirement CLI. Any future manual command needs explicit
wallet selection, fresh dual balance evidence, unresolved-liability and destination
checks, and durable audit support. It must not sweep funds or infer replacement
funding authority.

Specialized abandonment and unreconstructible original-record corruption remain
incident-specific operator work. Ordinary serving automatically rebuilds derived
chain checkpoints and current authorization dispositions from original identities
and fresh canonical evidence, retaining replaced proofs as history; see
[reconciliation policy](../../wallet-rotation.md#recovery-and-reconciliation-policy).
A future incident utility needs an exact reviewed source shape and preserved
original bytes/support. Recovery cannot manufacture missing accepted bytes or
immutable authorization identities.
There is no general-purpose migration or proof override.

## Related boundaries

Canonical chain replacement retains automatic reconstruction work even when old
evidence was finalized; it grants no replay or replacement funding authority.
Unrecoverable original records still require scoped operator investigation.
Live treasury exhaustion, arbitrary paid media routes
and mainnet crash injection were not established by the public experiment; existing
offline tests and dated reports state the evidence. Reusable live scenarios belong
to [the live integration runner](../../../tests/live/INTEGRATION.md), not to automatic
repetition of the completed public-swap test.
