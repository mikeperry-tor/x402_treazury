# Wallet merging and sweeping

## Objective and status

Plan an operator command to recover managed Base USDC into ZEC and send spendable
treasury ZEC to a specified shielded address. No implementation or funded execution
is authorized by this document. CLI names below are proposals.

Deliver a useful ZEC-only sweep independently of USDC recovery. Evaluate optional
USDC swaps and consolidation against their actual fees, minimum capital requirements
and privacy consequences before expanding the command.

Read the maintained [wallet CLI](../wallet-cli.md),
[wallet ownership and recovery rules](../wallet-rotation.md),
[selection and isolation design](../architecture.md#wallet-selection-and-network-isolation)
and [network policy](../network-egress.md) before implementation. These contracts
remain authoritative; this plan does not change ordinary funding or serving policy.

## Command scope and options

The intended complete flow is USDC recovery followed by a shielded ZEC send.
Offer an explicit ZEC-only mode that does not require Base gas funding or NEAR.
Decide the exact command structure and default mode before implementation; do not
silently include wallet merging or correlate wallets through a shared gas payer.

| Mode | Proposed behavior | Residual funds |
| --- | --- | --- |
| ZEC only | Send currently spendable treasury ZEC, after fees and reservations, to the shielded destination | Report unavailable, unconfirmed, reserved and pending incoming funds |
| Separate USDC swaps | Obtain an individual exact-input USDC-to-ZEC quote and deposit for each selected eligible wallet | Leave below-minimum balances and report route failures independently |
| Optional consolidation | Consolidate an explicitly selected group of below-minimum USDC wallets, then swap the combined balance | Leave excluded balances, uneconomic groups and unresolved work |

By default, never merge wallets. Individually swappable wallets do not need
consolidation. Evaluate live quotes rather than hardcoding a NEAR minimum.

The suggested `--merge-all-above USDC` flag needs narrower semantics. Prefer
`--merge-dust-above USDC`, meaning: among the selected wallets that cannot swap
individually because their balances are below the route minimum, merge only those
whose balances are at least the specified amount. Define the threshold as inclusive.
Do not treat arbitrary quote failures as evidence that a wallet is below minimum.
Provide explicit wallet selection/exclusion so operators can choose privacy groups;
a balance threshold alone cannot express which histories may be linked.

Before consolidating, establish that the selected total has a viable swap quote.
Revalidate the route before signing; do not merge an uneconomic group merely because
individual transfers are technically possible. A preview should identify participating
addresses, the consolidation destination, fee funding, expected output and leftovers.
Final CLI syntax and preview/execution interaction remain design work.

## Base fees and gas funding

Standard Base USDC transfers require ETH gas. USDC has six decimal places; its
transfer contract does not impose a swap-like positive transfer minimum above one
atomic unit (0.000001 USDC). Gas cost, transfer eligibility and service-specific
minimums still apply. There is no need to swap each source wallet's USDC into ETH.

Separate USDC-to-ZEC swaps also require a Base deposit transaction. Gas funding is
therefore a requirement for USDC sweeping even when consolidation is disabled.
NEAR's signed-intent execution for balances already inside Intents does not remove
the deposit requirement for USDC held in our Base wallets.

USDC's `transferWithAuthorization` permits a source wallet to sign an exact transfer
and another wallet to submit it and pay gas. This is a contract mechanism, not a
requirement for a Circle account or API. Verify the deployed Base contract/domain,
authorization behavior and bridge deposit recognition. Existing x402 signing support
is not itself a general-purpose transaction relayer.

| Gas-funding option | Flow | Tradeoff |
| --- | --- | --- |
| Shared fee payer | Fund one controlled Base address with ETH; it submits each source's authorization directly to a separate swap deposit address or to the selected consolidation address | Avoids ETH in every source wallet, but publicly correlates transactions through the common submitter |
| Separate source funding | Fund each selected USDC wallet with ETH; each submits its own swap deposit | Avoids a common Base submitter, but requires multiple funding swaps and minimum allocations; does not establish unlinkability |
| Manual ETH funding | Operator supplies ETH on Base to the chosen payer or source addresses | Simplest funding integration, with external operator work and funding-history exposure |

For an authorized consolidation group, the consolidation wallet can also be the
ETH fee payer. Alternatively keep these roles separate where useful; additional
wallets should have a concrete purpose. A shared fee payer can support separate
swaps without merging USDC, but that correlation must be an explicit choice.

To bootstrap automatically, swap treasury ZEC into **native ETH on Base**, with
the controlled fee-payer/source address as the swap recipient. The authorization
itself has no gas-funding address. WETH is not native gas currency. Persist the
destination key and operation before funding it, verify destination credit, then
submit the USDC transactions. Reserve actual required source funds before the final
ZEC sweep; do not empty the treasury before gas funding can finish.

Estimate each planned transaction's execution gas and fee parameters, include Base's
L1 data fee and the eventual consolidation-to-swap transfer where applicable, and
show the aggregate ETH requirement. Exact-output ZEC-to-ETH quoting can target that
requirement, subject to the live bridge minimum. Recheck fees before submission;
an estimate cannot guarantee future fee levels. Decide fee allowance and handling
of a shortfall without introducing speculative funding loops or arbitrary budgets.
Report unused ETH; recovering it may itself be below a swap minimum and costs gas.

## Minimums and economic decision

Live unauthenticated public-route dry quotes established a ZEC-to-native-Base-ETH
input boundary of **132,000 zatoshis (0.00132 ZEC)**: one zatoshi less was rejected,
while that amount produced a quote. Its indicative input valuation was about
**$1.40**, with about **0.0005953 ETH** output. This is an observation to recheck,
not a maintained service guarantee or a constant to embed in code.

The input is capital converted to ETH, not a fee of that size. The observed quoted
input/output valuation difference was about $0.006, excluding the SDK-calculated
source Zcash transaction fee and subsequent Base gas. Do not double-count quoted
withdrawal fees without verifying inclusion semantics or describe the route as
fee-free. The returned quote included provider-added application-fee treatment.

This was below the user's **$2 minimum-capital decision threshold** for considering
USDC recovery. That threshold is a product-scoping preference, not a new production
spending limit. Multiple independent gas-funding swaps would each face a minimum;
one shared fee payer needs only its own funding allocation, sized for the work.
Compare total fees and potentially stranded ETH with recoverable USDC before
deciding whether the expanded feature is worthwhile.

The `basic` and `advanced` quote requests required authentication. Their minimums
and route behavior remain unverified. Dry quotes establish neither funded settlement
nor gas distribution or privacy qualification. Preserve raw requests/responses under
ignored `target/wallet-sweep-quotes/`; do not commit generated reports or logs.

## Ownership, recovery and privacy

Use existing exclusive treasury ownership; the initial design is an offline wallet
administration operation, not concurrent serving. Do not start ordinary funding
workers while sweeping. Define inventory scope explicitly, including retained and
disabled managed wallets. Static wallets, unrelated configurations and external keys
must not be implicitly added to that inventory.

Stopping serving does not cancel disclosed EIP-3009 authorizations. Preserve their
liabilities and existing reconciliation semantics; unresolved funds are not free to
sweep merely because no request is currently running. Reuse existing ownership and
journal mechanisms rather than creating parallel monetary balances or a generic
workflow ledger. Review wallet roles after sweeping so a later serve cannot interpret
an emptied wallet as permission to reopen its consumed initial funding claim.

Journal accepted inputs, signed bytes, original identities, destinations and liabilities
before disclosure for Base transfers, swaps and the final ZEC send. Cancellation or
restart must preserve ownership and resume reconciliation of the same accepted work.
An ambiguous submission does not authorize another transfer, signature or swap
deposit. Keep failures local to affected operations while valid siblings progress.
Retain keys and pending refund ownership after a reported partial sweep.

Use the configured network factory for all production RPC, quote, status and submission
traffic. Preserve recipient/owner isolation, public-destination enforcement and Tor's
remote DNS/no-direct-fallback behavior. No new whole-operation deadline, polling cap
or readiness framework is justified by this feature.

Consolidation permanently connects its selected source histories. Shared fee payers,
ETH funding sources, quote recipients/refund addresses, bridge observations and timing
can also correlate otherwise separate wallets. Separate Base transactions or a
shielded final send do not establish end-to-end unlinkability. Resolve address and
network identity handling across the full flow before making privacy claims.

## Implementation boundaries and delivery

1. Implement a useful ZEC-only sweep through the existing treasury owner and durable
   submission path. Existing `src/treasury/send.rs` prepares transparent-recipient
   deposits; add a validated shielded-send path rather than bypassing journaling with
   an SDK quick-send helper. Require a shielded receiver, including when accepting a
   unified address. Decide how owned transparent funds are shielded under existing
   policy. Report pending/reserved funds separately from the amount sent.
2. Resolve USDC economics, gas-payer choice and CLI semantics. Extend quoting beyond
   `src/rotation/near.rs`'s current ZEC-to-USDC exact-output restriction. Validate
   reverse exact-input swaps, native Base ETH funding and bridge ZEC recipient support.
   If bridge payout is transparent-only, receive into owned addresses, shield under
   existing policy and then send shielded ZEC; do not assume a unified address selects
   a shielded receiver. Assess correlation through any reused bridge addresses.
3. Add separate USDC swaps with the explicitly chosen gas-funding policy, then optional
   selected-wallet consolidation. Do not require consolidation to recover wallets that
   already meet their own swap minimum. Preserve refunds and report partial results.

Use independent design and implementation reviews under AGENTS.md. For substantive
Rust changes, establish the relevant CogC/CycC baseline and review full changed call
chains under [complexity review discipline](../../tests/COMPLEXITY.md#review-discipline).
Favor existing ownership representations and remove unnecessary responsibilities.

Focused tests should cover shielded recipient validation, actual fee subtraction,
reserved inputs, below-minimum skipping, threshold/selection boundaries, forbidden
merges, shared-payer opt-in, native ETH selection, and exact transfer/deposit evidence.
Cover cancellation/restart and ambiguous submission without duplicate financial work,
outstanding authorizations, late refunds/credits and unrelated sibling progress.
Assert no implicit allocation, top-up or initial-claim reopening. Verify new tests are
wired and execute. Offline fixtures do not qualify live settlement or Tor; funded
qualification needs separate operator authorization.

Update the wallet CLI and other owning guides as behavior ships. Keep only unfinished
work here and remove the completed plan under the documentation policy.

## External contracts to recheck

- [NEAR quote API](https://docs.near-intents.org/api-reference/oneclick/request-a-swap-quote)
  and [supported tokens](https://1click.chaindefuser.com/v0/tokens).
- [NEAR swap types](https://docs.near-intents.org/integration/distribution-channels/1click-api/swap-types),
  [origin-chain deposits](https://docs.near-intents.org/integration/distribution-channels/1click-api/quickstart/making-a-request)
  and [signed-intent scope](https://docs.near-intents.org/integration/distribution-channels/1click-api/quickstart/signed-intent-execution).
- [Base fee estimation](https://docs.base.org/specifications/transactions/network-fees)
  and [standard USDC transfers](https://docs.base.org/build-on-base/accept-payments/make-a-simple-payment).
- Circle's [transfer contract](https://github.com/circlefin/stablecoin-evm/blob/master/contracts/v1/FiatTokenV1.sol)
  and [EIP-3009 implementation](https://github.com/circlefin/stablecoin-evm/blob/master/contracts/v2/EIP3009.sol).
