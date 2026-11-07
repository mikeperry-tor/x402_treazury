# Direct swap discovery, fee previews and receive timing

## Intent, baseline and authority

Continue the user's direct-swap review on `direct_swap_control`, starting at
`818aab3bc2a88eba5995f156f1ed0edefcc506cd`. Improve multi-word asset discovery,
make preview's limit/fee information truthful and clear, and expose receive
instructions with provider expiry/refund timing. The user also questioned the
historical network-fee ceiling and requested SDK advisory fee information.

Preserve the user's modified `examples/deployments/research_desk.toml`, untracked
`.vscode/` and `TODO`, operator state and private qualification evidence. This work
does not authorize paid transfers, quote/address creation for live testing,
deployment/restart, publishing or merging.

Follow [planning workflow](../planning-workflow.md), [independent reviews](../../AGENTS.md#independent-subagent-workflows),
[validation order](../testing.md#validation-order-and-continuation),
[tool documentation](../agent-sources.md#tool-documentation),
[wallet reconciliation](../wallet-rotation.md#recovery-and-reconciliation-policy),
[Ironwood policy](../zingo-integration.md#ironwood-only-policy) and
[network policy](../network-egress.md). Keep exact accepted terms/authority,
signed bytes, owner isolation, status-only recovery, disabled-admission semantics
and native no-NEAR behavior. No new dependency, cache, queue cap or work deadline.

## Established facts and design gate

- Live asset metadata has symbols, chains, IDs, contract addresses and CoinGecko
  identifiers but no descriptions/names. These support queries such as `base
  USDC` and `bitcoin`. Shared `relevance::rank` provides case-insensitive literal
  substring priority, then OR-term BM25 and stable-key ties; it does not promise
  exact-symbol equality priority, invented chain aliases or camel-case splitting.
  Independent `asset_search_review` approved reuse over stable identity strings,
  excluding price/timestamps/JSON keys and preserving blank enumeration.
- `treasury.max_funding_transaction_fee_zec` defaults to 0.0003 ZEC, introduced by
  `82d38baa`; this is an application rejection ceiling, not an SDK fee quote.
  No numerical derivation was found. Independent `fee_audit` verified that an
  unsigned Ironwood-only SDK proposal can provide an advisory fee without
  locking inputs, storing a proposal, signing or publishing. The stock SDK
  preview wrapper uses unrestricted pool selection and must not be used.
- A fee preview may be unavailable without spendable inputs or scan/anchor data,
  and it waits behind existing accepted SDK work on the actor. It must not clear
  a pending proposal or imply that advisory inputs are reserved. Foreign dry
  quotes have no actual deposit destination; any output-shape estimate must be
  explicit and remain incapable of signing or becoming accepted terms.
- Current official [request model](https://raw.githubusercontent.com/defuse-protocol/one-click-sdk-typescript/main/src/models/QuoteRequest.ts)
  requires a deadline but documents no maximum/default. It describes requested
  refund-start time. The separate [quote model](https://raw.githubusercontent.com/defuse-protocol/one-click-sdk-typescript/main/src/models/Quote.ts)
  describes address expiry (`deadline`) and cold-address time (`timeWhenInactive`).
  [Status](https://raw.githubusercontent.com/defuse-protocol/one-click-sdk-typescript/main/src/models/GetExecutionStatusResponse.ts)
  returns the original quote through `quoteResponse`; no countdown is supplied.
- Independent `receive_deadline_review` approved deriving countdowns from saved
  immutable quotes in common status presentation, without new I/O or persistence.
  Unknown optional metadata remains unknown and cannot block raw saved status.
  Zero remaining time never proves refund, terminal expiry, nonpayment or replay
  authority and must not stop accepted observation.
- Historical two-hour requests matched inspected Zodl iOS/Android behavior at
  `cccf3222`; this was not verified as a NEAR maximum. Defaults changed to one
  hour at `02174910` and thirty minutes at `afc6b1d3` on November 6 to request
  earlier refunds, without guaranteeing completion. Do not confuse the request
  preference, returned address expiry and local degraded-health threshold.

## Ordered implementation and acceptance

| Step | Work, consumers and exit evidence |
| --- | --- |
| P0 — Design | Commit this retained plan, complete independent design review of the concrete fee-preview/cap policy and receive-window choice before implementing affected financial decisions. BM25 and saved-status presentation designs are accepted. SDK advisory fees do not themselves authorize removal of managed-funding policy. |
| P1 — Asset search | Reuse shared request-local ranker with stable identity/name metadata, rank before pagination, preserve blank order and completeness. Update query/tool descriptions, direct-swap guide and endpoint/search references. Focused synthetic tests cover multi-word, literal priority, mixed case, stable ties/paging and excluded volatile fields; no live inventory snapshots. |
| P2 — Receive timing | Common create/retry/status presentation returns foreign deposit instructions, exact atomic amount/memo, refund-start and actual address expiry/cold time with current countdowns. Foreign payers must deposit exactly the requested amount and pay network fees separately; native partial-receipt behavior remains. Native has no provider expiry. Use the historical two-hour application receive preference, explicitly not a NEAR maximum, independently of managed/send windows. Optional positive refund_after_seconds gives caller-relative choice without requiring the agent's clock; omit it from serialized terms when absent for historical compatibility. New-request validation precedes SDK/provider effects; identical retries never renew their old deadlines. All presentation timestamps use UTC Z and include current_time_utc sampled once with corresponding countdowns; no timezone/geolocation inference. Test backend-free status, differing deadlines, zero boundaries, offsets, malformed optional data, exact payer instructions, original identity and absence of replacement/payment side effects. |
| P3 — SDK fee previews | Add a short serialized treasury query using the existing Ironwood-only proposal helper and SDK total_fee, discarding its result. Return advisory fee/total or a clear unavailable reason; expose limit inspection for native and foreign previews. No source reservation, address derivation, proof, snapshot write, sync or submission. Resolve whether foreign estimates use an explicit standard-transparent-output assumption. Any fee-ceiling policy change must have clear user scope, preserve already accepted numeric authority and avoid huge sentinel substitutes; pending policy decision must remain explicit. |
| G1 — Combined gate | Independently review every exact commit/fixup and all accumulated search/receive/preview/actor/funding consumers. Review source-matched complexity for substantive Rust changes, production compiler/all-target Clippy, compatibility where affected, cheap changed contracts then focused financial/actor tests. Reuse valid unchanged evidence and continue failed named stages; no live settlement claim. Resolve material findings and compelling simplifications. |
| P4 — Handoff and cleanup | Record final reviewed scope/evidence in this plan; maintained contracts belong in configuration, wallet rotation, SDK integration, agent-source and testing guides with relevant AGENTS references. Remove completed plan only after G1 and independently review exact cleanup and final branch. Preserve unsquashed history and private evidence. |

## Review and validation evidence

| Scope | State |
| --- | --- |
| Design | Exact plan eb560df8 independently accepted by asset_search_review. Search projection and native name Zcash approved. Receive reviewer accepted relative window/two-hour application preference, UTC clock/counters, original terms preservation. Exact-input guidance is an additional user requirement. Fee reviewer verified the unsigned SDK path; receive reviewer independently accepted typed private foreign output-shape estimates, discarded proposals, advisory labeling and existing actor queue priority. Fee-cap policy is intentionally retained: the user questioned global removal but did not direct it, and advisory preview work alone does not authorize removing managed-funding authority. Global fee-policy removal is outside this implemented scope, not unfinished preview work. |
| P1 | Exact c44bea0f independently reviewed; core ranking/pagination/financial isolation accepted. Focused selection passed11 (`target/direct-swap-followup-search-tests.log`). Reviewer found canonical description accidentally dropped ZEC keyword (tool discovery indexes canonical descriptions, not parameter prose) and a test unused_mut warning. Restore Zcash (ZEC), extend the actual tool-discovery regression, remove mut and complete agent-source search reference. Exact e60cce3f fixup independently accepted; subsequent direct50 and discovery2 tests and final compiler/lint checks cover these consumers. |
| P2 | Exact 958d30b4 independently reviewed; common payer instructions/timing and relative receive preference implemented with six focused tests. Early window validation precedes side effects; actual window starts at quote creation after SDK/assets waits. Exact user requirement appears in descriptions and returned instruction sentence, including memo, one deposit and separately paid network fees. Raw quotes and native partial semantics preserved. Combined reviewer found explicit extreme-duration retry revalidation and missing status Zcash discovery keyword; fixup resolves through new-only reservation validation and actual status-only search regression. User clarified no foreign address reuse vs reusable native addresses, now explicit in instructions, schemas and guides. Cheap initial contracts13 passed; unused alias removed. Exact c40403c3 fixup independently accepted; all 50 direct-swap tests passed, fmt-check passed. Final combined c404 review accepted all accumulated commits/fixups and call chains with no material findings. Exact b7db543e discovery continuation and final combined source review accepted; affected tests and validation completed as recorded below. |
| P3 | Exact d78a9e56 independently reviewed with no material findings in SDK core, actor, full preview consumers, tests and guides. Uses private standard transparent-output shape for foreign fees, typed unavailable reasons, principal-plus-fee estimate and advisory limit booleans. Existing fee ceilings unchanged. Production compiler and all-target Clippy passed at c404 and final b7db543e; real SDK fee feature tests passed3, focused compatibility passed16. A recurring-cause audit also identified preexisting missing preview ZEC keyword; exact b7db543e fixup adds it and exercises send-only ZEC discovery. Independently accepted without findings; all five descriptions checked for Zcash/ZEC discovery. |
| Complexity | Source-matched c404 report `target/complexity-runs/20261107T060627Z-1c052bd6`: complete 486 files/10734 spaces, zero exclusions or comparison warnings vs 818aab3b. Independent reviewer inspected full affected chains and moved receive logic. Unchanged preparation hotspot CogC18/CycC53 retains accepted ownership/preflight/journal/calculation ordering; preview never enters it. No unnecessary extraction justified. New receive10/17, instructions5/7, wallet_fee5/21; actor macro branches manually reviewed. Final source-matched b7db543e report `target/complexity-runs/20261107T061315Z-53694fe4` is complete with the same counts/no exclusions/no warnings. Independent reviewer verified all nine affected Rust source hashes and carried unchanged hotspot conclusions forward. |
| Final validation | Frozen Rust source b7db543e: production compiler/all-target zcash-regtest Clippy passed with clean diagnostics (`target/direct-swap-followup-clippy-final.log`, 41.67s/4m27). Changed discovery2 tests passed (`target/direct-swap-followup-discovery-continuation.log`); synthetic zcash-testutils fee3 passed (`target/direct-swap-followup-sdk-fees.log`). Focused SDK3/MCP11/network-audit2 compatibility passed (`target/direct-swap-followup-compat.log`). Earlier direct50 (`target/direct-swap-followup-direct-tests.log`) and cheap13 contracts remain applicable apart from the replaced discovery assertions, which were rerun. Test links emit the existing macOS large __eh_frame performance warning; final source and compiler/Clippy have no warnings. No full make check or live settlement qualification is claimed. |
| G1 | Independent asset_search_review inspected exact accumulated commits/fixups, full affected call chains, complexity and actual final logs through b7db543e; gate accepted with no unresolved material findings or compelling simplifications. Receive reviewer independently accepted exact P2/fixup and P3 core/consumers. New-only validation resolves the original retry edge; actual toolbox regression coverage resolves repeated description keyword omissions. Source review and validation complete; default-feature `make build-dev` passed (1m49, `target/direct-swap-followup-build.log`), refreshing the development executable. Final fmt-check passed (`target/direct-swap-followup-fmt-final.log`). Only P4 plan deletion/exact cleanup review remain. No restart, publish or push is authorized by these checks. |
| Live evidence | Read-only toolbox asset listing confirmed 216 entries and actual field inventory. No quote, address, payment, private wallet inspection or funded experiment was performed. |

The existing [deferred timing assessment](deferred/near_swap_timing_tightening.md)
does not become a prerequisite or a broader settlement-research project. Historical
quotes retain their exact deadlines and unresolved liabilities.

## Completion handoff

The implementation and G1 are complete on `direct_swap_control`, with final Rust
source `b7db543e030a11f746e8c51f8c844d1b2907cbd5`. Enduring contracts live in
configuration, wallet rotation, SDK integration and agent-source guides; existing
AGENTS direct-swap/Ironwood references lead to them. No material finding or
implementation portion is deferred. The existing fee ceiling remains intentionally
unchanged; advisory SDK fees now explain actual estimated cost independently.
The separate deferred timing assessment remains unchanged and is not a completion
prerequisite. Remove this plan in P4 and independently review the exact cleanup
SHA and final combined branch, preserving all unsquashed commits and fixups.

The operator's `research_desk.toml` edits and untracked `.vscode/`/`TODO` remain
untouched. Private wallet state, secrets and qualification artifacts were not
loaded or modified. The development executable is refreshed; the running MCP
service was not restarted. Offline evidence does not qualify paid settlement.
