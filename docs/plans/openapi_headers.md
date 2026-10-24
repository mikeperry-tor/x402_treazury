# OpenAPI headers and invocation idempotency

## Intent and baseline

Expose declared OpenAPI business headers as tool arguments and own a fresh
`Idempotency-Key` for each invocation that declares it. Enable VoIPStore number
purchases using a caller-supplied quote ID. Keep prepaid top-ups excluded by
explicit operator preference to avoid persistent balance linkability and excess
spending. Starting revision: `1d23d4d657e55cc5cdca07a3c46eefb2097b6b8e`, plus
the reviewed, uncommitted read-only VoIPStore provider addition. Preserve unrelated
untracked user files.

Currently catalog conversion skips all header parameters and the payment path
builds one request before its existing attempt traversal. Keep that request
ownership, cancellation/drain behavior, payment authority and financial journals.
No additional replay, cross-call deduplication, restart dispatch, recovery ledger,
network constructor, deadline or resource cap is authorized by this feature.
Raw-call arbitrary headers are outside scope.

Owning guides: [architecture](../architecture.md), [configuration](../configuration.md),
[agent sources](../agent-sources.md), [network policy](../network-egress.md),
[wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[provider policy](../../providers/README.md#provider-policy),
[provider caveats](../../providers/CAVEATS.md), [testing](../testing.md) and
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline).

## Ordered commit boundaries and evidence

1. **H0 — baseline and plan:** retain the existing VoIPStore read tools and reviewed
   required-header exclusions, bundle registration, captured boundary witnesses
   and this plan. Evidence: 27 provider regressions and formatting passed; independent
   design and integration review completed. Review the exact committed SHA.
2. **H1 — generic routing and idempotency:** serializable header routing metadata,
   case-insensitive header inheritance, faithful simple serialization, required
   argument checks, header name/value validation and reserved-header protection.
   Generate one UUID for a declared compatible idempotency header; hide that managed
   header from caller arguments. Reject unsupported key/serialization contracts
   visibly. Preserve existing payment/retry/relay ownership. Update owning guides
   with the same commit. Evidence: focused conversion/routing/serde, real payment
   challenge/signing/insufficient-funds and cancellation/uncertainty tests; relay
   preservation and nonrepresentability checks; static and on-demand call paths.
   Use socket-capable execution for socket fixtures, frozen sources and sequential
   builds. Run formatting and focused Clippy.
3. **H2 — VoIPStore purchases:** remove purchase exclusion, exclude the top-up
   namespace with privacy/spend rationale (including future methods/children), document catalog → quote → purchase →
   status and advertised prices. Adapt the captured header-boundary regression to
   assert quote header routing and automatic key ownership with no top-up route.
   Keep supported reads and future compatible children. Evidence: provider tests,
   bundle/offline validation, request previews without buying anything.
4. **H3 — completion cleanup:** verify enduring contracts are in the guides above,
   verify existing AGENTS.md links reach their owning guides, then remove this plan.
   Review the exact cleanup SHA and final combined branch; retain no plan archive.

## Independent gates and completion

Read-only design review before H1 challenges necessity and opportunities to remove
responsibilities. Resolve material findings before affected implementation. A
separate consumer audit checks persisted ToolSpec metadata, reserved headers,
relay disclosure and provider declarations. No competing edits or builds.

After every implementation commit and every `git commit --fixup=<introducing-SHA>`,
an independent reviewer checks the exact SHA, intent, baseline, relevant consumers
and test evidence. Keep fixups unsquashed. After H1, the request-ownership gate
reviews all prior commits/fixups and complete catalog → routing → payment → retry
and relay chains. Compare CogC/CycC against H0 using the pinned analyzer; inspect
changed functions and unchanged high-risk coordinators, consider concrete simpler
representations, and disposition relevant warnings. Generated metrics belong in
ignored target output.

After H2, the integration gate includes all earlier commits/fixups, provider
publication, static/dynamic invocation, payment/credential policy and documentation.
Review recurring fixup causes and assumptions for wider audits, simplification or
missing regressions. Material changes reopen affected gates and validation. Exit
requires resolved material findings, executed focused checks with nonzero test
counts, reviewed complexity results and no claims of live-funded qualification.
H3 receives exact-SHA and final integration review after plan removal as well.
