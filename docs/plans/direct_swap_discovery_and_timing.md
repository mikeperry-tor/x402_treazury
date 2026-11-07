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
| P2 — Receive timing | Common create/retry/status presentation returns foreign deposit instructions, exact amount/memo, refund-start and actual address expiry/cold time with current countdowns. Native has no provider expiry. Resolve requested-window behavior separately; no invented protocol maximum. If caller choice is used, optional RFC3339 refund_deadline preserves omitted historical terms, validates new calls before SDK/provider effects and never invalidates identical old retries after elapsed time. Update owning guides and concise schemas. Test backend-free status, differing deadlines, zero boundaries, malformed optional data, original identity and absence of replacement/payment side effects. |
| P3 — SDK fee previews | Add a short serialized treasury query using the existing Ironwood-only proposal helper and SDK total_fee, discarding its result. Return advisory fee/total or a clear unavailable reason; expose limit inspection for native and foreign previews. No source reservation, address derivation, proof, snapshot write, sync or submission. Resolve whether foreign estimates use an explicit standard-transparent-output assumption. Any fee-ceiling policy change must have clear user scope, preserve already accepted numeric authority and avoid huge sentinel substitutes; pending policy decision must remain explicit. |
| G1 — Combined gate | Independently review every exact commit/fixup and all accumulated search/receive/preview/actor/funding consumers. Review source-matched complexity for substantive Rust changes, production compiler/all-target Clippy, compatibility where affected, cheap changed contracts then focused financial/actor tests. Reuse valid unchanged evidence and continue failed named stages; no live settlement claim. Resolve material findings and compelling simplifications. |
| P4 — Handoff and cleanup | Record final reviewed scope/evidence in this plan; maintained contracts belong in configuration, wallet rotation, SDK integration, agent-source and testing guides with relevant AGENTS references. Remove completed plan only after G1 and independently review exact cleanup and final branch. Preserve unsquashed history and private evidence. |

## Review and validation evidence

| Scope | State |
| --- | --- |
| Design | Independent search and receive timing reviews accepted above. Fee origin/SDK audit complete; fee policy and exact preview integration design remain under review. No implementation checks claimed. |
| Implementation | Not started. |
| Live evidence | Read-only toolbox asset listing confirmed 216 entries and actual field inventory. No quote, address, payment, private wallet inspection or funded experiment was performed. |

The existing [deferred timing assessment](deferred/near_swap_timing_tightening.md)
does not become a prerequisite or a broader settlement-research project. Historical
quotes retain their exact deadlines and unresolved liabilities.
