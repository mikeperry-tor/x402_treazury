# Managed wallet follow-ups

The public-swap treasury, double-buffered pools, admission and Tor integration
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
with uncertain historical attempts lacking receipt data. Keep reporting honest when
`RESOLVED` cannot distinguish used from expired.

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
rotation and source budgets. Static upto signing is not sufficient qualification.

Define that lifecycle before implementation. Tests must cover approval failure,
allowance readiness/reorgs, maximum versus settled amounts and outstanding
liabilities across retirement. Approval failures cannot churn through new wallets.
Until this work is separately designed and qualified, retain the existing rejection.

## Related boundaries

Deep finalized-chain recovery remains an explicit operator investigation, not a
promise of automatic repair. Live treasury exhaustion, arbitrary paid media routes
and mainnet crash injection were not established by the public experiment; existing
offline tests and dated reports state the evidence. Reusable live scenarios belong
to [the live integration runner](../../../tests/live/INTEGRATION.md), not to automatic
repetition of the completed public-swap test.
