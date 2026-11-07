# Direct swap control

## Intent, baseline and status

Implement an optional first-party MCP toolset for both receiving payments through
asset-to-ZEC swaps and sending payments through ZEC-to-asset swaps. Implement and
review receiving first, then continue to sending. The receive gate is an
intermediate integration boundary, **not completion or a stopping point**. Delivery
requires both directions and the final combined review.

Starting branch: `main`. Starting code baseline:
`b708a06b340dfc7ee695eab5b66a6120c152af61`. At planning, only unrelated untracked
`.vscode/` and `TODO` were present; preserve them. Status: planned, no implementation
or funded validation. This request authorizes preparation of the plan; the plan
itself grants no funded execution, deployment, push or merge authority.

Follow [planning workflow](../planning-workflow.md),
[independent reviews](../../AGENTS.md#independent-subagent-workflows),
[validation selection and order](../testing.md#selecting-validation-by-change), and
[complexity review](../../tests/COMPLEXITY.md#review-discipline). Read the owning
[wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[observations](../wallet-rotation.md#wallet-observation-policy),
[reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy),
[SDK boundaries](../zingo-integration.md),
[network policy](../network-egress.md),
[configuration](../configuration.md), and
[agent-facing instructions](../agent-sources.md#tool-documentation) before editing
their consumers. Changes to resource behavior also require
[resource limits](../local-resource-limits.md) and both maintained inventories.

## Current behavior and proposed scope

`src/rotation/near.rs` implements the native-ZEC-to-Base-USDC funding route. It
validates that exact asset pair, EVM recipients, transparent ZEC deposits and
shielded ZEC refunds. Its amount parsing uses `u64`; its cost checks treat output
as six-decimal USDC. Managed funding uses an exact-output sizing preview followed
by an executable exact-input quote. Do not change that behavior as an incidental
consequence of adding direct swaps.

`src/treasury/actor.rs`, `src/treasury/send.rs` and the existing Store own serialized
SDK mutation, accepted inputs, encrypted snapshots, calculated bytes, submission
and recovery. Jobless treasury operations already exist. Some persisted identity
reload paths, including `PreparedTransaction::load`, assume an EVM recipient.
Current MCP tools expose neither general treasury payouts nor incoming invoices.

The new toolset uses 1Click origin-chain deposits and destination-chain delivery:

| Direction | Required behavior |
| --- | --- |
| Receive | Create a one-payment request for an exact amount of a supported source asset, using `EXACT_INPUT`, delivered as ZEC into the configured treasury. The payer supplies the foreign funds and source-chain refund address. |
| Send | Quote and execute `EXACT_OUTPUT` from treasury ZEC to an exact supported destination asset, amount and address. Return unused source funds to the treasury's bound shielded refund receiver. |

Support means tokens, networks, address forms and routes actually supported by
the service and the implemented request contract. A supported-token listing does
not promise liquidity or destination compatibility. Identify assets by full asset
ID and chain; symbols alone are ambiguous. Keep amounts exact using decimal
strings and checked arithmetic adequate for supported token quantities; convert
only ZEC quantities into the SDK's bounded zatoshi representation.

No native foreign-chain signer or NEAR wallet is required for these deposit-funded
transfers. This does not implement NEAR marketplace x402 authorization, general
foreign-wallet management, USDC sweeping/consolidation, recurring invoices, payment
escrow, fiat billing or a generic workflow engine. Coordinate reusable route work
with [wallet sweeping](wallet_merge_sweeping.md), without importing its separate
gas-funding and wallet-consolidation responsibilities.

## Tool and authority contract

Names below are proposals to finalize at D0, not current API promises:

| Tool | Inputs and useful result |
| --- | --- |
| `swap_assets` | Search/page current supported asset metadata, retaining chain, asset ID and decimals. Discovery creates no swap or wallet. |
| `payment_request_create` | Source asset, exact source amount, payer's source-chain refund address, and a caller request ID; return the saved invoice ID, exact deposit instructions, output estimate/minimum and provider deadlines. Destination is the listener's configured treasury. |
| `swap_preview` | Destination asset, exact destination amount and address; return an explicitly non-executable price preview, required ZEC and fee/slippage information. |
| `swap_send` | Destination terms and caller request ID; obtain and persist an executable quote, accept its exact debit under configured authority, then execute through the treasury owner. A dry preview is never executable authority; reject changed terms outside the caller's accepted authority. |
| `swap_status` | Owned invoice/send ID; return saved terms, source transaction and provider progress, actual delivery evidence and SDK receipt/spendability where known. No new swap or deposit. |

Finalize whether an executable quote needs a separate prepare/accept tool at D0
from the actual authority contract. Prefer a single authorized send call when it
can express the caller's exact destination terms and source-debit acceptance;
do not add mandatory round trips merely for presentation. Previews never spend.
Execution may return a durable pending operation ID rather than hold an MCP call
through cross-chain settlement. Later status calls inspect the same operation.

Receive and send are separate explicit listener capabilities, disabled by default.
Bind each enabled listener to the configured treasury and authenticated management
principal. Reject an unauthenticated financial-control configuration. Agent/session
fields, request IDs and foreign recipient addresses cannot select another owner,
treasury or credential. Owner-scope create, retry and status lookup; reject cross-owner
IDs without exposing their terms. Existing `auto_fund`, source-registration and
raw-call permissions grant no arbitrary payout authority.

Configure outbound spending deliberately. The accepted send must bind the exact
destination terms and permitted quoted ZEC principal; enforce actual transaction
fee authority through the existing treasury mechanism. Resolve how this acceptance
is expressed before implementation. It is per-request payment authority, not a new
daily/cumulative budget, generic per-transfer ZEC budget or USDC reserve mirror.
Actual SDK spendability and existing accepted input ownership govern capital.
Do not add parallel balances or reserve allocation machinery.

Advertising, search and dispatch must use the same capability/ownership policy.
Honor on-demand discovery without requiring provider registration or placing an
entire external API in the catalog. Review endpoint summaries, search keywords,
tool descriptions and every parameter description together. Omit disabled admission
tools and deny their dispatch; retain owned status access for accepted work after
disablement. Use host-owned configured 1Click credentials through
the existing client boundary; never expose keys or sessions as agent arguments.

## Financial, receipt and transport invariants

- Persist each receive address's SDK snapshot and exact invoice/deposit binding
  before disclosing payment instructions. Persist accepted outgoing terms, input
  ownership, signed bytes and the wallet snapshot before submission. Reuse the
  existing Store and serialized treasury owner, not SDK quick-send helpers or a
  second transaction engine.
- Deduplicate caller request IDs within owner and operation kind using persisted
  original inputs. Repetition returns the existing operation; conflicting reuse
  fails before new financial work. This is local acceptance semantics, not a claim
  that 1Click provides quote-creation idempotency. Resolve concurrent creation and
  lost-response/crash boundaries without disclosing an unpersisted quote. A new
  request ID is a separate payment request, not an automatic recovery attempt.
- Bind the accepted route, asset IDs, atomic amounts, source deposit address/memo,
  destination address and any supported destination metadata, refund address,
  deadlines and stable network identity. Add only operation facts required for
  authority, recovery and useful status; reuse existing financial journals.
- Exact-input receiving fixes the customer's source amount, not the eventual ZEC
  value. Return estimated/minimum output and fees accurately. Require a payer-owned
  origin-chain refund destination; never substitute the treasury's ZEC address.
  Preserve mandatory memos. Reject unsupported address/tag contracts before
  publishing instructions or accepting a send.
- One invoice describes one quoted payment. Do not promise address reuse, arbitrary
  partial payments, permanent exchange rates or invoice expiry that revokes an
  already disclosed deposit route. Provider quote deadlines, response inactivity,
  Zcash signed expiry and polling cadence have different meanings. Local expiry
  cannot manufacture provider `EXPIRED`, prove nonpayment or erase late delivery.
- Provider status owns the bound swap's outcome; SDK/blockchain state owns received
  ZEC and current spendability. Report these separately, including unknown or
  pending evidence. Repair derived state from established facts; optional receipt
  attribution and quote/report discrepancies cannot invalidate owned funds or a
  proven swap outcome. Do not duplicate accounting to force numerical agreement.
- Disconnection, cancellation, capability disablement and restart preserve accepted
  source work and disclosed invoice bindings. Disablement stops new admissions,
  not reconciliation. Maintain status/recovery access to an owner's accepted work
  without re-enabling spending. An ambiguous submission or returned refund grants
  no new swap, deposit or signature. Reuse exact accepted bytes only under existing
  submission/recovery rules; never replace a possibly executed payment.
- Direct sends and managed funding share SDK input ownership safely. Do not hold
  the treasury actor across remote quote/status waits. Failures affect their own
  operation; ordinary MCP availability and independent financial progress continue.
- All production HTTP/gRPC uses `src/network.rs`. Generalize a stable typed identity
  for direct operations, including persisted reload, rather than coercing foreign
  or Zcash addresses into EVM identities. Preserve existing funding identities,
  Tor token domains, remote DNS, public-destination rules and no direct fallback.
  No new whole-work timeout, queue cap or global readiness gate is selected.
- Preserve stable state formats through additive compatible evolution and tests.
  Never log keys, signed bytes or credential-bearing upstream prose; apply the
  owning severity and diagnostic-clarity policy to new and encountered call sites.

## Ordered commits and integration gates

Each implementation commit includes its affected permanent documentation and
focused regression tests. IDs remain stable as status and evidence evolve.

### D0 — Resolve contracts and independently review the design

Before substantive implementation, verify current 1Click token, quote and status
contracts against the pinned code. Settle executable quote/source-debit acceptance,
listener configuration, ownership, idempotency, durable identity, schema evolution
and receipt reporting. Trace full receive and send call chains with the serialized
treasury owner and existing maintenance/recovery workers.
Capture the relevant baseline CogC/CycC evidence before substantive Rust changes
and compare source-matched results at G1 and G2.

Resolve ZEC destination support specifically: the general chain guide currently
lists transparent addresses, while our SDK guide records native-bridge shielded
refund compatibility and unfunded quote evidence. Refund compatibility does not
qualify asset-to-ZEC delivery. Establish shielded recipient support for the intended
routes. Do not silently fall back to transparent delivery or enable transparent
address-discovery RPCs. If direct shielded delivery cannot be supported, review an
explicit receive-and-shield design and its privacy/SDK discovery implications before
implementation; record the decision here without dropping the receive scope.

Exit: a fresh independent read-only design review checks baseline facts, complete
consumers, contracts, tests and simplifications; material findings are resolved and
the committed plan is updated. Public unfunded contract checks are not funded
settlement evidence. Missing live qualification is identified explicitly.

### D1 — Shared route boundary and opt-in receive tools

Introduce the smallest reusable 1Click transport/route representation needed for
both directions while keeping managed funding's specialized policy intact. Add
asset discovery, receive permission/configuration, owned receive-address derivation,
durable invoice acceptance, origin-chain refund/memo handling, status retrieval and
MCP advertising/search/dispatch. Add operation identity/state evolution without
changing old funding journal meanings. Persist invoices independently of Base
wallet allocation and initial funding claims.

Update configuration, agent-source, wallet ownership, SDK and network guides as
applicable. Tests cover wide atomic amounts, asset/chain ambiguity, refund and
memo binding, invoice address persistence, same/conflicting IDs, concurrent duplicate
creation, cross-owner denial, lost responses and restart, late/under/excess payments,
unknown provider outcome, and independent SDK receipt/spendability. Assert no ZEC
spend, foreign signer, Base allocation, funding-claim reopening or secret output.

### G1 — Receive integration gate; continue to sending

Independently review D0/D1 together, all exact commits/fixups, configuration through
MCP call/search, Store/address persistence, transport identity, provider status and
SDK receipt consumers. Supply source-matched CogC/CycC review and selected compiler,
Clippy, compatibility and focused runtime evidence in maintained order. Resolve
material findings and compelling simplifications before building the send path.

Exit: receive works with offline/SDK fixtures under its stated support contract,
existing funding consumers remain compatible, and any live evidence gap is labeled.
Then proceed to D2. Passing G1 does not complete this plan.

### D2 — Direct exact-output sends and accepted-work recovery

Add send capability/configuration, useful dry previews and executable exact-output
sends under the D0 authority contract. Persist accepted swap bindings before source
preparation, tie treasury preparation to those bindings, and use the existing input,
snapshot, signed-byte, submission and canonical-recovery paths. Extend maintenance
only as needed to own direct operations and their original identities after waiter
loss, capability disablement and process restart. Keep provider waits outside the
SDK owner. Status exposes pending/completed/refunded/failed facts and uncertainty
without treating refund receipt or source confirmation as destination delivery.

Keep generic route validation separate from USDC reserve/cost policy; do not
reinterpret nominal reserves as direct-send authority or replace managed funding's
exact-input execution. Add coherent schema compatibility, configuration, authority,
recovery, network and agent-facing documentation with the code.

Tests cover exact output and quoted input/slippage/refund semantics, destination
and principal substitution, stale quote acceptance, actual network fee rejection,
concurrent funding/send input selection, duplicate acceptance, cancellation at
preparation/publication/submission, ambiguous broadcast, restart with original
bytes/identity, quote deadline versus signed expiry, late provider success/refund,
disablement and independent progress. Assert no duplicate deposits, automatic
replacement swaps, extra native signers, allocation/top-up or consumed-claim reset.

### G2 — Final combined implementation and correction gate

Independently review the full D0–D2 change, every accumulated commit/fixup, complete
receive/send/funding consumers, enduring documentation and plan acceptance claims.
Include the relevant unchanged high-risk financial/network paths and source-matched
CogC/CycC dispositions. Revisit prior findings for recurring assumptions and testing
gaps, then follow the planning workflow's correction loop until material findings
and compelling simplifications are resolved.

Exit requires both receive and send implemented, selected validation passed,
authority and persistence compatibility demonstrated, and review evidence tied to
the exact implementation. No receive-only handoff counts as completion. Do not
claim live settlement, shielded delivery or Tor qualification from fixtures.

### D3 — Documentation handoff and plan cleanup

After G2 and applicable user review rounds, ensure enduring contracts live in the
owning guides and the documentation index. Add/update the relevant AGENTS.md
change-area links for direct swap control, rather than copying detailed contracts
there. Keep resource/timer inventories synchronized if behavior changed. Move only
genuinely separate unfinished work to focused plans; do not defer send or a material
finding to declare this plan complete.

Delete this completed plan, repair links and commit cleanup. Independently review
the exact cleanup SHA and final combined branch. Preserve plan ancestry and all
unsquashed fixups; retain financial evidence under its existing private ownership.

## Validation and review evidence

Use [affected-behavior selection](../testing.md#selecting-validation-by-change) and
the maintained [validation order](../testing.md#validation-order-and-continuation):
cheap changed contracts first, applicable design/complexity review and selected
production compiler/all-target Clippy warning and compatibility stages before
broad runtime suites. Select treasury/Store/lifecycle, MCP permissions/discovery
and network/Tor-policy tests from the actual changed call chains. Run necessary
real-SDK synthetic-chain cases for accepted input ownership and restart behavior;
verify new financial tests are wired and execute nonzero tests. Do not introduce
a separate validation matrix or duplicate already valid evidence.

Use Make entrypoints and pinned tools, freeze source/executables during validation,
and run feature configurations sequentially. Socket fixtures require socket-capable
execution up front; socket-free tests stay in the ordinary sandbox. Record compiler
diagnostics as well as exit status. Keep generated complexity/contract captures
under ignored `target/`, retaining only concise conclusions here. This plan-only
change needs documentation/link/diff checks, not a Rust build.

Before each integration gate, independently review exact implementation SHAs and
relevant consumers. Reviewers are not authors and start fresh with self-contained
briefs: intent, baseline, retained plan, invariants, complete scope, exact SHAs or
content-identified uncommitted snapshot, required security context and known
validation gaps. Record their initial assessment before author rationale, then
reconcile prior findings. Fix findings with `git commit --fixup=<introducing-SHA>`;
independently review each exact fixup. A single review may cover exact-SHA and
combined scopes when it actually examines both.

Live-funded checks need a separately approved concrete recipe under
[the live runbook](../../tests/live/INTEGRATION.md); no real wallet secrets or funded
execution are required for plan preparation or offline implementation validation.
Recheck only external facts needed for the selected behavior; the
[deferred timing assessment](deferred/near_swap_timing_tightening.md) does not become
a mandatory research project or a source of artificial expiry rules.

| Scope | Status / evidence |
| --- | --- |
| Planning baseline | `main` at `b708a06b340dfc7ee695eab5b66a6120c152af61`; receive and send design assessment identified hardcoded route, amount, cost and identity assumptions. No implementation or live settlement evidence. |
| Plan design review | Fresh read-only reviewer `direct_swap_plan_review` examined baseline consumers and initial plan snapshot SHA256 `63e0a918b41a4d281fe02209d485825e6beb863cbd39525236d1635821c231e0`. No material blocker; clarified that a dry preview grants no executable authority and disabling admissions retains owned status access. Local link targets and whitespace checked; no Rust checks apply to this plan-only change. Exact committed-state review follows. |
| D0 / D1 / G1 | Pending. Receive gate is not completion. |
| D2 / G2 | Pending. Both directions required. |
| D3 | Pending final implementation/user review and documentation handoff. |

## External references to recheck

- [1Click quote](https://docs.near-intents.org/api-reference/oneclick/request-a-swap-quote),
  [swap types](https://docs.near-intents.org/integration/distribution-channels/1click-api/swap-types)
  and [status](https://docs.near-intents.org/api-reference/oneclick/check-swap-execution-status).
- [Supported assets](https://docs.near-intents.org/resources/asset-support),
  [chains/address restrictions](https://docs.near-intents.org/resources/chain-support)
  and [origin-chain deposit flow](https://docs.near-intents.org/integration/distribution-channels/1click-api/quickstart/making-a-request).
- Existing [shielded refund evidence and limitations](../zingo-integration.md#refund-receivers-and-seed-recovery).
