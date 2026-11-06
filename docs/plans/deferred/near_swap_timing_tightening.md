# Deferred NEAR swap timing and recovery assessment

## Intent, scope and baseline

Assess whether a demonstrated delayed transition can be improved without orphaning
accepted financial work. This is deferred assessment work selected by the operator;
it is not a prerequisite for the implemented NU7 cohort/dependency updates,
consensus-aware polling, SDK expiry recovery, 1,800-second quote preference or
disabled transparent refund/address-discovery policy. The known local polling
delays were removed. No measurement establishes that the common path is optimal,
and answers to service questions alone do not demonstrate a common-path speedup.
They may explain provider waits or support a narrowly justified future change.

The handoff code baseline is `d97b23b9d7473d5acee29baf36965e21c46b4f04` on
`nu7_release_readiness`. Capture a fresh exact baseline and current deployed sources
when scheduling this work. Follow [planning workflow](../../planning-workflow.md),
[wallet observation](../../wallet-rotation.md#wallet-observation-policy),
[reconciliation](../../wallet-rotation.md#recovery-and-reconciliation-policy),
[network policy](../../network-egress.md), [resource limits](../../local-resource-limits.md),
[timer inventory](../../network-egress.md#remaining-timers-and-budgets) and
[testing](../../testing.md#validation-order-and-continuation).

## Questions and why an answer could matter

Retain these as candidate questions, not a request to obtain every answer.
First determine whether established SDK/blockchain facts or proven nondisclosure
already resolve the liability. Reporting differences and unsigned service
observations remain advisory.

1. **Deployed ZEC route and confirmations.** Does production 1Click native ZEC
   use `zcash-connector.bridge.near`, and which watcher, relayer and additional
   confirmation rules apply? Public connector settings alone do not establish the
   service's policy. This could explain settlement delay or support timing estimates.
   It is not needed to add another local confirmation wait: SDK facts already own
   local spendability/source closure. No new wait is proposed.
2. **First-observation event and clock.** Does arrival mean mempool observation,
   block inclusion, sufficient confirmations, bridge credit or service observation?
   Which timestamp/clock is compared to which deadline, and is deposit-submit
   notification advisory? This matters if we intend to predict on-time acceptance
   or change behavior near a cutoff. Existing accepted work can remain owned and
   observed without resolving that clock; elapsed time alone grants no replay.
3. **Requested versus returned deadline and late/reorg behavior.** What happens
   when a deposit is first seen/mined before the requested deadline but confirmed
   later, arrives between requested and returned deadlines, arrives after both,
   or is recognized late after watcher downtime/reorg? Which fields establish swap,
   refund or manual recovery? These facts could support a stronger resolution-time
   promise or a changed cutoff policy. They do not invalidate the implemented
   1,800-second preference, rewrite old quotes or replace signed SDK block expiry.
4. **Automatic ZEC refund fees, minimums and transaction evidence.** For
   underpayment, surplus, late arrival or failure, what are the shielded-refund
   rules; is `refundFee` complete or estimated, is `refundedAmount` gross or net,
   and how are refund transactions bound to deposits? Distinguish swap refunds
   after bridge credit from rescue of an unfinalized connector UTXO. Answers could
   explain the retained amount discrepancy or qualify a controlled refund trigger.
   They are not needed to accept actual SDK-owned funds as spendable, and advisory
   accounting disagreement must not become another funding gate.
5. **Unused `PENDING_DEPOSIT` requests.** The original question asks whether an
   unfunded request can remain pending after both deadlines and whether terminal /
   cancel / tombstone evidence excludes later processing. **This is unnecessary
   for proven nondisclosure.** Existing `source_never_disclosed`,
   `non_disclosed_reset` and `status_needed` predicates exclude those attempts from
   bridge obligations; private PREPARED bytes with zero disclosures can close
   through signed expiry without status I/O. `funding_retry_allowed` already permits
   a new attempt after canonical source closure while provider outcome is unknown.
   Narrow any remaining question to previously disclosed originals and the exact
   delivery/retirement guarantee being sought. Canonical expiry does not exclude
   bridge ingestion before an earlier reorg or off-chain delivery already started.
   Current `destination_obligation_pending` preserves that uncertainty for
   retirement. This does not prevent new funding progress or prove that retaining
   signer eligibility is the only safe design: status inventory already runs
   independently of wallet role and retirement preserves encrypted keys. Any
   separation of delivery observation from signer eligibility needs focused
   financial-policy design and acceptance work; no such change is selected here.
6. **Second deposits and completed-quote reuse.** What happens to another deposit
   before execution and after SUCCESS/REFUNDED for exact-output/input requests;
   which refund binding and history attribution apply? This mainly informs the
   potential duplicate-deposit qualification trigger and any deliberate reuse.
   Ordinary operation does not need authority to make a second deposit, and no
   such funded experiment is authorized. Prefer a simpler documented excess-refund
   trigger if it satisfies the actual qualification question.


## Ordered assessment and acceptance

| ID | Work and exit evidence |
| --- | --- |
| T1 — Identify the actual delay | Classify source confirmation/expiry, provider delivery, destination credit or signer retirement. Trace the complete blocking predicate and ordinary versus exceptional cases using public fixtures or already authorized evidence. Identify the operator outcome sought. Exit with a concrete candidate improvement and required facts, or an evidence-backed decision that no further change is justified. Do not presume every question needs an answer. |
| T2 — Resolve only necessary facts | Prefer current authoritative public sources and local call-chain facts. Select only questions needed for T1's proposed behavior. Preserve uncertainty about the deployed watcher rather than substitute a connector setting or fixture timeout for the service contract. A provider question requires a concrete reviewed payload and explicit sending authority. Exit with attributable facts or explicit missing evidence; absence of an answer is not a local EXPIRED outcome. |
| T3 — Select and qualify a change, if justified | Independently review the design before implementation and revise this plan into coherent commits with owning guidance and acceptance evidence. Preserve original identities, accepted inputs, signed bytes, liabilities and independent funding progress. Review exact SHAs/fixups, affected financial/network chains and applicable source-matched complexity. Run required compiler/Clippy and affected tests in maintained order. Exit with a supported scoped change or a reasoned non-action decision, not a promised speedup without evidence. |

Question4 may explain the retained amount discrepancy, and question6 may inform a
controlled refund experiment, only if T1 selects such an operator outcome. Neither
is a requirement to add a qualification project. Existing live runbooks own
execution/evidence procedures; any private/funded work requires a separate reviewed
recipe and explicit authority. Complete SDK transparent retirement is not selected
work: the implemented privacy contract is shielded refunds without transparent-
address discovery RPCs, protected by real-SDK negative-query regressions. Local key
derivation and incidental owned-output recognition do not require removal.

No external answer is currently required for implemented polling, quote preference,
SDK source closure or transparent discovery suppression. Proven nondisclosure
already excludes a bridge obligation. For previously disclosed work, canonical
expiry cannot exclude earlier bridge ingestion before reorg or off-chain delivery
already started. The existing status inventory is independent of wallet role and
retirement preserves encrypted keys; assess those actual contracts before deciding
whether observation and signer eligibility must remain coupled. Do not implement
that financial-policy change merely by moving this plan.

## Authority, handoff and completion

No public question has been posted. A prepared issue draft remains local under
ignored `target/nu7-readiness/n4-public-contract/`; it is not sending authority.
The earlier offline selected-wallet inspection was executed; the separately
approved single Tor status GET was executed and its one-request authority is
consumed. This plan grants no further private query, key access, sync/rescan,
signing, funded experiment, deployment or submission authority. Keep existing
financial captures private and unchanged. Fixtures do not qualify live settlement,
real Tor or a guaranteed sub-hour result.

Current behavior belongs in wallet/network/configuration guides. Do not add
confirmation waits, replay authority, budget caps or whole-work deadlines merely
to make uncertainty disappear. A new dependency fork requires the
[explicit informed approval](../../development.md#dependency-vendoring) described
in AGENTS.md. Once this assessment and any selected implementation pass independent
combined review, retain the enduring decisions in their owning guides, remove the
completed plan and independently review cleanup. Preserve fixup history until the
operator's selected integration/squash step; documentation cleanup never deletes
financial evidence.
