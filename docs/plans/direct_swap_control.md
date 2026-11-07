# Direct swap control

## Intent, baseline and status

Implement an optional first-party MCP toolset for receiving foreign assets through
asset-to-ZEC swaps and sending ZEC-to-asset swaps, plus native shielded Zcash
receiving and sending without NEAR Intents. The user explicitly added native paths
on 2026-11-07 while D1 validation was running. Implement and
review receiving first, then continue to sending. The receive gate is an
intermediate integration boundary, **not completion or a stopping point**. Delivery
requires both directions and the final combined review.

Starting branch: `main`; implementation branch: `direct_swap_control`. Starting code baseline:
`b708a06b340dfc7ee695eab5b66a6120c152af61`. At planning, only unrelated untracked
`.vscode/` and `TODO` were present; preserve them. Status: D0/D1/D1N complete; G1 passed at `d2c84799`. D2 send implementation
(native and foreign, with optional total-debit cap) is now in progress.
No funded validation. Implementation starts from committed plan `8b861bbc141832887ec77a34c47b86f7e1c9fa71`. The implementation request authorizes local implementation and review; the plan
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

Native Zcash uses the existing embedded treasury signer; it never calls NEAR.
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

## D0 implementation decisions and evidence

- Single-call sends accept exact destination asset ID/chain/atomic amount/recipient
  and an explicit maximum source principal in zatoshis. Persist the executable
  quote's exact principal within that per-call authority; the existing transaction
  fee ceiling remains separate. Previews are dry and confer no authority.
- One additive encrypted direct-operation record, unique by authenticated listener
  owner, operation kind and caller request ID, preserves original terms and UUID.
  Reserve before quote creation; a lost creation response retains uncertainty and
  does not automatically create another quote. No parallel balance ledger.
- Receive and refund derivation use all-shielded SDK receivers with snapshot and
  operation binding persisted before disclosure. No transparent discovery fallback.
- Direct transport identity is domain separated by treasury UUID and operation UUID,
  retained through quote/status and treasury transaction reload. Existing funding
  identity domains and specialized route policy remain intact.
- Treasury preparation checks durable direct quote/deposit/principal/deadline
  bindings. Existing accepted input, byte publication and recovery owners remain.
- Receive/send listener capabilities default off and require authentication. Shared
  first-party definitions govern advertising/search/dispatch; owned status survives
  disablement. Receive demand uses the existing SDK observation cadence; provider
  outcome, receipt evidence and current spendability remain separate.
- Public unfunded reverse dry quote on 2026-11-07 returned HTTP 201 for Base USDC
  exact input to the public shielded-only test UA, with recipient echoed unchanged.
  Private captures: `target/direct-swap-contracts/direct-receive-dry.json` and
  `target/direct-swap-contracts/direct-receive-response.json`. Pinned bridge source at
  `d1361ac1c68fa579ed1a498d30f2124bc16e09cc`, `zcash.rs:378–450`, separately
  implements shielded withdrawals through `get_orchard_raw_with_memo`
  (capture SHA256 `3adb83481ffee7f5d4edb09a3cffd97ad64c2da7d78c35f2bcbe1bfb04614578`); its builder
  selects Ironwood after activation. This supports the intended shielded receive
  contract despite the general guide's transparent-only statement; it does not
  qualify funded delivery. No silent transparent fallback is selected.
- Fresh reviewer `design_review` identified the identity, jobless preparation,
  receive observation and on-demand dispatch consumers above. Resolution review
  approved the decisions without residual material design blockers. Baseline
  complexity: `target/complexity-runs/20261107T004039Z-02626c44`, complete 471 files
  and 10,390 callable spaces against `b708a06b`, using the pinned analyzer.

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

### Direct send limit amendment

The user additionally requested an optional listener toolset limit, clarified as
preferably total ZEC debit with no new fee preference. Implement
`swap_control.max_send_zec`: omission or `"none"` means unlimited; a decimal ZEC
amount caps each direct native send or swap's principal plus actual network fee.
This is per-operation authority for this path only, never a daily/cumulative
budget and never applied to managed funding, shielding or other ZEC use. Persist
the admission's total-debit authority; reject a principal already above it and
check actual SDK proposal cost before accepting inputs. Do not substitute principal
plus the configured maximum fee for actual cost, which would reject valid work.
The existing treasury network-fee safeguard remains, with no new fee setting.
Native exact amount fixes its principal; foreign exchanges retain the caller's
separate maximum quoted principal. Accepted work keeps its original authority
through capability/configuration changes.

### D1N — Native shielded receive and independent treasury capability

User scope addition, independently designed by fresh read-only `receive_review`:

- Source asset ZEC on chain `zec` selects native receive, without metadata/quote/
  status calls to NEAR or a foreign refund address. Each new caller request ID gets
  a fresh all-shielded SDK receiver. Identical retries preserve the original; an
  address-less native reservation safely resumes derivation after restart.
- Add an explicit compatible route distinction to direct records rather than fake
  provider quotes. Persist exact terms and snapshot/address binding before address
  disclosure. Native receipt/status derives from actual SDK note recipients and
  history, confirmations and spendability; partial funds are never called paid.
- Separate treasury sync/submission/fee capability from optional NEAR transport and
  quote settings. Native paths require no `[funding]` table or NEAR credentials.
  Reuse the existing observation defaults/cadence without new policy or limits.
- Accept supported Sapling receivers and shielded-only UAs; reject transparent/TEX
  and mixed UAs with transparent fallback. Bridge Orchard-specific refund rules
  remain separate. Preserve disabled transparent discovery across open/reconstruct/
  scanning and assert no transparent address RPCs.
- Test fresh/idempotent receivers, reservation-before-derivation restart, no NEAR
  calls, native partial/excess receipts, historical receipt versus spendability,
  table/listener disablement and no Base allocation or ZEC spend.
- Native send is completed in D2: exact native amount already fixes principal;
  foreign source-debit ceiling remains required only for exchange uncertainty.
  There is no provider deadline for native sends. Reuse optional durable facts and
  SDK consensus expiry without a huge substitute duration. Generalize actual
  outgoing Sapling/Orchard/Ironwood recipient/value and fee validation, including
  owned change/self-payments; do not trust UA memo metadata as output proof.

G1 now covers D0/D1/D1N together. It remains intermediate and sending must follow.

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
sends under the D0 authority contract, plus native shielded Zcash sends without
NEAR, using the D1N design above. Persist accepted swap bindings before source
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

Independently review the full D0–D2 change including D1N, every accumulated commit/fixup, complete
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
| D0 | Fresh read-only `design_review` traced financial, network, Store, SDK and MCP consumers; material findings resolved by the decisions above. No residual design blocker. Shielded reverse format/code evidence only; no funded qualification. |
| D1 / G1 | D1 `bae7aaa1`; fresh read-only `receive_review` examined exact D0/D1 commits and combined call chains. Two material findings: direct settings forced strict startup instead of scoped degradation; second-resolution timestamps could retire receive scan demand too early. Fixup in progress preserves Store-only status and direct-without-Base availability, and uses existing scan-start snapshot revision ordering. Production compiler stopped on newly test-only `search_catalog` dead-code warning; fix is `cfg(test)`. No runtime/Clippy pass claimed yet. |
| G1 continued corrections | `963fc840` and `aecc4932` reviewed with no remaining issue in those fixes. `aecc4932` production compiler/all-target `zcash-regtest` Clippy passed (logs `target/direct-swap-g1-clippy-continuation.log`). Later design review identified table-removal recovery gap and Store-commit notification ownership gap; correction preserves history-driven status/background work and gives durable quote publication its own existing-style Notify. Exact fixup `943dad95` reviewed with no remaining ownership finding; production compiler/all-target Clippy and fixed compatibility stage passed (`target/direct-swap-g1-clippy-history.log`, `target/direct-swap-g1-compat.log`). Search consumer correction and focused runtime validation remain pending. |
| Focused baseline receive run | `target/direct-swap-receive-focused.log`: 64 selected tests compiled under `zcash-testutils`; 61 passed and one child helper ignored before one startup assertion failure and one fixture teardown hang. Failure: late listener inventory completeness gate missed status-only table despite early validation allowing it. Hang: fixture waited for owner shutdown while retaining external Store handles. Both focused corrections prepared; no complete runtime pass claimed. macOS linker emitted the documented large `__eh_frame` diagnostic; unwind data/profile retained. |
| Native scope design | User added native shielded send/receive during D1 validation. `receive_review` independently traced pinned SDK actual outgoing notes, fee calculation, receiver discovery and expiry contracts. D1N above records accepted design and tests; D1N authored and awaiting exact-SHA combined review, complexity and compiler/runtime validation. |
| D1N review and correction | Exact `7ee18e10` and combined receive branch reviewed fresh/read-only by `native_receive_review`. One material finding: public completed native receipt froze spendability even after newer SDK snapshots. Correction refreshes public status locally from available SDK, marks stale fallback, guards reverse-order receipt saves using existing revisions, and restores incoming observation after a known reorg. Root recurrence audit also found per-row decode errors could stop shared SDK/direct owners; independent reviewer approved scoped typed decoder, retained rows/flags and continued valid siblings (`4e37ee68`, fixup to D1). Public SDK-snapshot spend/reorg regression and corrupt-row isolation tests added; no consensus claim. Native compile probe passed without diagnostics at `7ee18e10` (`target/direct-swap-native-receive-compile.log`), later fixes require revalidation. |
| Direct-send cap design | Fresh `receive_review` approved optional actual total-debit cap, persisted admission authority, actual proposal/publication checks and no unrelated ZEC enforcement. User confirmed omission/"none" unlimited and no new fee preference. Source spending implementation remains D2. |
| G1 preserved search contract | Root and fresh reviewer found mixed first-party/API search ranked rendered observed-price descriptions and projected complete schemas before pagination. Correction `3ab5dddc` uses borrowed candidates, canonical fallback ranking and page-only rendering; existing pricing-cache regression is extended. Exact fixup/combined code review accepted; compiler/all-target Clippy passed (`target/direct-swap-g1-clippy-search.log`), complete source-matched complexity at `target/complexity-runs/20261107T010723Z-de23df7c`; only previously reviewed serve warning remains. This is a material G1 consumer correction, not deferred. |
| G1 complexity | `target/complexity-runs/20261107T004651Z-840b8ac0`: complete 477 current / 471 baseline files, no exclusions; reviewer verified every source hash and lockfile. New warnings: `initialize_wallets_mode` Cog 40→43 / Cyc 46→54 (reassess after startup correction); `RunningDeployment::serve` Cog 30→33 / Cyc 43→46 retains coherent independent worker/actor shutdown ownership; score-only extraction rejected. |
| G1 accepted | Fresh `native_receive_review` accepted exact combined branch through `d2c84799`, including every prior commit/fixup; no material finding or compelling simplification remains. Compiler and all-target `zcash-regtest` Clippy passed (`target/direct-swap-native-receive-clippy.log`); later description-literal/test-field correction was reviewed and recompiled by focused continuation. Focused unique selection: 74 passed, one isolated child helper ignored; initial73pass/onefixturefailure plus bothaffectedcontinuationpasses (`target/direct-swap-native-receive-focused*.log`). Existing compatibility31passes remain valid exceptaffectedMCP/search consumed by new focusedtests. SDKprivacy, nativefresh/idempotent/restart/partial/excess/spent/reorg, per-row failure isolation and publicstatus freshness execute nonzero tests. Documented macOS linker unwind diagnostic retained, no test failure. Productioncomplexity from816ed993 complete/source-matched with retained dispositions. Receive remains intermediate. |
| D2 / G2 | In progress after G1. Both native and foreign sends, total-debit authority, exact-byte recovery and finalcombinedreview required. |
| D3 | Pending final implementation/user review and documentation handoff. |

## External references to recheck

- [1Click quote](https://docs.near-intents.org/api-reference/oneclick/request-a-swap-quote),
  [swap types](https://docs.near-intents.org/integration/distribution-channels/1click-api/swap-types)
  and [status](https://docs.near-intents.org/api-reference/oneclick/check-swap-execution-status).
- [Supported assets](https://docs.near-intents.org/resources/asset-support),
  [chains/address restrictions](https://docs.near-intents.org/resources/chain-support)
  and [origin-chain deposit flow](https://docs.near-intents.org/integration/distribution-channels/1click-api/quickstart/making-a-request).
- Existing [shielded refund evidence and limitations](../zingo-integration.md#refund-receivers-and-seed-recovery).
